//! Message bridge between the HTML UI / internal pages and the host.
//!
//! Requests: `{id, cmd, args}` -> responses `{re: id, ok: true, data}` / `{re: id, ok: false, error}`.
//! Host events: `{ev, data}`. Messages are only accepted from `solus://` documents; web pages never
//! get a bridge (see NavigationStarting, which disables web messaging for non-internal pages).

use serde_json::{json, Value};
use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::*;
use std::result::Result;
use windows::core::PWSTR;

use crate::app::{defer, App};
use crate::tab::{OpenOpts, NEWTAB};
use crate::util::{self, WStr};
use crate::window::TabGroup;

#[derive(Clone, Debug)]
pub struct Ctx {
    pub wid: u32,
    /// 0 = the browser UI; otherwise the internal page's tab
    pub tid: u32,
    /// internal page name ("settings", "newtab", ...), "chrome" for the UI
    pub page: String,
}

impl Ctx {
    pub fn is_ui(&self) -> bool {
        self.tid == 0
    }
}

pub enum Reply {
    Now(Value),
    Err(String),
    Later,
}

pub fn ok() -> Reply {
    Reply::Now(json!(true))
}

pub fn install_ui(wv: &ICoreWebView2, wid: u32) {
    install(wv, wid, 0);
}

pub fn install_page(wv: &ICoreWebView2, wid: u32, tid: u32) {
    install(wv, wid, tid);
}

fn install(wv: &ICoreWebView2, wid: u32, tid: u32) {
    let h = WebMessageReceivedEventHandler::create(Box::new(move |_, args| {
        let Some(args) = args else { return Ok(()) };
        unsafe {
            let mut p = PWSTR::null();
            let _ = args.Source(&mut p);
            let source = take_pwstr(p);
            if !source.starts_with("solus://") {
                return Ok(()); // never accept messages from web content
            }
            let page = url::Url::parse(&source).ok().and_then(|u| u.host_str().map(|s| s.to_string())).unwrap_or_default();
            if tid == 0 && page != "chrome" {
                return Ok(());
            }
            let mut p = PWSTR::null();
            if args.WebMessageAsJson(&mut p).is_err() {
                return Ok(());
            }
            let text = take_pwstr(p);
            let Ok(msg) = serde_json::from_str::<Value>(&text) else { return Ok(()) };
            let ctx = Ctx { wid, tid, page };
            defer(move |app| on_message(app, ctx, msg));
        }
        Ok(())
    }));
    let mut tok = Default::default();
    unsafe {
        let _ = wv.add_WebMessageReceived(&h, &mut tok);
    }
}

fn on_message(app: &mut App, ctx: Ctx, msg: Value) {
    let id = msg["id"].clone();
    let cmd = msg["cmd"].as_str().unwrap_or("").to_string();
    let args = msg["args"].clone();
    let r = dispatch(app, &ctx, &cmd, &args, &id);
    match r {
        Reply::Now(v) => respond(app, &ctx, &id, Ok(v)),
        Reply::Err(e) => {
            logf!("command {cmd} failed: {}", util::redact(&e));
            respond(app, &ctx, &id, Err(e))
        }
        Reply::Later => {}
    }
}

/// Webview that hosts the caller (the UI or an internal page's tab).
fn target_wv(app: &App, ctx: &Ctx) -> Option<ICoreWebView2> {
    let w = app.win(ctx.wid)?;
    if ctx.tid == 0 {
        w.ui.as_ref().map(|v| v.wv.clone())
    } else {
        let t = w.tab(ctx.tid)?;
        if !t.url.starts_with("solus://") {
            return None;
        }
        t.view.as_ref().map(|v| v.wv.clone())
    }
}

pub fn respond(app: &App, ctx: &Ctx, id: &Value, r: Result<Value, String>) {
    if id.is_null() {
        return;
    }
    // a delayed answer to a command the test-automation channel sent
    if let Some(n) = id["automation"].as_u64() {
        if let Some(done) = AUTOMATION_WAITING.with(|m| m.borrow_mut().remove(&n)) {
            done(match r {
                Ok(v) => json!({"ok": true, "data": v}),
                Err(e) => json!({"ok": false, "error": e}),
            });
        }
        return;
    }
    let Some(wv) = target_wv(app, ctx) else { return };
    let msg = match r {
        Ok(v) => json!({"re": id, "ok": true, "data": v}),
        Err(e) => json!({"re": id, "ok": false, "error": e}),
    };
    crate::webview::post_json(&wv, &msg.to_string());
}

