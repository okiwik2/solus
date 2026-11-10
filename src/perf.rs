//! Performance manager — Solus' low-memory engine.
//!
//! * Background tabs are first hinted to Chromium as low-priority (MemoryUsageTargetLevel=Low),
//!   then frozen (renderer suspended) and finally unloaded (webview destroyed, tab kept) according
//!   to the active performance profile.
//! * Under system memory pressure the least recently used tabs are unloaded early.
//! * On battery the Efficiency profile is used automatically (configurable).
//! * Predictive preloading is conservative: only from a fresh new-tab page, only for frequently
//!   visited sites, only when memory is plentiful, and the preloaded page is dropped after 20 s.

use std::cell::RefCell;
use std::collections::HashMap;

use serde_json::{json, Value};
use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::*;
use windows::core::{Interface, BOOL, PWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows::Win32::System::ProcessStatus::*;
use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows::Win32::System::Threading::*;

use crate::app::{defer, App};
use crate::bridge::{ok, Ctx, Reply};
use crate::settings::SuspendRules;
use crate::util::{self, WStr};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    pub low_target_after_s: i64,
    pub freeze_after_s: i64,
    pub discard_after_s: i64, // 0 = only under memory pressure
    pub pressure_load: u32,   // % system memory load that triggers early unloading
    pub preload: bool,
}

pub fn params_for(profile: &str) -> Params {
    match profile {
        "performance" => Params { low_target_after_s: 600, freeze_after_s: 30 * 60, discard_after_s: 0, pressure_load: 92, preload: true },
        "efficiency" => Params { low_target_after_s: 0, freeze_after_s: 60, discard_after_s: 10 * 60, pressure_load: 75, preload: false },
        _ => Params { low_target_after_s: 30, freeze_after_s: 5 * 60, discard_after_s: 45 * 60, pressure_load: 85, preload: true },
    }
}

pub fn on_battery() -> (bool, bool) {
    unsafe {
        let mut s = SYSTEM_POWER_STATUS::default();
        if GetSystemPowerStatus(&mut s).is_ok() {
            // ACLineStatus 0 = battery; SystemStatusFlag 1 = Windows energy saver on
            return (s.ACLineStatus == 0, s.SystemStatusFlag == 1);
        }
    }
    (false, false)
}

/// The profile actually in force (battery may override the user's choice).
pub fn effective_profile(app: &App) -> String {
    let (batt, saver) = on_battery();
    if app.local.auto_battery && (batt || saver) {
        "efficiency".into()
    } else {
        app.local.perf_profile.clone()
    }
}

pub fn effective_params(app: &App) -> Params {
    let mut p = params_for(&effective_profile(app));
    if app.local.custom_rules {
        let r: &SuspendRules = &app.local.rules;
        p.freeze_after_s = r.freeze_after_min as i64 * 60;
        p.discard_after_s = r.discard_after_min as i64 * 60;
    }
    if !app.local.preload {
        p.preload = false;
    }
    p
}

pub fn system_memory() -> (u32, u64, u64) {
    unsafe {
        let mut m = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
        if GlobalMemoryStatusEx(&mut m).is_ok() {
            return (m.dwMemoryLoad, m.ullAvailPhys, m.ullTotalPhys);
        }
    }
    (0, 0, 0)
}

thread_local! {
    static DOWNLOAD_TABS: RefCell<HashMap<u32, u32>> = RefCell::new(HashMap::new()); // tab -> active downloads
}

pub fn set_tab_downloads(tid: u32, n: u32) {
    DOWNLOAD_TABS.with(|m| {
        if n == 0 {
            m.borrow_mut().remove(&tid);
        } else {
            m.borrow_mut().insert(tid, n);
        }
    });
}
fn tab_has_downloads(tid: u32) -> bool {
    DOWNLOAD_TABS.with(|m| m.borrow().get(&tid).copied().unwrap_or(0) > 0)
}

