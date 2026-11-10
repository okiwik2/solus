//! Downloads manager (replaces the Edge download flyout).

use std::cell::RefCell;

use rusqlite::params;
use serde_json::{json, Value};
use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::*;
use windows::core::PWSTR;

use crate::app::{defer, App};
use crate::bridge::{ok, Ctx, Reply};
use crate::util::{self, now_ms, WStr};

pub struct Download {
    pub id: u32,
    pub op: Option<ICoreWebView2DownloadOperation>,
    pub profile: String,
    pub incognito: bool,
    pub tab: u32,
    pub url: String,
    pub path: String,
    pub mime: String,
    pub total: i64,
    pub received: i64,
    pub state: &'static str,
    pub paused: bool,
    pub reason: String,
    pub started: i64,
    pub ended: i64,
}

impl Download {
    fn json(&self) -> Value {
        json!({"id": self.id, "url": self.url, "path": self.path, "mime": self.mime, "total": self.total,
            "received": self.received, "state": self.state, "paused": self.paused, "reason": self.reason,
            "started": self.started, "ended": self.ended})
    }
}

thread_local! {
    static DOWNLOADS: RefCell<Vec<Download>> = const { RefCell::new(Vec::new()) };
    static NEXT: RefCell<u32> = const { RefCell::new(0) };
    static LAST_PUSH: RefCell<i64> = const { RefCell::new(0) };
}

pub fn default_dir(app: &mut App, profile: &str) -> std::path::PathBuf {
    let custom = app.settings(profile).download_dir.clone();
    if !custom.is_empty() && std::path::Path::new(&custom).is_dir() {
        return custom.into();
    }
    unsafe {
        if let Ok(p) = windows::Win32::UI::Shell::SHGetKnownFolderPath(
            &windows::Win32::UI::Shell::FOLDERID_Downloads,
            windows::Win32::UI::Shell::KNOWN_FOLDER_FLAG(0),
            None,
        ) {
            let s = p.to_string().unwrap_or_default();
            windows::Win32::System::Com::CoTaskMemFree(Some(p.0 as _));
            if !s.is_empty() {
                return s.into();
            }
        }
    }
    std::env::var("USERPROFILE").map(|u| std::path::PathBuf::from(u).join("Downloads")).unwrap_or_default()
}

fn unique_path(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
    let clean: String = name.chars().map(|c| if r#"<>:"/\|?*"#.contains(c) || c.is_control() { '_' } else { c }).collect();
    let clean = if clean.trim().is_empty() { "download".to_string() } else { clean };
    let p = dir.join(&clean);
    if !p.exists() {
        return p;
    }
    let (stem, ext) = match clean.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (clean.clone(), String::new()),
    };
    for i in 1..1000 {
        let c = dir.join(format!("{stem} ({i}){ext}"));
        if !c.exists() {
            return c;
        }
    }
    p
}

pub fn list(profile: &str, incognito: bool) -> Vec<Value> {
    DOWNLOADS.with(|d| d.borrow().iter().filter(|x| x.profile == profile && x.incognito == incognito).map(|x| x.json()).collect())
}

fn push(app: &mut App, profile: &str, force: bool) {
    let now = now_ms();
    if !force && LAST_PUSH.with(|l| now - *l.borrow() < 250) {
        return;
    }
    LAST_PUSH.with(|l| *l.borrow_mut() = now);
    let wins: Vec<(u32, bool)> = app.windows.iter().filter(|w| w.profile == profile).map(|w| (w.id, w.incognito)).collect();
    for (w, inc) in wins {
        crate::bridge::emit(app, w, "downloads", json!(list(profile, inc)));
    }
    let l = list(profile, false);
    crate::bridge::emit_pages(app, profile, "downloads", json!(l));
    // keep tabs with running downloads loaded
    let mut per_tab: std::collections::HashMap<u32, u32> = Default::default();
    DOWNLOADS.with(|d| {
        for x in d.borrow().iter().filter(|x| x.state == "in_progress") {
            *per_tab.entry(x.tab).or_default() += 1;
        }
    });
    DOWNLOADS.with(|d| {
        for x in d.borrow().iter() {
            crate::perf::set_tab_downloads(x.tab, per_tab.get(&x.tab).copied().unwrap_or(0));
        }
    });
}