/// Reply to a request later from any code path that kept the ctx + id.
pub fn respond_later(ctx: Ctx, id: Value, r: Result<Value, String>) {
    crate::app::post(move |app| respond(app, &ctx, &id, r));
}

pub fn emit(app: &App, wid: u32, ev: &str, data: Value) {
    let Some(w) = app.win(wid) else { return };
    let Some(ui) = &w.ui else { return };
    if !w.ui_ready && ev != "init" {
        return;
    }
    let msg = json!({"ev": ev, "data": data});
    crate::webview::post_json(&ui.wv, &msg.to_string());
}

/// Send an event to an internal page living in a tab.
pub fn emit_page(app: &App, wid: u32, tid: u32, ev: &str, data: Value) {
    if let Some(t) = app.win(wid).and_then(|w| w.tab(tid)) {
        if t.url.starts_with("solus://") {
            if let Some(v) = &t.view {
                crate::webview::post_json(&v.wv, &json!({"ev": ev, "data": data}).to_string());
            }
        }
    }
}

/// Broadcast to every open internal page of a profile (e.g. settings changed elsewhere).
pub fn emit_pages(app: &App, profile: &str, ev: &str, data: Value) {
    for w in app.windows.iter().filter(|w| w.profile == profile) {
        for t in &w.tabs {
            if t.url.starts_with("solus://") {
                if let Some(v) = &t.view {
                    crate::webview::post_json(&v.wv, &json!({"ev": ev, "data": data}).to_string());
                }
            }
        }
    }
}

pub fn groups_json(w: &crate::window::BrowserWindow) -> Value {
    serde_json::to_value(&w.groups).unwrap_or(json!([]))
}

pub fn restore_groups(w: &mut crate::window::BrowserWindow, v: &Value) {
    if let Ok(g) = serde_json::from_value::<Vec<TabGroup>>(v.clone()) {
        w.groups = g;
    }
}

pub fn push_tabs(app: &App, wid: u32) {
    let Some(w) = app.win(wid) else { return };
    let tabs: Vec<Value> = w.tabs.iter().filter(|t| !t.prerender).map(|t| t.to_json()).collect();
    let data = json!({
        "tabs": tabs,
        "active": w.active,
        "groups": groups_json(w),
        "workspace": w.workspace,
        "split": w.split.as_ref().map(|s| json!({"a": s.a, "b": s.b, "ratio": s.ratio})),
        "closedCount": w.closed.len(),
    });
    emit(app, wid, "tabs", data);
    update_window_title(app, wid);
}

pub fn push_tab(app: &App, wid: u32, tid: u32) {
    let Some(t) = app.win(wid).and_then(|w| w.tab(tid)) else { return };
    if t.prerender {
        return;
    }
    emit(app, wid, "tab", t.to_json());
    if app.win(wid).map(|w| w.active == tid).unwrap_or(false) {
        update_window_title(app, wid);
    }
}

fn update_window_title(app: &App, wid: u32) {
    let Some(w) = app.win(wid) else { return };
    let title = match w.active_tab() {
        Some(t) => {
            let tt = if t.title.is_empty() { crate::tab::display_title(&t.url) } else { t.title.clone() };
            if w.incognito {
                format!("{tt} - Solus (Incognito)")
            } else {
                format!("{tt} - Solus")
            }
        }
        None => "Solus".to_string(),
    };
    let s = WStr::new(&title);
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::SetWindowTextW(w.hwnd, s.pcwstr());
    }
}

pub fn effective_dark(app: &App, settings: &crate::settings::Settings) -> bool {
    match settings.theme.as_str() {
        "dark" => true,
        "light" => false,
        _ => app.system_dark,
    }
}