fn exempt(app: &App, t: &crate::tab::Tab) -> bool {
    let r = &app.local.rules;
    if t.audible && r.never_audio {
        return true;
    }
    if t.pinned && r.never_pinned {
        return true;
    }
    if tab_has_downloads(t.id) {
        return true;
    }
    if t.url.starts_with("solus://") && !t.url.starts_with(crate::tab::NEWTAB) {
        return false;
    }
    let site = util::site_of(&t.url);
    !site.is_empty() && r.exceptions.iter().any(|e| util::site_of_host(e.trim()) == site)
}

/// Periodic pass (every 15 s): hint, freeze and unload background tabs.
pub fn tick(app: &mut App) {
    let p = effective_params(app);
    let now = util::now_ms();
    let (load, _, _) = system_memory();
    let pressure = load >= p.pressure_load;
    let mut to_discard: Vec<(u32, u32, i64)> = vec![];
    let mut to_freeze: Vec<(u32, u32)> = vec![];
    for w in app.windows.iter() {
        let visible: Vec<u32> = if w.is_visible() { w.visible_tab_ids() } else { vec![] };
        // the page showing in a minimised window is what the user comes back to: keep it unless memory is short
        let minimised_front: Vec<u32> = if !w.is_visible() && w.hidden_since == 0 && !pressure { w.visible_tab_ids() } else { vec![] };
        for t in w.tabs.iter() {
            if visible.contains(&t.id) || minimised_front.contains(&t.id) || t.view.is_none() || t.creating {
                continue;
            }
            let idle_s = (now - t.last_active) / 1000;
            if let Some(v) = &t.view {
                if idle_s >= p.low_target_after_s {
                    unsafe {
                        if let Ok(w19) = v.wv.cast::<ICoreWebView2_19>() {
                            let _ = w19.SetMemoryUsageTargetLevel(COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW);
                        }
                    }
                }
            }
            if exempt(app, t) {
                continue;
            }
            let discard_due = p.discard_after_s > 0 && idle_s >= p.discard_after_s;
            if discard_due || (pressure && idle_s >= 60) {
                to_discard.push((w.id, t.id, t.last_active));
            } else if !t.frozen && idle_s >= p.freeze_after_s {
                to_freeze.push((w.id, t.id));
            }
        }
        // windows hidden in the tray for a while give back all their memory
        if w.hidden_since > 0 && now - w.hidden_since > app.local.tray_unload_after_min as i64 * 60_000 {
            for t in w.tabs.iter().filter(|t| t.view.is_some() && !t.audible && !tab_has_downloads(t.id)) {
                to_discard.push((w.id, t.id, t.last_active));
            }
        }
    }
    for (wid, tid) in to_freeze {
        freeze(app, wid, tid);
    }
    // least recently used first; under pressure stop once memory recovers
    to_discard.sort_by_key(|x| x.2);
    for (wid, tid, _) in to_discard {
        discard_checked(app, wid, tid);
    }
    unload_hidden_ui(app, now);
    expire_prerender(app, now);
}

pub fn freeze(app: &mut App, wid: u32, tid: u32) {
    let Some(t) = app.win_mut(wid).and_then(|w| w.tab_mut(tid)) else { return };
    let Some(v) = &t.view else { return };
    if t.shown || t.frozen {
        return;
    }
    unsafe {
        if let Ok(w3) = v.wv.cast::<ICoreWebView2_3>() {
            let h = TrySuspendCompletedHandler::create(Box::new(move |hr, ok| {
                if !(hr.is_ok() && ok) {
                    logf!("tab {tid}: engine declined to pause it (hr={hr:?}, ok={ok})");
                }
                if hr.is_ok() && ok {
                    defer(move |app| {
                        if let Some(t) = app.win_mut(wid).and_then(|w| w.tab_mut(tid)) {
                            if !t.shown {
                                t.frozen = true;
                            }
                        }
                        crate::bridge::push_tab(app, wid, tid);
                    });
                }
                Ok(())
            }));
            let _ = w3.TrySuspend(&h);
        }
    }
}