/// Load past downloads from the database on start-up.
pub fn load_history(app: &mut App, profile: &str) {
    let Some(db) = app.profile(profile).db() else { return };
    let Ok(mut st) = db.conn.prepare("SELECT id,url,path,mime,total,received,state,started,ended FROM downloads ORDER BY started DESC LIMIT 100") else { return };
    let rows: Vec<Download> = st
        .query_map([], |r| {
            Ok(Download {
                id: 0,
                op: None,
                profile: profile.to_string(),
                incognito: false,
                tab: 0,
                url: r.get(1)?,
                path: r.get(2)?,
                mime: r.get(3)?,
                total: r.get(4)?,
                received: r.get(5)?,
                state: match r.get::<_, String>(6)?.as_str() {
                    "completed" => "completed",
                    "cancelled" => "cancelled",
                    _ => "interrupted",
                },
                paused: false,
                reason: String::new(),
                started: r.get(7)?,
                ended: r.get(8)?,
            })
        })
        .map(|r| r.flatten().collect())
        .unwrap_or_default();
    DOWNLOADS.with(|d| {
        let mut d = d.borrow_mut();
        for mut x in rows.into_iter().rev() {
            x.id = NEXT.with(|n| {
                *n.borrow_mut() += 1;
                *n.borrow()
            });
            d.push(x);
        }
    });
}

fn persist(app: &mut App, id: u32) {
    let Some((profile, url, path, mime, total, received, state, started, ended, inc)) = DOWNLOADS.with(|d| {
        d.borrow().iter().find(|x| x.id == id).map(|x| (x.profile.clone(), x.url.clone(), x.path.clone(), x.mime.clone(), x.total, x.received, x.state, x.started, x.ended, x.incognito))
    }) else {
        return;
    };
    if inc {
        return;
    }
    if let Some(db) = app.profile(&profile).db() {
        let _ = db.conn.execute(
            "INSERT INTO downloads(url,path,mime,total,received,state,started,ended) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![url, path, mime, total, received, state, started, ended],
        );
    }
}

fn read_state(op: &ICoreWebView2DownloadOperation) -> (i64, i64, &'static str, String, String) {
    unsafe {
        let mut total = 0i64;
        let mut rec = 0i64;
        let _ = op.TotalBytesToReceive(&mut total);
        let _ = op.BytesReceived(&mut rec);
        let mut st = COREWEBVIEW2_DOWNLOAD_STATE_IN_PROGRESS;
        let _ = op.State(&mut st);
        let state = match st {
            COREWEBVIEW2_DOWNLOAD_STATE_COMPLETED => "completed",
            COREWEBVIEW2_DOWNLOAD_STATE_INTERRUPTED => "interrupted",
            _ => "in_progress",
        };
        let mut reason = String::new();
        if state == "interrupted" {
            let mut r = COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_NONE;
            let _ = op.InterruptReason(&mut r);
            reason = interrupt_text(r).to_string();
        }
        let mut p = PWSTR::null();
        let path = if op.ResultFilePath(&mut p).is_ok() { take_pwstr(p) } else { String::new() };
        (total, rec, state, reason, path)
    }
}

fn interrupt_text(r: COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON) -> &'static str {
    match r {
        COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_USER_CANCELED => "cancelled",
        COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_FILE_NO_SPACE => "disk full",
        COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_FILE_ACCESS_DENIED => "access denied",
        COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_FILE_BLOCKED_BY_POLICY => "blocked by policy",
        COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_FILE_TOO_LARGE => "file too large",
        COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_SERVER_FORBIDDEN => "the server refused the download",
        COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_USER_SHUTDOWN => "Solus was closed",
        COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_FILE_SECURITY_CHECK_FAILED => "virus scan failed",
        COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_FILE_MALICIOUS => "blocked as dangerous (SmartScreen)",
        COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_NETWORK_FAILED | COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_NETWORK_DISCONNECTED => "network error",
        COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_NETWORK_TIMEOUT => "network timeout",
        COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_SERVER_FAILED | COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_SERVER_NO_RANGE => "server error",
        COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_SERVER_UNAUTHORIZED => "not authorised",
        COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_SERVER_BAD_CONTENT => "file not found on server",
        COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_DOWNLOAD_PROCESS_CRASHED => "browser crashed",
        _ => "failed",
    }
}

