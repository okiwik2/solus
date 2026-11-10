//! Tabs: state, webview lifecycle (create / freeze / discard), navigation, and session save/restore.

use std::collections::HashMap;

use base64::Engine;
use serde_json::{json, Value};
use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::*;
use windows::core::{Interface, BOOL, PWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::IStream;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::app::{defer, App};
use crate::util::{self, WStr};
use crate::window::{key_down, ClosedTab, View};

pub const NEWTAB: &str = "solus://newtab/";

pub struct Tab {
    pub id: u32,
    pub url: String,
    pub title: String,
    pub favicon: String,
    pub view: Option<View>,
    pub creating: bool,
    pub loading: bool,
    pub can_back: bool,
    pub can_fwd: bool,
    pub shown: bool,
    pub frozen: bool,
    pub last_active: i64,
    pub pinned: bool,
    pub group: String,
    pub workspace: String,
    pub audible: bool,
    pub muted: bool,
    pub crashed: String,
    pub zoom: f64,
    pub opener: u32,
    pub blocked: u32,
    pub created: i64,
    pub https_upgraded: bool,
    pub cert_error: bool,
    pub load_error: String,
    pub has_input: bool,
    pub transition: String,
    pub prerender: bool,
    pub pending_new_window: Option<(ICoreWebView2NewWindowRequestedEventArgs, ICoreWebView2Deferral)>,
    pub rect: RECT,
}

impl Tab {
    pub fn new(id: u32, url: &str, workspace: &str) -> Tab {
        Tab {
            id,
            url: url.to_string(),
            title: String::new(),
            favicon: String::new(),
            view: None,
            creating: false,
            loading: false,
            can_back: false,
            can_fwd: false,
            shown: false,
            frozen: false,
            last_active: util::now_ms(),
            pinned: false,
            group: String::new(),
            workspace: workspace.to_string(),
            audible: false,
            muted: false,
            crashed: String::new(),
            zoom: 1.0,
            opener: 0,
            blocked: 0,
            created: util::now_ms(),
            https_upgraded: false,
            cert_error: false,
            load_error: String::new(),
            has_input: false,
            transition: "link".into(),
            prerender: false,
            pending_new_window: None,
            rect: RECT::default(),
        }
    }

    pub fn is_discarded(&self) -> bool {
        self.view.is_none() && !self.creating
    }

    pub fn place(&mut self, r: RECT) {
        let Some(v) = &self.view else { return };
        unsafe {
            if !self.shown || self.rect != r {
                let _ = SetWindowPos(
                    v.container,
                    Some(HWND_BOTTOM),
                    r.left,
                    r.top,
                    r.right - r.left,
                    r.bottom - r.top,
                    SWP_NOACTIVATE | SWP_SHOWWINDOW,
                );
                let _ = v.ctl.SetBounds(RECT { left: 0, top: 0, right: r.right - r.left, bottom: r.bottom - r.top });
            }
            if !self.shown {
                if self.frozen {
                    if let Ok(w3) = v.wv.cast::<ICoreWebView2_3>() {
                        let _ = w3.Resume();
                    }
                    self.frozen = false;
                }
                if let Ok(w19) = v.wv.cast::<ICoreWebView2_19>() {
                    let _ = w19.SetMemoryUsageTargetLevel(COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL);
                }
                let _ = v.ctl.SetIsVisible(true);
            }
        }
        self.rect = r;
        self.shown = true;
        self.last_active = util::now_ms();
    }

    pub fn set_visible(&mut self, vis: bool) {
        if vis == self.shown {
            return;
        }
        if let Some(v) = &self.view {
            unsafe {
                let _ = v.ctl.SetIsVisible(vis);
                let _ = ShowWindow(v.container, if vis { SW_SHOWNA } else { SW_HIDE });
            }
        }
        if !vis {
            self.last_active = util::now_ms();
        }
        self.shown = vis;
    }

    pub fn drop_view(&mut self) {
        if let Some(v) = self.view.take() {
            crate::privacy::detach(&v.wv);
            v.close();
        }
        if let Some((_, d)) = self.pending_new_window.take() {
            unsafe {
                let _ = d.Complete();
            }
        }
        self.creating = false;
        self.shown = false;
        self.frozen = false;
        self.loading = false;
        self.audible = false;
    }

    pub fn security(&self) -> &'static str {
        if self.url.starts_with("solus:") {
            "internal"
        } else if self.cert_error {
            "danger"
        } else if self.url.starts_with("https:") {
            "secure"
        } else if self.url.starts_with("http:") {
            "insecure"
        } else if self.url.starts_with("file:") {
            "file"
        } else {
            "other"
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "url": self.url,
            "title": if self.title.is_empty() { display_title(&self.url) } else { self.title.clone() },
            "favicon": self.favicon,
            "loading": self.loading,
            "canBack": self.can_back,
            "canFwd": self.can_fwd,
            "pinned": self.pinned,
            "group": self.group,
            "workspace": self.workspace,
            "audible": self.audible,
            "muted": self.muted,
            "crashed": self.crashed,
            "discarded": self.is_discarded(),
            "frozen": self.frozen,
            "zoom": self.zoom,
            "blocked": self.blocked,
            "security": self.security(),
            "loadError": self.load_error,
            "opener": self.opener,
            "lastActive": self.last_active,
        })
    }
}

pub fn display_title(url: &str) -> String {
    if url.starts_with(NEWTAB) {
        return "New Tab".into();
    }
    if let Some(rest) = url.strip_prefix("solus://") {
        let name = rest.trim_end_matches('/').split('/').next().unwrap_or("");
        let mut c = name.chars();
        return match c.next() {
            Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            None => "Solus".into(),
        };
    }
    let h = util::host_of(url);
    if h.is_empty() {
        url.to_string()
    } else {
        h.trim_start_matches("www.").to_string()
    }
}

#[derive(Default, Clone)]
pub struct OpenOpts {
    pub activate: bool,
    pub index: Option<usize>,
    pub opener: u32,
    pub lazy: bool,
    pub pinned: bool,
    pub group: String,
    pub title: String,
    pub favicon: String,
    pub workspace: Option<String>,
}

/// Turn omnibox text into a URL: URLs pass through, bare domains get https://, search shortcuts
/// expand, everything else becomes a search.
pub fn fixup_input(app: &mut App, profile: &str, input: &str) -> String {
    let s = input.trim();
    let settings = app.settings(profile).clone();
    fixup_with(&settings, s)
}