/// Unload a tab unless the page has unsaved form input (checked first when the rule is on).
pub fn discard_checked(app: &mut App, wid: u32, tid: u32) {
    let never_forms = app.local.rules.never_forms;
    let Some(t) = app.win(wid).and_then(|w| w.tab(tid)) else { return };
    let Some(v) = &t.view else { return };
    if !never_forms || t.frozen {
        // a frozen page can't run script; frozen pages were idle long enough that typing is unlikely,
        // and Chromium keeps form state in session history only when navigating, so check before freezing instead
        if never_forms && t.has_input {
            return;
        }
        discard(app, wid, tid);
        return;
    }
    const DIRTY: &str = "(() => { for (const el of document.querySelectorAll('input,textarea,select,[contenteditable=true]')) { \
        if (el.isContentEditable && el.textContent.trim().length > 0 && document.activeElement === el) return true; \
        if (el.type === 'checkbox' || el.type === 'radio') { if (el.checked !== el.defaultChecked) return true; } \
        else if (el.tagName === 'SELECT') { for (const o of el.options) if (o.selected !== o.defaultSelected) return true; } \
        else if ('value' in el && el.type !== 'hidden' && el.type !== 'password' && el.type !== 'search' && el.value !== el.defaultValue) return true; } return false; })()";
    crate::webview::exec_script(&v.wv, DIRTY, move |app, r| {
        let dirty = r.as_deref() == Some("true");
        if let Some(t) = app.win_mut(wid).and_then(|w| w.tab_mut(tid)) {
            t.has_input = dirty;
        }
        if !dirty {
            discard(app, wid, tid);
        }
    });
}

pub fn discard(app: &mut App, wid: u32, tid: u32) {
    let Some(w) = app.win_mut(wid) else { return };
    if w.visible_tab_ids().contains(&tid) && w.is_visible() {
        return;
    }
    let Some(t) = w.tab_mut(tid) else { return };
    if t.view.is_none() {
        return;
    }
    t.drop_view();
    logf!("unloaded tab {tid} to save memory");
    crate::bridge::push_tab(app, wid, tid);
}

fn unload_hidden_ui(app: &mut App, now: i64) {
    let limit = app.local.tray_unload_after_min as i64 * 60_000;
    for w in app.windows.iter_mut() {
        if w.hidden_since > 0 && now - w.hidden_since > limit && w.ui.is_some() && w.tabs.iter().all(|t| t.view.is_none()) {
            if let Some(ui) = w.ui.take() {
                ui.close();
            }
            w.ui_ready = false;
            logf!("window {} unloaded while in the tray", w.id);
        }
    }
}

// ---------------------------------------------------------------- predictive preloading

pub struct Prerender {
    pub url: String,
    pub wid: u32,
    pub tid: u32,
    pub started: i64,
}

thread_local! {
    pub static PRERENDER: RefCell<Option<Prerender>> = const { RefCell::new(None) };
}

/// Called by the omnibox when the user lingers on a suggestion.
pub fn maybe_prerender(app: &mut App, wid: u32, url: &str) {
    let p = effective_params(app);
    let (load, avail, _) = system_memory();
    if !p.preload || load > 70 || avail < 2 * 1024 * 1024 * 1024 {
        return;
    }
    if !url.starts_with("https://") {
        return;
    }
    let Some(w) = app.win(wid) else { return };
    if w.incognito {
        return;
    }
    // only from an empty new tab, so committing never loses back/forward history
    let Some(active) = w.active_tab() else { return };
    if !active.url.starts_with(crate::tab::NEWTAB) {
        return;
    }
    let profile = w.profile.clone();
    let visits = app
        .profile(&profile)
        .db()
        .map(|d| d.history_matches(url.trim_start_matches("https://"), 1).first().map(|m| m.0 == url).unwrap_or(false))
        .unwrap_or(false);
    if !visits {
        return;
    }
    if PRERENDER.with(|p| p.borrow().as_ref().map(|x| x.url == url).unwrap_or(false)) {
        return;
    }
    cancel_prerender(app);
    let tid = app.next_id();
    if let Some(w) = app.win_mut(wid) {
        // a hidden, not-yet-listed tab that loads in the background
        let mut t = crate::tab::Tab::new(tid, url, &w.workspace.clone());
        t.title = String::new();
        t.prerender = true;
        w.tabs.push(t);
    }
    PRERENDER.with(|p| *p.borrow_mut() = Some(Prerender { url: url.to_string(), wid, tid, started: util::now_ms() }));
    app.ensure_tab_view(wid, tid);
    logf!("prerendering {url}");
}