pub fn theme_json(app: &mut App, profile: &str, incognito: bool) -> Value {
    let s = app.settings(profile).clone();
    let dark = incognito || effective_dark(app, &s);
    json!({
        "dark": dark,
        "incognito": incognito,
        "mode": s.theme,
        "density": s.density,
        "themeId": s.theme_id,
        "customCss": s.custom_css,
        "animations": s.animations,
        "themes": crate::themes::list(app, profile),
    })
}

pub fn push_theme(app: &mut App, wid: u32) {
    let Some(w) = app.win(wid) else { return };
    let (profile, incognito, hwnd) = (w.profile.clone(), w.incognito, w.hwnd);
    let t = theme_json(app, &profile, incognito);
    crate::window::set_window_dark(hwnd, t["dark"].as_bool().unwrap_or(false));
    emit(app, wid, "theme", t.clone());
    emit_pages(app, &profile, "theme", t);
}

fn settings_json(app: &mut App, profile: &str) -> Value {
    let s = app.settings(profile).clone();
    let mut v = serde_json::to_value(&s).unwrap_or(json!({}));
    v["engines"] = json!(crate::settings::search_engines()
        .iter()
        .map(|e| json!({"id": e.0, "name": e.1}))
        .collect::<Vec<_>>());
    v
}

pub fn push_settings(app: &mut App, profile: &str) {
    let v = settings_json(app, profile);
    let ids: Vec<u32> = app.windows.iter().filter(|w| w.profile == profile).map(|w| w.id).collect();
    for id in ids {
        emit(app, id, "settings", v.clone());
    }
    emit_pages(app, profile, "settings", v);
}

fn init_payload(app: &mut App, wid: u32) -> Value {
    let Some(w) = app.win(wid) else { return Value::Null };
    let (profile, incognito) = (w.profile.clone(), w.incognito);
    let pinfo = app.local.profiles.iter().find(|p| p.id == profile).cloned().unwrap_or_default();
    let shortcuts: Vec<Value> = crate::tab::default_shortcuts()
        .iter()
        .map(|(a, k, l)| {
            let cur = app.settings(&profile).shortcuts.get(*a).cloned().unwrap_or_else(|| k.to_string());
            json!({"action": a, "keys": cur, "default": k, "label": l})
        })
        .collect();
    json!({
        "wid": wid,
        "incognito": incognito,
        "version": crate::VERSION,
        "profile": pinfo,
        "profiles": app.local.profiles,
        "settings": settings_json(app, &profile),
        "theme": theme_json(app, &profile, incognito),
        "shortcuts": shortcuts,
        "perf": app.local.perf_profile,
    })
}

/// Open an internal page, reusing an existing tab that already shows it.
pub fn open_internal(app: &mut App, wid: u32, page: &str) {
    let url = if page.contains("://") { page.to_string() } else { format!("solus://{page}") };
    let base: String = url.split('#').next().unwrap_or(&url).trim_end_matches('/').to_string();
    let existing = app.win(wid).and_then(|w| {
        w.tabs.iter().find(|t| t.url.split('#').next().unwrap_or("").trim_end_matches('/') == base).map(|t| t.id)
    });
    match existing {
        Some(tid) => {
            app.activate_tab(wid, tid);
            if url.contains('#') {
                app.navigate_tab(wid, tid, &url);
            }
        }
        None => {
            let (active_is_blank, active) = app
                .win(wid)
                .and_then(|w| w.active_tab().map(|t| (t.url.starts_with(NEWTAB), t.id)))
                .unwrap_or((false, 0));
            if active_is_blank {
                app.navigate_tab(wid, active, &url);
            } else {
                app.open_tab(wid, &url, OpenOpts { activate: true, ..Default::default() });
            }
        }
    }
}