pub fn fixup_with(settings: &crate::settings::Settings, s: &str) -> String {
    if s.is_empty() {
        return NEWTAB.to_string();
    }
    let lower = s.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("chrome://").or_else(|| lower.strip_prefix("edge://")) {
        return format!("solus://{rest}");
    }
    for p in ["solus:", "http:", "https:", "file:", "about:", "data:", "view-source:", "mailto:", "chrome-extension:"] {
        if lower.starts_with(p) {
            if p == "solus:" && !s.contains("//") {
                return format!("solus://{}", &s[6..]);
            }
            return s.to_string();
        }
    }
    // keyword search shortcuts: "yt cats"
    if let Some((kw, rest)) = s.split_once(' ') {
        if let Some(sc) = settings.search_shortcuts.iter().find(|x| x.keyword.eq_ignore_ascii_case(kw)) {
            if !rest.trim().is_empty() {
                return sc.url.replace("%s", &crate::settings::url_encode(rest.trim()));
            }
        }
    }
    // Windows paths
    let b = s.as_bytes();
    if b.len() > 2 && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/') {
        return format!("file:///{}", s.replace('\\', "/"));
    }
    if !s.contains(' ') && looks_like_host(s) {
        let host = s.split(['/', '?', '#']).next().unwrap_or("");
        let h = host.split(':').next().unwrap_or("");
        let local = h == "localhost" || h.parse::<std::net::IpAddr>().is_ok() || h.ends_with(".local") || h.ends_with(".lan");
        return if local { format!("http://{s}") } else { format!("https://{s}") };
    }
    settings.search_url(s)
}

fn looks_like_host(s: &str) -> bool {
    let host = s.split(['/', '?', '#']).next().unwrap_or("");
    let h = host.rsplit_once(':').map(|(a, p)| if p.chars().all(|c| c.is_ascii_digit()) { a } else { host }).unwrap_or(host);
    if h.eq_ignore_ascii_case("localhost") || h.parse::<std::net::IpAddr>().is_ok() {
        return true;
    }
    if !h.contains('.') || h.starts_with('.') || h.ends_with('.') {
        return false;
    }
    if !h.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '.') {
        return false;
    }
    // must end in a known public suffix
    psl::suffix(h.to_ascii_lowercase().as_bytes()).map(|x| x.is_known()).unwrap_or(false)
}

// ---------------------------------------------------------------------------------------------
// Tab operations on the app
// ---------------------------------------------------------------------------------------------

impl App {
    pub fn open_tab(&mut self, wid: u32, url: &str, o: OpenOpts) -> u32 {
        let tid = self.next_id();
        let Some(w) = self.win_mut(wid) else { return 0 };
        let mut t = Tab::new(tid, url, &o.workspace.clone().unwrap_or_else(|| w.workspace.clone()));
        t.opener = o.opener;
        t.pinned = o.pinned;
        t.group = o.group.clone();
        t.title = o.title.clone();
        t.favicon = o.favicon.clone();
        let idx = match o.index {
            Some(i) => i.min(w.tabs.len()),
            None if o.opener != 0 => {
                // after the opener and its earlier children (Chrome behaviour)
                match w.tab_index(o.opener) {
                    Some(oi) => {
                        let mut i = oi + 1;
                        while i < w.tabs.len() && w.tabs[i].opener == o.opener {
                            i += 1;
                        }
                        i
                    }
                    None => w.tabs.len(),
                }
            }
            None => w.tabs.len(),
        };
        if t.group.is_empty() && o.opener != 0 {
            if let Some(op) = w.tab(o.opener) {
                t.group = op.group.clone();
            }
        }
        w.tabs.insert(idx, t);
        w.session_dirty = true;
        if o.activate || w.active == 0 {
            self.activate_tab(wid, tid);
        } else if !o.lazy {
            self.ensure_tab_view(wid, tid);
        }
        crate::bridge::push_tabs(self, wid);
        if o.activate && url.starts_with(NEWTAB) {
            crate::bridge::emit(self, wid, "focusOmnibox", json!({}));
        }
        tid
    }

    pub fn activate_tab(&mut self, wid: u32, tid: u32) {
        let Some(w) = self.win_mut(wid) else { return };
        if w.tab(tid).is_none() {
            return;
        }
        let prev = w.active;
        w.active = tid;
        let cur_ws = w.workspace.clone();
        let mut new_ws = None;
        if let Some(t) = w.tab_mut(tid) {
            t.last_active = util::now_ms();
            if !t.workspace.is_empty() && t.workspace != cur_ws {
                new_ws = Some(t.workspace.clone());
            }
        }
        if let Some(ws) = new_ws {
            w.workspace = ws;
        }
        // leaving split view when switching to a tab outside the pair
        if let Some(s) = &w.split {
            if s.a != tid && s.b != tid {
                w.split = None;
            }
        }
        w.session_dirty = true;
        w.layout();
        self.ensure_window_views(wid);
        if prev != tid {
            self.focus_content(wid);
        }
        crate::bridge::emit(self, wid, "active", json!({"id": tid}));
        crate::bridge::push_tabs(self, wid);
    }

    pub fn ensure_tab_view(&mut self, wid: u32, tid: u32) {
        let Some(env) = self.env() else { return };
        let Some(w) = self.win_mut(wid) else { return };
        let hwnd = w.hwnd;
        let profile = w.profile.clone();
        let incognito = w.incognito;
        let Some(t) = w.tab_mut(tid) else { return };
        if t.view.is_some() || t.creating {
            return;
        }
        t.creating = true;
        t.crashed.clear();
        let container = crate::window::create_tab_container(hwnd);
        let req = crate::webview::ControllerReq { parent: container, profile: profile.clone(), incognito };
        crate::webview::create_controller(&env, req, move |app, res| {
            let settings = app.settings(&profile).clone();
            let Some(w) = app.win_mut(wid) else {
                if let Ok(c) = res {
                    unsafe {
                        let _ = c.Close();
                    }
                }
                unsafe {
                    let _ = DestroyWindow(container);
                }
                return;
            };
            let Some(t) = w.tab_mut(tid) else {
                if let Ok(c) = res {
                    unsafe {
                        let _ = c.Close();
                    }
                }
                unsafe {
                    let _ = DestroyWindow(container);
                }
                return;
            };
            t.creating = false;
            let ctl = match res {
                Ok(c) => c,
                Err(e) => {
                    logf!("tab webview creation failed: {e}");
                    t.crashed = "Could not create the page view".into();
                    unsafe {
                        let _ = DestroyWindow(container);
                    }
                    return;
                }
            };
            let wv = unsafe { ctl.CoreWebView2() }.expect("CoreWebView2");
            let dark = incognito || app_dark(&settings);
            if dark {
                crate::webview::set_background(&ctl, 0x20, 0x21, 0x24);
            } else {
                crate::webview::set_background(&ctl, 0xff, 0xff, 0xff);
            }
            crate::webview::configure_content(&wv, &settings);
            crate::scheme::install(&wv);
            install_tab_handlers(&wv, &ctl, wid, tid);
            install_accelerators(&ctl, wid, tid);
            crate::bridge::install_page(&wv, wid, tid);
            crate::downloads::install(&wv, wid, tid);
            crate::content::install(&wv, wid, tid);
            unsafe {
                let _ = ctl.SetIsVisible(false);
                if t.zoom != 1.0 {
                    let _ = ctl.SetZoomFactor(t.zoom);
                }
                if t.muted {
                    if let Ok(w8) = wv.cast::<ICoreWebView2_8>() {
                        let _ = w8.SetIsMuted(true);
                    }
                }
            }
            t.view = Some(View { container, ctl, wv: wv.clone() });
            t.shown = false;
            if let Some((args, deferral)) = t.pending_new_window.take() {
                unsafe {
                    let _ = args.SetNewWindow(&wv);
                    let _ = args.SetHandled(true);
                    let _ = deferral.Complete();
                }
            } else {
                let url = t.url.clone();
                t.loading = true;
                crate::webview::navigate(&wv, &url);
            }
            w.layout();
            if w.active == tid {
                app.focus_content(wid);
            }
            crate::privacy::attach(app, &wv, wid, tid);
            crate::bridge::push_tab(app, wid, tid);
        });
    }