/// If `url` was prerendered for this window, swap it in for the active new-tab page.
pub fn take_prerender(app: &mut App, wid: u32, url: &str) -> bool {
    let hit = PRERENDER.with(|p| p.borrow().as_ref().map(|x| x.url == url && x.wid == wid).unwrap_or(false));
    if !hit {
        return false;
    }
    let pr = PRERENDER.with(|p| p.borrow_mut().take()).unwrap();
    let Some(w) = app.win_mut(wid) else { return false };
    let Some(active) = w.active_tab().map(|t| t.id) else { return false };
    let (Some(ai), Some(pi)) = (w.tab_index(active), w.tab_index(pr.tid)) else { return false };
    let mut t = w.tabs.remove(pi);
    t.prerender = false;
    t.workspace = w.tabs[ai.min(w.tabs.len() - 1)].workspace.clone();
    let ai = w.tab_index(active).unwrap();
    w.tabs.insert(ai + 1, t);
    // the new-tab page is replaced; it is not added to "recently closed"
    let mut old = w.tabs.remove(ai);
    old.drop_view();
    w.active = 0;
    app.activate_tab(wid, pr.tid);
    logf!("prerender hit for {url}");
    true
}

pub fn cancel_prerender(app: &mut App) {
    if let Some(pr) = PRERENDER.with(|p| p.borrow_mut().take()) {
        if let Some(w) = app.win_mut(pr.wid) {
            if let Some(i) = w.tab_index(pr.tid) {
                let mut t = w.tabs.remove(i);
                t.drop_view();
            }
        }
    }
}

fn expire_prerender(app: &mut App, now: i64) {
    let expired = PRERENDER.with(|p| p.borrow().as_ref().map(|x| now - x.started > 20_000).unwrap_or(false));
    if expired {
        cancel_prerender(app);
    }
}

// ---------------------------------------------------------------- task manager data

thread_local! {
    static CPU_PREV: RefCell<HashMap<u32, (u64, i64)>> = RefCell::new(HashMap::new());
}

fn proc_stats(pid: u32) -> Option<(u64, u64, f64)> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut c = PROCESS_MEMORY_COUNTERS_EX2 { cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX2>() as u32, ..Default::default() };
        let ok = GetProcessMemoryInfo(h, &mut c as *mut _ as *mut PROCESS_MEMORY_COUNTERS, c.cb).is_ok();
        let (mut ct, mut et, mut kt, mut ut) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
        let _ = GetProcessTimes(h, &mut ct, &mut et, &mut kt, &mut ut);
        let _ = CloseHandle(h);
        if !ok {
            return None;
        }
        let ft = |f: FILETIME| ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64;
        let cpu_100ns = ft(kt) + ft(ut);
        let now = util::now_ms();
        let pct = CPU_PREV.with(|m| {
            let mut m = m.borrow_mut();
            let r = match m.get(&pid) {
                Some((prev, t)) if now > *t => {
                    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1) as f64;
                    ((cpu_100ns.saturating_sub(*prev)) as f64 / 10_000.0) / ((now - t) as f64) * 100.0 / cores
                }
                _ => 0.0,
            };
            m.insert(pid, (cpu_100ns, now));
            r
        });
        Some((c.PrivateWorkingSetSize as u64, c.PrivateUsage as u64, pct))
    }
}