/// Register the DownloadStarting handler on a content webview.
pub fn install(wv: &ICoreWebView2, wid: u32, tid: u32) {
    unsafe {
        let Ok(w4) = windows::core::Interface::cast::<ICoreWebView2_4>(wv) else { return };
        let h = DownloadStartingEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else { return Ok(()) };
            let op = args.DownloadOperation()?;
            // our own downloads UI replaces the default flyout
            args.SetHandled(true)?;
            let deferral = args.GetDeferral()?;
            let mut p = PWSTR::null();
            let _ = args.ResultFilePath(&mut p);
            let suggested = take_pwstr(p);
            let mut p = PWSTR::null();
            let _ = op.Uri(&mut p);
            let url = take_pwstr(p);
            let mut p = PWSTR::null();
            let _ = op.MimeType(&mut p);
            let mime = take_pwstr(p);
            defer(move |app| start(app, wid, tid, args, deferral, op, url, mime, suggested));
            Ok(())
        }));
        let mut tok = Default::default();
        let _ = w4.add_DownloadStarting(&h, &mut tok);
    }
}

fn start(
    app: &mut App,
    wid: u32,
    tid: u32,
    args: ICoreWebView2DownloadStartingEventArgs,
    deferral: ICoreWebView2Deferral,
    op: ICoreWebView2DownloadOperation,
    url: String,
    mime: String,
    suggested: String,
) {
    let Some(w) = app.win(wid) else {
        unsafe {
            let _ = deferral.Complete();
        }
        return;
    };
    let (profile, incognito) = (w.profile.clone(), w.incognito);
    let s = app.settings(&profile).clone();
    let name = std::path::Path::new(&suggested).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "download".into());
    let dir = default_dir(app, &profile);
    let mut target = unique_path(&dir, &name);
    if s.download_ask {
        match crate::platform::save_file_dialog(app, wid, &name, "All files|*.*") {
            Some(p) => target = p.into(),
            None => unsafe {
                let _ = args.SetCancel(true);
                let _ = deferral.Complete();
                return;
            },
        }
    }
    let t = WStr::path(&target);
    unsafe {
        let _ = args.SetResultFilePath(t.pcwstr());
    }
    let id = NEXT.with(|n| {
        *n.borrow_mut() += 1;
        *n.borrow()
    });
    DOWNLOADS.with(|d| {
        d.borrow_mut().push(Download {
            id,
            op: Some(op.clone()),
            profile: profile.clone(),
            incognito,
            tab: tid,
            url: url.clone(),
            path: target.to_string_lossy().to_string(),
            mime,
            total: 0,
            received: 0,
            state: "in_progress",
            paused: false,
            reason: String::new(),
            started: now_ms(),
            ended: 0,
        });
    });
    unsafe {
        let o2 = op.clone();
        let h = BytesReceivedChangedEventHandler::create(Box::new(move |_, _| {
            let st = read_state(&o2);
            defer(move |app| update(app, id, st, false));
            Ok(())
        }));
        let mut tok = Default::default();
        let _ = op.add_BytesReceivedChanged(&h, &mut tok);
        let o3 = op.clone();
        let h = StateChangedEventHandler::create(Box::new(move |_, _| {
            let st = read_state(&o3);
            defer(move |app| update(app, id, st, true));
            Ok(())
        }));
        let _ = op.add_StateChanged(&h, &mut tok);
        let _ = deferral.Complete();
    }
    logf!("download started: {}", util::redact(&url));
    push(app, &profile, true);
    crate::bridge::emit(app, wid, "panel", json!({"panel": "downloads"}));
}