    pub fn close_tab(&mut self, wid: u32, tid: u32) {
        let Some(w) = self.win_mut(wid) else { return };
        let Some(idx) = w.tab_index(tid) else { return };
        let mut t = w.tabs.remove(idx);
        if !w.incognito && !t.url.starts_with(NEWTAB) {
            w.closed.push(ClosedTab {
                url: t.url.clone(),
                title: t.title.clone(),
                favicon: t.favicon.clone(),
                index: idx,
                workspace: t.workspace.clone(),
                closed_at: util::now_ms(),
            });
            if w.closed.len() > 25 {
                w.closed.remove(0);
            }
        }
        t.drop_view();
        if let Some(s) = &w.split {
            if s.a == tid || s.b == tid {
                w.split = None;
            }
        }
        w.session_dirty = true;
        if w.active == tid {
            w.active = 0;
            // prefer the opener, then the tab to the right, then left (same workspace)
            let ws = w.workspace.clone();
            let same: Vec<(usize, u32, u32)> =
                w.tabs.iter().enumerate().filter(|(_, x)| x.workspace == ws).map(|(i, x)| (i, x.id, x.opener)).collect();
            let next = same
                .iter()
                .find(|(_, id, _)| *id == t.opener && t.opener != 0)
                .or_else(|| same.iter().find(|(i, _, _)| *i >= idx))
                .or_else(|| same.last())
                .map(|x| x.1);
            match next {
                Some(n) => {
                    self.activate_tab(wid, n);
                }
                None => {
                    let ws_has_tabs = self.win(wid).map(|w| !w.tabs.is_empty()).unwrap_or(false);
                    if ws_has_tabs || !self.windows.iter().any(|x| x.id != wid) {
                        // keep the window open with a fresh tab
                        self.open_tab(wid, NEWTAB, OpenOpts { activate: true, ..Default::default() });
                    } else {
                        crate::bridge::push_tabs(self, wid);
                        self.close_window(wid);
                        return;
                    }
                }
            }
        } else {
            w.layout();
        }
        crate::bridge::push_tabs(self, wid);
    }

    pub fn reopen_closed(&mut self, wid: u32) {
        let Some(w) = self.win_mut(wid) else { return };
        let Some(c) = w.closed.pop() else { return };
        self.open_tab(
            wid,
            &c.url,
            OpenOpts {
                activate: true,
                index: Some(c.index),
                title: c.title,
                favicon: c.favicon,
                workspace: Some(c.workspace),
                ..Default::default()
            },
        );
    }

    pub fn navigate_tab(&mut self, wid: u32, tid: u32, input: &str) {
        let profile = match self.win(wid) {
            Some(w) => w.profile.clone(),
            None => return,
        };
        let url = fixup_input(self, &profile, input);
        if url.to_ascii_lowercase().starts_with("javascript:") {
            return;
        }
        if self.win(wid).map(|w| w.active == tid).unwrap_or(false) && crate::perf::take_prerender(self, wid, &url) {
            return;
        }
        let Some(w) = self.win_mut(wid) else { return };
        let Some(t) = w.tab_mut(tid) else { return };
        t.url = url.clone();
        t.load_error.clear();
        t.https_upgraded = false;
        t.transition = "typed".into();
        if let Some(v) = &t.view {
            crate::webview::navigate(&v.wv, &url);
        } else if !t.creating {
            self.ensure_tab_view(wid, tid);
        }
        crate::bridge::push_tab(self, wid, tid);
    }

    pub fn tab_action(&mut self, wid: u32, tid: u32, action: &str) {
        let Some(w) = self.win_mut(wid) else { return };
        let Some(t) = w.tab_mut(tid) else { return };
        if t.view.is_none() {
            if action == "reload" {
                self.ensure_tab_view(wid, tid);
            }
            return;
        }
        let v = t.view.as_ref().unwrap();
        unsafe {
            match action {
                "back" => {
                    let _ = v.wv.GoBack();
                }
                "forward" => {
                    let _ = v.wv.GoForward();
                }
                "reload" => {
                    let _ = v.wv.Reload();
                }
                "hardReload" => {
                    // bypass cache via DevTools protocol
                    let m = WStr::new("Page.reload");
                    let p = WStr::new("{\"ignoreCache\":true}");
                    let _ = v.wv.CallDevToolsProtocolMethod(m.pcwstr(), p.pcwstr(), None);
                }
                "stop" => {
                    let _ = v.wv.Stop();
                }
                "devtools" => {
                    let _ = v.wv.OpenDevToolsWindow();
                }
                "print" => {
                    if let Ok(w16) = v.wv.cast::<ICoreWebView2_16>() {
                        let _ = w16.ShowPrintUI(COREWEBVIEW2_PRINT_DIALOG_KIND_BROWSER);
                    }
                }
                "saveAs" => {
                    if let Ok(w25) = v.wv.cast::<ICoreWebView2_25>() {
                        let _ = w25.ShowSaveAsUI(None);
                    }
                }
                "mute" | "unmute" => {
                    if let Ok(w8) = v.wv.cast::<ICoreWebView2_8>() {
                        let _ = w8.SetIsMuted(action == "mute");
                    }
                    t.muted = action == "mute";
                }
                "zoomIn" | "zoomOut" | "zoomReset" => {
                    const STEPS: [f64; 15] = [0.25, 0.33, 0.5, 0.67, 0.75, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0];
                    let mut z = 1.0;
                    let _ = v.ctl.ZoomFactor(&mut z);
                    let nz = match action {
                        "zoomIn" => STEPS.iter().copied().find(|s| *s > z + 0.001).unwrap_or(3.0),
                        "zoomOut" => STEPS.iter().rev().copied().find(|s| *s < z - 0.001).unwrap_or(0.25),
                        _ => 1.0,
                    };
                    let _ = v.ctl.SetZoomFactor(nz);
                    t.zoom = nz;
                }
                _ => {}
            }
        }
        crate::bridge::push_tab(self, wid, tid);
    }