fn kind_name(k: COREWEBVIEW2_PROCESS_KIND) -> &'static str {
    match k {
        COREWEBVIEW2_PROCESS_KIND_BROWSER => "Browser",
        COREWEBVIEW2_PROCESS_KIND_RENDERER => "Renderer",
        COREWEBVIEW2_PROCESS_KIND_GPU => "GPU",
        COREWEBVIEW2_PROCESS_KIND_UTILITY => "Utility",
        COREWEBVIEW2_PROCESS_KIND_PPAPI_PLUGIN => "Plugin",
        COREWEBVIEW2_PROCESS_KIND_PPAPI_BROKER => "Plugin broker",
        COREWEBVIEW2_PROCESS_KIND_SANDBOX_HELPER => "Sandbox helper",
        _ => "Process",
    }
}

/// Synchronous snapshot using the basic process list (used by automation/benchmarks).
pub fn memory_snapshot(app: &App) -> Value {
    let mut total_ws = 0u64;
    let mut procs = vec![];
    let me = std::process::id();
    if let Some((ws, private, cpu)) = proc_stats(me) {
        total_ws += ws;
        procs.push(json!({"pid": me, "kind": "Solus host", "privateWs": ws, "private": private, "cpu": cpu}));
    }
    if let Some(env) = app.env() {
        unsafe {
            if let Ok(e8) = env.cast::<ICoreWebView2Environment8>() {
                if let Ok(list) = e8.GetProcessInfos() {
                    let mut n = 0u32;
                    let _ = list.Count(&mut n);
                    for i in 0..n {
                        if let Ok(pi) = list.GetValueAtIndex(i) {
                            let mut pid = 0i32;
                            let mut kind = COREWEBVIEW2_PROCESS_KIND_BROWSER;
                            let _ = pi.ProcessId(&mut pid);
                            let _ = pi.Kind(&mut kind);
                            if let Some((ws, private, cpu)) = proc_stats(pid as u32) {
                                total_ws += ws;
                                procs.push(json!({"pid": pid, "kind": kind_name(kind), "privateWs": ws, "private": private, "cpu": cpu}));
                            }
                        }
                    }
                }
            }
        }
    }
    let (load, avail, total) = system_memory();
    json!({"totalPrivateWs": total_ws, "processes": procs, "system": {"load": load, "available": avail, "total": total}})
}