pub fn run_action(app: &mut App, wid: u32, action: &str, data: Value) {
    let Some(w) = app.win(wid) else { return };
    let active = w.active;
    let profile = w.profile.clone();
    let incognito = w.incognito;
    match action {
        "newTab" => {
            app.open_tab(wid, NEWTAB, OpenOpts { activate: true, ..Default::default() });
        }
        "closeTab" => app.close_tab(wid, active),
        "reopenTab" => app.reopen_closed(wid),
        "newWindow" => {
            app.new_window(&profile, false, vec![], false);
        }
        "newIncognito" => {
            app.new_window(&profile, true, vec![], false);
        }
        "nextTab" => app.cycle_tab(wid, 1),
        "prevTab" => app.cycle_tab(wid, -1),
        "back" | "forward" | "reload" | "hardReload" | "stop" | "devtools" | "print" | "saveAs" | "zoomIn" | "zoomOut"
        | "zoomReset" => app.tab_action(wid, active, action),
        "home" => {
            let home = app.settings(&profile).homepage.clone();
            let url = if home.is_empty() { NEWTAB.to_string() } else { home };
            app.navigate_tab(wid, active, &url);
        }
        "fullscreen" => {
            if let Some(w) = app.win_mut(wid) {
                let on = !w.html_fullscreen;
                w.set_html_fullscreen(on);
            }
        }
        "history" => open_internal(app, wid, "history"),
        "downloads" => emit(app, wid, "panel", json!({"panel": "downloads"})),
        "bookmarksManager" => open_internal(app, wid, "bookmarks"),
        "settings" => open_internal(app, wid, "settings"),
        "taskManager" => open_internal(app, wid, "taskmanager"),
        "privacy" => open_internal(app, wid, "privacy"),
        "clearData" => open_internal(app, wid, "settings/#clear"),
        "splitView" => toggle_split(app, wid, data["with"].as_u64().map(|x| x as u32)),
        "find" | "findNext" | "findPrev" | "focusOmnibox" | "search" | "bookmark" | "tabSearch" | "aiSidebar" | "aiPopup"
        | "sidebar" | "notes" => emit(app, wid, "action", json!({"action": action, "data": data})),
        a if a.starts_with("tab") && a.len() == 4 => {
            if let Some(n) = a[3..].parse::<usize>().ok() {
                app.select_tab_n(wid, n);
            }
        }
        a if a.starts_with("workspace") => {
            if let Ok(n) = a[9..].parse::<usize>() {
                emit(app, wid, "action", json!({"action": "workspace", "data": {"n": n}}));
            }
        }
        _ => {
            let _ = incognito;
            logf!("unknown action {action}");
        }
    }
}

pub fn toggle_split(app: &mut App, wid: u32, with: Option<u32>) {
    let Some(w) = app.win_mut(wid) else { return };
    if w.split.is_some() && with.is_none() {
        w.split = None;
    } else {
        let a = w.active;
        let ws = w.workspace.clone();
        let b = with.filter(|b| *b != a && w.tab(*b).is_some()).or_else(|| {
            // pick the most recently used other tab in this workspace
            w.tabs.iter().filter(|t| t.id != a && t.workspace == ws).max_by_key(|t| t.last_active).map(|t| t.id)
        });
        match b {
            Some(b) => w.split = Some(crate::window::Split { a, b, ratio: 0.5 }),
            None => {
                let _ = w;
                let nt = app.open_tab(wid, NEWTAB, OpenOpts { activate: false, ..Default::default() });
                if let Some(w) = app.win_mut(wid) {
                    w.split = Some(crate::window::Split { a, b: nt, ratio: 0.5 });
                }
            }
        }
    }
    if let Some(w) = app.win_mut(wid) {
        w.session_dirty = true;
        w.layout();
    }
    app.ensure_window_views(wid);
    push_tabs(app, wid);
}

/// Called after every completed navigation of a tab.
pub fn page_loaded(app: &mut App, wid: u32, tid: u32, ok: bool) {
    crate::hooks::page_loaded(app, wid, tid, ok);
}