    pub fn move_tab(&mut self, wid: u32, tid: u32, to: usize) {
        let Some(w) = self.win_mut(wid) else { return };
        let Some(i) = w.tab_index(tid) else { return };
        let t = w.tabs.remove(i);
        let to = to.min(w.tabs.len());
        w.tabs.insert(to, t);
        w.session_dirty = true;
        crate::bridge::push_tabs(self, wid);
    }

    pub fn cycle_tab(&mut self, wid: u32, delta: i32) {
        let Some(w) = self.win(wid) else { return };
        let ws = w.workspace.clone();
        let ids: Vec<u32> = w.tabs.iter().filter(|t| t.workspace == ws && !t.prerender).map(|t| t.id).collect();
        if ids.is_empty() {
            return;
        }
        let cur = ids.iter().position(|x| *x == w.active).unwrap_or(0) as i32;
        let n = ids.len() as i32;
        let next = ids[((cur + delta) % n + n) as usize % n as usize];
        self.activate_tab(wid, next);
    }

    pub fn select_tab_n(&mut self, wid: u32, n: usize) {
        let Some(w) = self.win(wid) else { return };
        let ws = w.workspace.clone();
        let ids: Vec<u32> = w.tabs.iter().filter(|t| t.workspace == ws).map(|t| t.id).collect();
        let pick = if n == 9 { ids.last().copied() } else { ids.get(n - 1).copied() };
        if let Some(id) = pick {
            self.activate_tab(wid, id);
        }
    }
}

fn app_dark(s: &crate::settings::Settings) -> bool {
    match s.theme.as_str() {
        "dark" => true,
        "light" => false,
        _ => crate::window::system_prefers_dark(),
    }
}

// ---------------------------------------------------------------------------------------------
// Webview event handlers for content tabs
// ---------------------------------------------------------------------------------------------

fn with_tab(app: &mut App, wid: u32, tid: u32, f: impl FnOnce(&mut Tab)) -> bool {
    if let Some(t) = app.win_mut(wid).and_then(|w| w.tab_mut(tid)) {
        f(t);
        true
    } else {
        false
    }
}