/// Detailed snapshot for the task manager: processes mapped to the tabs they serve.
fn task_manager(app: &mut App, ctx: Ctx, id: Value) {
    let Some(env) = app.env() else {
        crate::bridge::respond(app, &ctx, &id, Ok(memory_snapshot(app)));
        return;
    };
    // main-frame id -> (window, tab)
    let mut frames: HashMap<u32, (u32, u32)> = HashMap::new();
    let mut ui_frames: HashMap<u32, u32> = HashMap::new();
    for w in app.windows.iter() {
        for t in w.tabs.iter() {
            if let Some(v) = &t.view {
                unsafe {
                    if let Ok(w20) = v.wv.cast::<ICoreWebView2_20>() {
                        let mut f = 0u32;
                        if w20.FrameId(&mut f).is_ok() {
                            frames.insert(f, (w.id, t.id));
                        }
                    }
                }
            }
        }
        if let Some(ui) = &w.ui {
            unsafe {
                if let Ok(w20) = ui.wv.cast::<ICoreWebView2_20>() {
                    let mut f = 0u32;
                    if w20.FrameId(&mut f).is_ok() {
                        ui_frames.insert(f, w.id);
                    }
                }
            }
        }
    }
    unsafe {
        let Ok(e13) = env.cast::<ICoreWebView2Environment13>() else {
            crate::bridge::respond(app, &ctx, &id, Ok(memory_snapshot(app)));
            return;
        };
        let h = GetProcessExtendedInfosCompletedHandler::create(Box::new(move |hr, coll| {
            let mut out: Vec<Value> = vec![];
            if let (Ok(()), Some(coll)) = (hr, coll) {
                let mut n = 0u32;
                let _ = coll.Count(&mut n);
                for i in 0..n {
                    let Ok(ext) = coll.GetValueAtIndex(i) else { continue };
                    let Ok(pi) = ext.ProcessInfo() else { continue };
                    let mut pid = 0i32;
                    let mut kind = COREWEBVIEW2_PROCESS_KIND_BROWSER;
                    let _ = pi.ProcessId(&mut pid);
                    let _ = pi.Kind(&mut kind);
                    let mut tabs: Vec<Value> = vec![];
                    let mut ui = false;
                    let mut sources: Vec<String> = vec![];
                    if let Ok(fc) = ext.AssociatedFrameInfos() {
                        if let Ok(it) = fc.GetIterator() {
                            let mut has = BOOL(0);
                            let _ = it.HasCurrent(&mut has);
                            while has.as_bool() {
                                if let Ok(fi) = it.GetCurrent() {
                                    if let Ok(fi2) = fi.cast::<ICoreWebView2FrameInfo2>() {
                                        let mut fid = 0u32;
                                        if fi2.FrameId(&mut fid).is_ok() {
                                            if let Some((w, t)) = frames.get(&fid) {
                                                tabs.push(json!({"win": w, "tab": t}));
                                            }
                                            if ui_frames.contains_key(&fid) {
                                                ui = true;
                                            }
                                        }
                                    }
                                    let mut p = PWSTR::null();
                                    if fi.Source(&mut p).is_ok() {
                                        let s = take_pwstr(p);
                                        if !s.is_empty() && sources.len() < 4 {
                                            sources.push(s);
                                        }
                                    }
                                }
                                let _ = it.MoveNext(&mut has);
                            }
                        }
                    }
                    let (ws, private, cpu) = proc_stats(pid as u32).unwrap_or((0, 0, 0.0));
                    out.push(json!({"pid": pid, "kind": kind_name(kind), "privateWs": ws, "private": private, "cpu": cpu,
                        "tabs": tabs, "ui": ui, "frames": sources}));
                }
            }
            let ctx2 = ctx.clone();
            let id2 = id.clone();
            defer(move |app| {
                let me = std::process::id();
                let (ws, private, cpu) = proc_stats(me).unwrap_or((0, 0, 0.0));
                out.insert(0, json!({"pid": me, "kind": "Solus host", "privateWs": ws, "private": private, "cpu": cpu, "tabs": [], "ui": false}));
                let total: u64 = out.iter().map(|p| p["privateWs"].as_u64().unwrap_or(0)).sum();
                let tabs: Vec<Value> = app
                    .windows
                    .iter()
                    .flat_map(|w| w.tabs.iter().filter(|t| !t.prerender).map(move |t| {
                        let mut j = t.to_json();
                        j["win"] = json!(w.id);
                        j["incognito"] = json!(w.incognito);
                        j
                    }))
                    .collect();
                let (load, avail, tot) = system_memory();
                let (batt, saver) = on_battery();
                let r = json!({"processes": out, "totalPrivateWs": total, "tabs": tabs,
                    "system": {"load": load, "available": avail, "total": tot, "battery": batt, "energySaver": saver},
                    "profile": app.local.perf_profile, "effective": effective_profile(app)});
                crate::bridge::respond(app, &ctx2, &id2, Ok(r));
            });
            Ok(())
        }));
        let _ = e13.GetProcessExtendedInfos(&h);
    }
}