fn update(app: &mut App, id: u32, st: (i64, i64, &'static str, String, String), state_change: bool) {
    let (total, rec, state, reason, path) = st;
    let mut finished = false;
    let mut profile = String::new();
    DOWNLOADS.with(|d| {
        if let Some(x) = d.borrow_mut().iter_mut().find(|x| x.id == id) {
            x.total = total.max(0);
            x.received = rec;
            if !path.is_empty() {
                x.path = path.clone();
            }
            if x.state != state {
                finished = state != "in_progress";
                x.state = if state == "interrupted" && reason == "cancelled" { "cancelled" } else { state };
                x.reason = reason.clone();
                if finished {
                    x.ended = now_ms();
                    x.paused = false;
                }
            }
            profile = x.profile.clone();
        }
    });
    if finished {
        persist(app, id);
    }
    if !profile.is_empty() {
        push(app, &profile, state_change || finished);
    }
}

pub fn cmd(app: &mut App, ctx: &Ctx, cmd: &str, a: &Value) -> Reply {
    let (profile, incognito) = app.win(ctx.wid).map(|w| (w.profile.clone(), w.incognito)).unwrap_or_default();
    match cmd {
        "downloads.list" => Reply::Now(json!(list(&profile, incognito))),
        "downloads.openFolder" => {
            let d = default_dir(app, &profile);
            crate::platform::open_path(&d);
            ok()
        }
        "downloads.clear" => {
            DOWNLOADS.with(|d| d.borrow_mut().retain(|x| !(x.profile == profile && x.state != "in_progress")));
            if let Some(db) = app.profile(&profile).db() {
                let _ = db.conn.execute("DELETE FROM downloads", []);
            }
            push(app, &profile, true);
            ok()
        }
        "downloads.action" => {
            let id = a["id"].as_u64().unwrap_or(0) as u32;
            let action = a["action"].as_str().unwrap_or("");
            let info = DOWNLOADS.with(|d| d.borrow().iter().find(|x| x.id == id).map(|x| (x.op.clone(), x.path.clone(), x.url.clone())));
            let Some((op, path, url)) = info else { return Reply::Err("no such download".into()) };
            unsafe {
                match action {
                    "open" => {
                        let p = std::path::Path::new(&path);
                        if p.exists() {
                            crate::platform::open_path(p);
                        } else {
                            return Reply::Err("The file has been moved or deleted".into());
                        }
                    }
                    "show" => crate::platform::reveal(std::path::Path::new(&path)),
                    "pause" => {
                        if let Some(o) = &op {
                            let _ = o.Pause();
                        }
                        DOWNLOADS.with(|d| d.borrow_mut().iter_mut().filter(|x| x.id == id).for_each(|x| x.paused = true));
                    }
                    "resume" => {
                        if let Some(o) = &op {
                            let _ = o.Resume();
                        }
                        DOWNLOADS.with(|d| d.borrow_mut().iter_mut().filter(|x| x.id == id).for_each(|x| x.paused = false));
                    }
                    "cancel" => {
                        if let Some(o) = &op {
                            let _ = o.Cancel();
                        }
                    }
                    "retry" => {
                        if let Some(v) = app.win(ctx.wid).and_then(|w| w.active_tab()).and_then(|t| t.view.as_ref()) {
                            crate::webview::navigate(&v.wv, &url);
                        }
                    }
                    "remove" => DOWNLOADS.with(|d| d.borrow_mut().retain(|x| x.id != id || x.state == "in_progress")),
                    _ => return Reply::Err("unknown action".into()),
                }
            }
            push(app, &profile, true);
            ok()
        }
        _ => Reply::Err("unknown downloads command".into()),
    }
}

/// Number of downloads still running (used before quitting).
pub fn active_count() -> usize {
    DOWNLOADS.with(|d| d.borrow().iter().filter(|x| x.state == "in_progress").count())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unique_names() {
        let dir = std::env::temp_dir().join(format!("solus-dl-{}", util::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = unique_path(&dir, "report.pdf");
        std::fs::write(&a, b"x").unwrap();
        let b = unique_path(&dir, "report.pdf");
        assert!(b.ends_with("report (1).pdf"));
        assert!(unique_path(&dir, "a:b?.txt").ends_with("a_b_.txt"));
    }
}