fn install_tab_handlers(wv: &ICoreWebView2, ctl: &ICoreWebView2Controller, wid: u32, tid: u32) {
    let mut tok = Default::default();
    unsafe {
        // NavigationStarting: policy decisions must be synchronous
        let h = NavigationStartingEventHandler::create(Box::new(move |sender, args| {
            let (Some(wv), Some(args)) = (sender, args) else { return Ok(()) };
            let mut p = PWSTR::null();
            let _ = args.Uri(&mut p);
            let uri = take_pwstr(p);
            let current = crate::webview::get_source(&wv);
            let mut history_nav = false;
            if let Ok(a3) = args.cast::<ICoreWebView2NavigationStartingEventArgs3>() {
                let mut k = COREWEBVIEW2_NAVIGATION_KIND_NEW_DOCUMENT;
                let _ = a3.NavigationKind(&mut k);
                history_nav = k != COREWEBVIEW2_NAVIGATION_KIND_NEW_DOCUMENT;
            }
            let decision = crate::scheme::navigation_policy_for(&wv, &current, &uri, history_nav);
            crate::privacy::on_nav_start(tid, &uri);
            if let crate::scheme::NavDecision::Block = decision {
                let _ = args.SetCancel(true);
                logf!("blocked navigation from {current} to internal page {uri}");
                return Ok(());
            }
            // internal pages get the message bridge; the web does not see chrome.webview.postMessage
            if let Ok(st) = wv.Settings() {
                let _ = st.SetIsWebMessageEnabled(uri.starts_with("solus:"));
            }
            if crate::scheme::https_first_upgrade(&uri) {
                let https = format!("https://{}", &uri[7..]);
                let _ = args.SetCancel(true);
                defer(move |app| {
                    with_tab(app, wid, tid, |t| t.https_upgraded = true);
                    if let Some(v) = app.win(wid).and_then(|w| w.tab(tid)).and_then(|t| t.view.as_ref()) {
                        crate::webview::navigate(&v.wv, &https);
                    }
                });
                return Ok(());
            }
            defer(move |app| {
                crate::privacy::cancel_prompts_for_tab(tid);
                with_tab(app, wid, tid, |t| {
                    t.has_input = false;
                    t.loading = true;
                    t.load_error.clear();
                    t.cert_error = false;
                    if !uri.starts_with("about:") {
                        t.url = uri.clone();
                    }
                });
                crate::bridge::push_tab(app, wid, tid);
            });
            Ok(())
        }));
        let _ = wv.add_NavigationStarting(&h, &mut tok);

        let h = SourceChangedEventHandler::create(Box::new(move |sender, _| {
            if let Some(wv) = sender {
                let url = crate::webview::get_source(&wv);
                defer(move |app| {
                    with_tab(app, wid, tid, |t| {
                        if !url.is_empty() && url != "about:blank" || t.url.is_empty() {
                            t.url = url.clone();
                        }
                    });
                    if let Some(w) = app.win_mut(wid) {
                        w.session_dirty = true;
                    }
                    crate::bridge::push_tab(app, wid, tid);
                });
            }
            Ok(())
        }));
        let _ = wv.add_SourceChanged(&h, &mut tok);

        let h = HistoryChangedEventHandler::create(Box::new(move |sender, _| {
            if let Some(wv) = sender {
                let mut b = BOOL(0);
                let mut f = BOOL(0);
                let _ = wv.CanGoBack(&mut b);
                let _ = wv.CanGoForward(&mut f);
                defer(move |app| {
                    with_tab(app, wid, tid, |t| {
                        t.can_back = b.as_bool();
                        t.can_fwd = f.as_bool();
                    });
                    crate::bridge::push_tab(app, wid, tid);
                });
            }
            Ok(())
        }));
        let _ = wv.add_HistoryChanged(&h, &mut tok);

        let h = DocumentTitleChangedEventHandler::create(Box::new(move |sender, _| {
            if let Some(wv) = sender {
                let title = crate::webview::get_title(&wv);
                defer(move |app| {
                    with_tab(app, wid, tid, |t| t.title = title);
                    crate::hooks::title_changed(app, wid, tid);
                    if let Some(w) = app.win_mut(wid) {
                        w.session_dirty = true;
                    }
                    crate::bridge::push_tab(app, wid, tid);
                });
            }
            Ok(())
        }));
        let _ = wv.add_DocumentTitleChanged(&h, &mut tok);

        let h = NavigationCompletedEventHandler::create(Box::new(move |sender, args| {
            let (Some(wv), Some(args)) = (sender, args) else { return Ok(()) };
            let mut ok = BOOL(0);
            let _ = args.IsSuccess(&mut ok);
            let mut status = COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN;
            let _ = args.WebErrorStatus(&mut status);
            let url = crate::webview::get_source(&wv);
            let title = crate::webview::get_title(&wv);
            defer(move |app| on_nav_completed(app, wid, tid, ok.as_bool(), status, url, title));
            Ok(())
        }));
        let _ = wv.add_NavigationCompleted(&h, &mut tok);

        if let Ok(wv15) = wv.cast::<ICoreWebView2_15>() {
            let h = FaviconChangedEventHandler::create(Box::new(move |sender, _| {
                if let Some(wv) = sender {
                    fetch_favicon(&wv, wid, tid);
                }
                Ok(())
            }));
            let _ = wv15.add_FaviconChanged(&h, &mut tok);
        }

        let h = NewWindowRequestedEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else { return Ok(()) };
            let mut p = PWSTR::null();
            let _ = args.Uri(&mut p);
            let uri = take_pwstr(p);
            let background = key_down(VK_CONTROL) || key_down(VK_MBUTTON);
            let new_window = key_down(VK_SHIFT);
            let deferral = args.GetDeferral()?;
            defer(move |app| {
                let Some(w) = app.win(wid) else {
                    let _ = deferral.Complete();
                    return;
                };
                let (target_wid, opener) = if new_window && !w.incognito {
                    let p = w.profile.clone();
                    (app.new_window(&p, false, vec![], false), 0)
                } else {
                    (wid, tid)
                };
                let ntid = app.open_tab(
                    target_wid,
                    &uri,
                    OpenOpts { activate: !background, opener, lazy: true, ..Default::default() },
                );
                if let Some(t) = app.win_mut(target_wid).and_then(|w| w.tab_mut(ntid)) {
                    t.pending_new_window = Some((args, deferral));
                }
                app.ensure_tab_view(target_wid, ntid);
            });
            Ok(())
        }));
        let _ = wv.add_NewWindowRequested(&h, &mut tok);

        let h = WindowCloseRequestedEventHandler::create(Box::new(move |_, _| {
            defer(move |app| app.close_tab(wid, tid));
            Ok(())
        }));
        let _ = wv.add_WindowCloseRequested(&h, &mut tok);

        let h = ContainsFullScreenElementChangedEventHandler::create(Box::new(move |sender, _| {
            if let Some(wv) = sender {
                let mut fs = BOOL(0);
                let _ = wv.ContainsFullScreenElement(&mut fs);
                defer(move |app| {
                    if let Some(w) = app.win_mut(wid) {
                        if w.active == tid || !fs.as_bool() {
                            w.set_html_fullscreen(fs.as_bool());
                        }
                    }
                });
            }
            Ok(())
        }));
        let _ = wv.add_ContainsFullScreenElementChanged(&h, &mut tok);

        let h = ProcessFailedEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else { return Ok(()) };
            let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED;
            let _ = args.ProcessFailedKind(&mut kind);
            defer(move |app| on_process_failed(app, wid, tid, kind));
            Ok(())
        }));
        let _ = wv.add_ProcessFailed(&h, &mut tok);

        if let Ok(wv8) = wv.cast::<ICoreWebView2_8>() {
            let h = IsDocumentPlayingAudioChangedEventHandler::create(Box::new(move |sender, _| {
                if let Some(wv) = sender {
                    if let Ok(w8) = wv.cast::<ICoreWebView2_8>() {
                        let mut a = BOOL(0);
                        let _ = w8.IsDocumentPlayingAudio(&mut a);
                        defer(move |app| {
                            with_tab(app, wid, tid, |t| t.audible = a.as_bool());
                            crate::bridge::push_tab(app, wid, tid);
                        });
                    }
                }
                Ok(())
            }));
            let _ = wv8.add_IsDocumentPlayingAudioChanged(&h, &mut tok);
        }

        if let Ok(wv14) = wv.cast::<ICoreWebView2_14>() {
            let h = ServerCertificateErrorDetectedEventHandler::create(Box::new(move |_, _| {
                defer(move |app| {
                    with_tab(app, wid, tid, |t| t.cert_error = true);
                    crate::bridge::push_tab(app, wid, tid);
                });
                Ok(())
            }));
            let _ = wv14.add_ServerCertificateErrorDetected(&h, &mut tok);
        }

        let h = ZoomFactorChangedEventHandler::create(Box::new(move |sender, _| {
            if let Some(c) = sender {
                let mut z = 1.0;
                let _ = c.ZoomFactor(&mut z);
                defer(move |app| {
                    with_tab(app, wid, tid, |t| t.zoom = z);
                    crate::bridge::push_tab(app, wid, tid);
                });
            }
            Ok(())
        }));
        let _ = ctl.add_ZoomFactorChanged(&h, &mut tok);

        let h = FocusChangedEventHandler::create(Box::new(move |_, _| {
            defer(move |app| {
                crate::bridge::emit(app, wid, "contentFocus", json!({"tab": tid}));
                if let Some(w) = app.win_mut(wid) {
                    if w.split.is_some() && w.active != tid {
                        w.active = tid;
                        crate::bridge::emit(app, wid, "active", json!({"id": tid}));
                        crate::bridge::push_tabs(app, wid);
                    }
                }
            });
            Ok(())
        }));
        let _ = ctl.add_GotFocus(&h, &mut tok);
    }
}