pub fn cmd(app: &mut App, ctx: &Ctx, cmd: &str, args: &Value, id: &Value) -> Reply {
    let tab_arg = |app: &App| -> Option<(u32, u32)> {
        let tid = args["id"].as_u64()? as u32;
        let wid = args["win"].as_u64().map(|x| x as u32).unwrap_or(ctx.wid);
        if app.win(wid).and_then(|w| w.tab(tid)).is_some() {
            return Some((wid, tid));
        }
        app.windows.iter().find(|w| w.tab(tid).is_some()).map(|w| (w.id, tid))
    };
    match cmd {
        "perf.snapshot" => {
            task_manager(app, ctx.clone(), id.clone());
            Reply::Later
        }
        "perf.discard" => {
            if let Some((w, t)) = tab_arg(app) {
                discard(app, w, t);
            }
            ok()
        }
        "perf.freeze" => {
            if let Some((w, t)) = tab_arg(app) {
                freeze(app, w, t);
            }
            ok()
        }
        "perf.activate" => {
            if let Some((w, t)) = tab_arg(app) {
                app.activate_tab(w, t);
                app.show_window(w);
            }
            ok()
        }
        "perf.endProcess" => {
            let pid = args["pid"].as_u64().unwrap_or(0) as u32;
            if pid == 0 || pid == std::process::id() {
                return Reply::Err("cannot end that process".into());
            }
            // only processes that belong to our engine may be ended
            let ours = memory_snapshot(app)["processes"]
                .as_array()
                .map(|a| a.iter().any(|p| p["pid"].as_u64() == Some(pid as u64) && p["kind"] != "Browser"))
                .unwrap_or(false);
            if !ours {
                return Reply::Err("not a Solus page process".into());
            }
            unsafe {
                if let Ok(h) = OpenProcess(PROCESS_TERMINATE, false, pid) {
                    let _ = TerminateProcess(h, 1);
                    let _ = CloseHandle(h);
                }
            }
            ok()
        }
        "perf.status" => {
            let (batt, saver) = on_battery();
            let p = effective_params(app);
            Reply::Now(json!({
                "profile": app.local.perf_profile, "effective": effective_profile(app), "battery": batt, "energySaver": saver,
                "freezeAfterS": p.freeze_after_s, "discardAfterS": p.discard_after_s, "pressureLoad": p.pressure_load, "preload": p.preload,
            }))
        }
        "perf.pip" => {
            // Picture-in-picture needs a user gesture; the DevTools protocol can supply one
            let Some(v) = app.win(ctx.wid).and_then(|w| w.active_tab()).and_then(|t| t.view.as_ref()) else { return ok() };
            let expr = "(async () => { const vids = [...document.querySelectorAll('video')].filter(v => v.readyState > 0 && !v.disablePictureInPicture); \
                if (!vids.length) return 'novideo'; vids.sort((a,b) => (b.clientWidth*b.clientHeight) - (a.clientWidth*a.clientHeight)); \
                if (document.pictureInPictureElement) { await document.exitPictureInPicture(); return 'exited'; } \
                await vids[0].requestPictureInPicture(); return 'ok'; })()";
            let params = json!({"expression": expr, "userGesture": true, "awaitPromise": true}).to_string();
            let m = WStr::new("Runtime.evaluate");
            let p = WStr::new(&params);
            let wid = ctx.wid;
            let h = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(move |_, result| {
                if result.contains("novideo") {
                    defer(move |app| crate::bridge::emit(app, wid, "toast", json!({"text": "There is no video on this page to show in picture-in-picture."})));
                }
                Ok(())
            }));
            unsafe {
                let _ = v.wv.CallDevToolsProtocolMethod(m.pcwstr(), p.pcwstr(), &h);
            }
            ok()
        }
        "perf.prerender" => {
            maybe_prerender(app, ctx.wid, args["url"].as_str().unwrap_or(""));
            ok()
        }
        _ => Reply::Err("unknown perf command".into()),
    }
}

pub fn is_prerender_tab(app: &App, wid: u32, tid: u32) -> bool {
    app.win(wid).and_then(|w| w.tab(tid)).map(|t| t.prerender).unwrap_or(false)
}