fn dispatch(app: &mut App, ctx: &Ctx, cmd: &str, args: &Value, id: &Value) -> Reply {
    // UI-only commands
    if ctx.is_ui() {
        match cmd {
            "ready" => {
                if let Some(w) = app.win_mut(ctx.wid) {
                    w.ui_ready = true;
                }
                let init = init_payload(app, ctx.wid);
                emit(app, ctx.wid, "init", init.clone());
                push_tabs(app, ctx.wid);
                return Reply::Now(init);
            }
            "layout" => {
                if let Some(w) = app.win_mut(ctx.wid) {
                    let i = &args["insets"];
                    w.insets = crate::window::Insets {
                        top: i["top"].as_f64().unwrap_or(w.insets.top),
                        left: i["left"].as_f64().unwrap_or(0.0),
                        right: i["right"].as_f64().unwrap_or(0.0),
                        bottom: i["bottom"].as_f64().unwrap_or(0.0),
                    };
                    w.overlays = args["overlays"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|r| {
                                    let r = r.as_array()?;
                                    Some([r.first()?.as_f64()?, r.get(1)?.as_f64()?, r.get(2)?.as_f64()?, r.get(3)?.as_f64()?])
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    if let Some(h) = args["captionH"].as_f64() {
                        crate::caption::with_state(w.hwnd, |s| s.height_css = h);
                    }
                    if let Some(r) = args["splitRatio"].as_f64() {
                        if let Some(s) = w.split.as_mut() {
                            s.ratio = r.clamp(0.15, 0.85);
                        }
                    }
                    w.layout();
                }
                app.ensure_window_views(ctx.wid);
                return ok();
            }
            "colors" => {
                if let Some(w) = app.win(ctx.wid) {
                    let hwnd = w.hwnd;
                    crate::caption::with_state(hwnd, |s| {
                        if let Some(c) = crate::caption::parse_color(args["bg"].as_str().unwrap_or("")) {
                            s.bg = c;
                        }
                        if let Some(c) = crate::caption::parse_color(args["fg"].as_str().unwrap_or("")) {
                            s.fg = c;
                        }
                        if let Some(c) = crate::caption::parse_color(args["hover"].as_str().unwrap_or("")) {
                            s.hover_bg = c;
                        }
                        if let Some(c) = crate::caption::parse_color(args["inactive"].as_str().unwrap_or("")) {
                            s.inactive_fg = c;
                        }
                    });
                    unsafe {
                        let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, false);
                    }
                }
                return ok();
            }
            "window" => {
                app.window_command(ctx.wid, args["action"].as_str().unwrap_or(""));
                return ok();
            }
            "focusContent" => {
                app.focus_content(ctx.wid);
                return ok();
            }
            _ => {}
        }
    }
    match cmd {
        "action" => {
            run_action(app, ctx.wid, args["name"].as_str().unwrap_or(""), args["data"].clone());
            ok()
        }
        "newTab" => {
            let url = args["url"].as_str().unwrap_or(NEWTAB).to_string();
            let profile = app.win(ctx.wid).map(|w| w.profile.clone()).unwrap_or_default();
            let url = crate::tab::fixup_input(app, &profile, &url);
            let opener = if ctx.tid != 0 { ctx.tid } else { args["opener"].as_u64().unwrap_or(0) as u32 };
            let id = app.open_tab(
                ctx.wid,
                &url,
                OpenOpts {
                    activate: !args["background"].as_bool().unwrap_or(false),
                    opener,
                    group: args["group"].as_str().unwrap_or("").to_string(),
                    index: args["index"].as_u64().map(|x| x as usize),
                    ..Default::default()
                },
            );
            Reply::Now(json!(id))
        }
        "activate" => {
            app.activate_tab(ctx.wid, args["id"].as_u64().unwrap_or(0) as u32);
            ok()
        }
        "closeTab" => {
            let tid = args["id"].as_u64().map(|x| x as u32).unwrap_or(ctx.tid);
            app.close_tab(ctx.wid, tid);
            ok()
        }
        "closeTabs" => {
            for t in args["ids"].as_array().cloned().unwrap_or_default() {
                if let Some(t) = t.as_u64() {
                    app.close_tab(ctx.wid, t as u32);
                }
            }
            ok()
        }
        "navigate" => {
            let tid = args["id"].as_u64().map(|x| x as u32).unwrap_or_else(|| {
                if ctx.tid != 0 {
                    ctx.tid
                } else {
                    app.win(ctx.wid).map(|w| w.active).unwrap_or(0)
                }
            });
            let input = args["input"].as_str().unwrap_or("").to_string();
            if args["newTab"].as_bool().unwrap_or(false) {
                let profile = app.win(ctx.wid).map(|w| w.profile.clone()).unwrap_or_default();
                let url = crate::tab::fixup_input(app, &profile, &input);
                app.open_tab(ctx.wid, &url, OpenOpts { activate: !args["background"].as_bool().unwrap_or(false), opener: tid, ..Default::default() });
            } else {
                app.navigate_tab(ctx.wid, tid, &input);
                app.focus_content(ctx.wid);
            }
            ok()
        }
        "tabAction" => {
            let tid = args["id"].as_u64().map(|x| x as u32).unwrap_or_else(|| app.win(ctx.wid).map(|w| w.active).unwrap_or(0));
            app.tab_action(ctx.wid, tid, args["action"].as_str().unwrap_or(""));
            ok()
        }
        "moveTab" => {
            app.move_tab(ctx.wid, args["id"].as_u64().unwrap_or(0) as u32, args["to"].as_u64().unwrap_or(0) as usize);
            ok()
        }
        "pinTab" => {
            let tid = args["id"].as_u64().unwrap_or(0) as u32;
            let pin = args["pinned"].as_bool().unwrap_or(true);
            if let Some(w) = app.win_mut(ctx.wid) {
                if let Some(i) = w.tab_index(tid) {
                    let mut t = w.tabs.remove(i);
                    t.pinned = pin;
                    // pinned tabs live at the front
                    let pos = w.tabs.iter().take_while(|x| x.pinned).count();
                    w.tabs.insert(pos, t);
                    w.session_dirty = true;
                }
            }
            push_tabs(app, ctx.wid);
            ok()
        }
        "duplicateTab" => {
            let tid = args["id"].as_u64().unwrap_or(0) as u32;
            if let Some(t) = app.win(ctx.wid).and_then(|w| w.tab(tid)) {
                let (url, group) = (t.url.clone(), t.group.clone());
                let idx = app.win(ctx.wid).and_then(|w| w.tab_index(tid)).map(|i| i + 1);
                app.open_tab(ctx.wid, &url, OpenOpts { activate: true, index: idx, group, ..Default::default() });
            }
            ok()
        }
        "reopenClosed" => {
            app.reopen_closed(ctx.wid);
            ok()
        }
        "closedTabs" => {
            let list: Vec<Value> = app
                .win(ctx.wid)
                .map(|w| {
                    w.closed.iter().rev().map(|c| json!({"url": c.url, "title": c.title, "favicon": c.favicon, "closedAt": c.closed_at})).collect()
                })
                .unwrap_or_default();
            Reply::Now(json!(list))
        }
        "reopenClosedAt" => {
            let i = args["index"].as_u64().unwrap_or(0) as usize;
            if let Some(w) = app.win_mut(ctx.wid) {
                let n = w.closed.len();
                if i < n {
                    let c = w.closed.remove(n - 1 - i);
                    w.closed.push(c);
                }
            }
            app.reopen_closed(ctx.wid);
            ok()
        }
        "split" => {
            toggle_split(app, ctx.wid, args["with"].as_u64().map(|x| x as u32));
            ok()
        }
        "unsplit" => {
            if let Some(w) = app.win_mut(ctx.wid) {
                w.split = None;
                w.layout();
            }
            push_tabs(app, ctx.wid);
            ok()
        }
        "swapSplit" => {
            if let Some(w) = app.win_mut(ctx.wid) {
                if let Some(s) = w.split.as_mut() {
                    std::mem::swap(&mut s.a, &mut s.b);
                }
                w.layout();
            }
            push_tabs(app, ctx.wid);
            ok()
        }
        "shortcuts.list" => {
            let profile = app.win(ctx.wid).map(|w| w.profile.clone()).unwrap_or_default();
            let list: Vec<Value> = crate::tab::default_shortcuts()
                .iter()
                .map(|(a, k, l)| {
                    let cur = app.settings(&profile).shortcuts.get(*a).cloned().unwrap_or_else(|| k.to_string());
                    json!({"action": a, "keys": cur, "default": k, "label": l})
                })
                .collect();
            Reply::Now(json!(list))
        }
        "getTheme" => {
            let (profile, incognito) = app.win(ctx.wid).map(|w| (w.profile.clone(), w.incognito)).unwrap_or_default();
            Reply::Now(theme_json(app, &profile, incognito))
        }
        "fetchText" => {
            // used by new-tab widgets (feeds) that cannot fetch cross-origin themselves
            if ctx.page != "newtab" {
                return Reply::Err("not allowed".into());
            }
            let url = args["url"].as_str().unwrap_or("").to_string();
            if !(url.starts_with("https://") || url.starts_with("http://")) {
                return Reply::Err("only web URLs".into());
            }
            let (c, i) = (ctx.clone(), id.clone());
            std::thread::spawn(move || {
                let r = crate::net::get_bytes(&url, 2 * 1024 * 1024).map(|b| json!(String::from_utf8_lossy(&b)));
                respond_later(c, i, r);
            });
            Reply::Later
        }
        "getSettings" => {
            let profile = app.win(ctx.wid).map(|w| w.profile.clone()).unwrap_or_default();
            Reply::Now(settings_json(app, &profile))
        }
        "setSetting" => {
            let profile = app.win(ctx.wid).map(|w| w.profile.clone()).unwrap_or_default();
            match set_setting(app, &profile, args["key"].as_str().unwrap_or(""), args["value"].clone()) {
                Ok(v) => Reply::Now(v),
                Err(e) => Reply::Err(e),
            }
        }
        "suggest" => {
            crate::omnibox::suggest(app, ctx.clone(), id.clone(), args);
            Reply::Later
        }
        "find" => {
            crate::hooks::find(app, ctx.wid, args);
            ok()
        }
        "openInternal" => {
            open_internal(app, ctx.wid, args["page"].as_str().unwrap_or("newtab"));
            ok()
        }
        "info" => Reply::Now(json!({
            "version": crate::VERSION,
            "engine": app.env().map(|e| crate::webview::runtime_version(&e)).unwrap_or_default(),
            "dataDir": util::user_data_root(),
            "exe": std::env::current_exe().unwrap_or_default(),
        })),
        _ => crate::hooks::dispatch(app, ctx, cmd, args, id),
    }
}

thread_local! {
    static AUTOMATION_WAITING: std::cell::RefCell<std::collections::HashMap<u64, Box<dyn FnOnce(Value)>>> = Default::default();
    static AUTOMATION_SEQ: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Run a bridge command for the test-automation channel; `done` gets the result, including
/// results of commands that answer later (sync setup, key tests, update checks).
pub fn run_for_automation(app: &mut App, wid: u32, msg: Value, done: Box<dyn FnOnce(Value)>) {
    let ctx = Ctx { wid, tid: 0, page: "chrome".into() };
    let cmd = msg["cmd"].as_str().unwrap_or("").to_string();
    let n = AUTOMATION_SEQ.with(|s| {
        s.set(s.get() + 1);
        s.get()
    });
    let id = json!({"automation": n});
    match dispatch(app, &ctx, &cmd, &msg["args"], &id) {
        Reply::Now(v) => done(json!({"ok": true, "data": v})),
        Reply::Err(e) => done(json!({"ok": false, "error": e})),
        Reply::Later => {
            AUTOMATION_WAITING.with(|m| m.borrow_mut().insert(n, done));
        }
    }
}

/// Update one setting by dotted key ("ai.model") and apply its side effects.
pub fn set_setting(app: &mut App, profile: &str, key: &str, value: Value) -> Result<Value, String> {
    if key.is_empty() {
        return Err("missing key".into());
    }
    let p = app.profile(profile);
    let mut v = serde_json::to_value(&p.settings).map_err(|e| e.to_string())?;
    let parts: Vec<&str> = key.split('.').collect();
    {
        let mut cur = &mut v;
        for (i, k) in parts.iter().enumerate() {
            if i == parts.len() - 1 {
                if cur.get(*k).is_none() {
                    return Err(format!("unknown setting {key}"));
                }
                cur[*k] = value.clone();
            } else {
                cur = cur.get_mut(*k).ok_or_else(|| format!("unknown setting {key}"))?;
            }
        }
    }
    let new: crate::settings::Settings = serde_json::from_value(v).map_err(|e| format!("invalid value for {key}: {e}"))?;
    p.settings = new;
    p.settings_dirty = true;
    if let Some(db) = p.db() {
        db.kv_set("settings_updated", &util::now_ms().to_string());
    }
    let mut restart = false;
    crate::hooks::setting_changed(app, profile, key, &mut restart);
    push_settings(app, profile);
    Ok(json!({"restartRequired": restart}))
}