fn on_nav_completed(app: &mut App, wid: u32, tid: u32, ok: bool, status: COREWEBVIEW2_WEB_ERROR_STATUS, url: String, title: String) {
    let mut fallback: Option<String> = None;
    with_tab(app, wid, tid, |t| {
        t.loading = false;
        if !url.is_empty() {
            t.url = url.clone();
        }
        if !title.is_empty() {
            t.title = title.clone();
        }
        if !ok {
            let connection_failed = matches!(
                status,
                COREWEBVIEW2_WEB_ERROR_STATUS_CANNOT_CONNECT
                    | COREWEBVIEW2_WEB_ERROR_STATUS_CONNECTION_ABORTED
                    | COREWEBVIEW2_WEB_ERROR_STATUS_CONNECTION_RESET
                    | COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_COMMON_NAME_IS_INCORRECT
                    | COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_EXPIRED
                    | COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_IS_INVALID
                    | COREWEBVIEW2_WEB_ERROR_STATUS_CLIENT_CERTIFICATE_CONTAINS_ERRORS
                    | COREWEBVIEW2_WEB_ERROR_STATUS_SERVER_UNREACHABLE
                    | COREWEBVIEW2_WEB_ERROR_STATUS_TIMEOUT
            );
            if t.https_upgraded && connection_failed && t.url.starts_with("https://") {
                // HTTPS-First: the site has no working HTTPS, fall back to http and remember it
                let http = format!("http://{}", &t.url[8..]);
                crate::scheme::allow_http_host(&util::host_of(&http));
                fallback = Some(http);
            } else if status != COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED {
                t.load_error = format!("{}", status.0);
            }
        }
        t.https_upgraded = false;
    });
    if let Some(http) = fallback {
        if let Some(v) = app.win(wid).and_then(|w| w.tab(tid)).and_then(|t| t.view.as_ref()) {
            crate::webview::navigate(&v.wv, &http);
        }
        return;
    }
    if let Some(w) = app.win_mut(wid) {
        w.session_dirty = true;
    }
    crate::bridge::push_tab(app, wid, tid);
    crate::bridge::page_loaded(app, wid, tid, ok);
}

fn on_process_failed(app: &mut App, wid: u32, tid: u32, kind: COREWEBVIEW2_PROCESS_FAILED_KIND) {
    match kind {
        COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED => {
            // handled by the environment's BrowserProcessExited event
        }
        COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED | COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE => {
            let msg = if kind == COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE {
                "This page stopped responding"
            } else {
                "This page crashed"
            };
            logf!("tab {tid}: {msg}");
            // drop the dead view; the UI shows a "reload" panel in its place
            with_tab(app, wid, tid, |t| {
                t.drop_view();
                t.crashed = msg.to_string();
            });
            if let Some(w) = app.win_mut(wid) {
                w.layout();
            }
            crate::bridge::push_tab(app, wid, tid);
        }
        _ => {
            // GPU / utility / frame crashes recover on their own
            logf!("tab {tid}: process failure kind {}", kind.0);
        }
    }
}

thread_local! {
    static FAVICONS: std::cell::RefCell<HashMap<String, String>> = std::cell::RefCell::new(HashMap::new());
}

fn fetch_favicon(wv: &ICoreWebView2, wid: u32, tid: u32) {
    unsafe {
        let Ok(w15) = wv.cast::<ICoreWebView2_15>() else { return };
        let mut p = PWSTR::null();
        let _ = w15.FaviconUri(&mut p);
        let uri = take_pwstr(p);
        if uri.is_empty() {
            defer(move |app| {
                with_tab(app, wid, tid, |t| t.favicon.clear());
                crate::bridge::push_tab(app, wid, tid);
            });
            return;
        }
        if let Some(d) = FAVICONS.with(|f| f.borrow().get(&uri).cloned()) {
            defer(move |app| {
                with_tab(app, wid, tid, |t| t.favicon = d);
                crate::bridge::push_tab(app, wid, tid);
            });
            return;
        }
        let h = GetFaviconCompletedHandler::create(Box::new(move |hr, stream| {
            if hr.is_ok() {
                if let Some(s) = stream {
                    let bytes = read_stream(&s);
                    if !bytes.is_empty() {
                        let d = format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(&bytes));
                        FAVICONS.with(|f| {
                            let mut m = f.borrow_mut();
                            if m.len() > 500 {
                                m.clear();
                            }
                            m.insert(uri.clone(), d.clone());
                        });
                        defer(move |app| {
                            with_tab(app, wid, tid, |t| t.favicon = d);
                            crate::hooks::favicon_changed(app, wid, tid);
                            crate::bridge::push_tab(app, wid, tid);
                        });
                    }
                }
            }
            Ok(())
        }));
        let _ = w15.GetFavicon(COREWEBVIEW2_FAVICON_IMAGE_FORMAT_PNG, &h);
    }
}

pub fn read_stream_from_start(s: &IStream) -> Vec<u8> {
    unsafe {
        let _ = s.Seek(0, windows::Win32::System::Com::STREAM_SEEK_SET, None);
    }
    read_stream(s)
}

pub fn read_stream(s: &IStream) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buf = [0u8; 16384];
    loop {
        let mut read = 0u32;
        let hr = unsafe { s.Read(buf.as_mut_ptr() as _, buf.len() as u32, Some(&mut read)) };
        if hr.is_err() || read == 0 {
            break;
        }
        out.extend_from_slice(&buf[..read as usize]);
        if out.len() > 8 * 1024 * 1024 {
            break;
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Keyboard shortcuts
// ---------------------------------------------------------------------------------------------

pub fn default_shortcuts() -> Vec<(&'static str, &'static str, &'static str)> {
    // action, accelerator, label
    vec![
        ("newTab", "Ctrl+T", "New tab"),
        ("closeTab", "Ctrl+W", "Close tab"),
        ("reopenTab", "Ctrl+Shift+T", "Reopen closed tab"),
        ("newWindow", "Ctrl+N", "New window"),
        ("newIncognito", "Ctrl+Shift+N", "New Incognito window"),
        ("nextTab", "Ctrl+Tab", "Next tab"),
        ("prevTab", "Ctrl+Shift+Tab", "Previous tab"),
        ("focusOmnibox", "Ctrl+L", "Focus address bar"),
        ("search", "Ctrl+K", "Search in address bar"),
        ("back", "Alt+Left", "Back"),
        ("forward", "Alt+Right", "Forward"),
        ("reload", "F5", "Reload"),
        ("hardReload", "Ctrl+F5", "Reload without cache"),
        ("find", "Ctrl+F", "Find in page"),
        ("bookmark", "Ctrl+D", "Bookmark this page"),
        ("history", "Ctrl+H", "History"),
        ("downloads", "Ctrl+J", "Downloads"),
        ("bookmarksManager", "Ctrl+Shift+O", "Bookmarks manager"),
        ("tabSearch", "Ctrl+Shift+A", "Search tabs"),
        ("aiSidebar", "Ctrl+Shift+E", "AI assistant sidebar"),
        ("aiPopup", "Ctrl+Shift+Space", "Floating AI assistant"),
        ("sidebar", "Ctrl+Shift+S", "Toggle sidebar"),
        ("splitView", "Ctrl+Shift+X", "Split screen"),
        ("taskManager", "Shift+Esc", "Task manager"),
        ("fullscreen", "F11", "Full screen"),
        ("zoomIn", "Ctrl+=", "Zoom in"),
        ("zoomOut", "Ctrl+-", "Zoom out"),
        ("zoomReset", "Ctrl+0", "Reset zoom"),
        ("clearData", "Ctrl+Shift+Delete", "Clear browsing data"),
        ("home", "Alt+Home", "Home page"),
        ("notes", "Ctrl+Shift+K", "Notes for this page"),
        ("settings", "Ctrl+,", "Settings"),
    ]
}

/// Accelerators that cannot be rebound (they mirror Chrome and fixed key families).
fn fixed_action(ctrl: bool, shift: bool, alt: bool, vk: u32) -> Option<String> {
    let k = VIRTUAL_KEY(vk as u16);
    if ctrl && !alt && (0x31..=0x39).contains(&vk) && !shift {
        return Some(format!("tab{}", vk - 0x30));
    }
    if ctrl && shift && !alt && (0x31..=0x39).contains(&vk) {
        return Some(format!("workspace{}", vk - 0x30));
    }
    match (ctrl, shift, alt, k) {
        (true, false, false, VK_F4) => Some("closeTab".into()),
        (true, false, false, VK_NEXT) => Some("nextTab".into()),
        (true, false, false, VK_PRIOR) => Some("prevTab".into()),
        (false, false, true, VK_D) => Some("focusOmnibox".into()),
        (false, false, false, VK_F6) => Some("focusOmnibox".into()),
        (true, false, false, VK_E) => Some("search".into()),
        (true, false, false, VK_R) => Some("reload".into()),
        (false, true, false, VK_F5) => Some("hardReload".into()),
        (true, true, false, VK_R) => Some("hardReload".into()),
        (false, false, false, VK_F3) => Some("findNext".into()),
        (true, false, false, VK_G) => Some("findNext".into()),
        (false, true, false, VK_F3) => Some("findPrev".into()),
        (true, false, false, VK_OEM_PLUS) | (true, true, false, VK_OEM_PLUS) | (true, false, false, VK_ADD) => Some("zoomIn".into()),
        (true, false, false, VK_OEM_MINUS) | (true, false, false, VK_SUBTRACT) => Some("zoomOut".into()),
        (true, false, false, VK_NUMPAD0) => Some("zoomReset".into()),
        (false, false, false, VK_BROWSER_BACK) => Some("back".into()),
        (false, false, false, VK_BROWSER_FORWARD) => Some("forward".into()),
        (false, false, false, VK_BROWSER_REFRESH) => Some("reload".into()),
        (false, false, false, VK_ESCAPE) => Some("escape".into()),
        _ => None,
    }
}

pub fn accel_string(ctrl: bool, shift: bool, alt: bool, vk: u32) -> Option<String> {
    let k = VIRTUAL_KEY(vk as u16);
    let name = match k {
        VK_LEFT => "Left".to_string(),
        VK_RIGHT => "Right".into(),
        VK_UP => "Up".into(),
        VK_DOWN => "Down".into(),
        VK_TAB => "Tab".into(),
        VK_SPACE => "Space".into(),
        VK_ESCAPE => "Esc".into(),
        VK_DELETE => "Delete".into(),
        VK_HOME => "Home".into(),
        VK_END => "End".into(),
        VK_RETURN => "Enter".into(),
        VK_OEM_PLUS => "=".into(),
        VK_OEM_MINUS => "-".into(),
        VK_OEM_COMMA => ",".into(),
        VK_OEM_PERIOD => ".".into(),
        VK_OEM_2 => "/".into(),
        VK_OEM_5 => "\\".into(),
        _ if (0x30..=0x39).contains(&vk) || (0x41..=0x5A).contains(&vk) => char::from_u32(vk).unwrap().to_string(),
        _ if (0x70..=0x87).contains(&vk) => format!("F{}", vk - 0x6F),
        _ => return None,
    };
    let mut s = String::new();
    if ctrl {
        s.push_str("Ctrl+");
    }
    if shift {
        s.push_str("Shift+");
    }
    if alt {
        s.push_str("Alt+");
    }
    s.push_str(&name);
    Some(s)
}

thread_local! {
    /// accelerator -> action, rebuilt when shortcut settings change
    pub static SHORTCUTS: std::cell::RefCell<HashMap<String, String>> = std::cell::RefCell::new(build_shortcuts(&Default::default()));
}

pub fn build_shortcuts(overrides: &std::collections::BTreeMap<String, String>) -> HashMap<String, String> {
    let mut m = HashMap::new();
    for (action, accel, _) in default_shortcuts() {
        let a = overrides.get(action).cloned().unwrap_or_else(|| accel.to_string());
        if !a.is_empty() {
            m.insert(a, action.to_string());
        }
    }
    m
}

pub fn install_accelerators(ctl: &ICoreWebView2Controller, wid: u32, tid: u32) {
    let h = AcceleratorKeyPressedEventHandler::create(Box::new(move |_, args| {
        let Some(args) = args else { return Ok(()) };
        unsafe {
            let mut kind = COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN;
            let _ = args.KeyEventKind(&mut kind);
            if kind != COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN && kind != COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN {
                return Ok(());
            }
            let mut vk = 0u32;
            let _ = args.VirtualKey(&mut vk);
            let mut st = COREWEBVIEW2_PHYSICAL_KEY_STATUS::default();
            let _ = args.PhysicalKeyStatus(&mut st);
            let ctrl = key_down(VK_CONTROL);
            let shift = key_down(VK_SHIFT);
            let alt = key_down(VK_MENU);
            let action = accel_string(ctrl, shift, alt, vk)
                .and_then(|a| SHORTCUTS.with(|s| s.borrow().get(&a).cloned()))
                .or_else(|| fixed_action(ctrl, shift, alt, vk));
            let Some(action) = action else { return Ok(()) };
            if action == "escape" {
                // let pages and the UI handle Escape; we only use it to exit split/fullscreen popups
                defer(move |app| crate::bridge::emit(app, wid, "escape", json!({"tab": tid})));
                return Ok(());
            }
            // the UI webview handles its own text-editing keys
            let repeat_ok = matches!(action.as_str(), "nextTab" | "prevTab" | "zoomIn" | "zoomOut" | "findNext" | "findPrev");
            let _ = args.SetHandled(true);
            if st.WasKeyDown.as_bool() && !repeat_ok {
                return Ok(());
            }
            defer(move |app| crate::bridge::run_action(app, wid, &action, Value::Null));
        }
        Ok(())
    }));
    let mut tok = Default::default();
    unsafe {
        let _ = ctl.add_AcceleratorKeyPressed(&h, &mut tok);
    }
}

// ---------------------------------------------------------------------------------------------
// Session save / restore
// ---------------------------------------------------------------------------------------------

fn session_path(app: &mut App, profile: &str) -> std::path::PathBuf {
    app.profile(profile).dir.join("session.json")
}

pub fn window_session(w: &crate::window::BrowserWindow) -> Value {
    let tabs: Vec<Value> = w
        .tabs
        .iter()
        .filter(|t| !t.prerender)
        .map(|t| {
            json!({
                "url": t.url, "title": t.title, "pinned": t.pinned, "group": t.group,
                "workspace": t.workspace, "zoom": t.zoom, "muted": t.muted,
                "favicon": if t.favicon.len() < 12000 { t.favicon.clone() } else { String::new() },
            })
        })
        .collect();
    json!({
        "tabs": tabs,
        "active": w.tab_index(w.active).unwrap_or(0),
        "workspace": w.workspace,
        "placement": w.placement(),
        "split": w.split.as_ref().map(|s| json!({"a": w.tab_index(s.a), "b": w.tab_index(s.b), "ratio": s.ratio})),
        "groups": crate::bridge::groups_json(w),
    })
}

pub fn save_session(app: &mut App, wid: u32) {
    let Some(w) = app.win(wid) else { return };
    if w.incognito {
        return;
    }
    let profile = w.profile.clone();
    let wins: Vec<Value> = app
        .windows
        .iter()
        .filter(|x| x.profile == profile && !x.incognito && !x.tabs.is_empty())
        .map(window_session)
        .collect();
    let path = session_path(app, &profile);
    let data = json!({ "version": 1, "saved": util::now_ms(), "windows": wins });
    if let Ok(b) = serde_json::to_vec(&data) {
        if let Err(e) = util::write_atomic(&path, &b) {
            logf!("session save failed: {e}");
        }
    }
    for x in app.windows.iter_mut().filter(|x| x.profile == profile) {
        x.session_dirty = false;
    }
    // remember placement of the most recent window for the next launch
    if let Some(p) = app.win(wid).and_then(|w| w.placement()) {
        if app.win(wid).map(|w| w.is_visible()).unwrap_or(false) {
            app.local.placement = p;
            app.local_dirty = true;
        }
    }
}

/// Restore the saved session into `wid` (first saved window) and new windows (others).
/// Only the active tab of each window is loaded; the rest stay unloaded until clicked.
pub fn restore_session(app: &mut App, wid: u32, profile: &str) -> usize {
    let path = session_path(app, profile);
    let Ok(bytes) = std::fs::read(&path) else { return 0 };
    let Ok(v) = serde_json::from_slice::<Value>(&bytes) else {
        let _ = std::fs::copy(&path, path.with_extension("corrupt.json"));
        return 0;
    };
    let wins = v["windows"].as_array().cloned().unwrap_or_default();
    let mut total = 0;
    for (i, wv) in wins.iter().enumerate() {
        let target = if i == 0 {
            wid
        } else {
            let id = app.next_id();
            let placement = app.local.placement.clone();
            let n = app.windows.len();
            let w = crate::window::BrowserWindow::create(id, profile, false, &placement, n);
            app.windows.push(w);
            app.attach_ui(id);
            id
        };
        total += restore_window(app, target, wv);
    }
    total
}

fn restore_window(app: &mut App, wid: u32, v: &Value) -> usize {
    let tabs = v["tabs"].as_array().cloned().unwrap_or_default();
    let active_idx = v["active"].as_u64().unwrap_or(0) as usize;
    if let Some(w) = app.win_mut(wid) {
        w.workspace = v["workspace"].as_str().unwrap_or("").to_string();
        crate::bridge::restore_groups(w, &v["groups"]);
    }
    let mut ids = vec![];
    for t in tabs.iter() {
        let url = t["url"].as_str().unwrap_or(NEWTAB).to_string();
        let id = app.next_id();
        let Some(w) = app.win_mut(wid) else { return 0 };
        let mut tab = Tab::new(id, &url, t["workspace"].as_str().unwrap_or(""));
        tab.title = t["title"].as_str().unwrap_or("").to_string();
        tab.pinned = t["pinned"].as_bool().unwrap_or(false);
        tab.group = t["group"].as_str().unwrap_or("").to_string();
        tab.zoom = t["zoom"].as_f64().unwrap_or(1.0);
        tab.muted = t["muted"].as_bool().unwrap_or(false);
        tab.favicon = t["favicon"].as_str().unwrap_or("").to_string();
        w.tabs.push(tab);
        ids.push(id);
    }
    if let (Some(a), Some(b)) = (v["split"]["a"].as_u64(), v["split"]["b"].as_u64()) {
        if let (Some(&ta), Some(&tb)) = (ids.get(a as usize), ids.get(b as usize)) {
            if let Some(w) = app.win_mut(wid) {
                w.split = Some(crate::window::Split { a: ta, b: tb, ratio: v["split"]["ratio"].as_f64().unwrap_or(0.5) });
            }
        }
    }
    if let Some(&a) = ids.get(active_idx).or(ids.first()) {
        app.activate_tab(wid, a);
    }
    crate::bridge::push_tabs(app, wid);
    ids.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;

    #[test]
    fn omnibox_fixup() {
        let s = Settings::default();
        assert_eq!(fixup_with(&s, "example.com"), "https://example.com");
        assert_eq!(fixup_with(&s, "localhost:3000/x"), "http://localhost:3000/x");
        assert_eq!(fixup_with(&s, "192.168.1.1"), "http://192.168.1.1");
        assert_eq!(fixup_with(&s, "https://a.b/c"), "https://a.b/c");
        assert_eq!(fixup_with(&s, "hello world"), "https://www.google.com/search?q=hello+world");
        assert_eq!(fixup_with(&s, "file.txt"), "https://www.google.com/search?q=file.txt");
        assert_eq!(fixup_with(&s, "yt lofi beats"), "https://www.youtube.com/results?search_query=lofi+beats");
        assert_eq!(fixup_with(&s, "chrome://settings"), "solus://settings");
        assert_eq!(fixup_with(&s, "solus:history"), "solus://history");
        assert_eq!(fixup_with(&s, "C:\\Users\\a.pdf"), "file:///C:/Users/a.pdf");
        assert_eq!(fixup_with(&s, "news.bbc.co.uk/sport"), "https://news.bbc.co.uk/sport");
    }

    #[test]
    fn accel_names() {
        assert_eq!(accel_string(true, true, false, 0x54).as_deref(), Some("Ctrl+Shift+T"));
        assert_eq!(accel_string(false, false, true, VK_LEFT.0 as u32).as_deref(), Some("Alt+Left"));
        assert_eq!(accel_string(false, false, false, 0x74).as_deref(), Some("F5"));
    }
}
