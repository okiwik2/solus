//! Top-level browser windows: Win32 window with a custom frame, the UI webview (tab strip, toolbar,
//! panels) and the content webviews of the visible tab(s).
//!
//! Layering: the UI webview covers the whole client area and sits above the tab webviews. Its window
//! region is "everything except the content area, plus any open popups", so menus and the omnibox
//! dropdown draw over the live page while the page stays fully interactive elsewhere.

use std::cell::Cell;

use webview2_com::Microsoft::Web::WebView2::Win32::*;
use windows::core::{w, BOOL};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Registry::*;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::app::{defer, try_with_app, App};
use crate::caption::{self, Btn};
use crate::settings::WindowPlacement;
use crate::tab::Tab;
use crate::util::{self, WStr};

pub struct View {
    pub container: HWND,
    pub ctl: ICoreWebView2Controller,
    pub wv: ICoreWebView2,
}

impl View {
    pub fn close(&self) {
        unsafe {
            let _ = self.ctl.Close();
            let _ = DestroyWindow(self.container);
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Insets {
    pub top: f64,
    pub left: f64,
    pub right: f64,
    pub bottom: f64,
}

#[derive(Clone, Debug)]
pub struct Split {
    pub a: u32,
    pub b: u32,
    pub ratio: f64,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct TabGroup {
    pub id: String,
    pub name: String,
    pub color: String,
    pub collapsed: bool,
    pub auto: bool,
}

pub struct ClosedTab {
    pub url: String,
    pub title: String,
    pub favicon: String,
    pub index: usize,
    pub workspace: String,
    pub closed_at: i64,
}

pub struct BrowserWindow {
    pub id: u32,
    pub hwnd: HWND,
    pub profile: String,
    pub incognito: bool,
    pub ui: Option<View>,
    pub ui_creating: bool,
    pub ui_ready: bool,
    pub tabs: Vec<Tab>,
    pub active: u32,
    pub split: Option<Split>,
    pub insets: Insets,
    pub overlays: Vec<[f64; 4]>,
    pub closing: bool,
    pub last_focus: i64,
    pub session_dirty: bool,
    pub html_fullscreen: bool,
    pub saved_placement: Option<WINDOWPLACEMENT>,
    pub saved_style: isize,
    pub hidden_since: i64,
    pub workspace: String,
    pub closed: Vec<ClosedTab>,
    pub focus_omnibox_pending: bool,
    pub groups: Vec<TabGroup>,
}

impl BrowserWindow {
    pub fn create(id: u32, profile: &str, incognito: bool, p: &WindowPlacement, nth: usize) -> BrowserWindow {
        let hwnd = unsafe {
            let hinst = GetModuleHandleW(None).unwrap();
            let title = WStr::new(if incognito { "Solus (Incognito)" } else { "Solus" });
            let (mut x, mut y, w, h) = (p.x, p.y, p.w.max(500), p.h.max(400));
            if x == i32::MIN || !rect_on_screen(x, y, w, h) {
                x = CW_USEDEFAULT;
                y = CW_USEDEFAULT;
            } else {
                x += nth as i32 * 32;
                y += nth as i32 * 32;
            }
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("SolusWindow"),
                title.pcwstr(),
                WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
                x,
                y,
                w,
                h,
                None,
                None,
                Some(hinst.into()),
                None,
            )
            .expect("CreateWindowExW");
            let dark: BOOL = (incognito || system_prefers_dark()).into();
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                &dark as *const _ as _,
                std::mem::size_of::<BOOL>() as u32,
            );
            // force a frame recalculation so WM_NCCALCSIZE removes the standard caption
            let _ = SetWindowPos(hwnd, None, 0, 0, 0, 0, SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER);
            let show = if p.maximized && nth == 0 { SW_SHOWMAXIMIZED } else { SW_SHOWNORMAL };
            let _ = ShowWindow(hwnd, show);
            let _ = UpdateWindow(hwnd);
            hwnd
        };
        if incognito {
            caption::with_state(hwnd, |s| {
                s.bg = caption::rgb(0x23, 0x1f, 0x2e);
                s.fg = caption::rgb(0xee, 0xee, 0xee);
                s.hover_bg = caption::rgb(0x3a, 0x34, 0x4a);
            });
        } else if system_prefers_dark() {
            caption::with_state(hwnd, |s| {
                s.bg = caption::rgb(0x1f, 0x20, 0x23);
                s.fg = caption::rgb(0xe8, 0xe8, 0xe8);
                s.hover_bg = caption::rgb(0x36, 0x38, 0x3d);
            });
        }
        BrowserWindow {
            id,
            hwnd,
            profile: profile.to_string(),
            incognito,
            ui: None,
            ui_creating: false,
            ui_ready: false,
            tabs: vec![],
            active: 0,
            split: None,
            insets: Insets { top: 112.0, left: 0.0, right: 0.0, bottom: 0.0 },
            overlays: vec![],
            closing: false,
            last_focus: util::now_ms(),
            session_dirty: false,
            html_fullscreen: false,
            saved_placement: None,
            saved_style: 0,
            hidden_since: 0,
            workspace: String::new(),
            closed: vec![],
            focus_omnibox_pending: false,
            groups: vec![],
        }
    }

    pub fn tab(&self, id: u32) -> Option<&Tab> {
        self.tabs.iter().find(|t| t.id == id)
    }
    pub fn tab_mut(&mut self, id: u32) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }
    pub fn tab_index(&self, id: u32) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }
    pub fn active_tab(&self) -> Option<&Tab> {
        self.tab(self.active)
    }
    pub fn visible_tab_ids(&self) -> Vec<u32> {
        match &self.split {
            Some(s) if (s.a == self.active || s.b == self.active) && self.tab(s.a).is_some() && self.tab(s.b).is_some() => {
                vec![s.a, s.b]
            }
            _ if self.active != 0 => vec![self.active],
            _ => vec![],
        }
    }

    pub fn is_visible(&self) -> bool {
        unsafe { IsWindowVisible(self.hwnd).as_bool() && !IsIconic(self.hwnd).as_bool() }
    }

    pub fn show_and_focus(&mut self) {
        self.hidden_since = 0;
        unsafe {
            if IsIconic(self.hwnd).as_bool() {
                let _ = ShowWindow(self.hwnd, SW_RESTORE);
            } else if !IsWindowVisible(self.hwnd).as_bool() {
                let _ = ShowWindow(self.hwnd, SW_SHOW);
            }
            let _ = SetForegroundWindow(self.hwnd);
        }
        self.last_focus = util::now_ms();
    }

    pub fn hide_to_tray(&mut self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
        self.hidden_since = util::now_ms();
        for t in self.tabs.iter_mut() {
            t.set_visible(false);
        }
    }

    pub fn drop_all_views(&mut self) {
        if let Some(ui) = self.ui.take() {
            ui.close();
        }
        self.ui_ready = false;
        self.ui_creating = false;
        for t in self.tabs.iter_mut() {
            t.drop_view();
        }
    }

    pub fn destroy(&mut self) {
        self.closing = true;
        self.drop_all_views();
        caption::remove(self.hwnd);
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }

    pub fn scale(&self) -> f64 {
        caption::scale(self.hwnd)
    }

    pub fn client_size(&self) -> (i32, i32) {
        let mut rc = RECT::default();
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut rc);
        }
        (rc.right - rc.left, rc.bottom - rc.top)
    }

    /// Physical-pixel rectangles where tab content is shown.
    pub fn content_rects(&self) -> Vec<(u32, RECT)> {
        let (w, h) = self.client_size();
        let s = self.scale();
        let ins = if self.html_fullscreen { Insets::default() } else { self.insets };
        let area = RECT {
            left: (ins.left * s).round() as i32,
            top: (ins.top * s).round() as i32,
            right: w - (ins.right * s).round() as i32,
            bottom: h - (ins.bottom * s).round() as i32,
        };
        let ids = self.visible_tab_ids();
        if ids.len() == 2 && !self.html_fullscreen {
            // the UI draws a small header above each pane
            let mut area = area;
            area.top += (24.0 * s).round() as i32;
            let ratio = self.split.as_ref().map(|s| s.ratio).unwrap_or(0.5).clamp(0.15, 0.85);
            let gap = (6.0 * s).round() as i32;
            let width = area.right - area.left;
            let mid = area.left + ((width as f64) * ratio).round() as i32;
            vec![
                (ids[0], RECT { left: area.left, top: area.top, right: mid - gap / 2, bottom: area.bottom }),
                (ids[1], RECT { left: mid + (gap - gap / 2), top: area.top, right: area.right, bottom: area.bottom }),
            ]
        } else if ids.len() == 2 && self.html_fullscreen {
            vec![(self.active, area)]
        } else {
            ids.into_iter().map(|id| (id, area)).collect()
        }
    }

    pub fn layout(&mut self) {
        let (w, h) = self.client_size();
        if w <= 0 || h <= 0 {
            return;
        }
        let rects = self.content_rects();
        let visible_now = self.is_visible();
        for t in self.tabs.iter_mut() {
            match rects.iter().find(|(id, _)| *id == t.id) {
                Some((_, r)) if visible_now => t.place(*r),
                _ => t.set_visible(false),
            }
        }
        let Some(ui) = &self.ui else { return };
        let s = self.scale();
        unsafe {
            let _ = SetWindowPos(ui.container, Some(HWND_TOP), 0, 0, w, h, SWP_NOACTIVATE);
            let _ = ui.ctl.SetBounds(RECT { left: 0, top: 0, right: w, bottom: h });
            let rgn = CreateRectRgn(0, 0, w, h);
            if self.html_fullscreen {
                let _ = CombineRgn(Some(rgn), Some(rgn), Some(rgn), RGN_DIFF);
            } else {
                // only tabs with a live page punch through; otherwise the UI shows its placeholder
                // (crashed page, page still loading after being unloaded)
                let live = |id: u32| self.tabs.iter().any(|t| t.id == id && t.view.is_some());
                for (_, r) in rects.iter().filter(|(id, _)| live(*id)) {
                    let r2 = CreateRectRgn(r.left, r.top, r.right, r.bottom);
                    CombineRgn(Some(rgn), Some(rgn), Some(r2), RGN_DIFF);
                    let _ = DeleteObject(r2.into());
                }
                for o in &self.overlays {
                    let r2 = CreateRectRgn(
                        (o[0] * s).floor() as i32,
                        (o[1] * s).floor() as i32,
                        ((o[0] + o[2]) * s).ceil() as i32,
                        ((o[1] + o[3]) * s).ceil() as i32,
                    );
                    CombineRgn(Some(rgn), Some(rgn), Some(r2), RGN_OR);
                    let _ = DeleteObject(r2.into());
                }
                // native caption buttons and the top resize edge belong to the frame window
                let cb = caption::buttons_rect(self.hwnd);
                let r2 = CreateRectRgn(cb.left, cb.top, cb.right, cb.bottom);
                CombineRgn(Some(rgn), Some(rgn), Some(r2), RGN_DIFF);
                let _ = DeleteObject(r2.into());
                if !IsZoomed(self.hwnd).as_bool() {
                    let edge = resize_border(self.hwnd);
                    let r3 = CreateRectRgn(0, 0, w, edge);
                    CombineRgn(Some(rgn), Some(rgn), Some(r3), RGN_DIFF);
                    let _ = DeleteObject(r3.into());
                }
            }
            if std::env::var_os("SOLUS_NO_REGION").is_some() {
                let _ = DeleteObject(rgn.into());
            } else {
                SetWindowRgn(ui.container, Some(rgn), true);
            }
            caption::with_state(self.hwnd, |st| st.visible = !self.html_fullscreen);
            caption::invalidate(self.hwnd);
        }
    }

    pub fn set_html_fullscreen(&mut self, on: bool) {
        if self.html_fullscreen == on {
            return;
        }
        self.html_fullscreen = on;
        unsafe {
            if on {
                let mut wp = WINDOWPLACEMENT { length: std::mem::size_of::<WINDOWPLACEMENT>() as u32, ..Default::default() };
                let _ = GetWindowPlacement(self.hwnd, &mut wp);
                self.saved_placement = Some(wp);
                self.saved_style = GetWindowLongPtrW(self.hwnd, GWL_STYLE);
                let mon = MonitorFromWindow(self.hwnd, MONITOR_DEFAULTTONEAREST);
                let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
                let _ = GetMonitorInfoW(mon, &mut mi);
                SetWindowLongPtrW(self.hwnd, GWL_STYLE, (WS_POPUP | WS_VISIBLE | WS_CLIPCHILDREN).0 as isize);
                let r = mi.rcMonitor;
                let _ = SetWindowPos(
                    self.hwnd,
                    Some(HWND_TOP),
                    r.left,
                    r.top,
                    r.right - r.left,
                    r.bottom - r.top,
                    SWP_FRAMECHANGED | SWP_NOOWNERZORDER,
                );
            } else {
                SetWindowLongPtrW(self.hwnd, GWL_STYLE, self.saved_style);
                if let Some(wp) = self.saved_placement.take() {
                    let _ = SetWindowPlacement(self.hwnd, &wp);
                }
                let _ = SetWindowPos(
                    self.hwnd,
                    None,
                    0,
                    0,
                    0,
                    0,
                    SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOOWNERZORDER,
                );
            }
        }
        self.layout();
    }

    pub fn placement(&self) -> Option<WindowPlacement> {
        unsafe {
            let mut wp = WINDOWPLACEMENT { length: std::mem::size_of::<WINDOWPLACEMENT>() as u32, ..Default::default() };
            GetWindowPlacement(self.hwnd, &mut wp).ok()?;
            let r = wp.rcNormalPosition;
            Some(WindowPlacement {
                x: r.left,
                y: r.top,
                w: r.right - r.left,
                h: r.bottom - r.top,
                maximized: wp.showCmd == SW_SHOWMAXIMIZED.0 as u32 || IsZoomed(self.hwnd).as_bool(),
            })
        }
    }
}

fn rect_on_screen(x: i32, y: i32, w: i32, h: i32) -> bool {
    unsafe {
        let r = RECT { left: x, top: y, right: x + w, bottom: y + h };
        !MonitorFromRect(&r, MONITOR_DEFAULTTONULL).is_invalid()
    }
}

fn resize_border(hwnd: HWND) -> i32 {
    unsafe {
        let dpi = GetDpiForWindow(hwnd);
        GetSystemMetricsForDpi(SM_CYFRAME, dpi) + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi)
    }
}

pub fn system_prefers_dark() -> bool {
    unsafe {
        let mut data: u32 = 1;
        let mut size = 4u32;
        let r = RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut data as *mut _ as _),
            Some(&mut size),
        );
        r.is_ok() && data == 0
    }
}

pub fn set_window_dark(hwnd: HWND, dark: bool) {
    unsafe {
        let v: BOOL = dark.into();
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &v as *const _ as _, 4);
    }
}

// ---------------------------------------------------------------------------------------------
// App-level window operations
// ---------------------------------------------------------------------------------------------

impl App {
    /// Create the UI webview for a window (also used after an engine restart).
    pub fn attach_ui(&mut self, wid: u32) {
        let Some(env) = self.env() else { return };
        let Some(w) = self.win_mut(wid) else { return };
        if w.ui.is_some() || w.ui_creating {
            return;
        }
        w.ui_creating = true;
        let container = create_container(w.hwnd);
        let req = crate::webview::ControllerReq { parent: container, profile: w.profile.clone(), incognito: w.incognito };
        crate::webview::create_controller(&env, req, move |app, res| {
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
            w.ui_creating = false;
            match res {
                Ok(ctl) => {
                    let wv = unsafe { ctl.CoreWebView2() }.expect("CoreWebView2");
                    let (r, g, b) = if w.incognito {
                        (0x23, 0x1f, 0x2e)
                    } else if system_prefers_dark() {
                        (0x1f, 0x20, 0x23)
                    } else {
                        (0xe8, 0xeb, 0xf0)
                    };
                    crate::webview::set_background(&ctl, r, g, b);
                    crate::webview::configure_ui(&wv);
                    crate::scheme::install(&wv);
                    crate::bridge::install_ui(&wv, wid);
                    crate::tab::install_accelerators(&ctl, wid, 0);
                    w.ui = Some(View { container, ctl, wv: wv.clone() });
                    unsafe {
                        let _ = ShowWindow(container, SW_SHOW);
                    }
                    w.layout();
                    crate::webview::navigate(&wv, "solus://chrome/");
                }
                Err(e) => {
                    logf!("UI webview creation failed: {e}");
                    unsafe {
                        let _ = DestroyWindow(container);
                    }
                }
            }
        });
    }

    pub fn layout_window(&mut self, wid: u32) {
        if let Some(w) = self.win_mut(wid) {
            w.layout();
        }
        self.ensure_window_views(wid);
    }

    /// Make sure every tab that is on screen has a live webview.
    pub fn ensure_window_views(&mut self, wid: u32) {
        let ids = match self.win(wid) {
            Some(w) if w.is_visible() => w.visible_tab_ids(),
            _ => return,
        };
        for id in ids {
            self.ensure_tab_view(wid, id);
        }
    }

    pub fn focus_content(&mut self, wid: u32) {
        let Some(w) = self.win(wid) else { return };
        if let Some(t) = w.active_tab() {
            if let Some(v) = &t.view {
                unsafe {
                    let _ = v.ctl.MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC);
                }
                return;
            }
        }
        self.focus_ui(wid);
    }

    pub fn focus_ui(&mut self, wid: u32) {
        if let Some(ui) = self.win(wid).and_then(|w| w.ui.as_ref()) {
            unsafe {
                let _ = ui.ctl.MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC);
            }
        }
    }

    pub fn window_command(&mut self, wid: u32, cmd: &str) {
        let Some(w) = self.win_mut(wid) else { return };
        let hwnd = w.hwnd;
        unsafe {
            match cmd {
                "minimize" => {
                    let _ = ShowWindow(hwnd, SW_MINIMIZE);
                }
                "maximize" => {
                    let _ = ShowWindow(hwnd, if IsZoomed(hwnd).as_bool() { SW_RESTORE } else { SW_MAXIMIZE });
                }
                "close" => {
                    let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                }
                "drag" => {
                    // fallback drag for the HTML tab strip when app-region is unavailable
                    let _ = ReleaseCapture();
                    let _ = PostMessageW(Some(hwnd), WM_NCLBUTTONDOWN, WPARAM(HTCAPTION as usize), LPARAM(0));
                }
                "sysmenu" => {
                    let mut pt = POINT::default();
                    let _ = GetCursorPos(&mut pt);
                    let menu = GetSystemMenu(hwnd, false);
                    let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, pt.x, pt.y, Some(0), hwnd, None);
                    if cmd.0 != 0 {
                        let _ = PostMessageW(Some(hwnd), WM_SYSCOMMAND, WPARAM(cmd.0 as usize), LPARAM(0));
                    }
                }
                _ => {}
            }
        }
    }
}

fn create_container(parent: HWND) -> HWND {
    unsafe {
        let hinst = GetModuleHandleW(None).unwrap();
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("SolusView"),
            w!(""),
            WS_CHILD | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
            0,
            0,
            0,
            0,
            Some(parent),
            None,
            Some(hinst.into()),
            None,
        )
        .expect("container")
    }
}

pub fn create_tab_container(parent: HWND) -> HWND {
    let h = create_container(parent);
    // tab content sits below the UI layer
    unsafe {
        let _ = SetWindowPos(h, Some(HWND_BOTTOM), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    }
    h
}

// ---------------------------------------------------------------------------------------------
// Window procedures
// ---------------------------------------------------------------------------------------------

pub fn register_classes() {
    unsafe {
        let hinst = GetModuleHandleW(None).unwrap();
        let icon = LoadIconW(Some(hinst.into()), windows::core::PCWSTR(1 as _)).unwrap_or_default();
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_DBLCLKS,
            lpfnWndProc: Some(main_wndproc),
            hInstance: hinst.into(),
            hIcon: icon,
            hIconSm: icon,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: w!("SolusWindow"),
            ..Default::default()
        };
        RegisterClassExW(&wc);
        let wc2 = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(view_wndproc),
            hInstance: hinst.into(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: w!("SolusView"),
            ..Default::default()
        };
        RegisterClassExW(&wc2);
    }
}

unsafe extern "system" fn view_wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_ERASEBKGND => LRESULT(1),
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

fn with_window_deferred(hwnd: HWND, f: impl FnOnce(&mut App, u32) + 'static) {
    // Prefer running now (keeps resizing smooth); defer if the app is busy.
    let f = Cell::new(Some(f));
    let ran = try_with_app(|app| {
        if let Some(wid) = app.win_by_hwnd(hwnd) {
            if let Some(f) = f.take() {
                f(app, wid);
            }
        }
    });
    if ran.is_none() {
        if let Some(f) = f.take() {
            defer(move |app| {
                if let Some(wid) = app.win_by_hwnd(hwnd) {
                    f(app, wid)
                }
            });
        }
    }
}

unsafe extern "system" fn main_wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_NCCALCSIZE if wp.0 != 0 => {
            let params = &mut *(lp.0 as *mut NCCALCSIZE_PARAMS);
            let orig_top = params.rgrc[0].top;
            let r = DefWindowProcW(hwnd, msg, wp, lp);
            let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
            if (style & WS_CAPTION.0 as isize) != 0 {
                params.rgrc[0].top = orig_top;
                if IsZoomed(hwnd).as_bool() {
                    params.rgrc[0].top += resize_border(hwnd);
                }
            }
            r
        }
        WM_NCHITTEST => {
            let x = (lp.0 & 0xffff) as i16 as i32;
            let y = ((lp.0 >> 16) & 0xffff) as i16 as i32;
            let mut pt = POINT { x, y };
            let _ = ScreenToClient(hwnd, &mut pt);
            if let Some(b) = caption::hit(hwnd, pt) {
                return LRESULT(match b {
                    Btn::Min => HTMINBUTTON,
                    Btn::Max => HTMAXBUTTON,
                    Btn::Close => HTCLOSE,
                } as isize);
            }
            let d = DefWindowProcW(hwnd, msg, wp, lp);
            if d.0 != HTCLIENT as isize {
                return d;
            }
            let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
            if (style & WS_CAPTION.0 as isize) == 0 {
                return LRESULT(HTCLIENT as isize);
            }
            let edge = resize_border(hwnd);
            if !IsZoomed(hwnd).as_bool() && pt.y < edge {
                let mut rc = RECT::default();
                let _ = GetClientRect(hwnd, &mut rc);
                let corner = edge * 3;
                return LRESULT(if pt.x < corner {
                    HTTOPLEFT
                } else if pt.x > rc.right - corner {
                    HTTOPRIGHT
                } else {
                    HTTOP
                } as isize);
            }
            let cap_h = (caption::with_state(hwnd, |s| s.height_css) * caption::scale(hwnd)) as i32;
            if pt.y < cap_h {
                return LRESULT(HTCAPTION as isize);
            }
            LRESULT(HTCLIENT as isize)
        }
        WM_NCMOUSEMOVE => {
            let hot = match wp.0 as u32 {
                HTMINBUTTON => Some(Btn::Min),
                HTMAXBUTTON => Some(Btn::Max),
                HTCLOSE => Some(Btn::Close),
                _ => None,
            };
            let changed = caption::with_state(hwnd, |s| {
                let c = s.hover != hot;
                s.hover = hot;
                c
            });
            if changed {
                caption::invalidate(hwnd);
            }
            let mut tme = TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE | TME_NONCLIENT,
                hwndTrack: hwnd,
                dwHoverTime: 0,
            };
            let _ = TrackMouseEvent(&mut tme);
            if hot.is_some() {
                return LRESULT(0);
            }
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        WM_NCMOUSELEAVE | windows::Win32::UI::Controls::WM_MOUSELEAVE => {
            caption::with_state(hwnd, |s| {
                s.hover = None;
                s.pressed = None;
            });
            caption::invalidate(hwnd);
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK => {
            let b = match wp.0 as u32 {
                HTMINBUTTON => Some(Btn::Min),
                HTMAXBUTTON => Some(Btn::Max),
                HTCLOSE => Some(Btn::Close),
                _ => None,
            };
            if let Some(b) = b {
                caption::with_state(hwnd, |s| s.pressed = Some(b));
                caption::invalidate(hwnd);
                return LRESULT(0);
            }
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        WM_NCLBUTTONUP => {
            let b = match wp.0 as u32 {
                HTMINBUTTON => Some(Btn::Min),
                HTMAXBUTTON => Some(Btn::Max),
                HTCLOSE => Some(Btn::Close),
                _ => None,
            };
            let pressed = caption::with_state(hwnd, |s| s.pressed.take());
            caption::invalidate(hwnd);
            if let (Some(b), Some(p)) = (b, pressed) {
                if b == p {
                    match b {
                        Btn::Min => {
                            let _ = ShowWindow(hwnd, SW_MINIMIZE);
                        }
                        Btn::Max => {
                            let _ = ShowWindow(hwnd, if IsZoomed(hwnd).as_bool() { SW_RESTORE } else { SW_MAXIMIZE });
                        }
                        Btn::Close => {
                            let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                        }
                    }
                }
                return LRESULT(0);
            }
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            caption::paint(hwnd, hdc);
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_SIZE => {
            if wp.0 as u32 != SIZE_MINIMIZED {
                with_window_deferred(hwnd, |app, wid| app.layout_window(wid));
            }
            let _ = InvalidateRect(Some(hwnd), None, false);
            LRESULT(0)
        }
        WM_MOVE => {
            with_window_deferred(hwnd, |app, wid| {
                if let Some(w) = app.win(wid) {
                    for v in w.ui.iter().chain(w.tabs.iter().filter_map(|t| t.view.as_ref())) {
                        let _ = v.ctl.NotifyParentWindowPositionChanged();
                    }
                }
            });
            LRESULT(0)
        }
        WM_DPICHANGED => {
            let r = &*(lp.0 as *const RECT);
            let _ = SetWindowPos(
                hwnd,
                None,
                r.left,
                r.top,
                r.right - r.left,
                r.bottom - r.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            LRESULT(0)
        }
        WM_GETMINMAXINFO => {
            let mmi = &mut *(lp.0 as *mut MINMAXINFO);
            let s = caption::scale(hwnd);
            mmi.ptMinTrackSize.x = (520.0 * s) as i32;
            mmi.ptMinTrackSize.y = (380.0 * s) as i32;
            LRESULT(0)
        }
        WM_ACTIVATE => {
            let active = (wp.0 & 0xffff) as u32 != WA_INACTIVE;
            caption::with_state(hwnd, |s| s.active = active);
            caption::invalidate(hwnd);
            if active {
                defer(move |app| {
                    if let Some(wid) = app.win_by_hwnd(hwnd) {
                        if let Some(w) = app.win_mut(wid) {
                            w.last_focus = util::now_ms();
                        }
                        crate::bridge::emit(app, wid, "windowFocus", serde_json::json!({"active": true}));
                    }
                });
            } else {
                defer(move |app| {
                    if let Some(wid) = app.win_by_hwnd(hwnd) {
                        crate::bridge::emit(app, wid, "windowFocus", serde_json::json!({"active": false}));
                    }
                });
            }
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        WM_SETFOCUS => {
            defer(move |app| {
                if let Some(wid) = app.win_by_hwnd(hwnd) {
                    app.focus_content(wid);
                }
            });
            LRESULT(0)
        }
        WM_CLOSE => {
            defer(move |app| {
                if let Some(wid) = app.win_by_hwnd(hwnd) {
                    app.close_window(wid);
                }
            });
            LRESULT(0)
        }
        WM_SHOWWINDOW => {
            let shown = wp.0 != 0;
            defer(move |app| {
                if let Some(wid) = app.win_by_hwnd(hwnd) {
                    if shown {
                        app.layout_window(wid);
                    }
                }
            });
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        WM_SYSCOMMAND => {
            // restore from minimised: make sure the visible tab is loaded
            let r = DefWindowProcW(hwnd, msg, wp, lp);
            if (wp.0 & 0xfff0) as u32 == SC_RESTORE {
                with_window_deferred(hwnd, |app, wid| app.layout_window(wid));
            }
            r
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

// ---------------------------------------------------------------------------------------------
// System tray
// ---------------------------------------------------------------------------------------------

thread_local! {
    static TRAY_ADDED: Cell<bool> = const { Cell::new(false) };
    static TASKBAR_CREATED: Cell<u32> = const { Cell::new(0) };
}

pub fn taskbar_created_msg() -> u32 {
    TASKBAR_CREATED.with(|c| {
        if c.get() == 0 {
            c.set(unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) });
        }
        c.get()
    })
}

fn tray_data() -> NOTIFYICONDATAW {
    let mut nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: crate::app::msg_hwnd(),
        uID: 1,
        ..Default::default()
    };
    nid.Anonymous.uVersion = NOTIFYICON_VERSION_4;
    nid
}

pub fn ensure_tray(_app: &mut App) {
    if TRAY_ADDED.with(|c| c.get()) {
        return;
    }
    add_tray();
}

fn add_tray() {
    unsafe {
        let hinst = GetModuleHandleW(None).unwrap();
        let mut nid = tray_data();
        nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP | NIF_SHOWTIP;
        nid.uCallbackMessage = crate::app::WM_APP_TRAY;
        let cx = GetSystemMetrics(SM_CXSMICON);
        nid.hIcon = HICON(
            LoadImageW(Some(hinst.into()), windows::core::PCWSTR(1 as _), IMAGE_ICON, cx, cx, LR_DEFAULTCOLOR)
                .map(|h| h.0)
                .unwrap_or_default(),
        );
        let tip: Vec<u16> = "Solus".encode_utf16().collect();
        nid.szTip[..tip.len()].copy_from_slice(&tip);
        if Shell_NotifyIconW(NIM_ADD, &nid).as_bool() {
            let _ = Shell_NotifyIconW(NIM_SETVERSION, &nid);
            TRAY_ADDED.with(|c| c.set(true));
        }
    }
}

pub fn readd_tray(app: &mut App) {
    if TRAY_ADDED.with(|c| c.get()) {
        TRAY_ADDED.with(|c| c.set(false));
        ensure_tray(app);
    }
}

pub fn remove_tray() {
    if TRAY_ADDED.with(|c| c.get()) {
        unsafe {
            let nid = tray_data();
            let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
        }
        TRAY_ADDED.with(|c| c.set(false));
    }
}

/// Show a notification balloon (Windows turns it into a toast).
pub fn tray_notify(title: &str, body: &str) {
    if !TRAY_ADDED.with(|c| c.get()) {
        add_tray();
    }
    unsafe {
        let mut nid = tray_data();
        nid.uFlags = NIF_INFO;
        let t: Vec<u16> = title.encode_utf16().take(63).collect();
        let b: Vec<u16> = body.encode_utf16().take(255).collect();
        nid.szInfoTitle[..t.len()].copy_from_slice(&t);
        nid.szInfo[..b.len()].copy_from_slice(&b);
        nid.dwInfoFlags = NIIF_USER | NIIF_LARGE_ICON;
        let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
    }
}

const ID_OPEN: usize = 1;
const ID_NEW: usize = 2;
const ID_INCOG: usize = 3;
const ID_EXIT: usize = 9;

pub fn on_tray_message(event: u32) {
    match event {
        WM_LBUTTONUP | 0x400 | 0x401 => defer(|app| tray_open(app)),
        0x405 => defer(|app| crate::productivity::on_notification_click(app)),
        WM_RBUTTONUP | WM_CONTEXTMENU => unsafe {
            let menu = CreatePopupMenu().unwrap();
            let _ = AppendMenuW(menu, MF_STRING, ID_OPEN, w!("Open Solus"));
            let _ = AppendMenuW(menu, MF_STRING, ID_NEW, w!("New window"));
            let _ = AppendMenuW(menu, MF_STRING, ID_INCOG, w!("New Incognito window"));
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
            let _ = AppendMenuW(menu, MF_STRING, ID_EXIT, w!("Exit Solus"));
            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);
            let owner = crate::app::msg_hwnd();
            let _ = SetForegroundWindow(owner);
            let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN, pt.x, pt.y, Some(0), owner, None);
            let _ = DestroyMenu(menu);
            let id = cmd.0 as usize;
            defer(move |app| match id {
                ID_OPEN => tray_open(app),
                ID_NEW => {
                    let p = app.local.last_profile.clone();
                    app.new_window(&p, false, vec![], false);
                }
                ID_INCOG => {
                    let p = app.local.last_profile.clone();
                    app.new_window(&p, true, vec![], false);
                }
                ID_EXIT => app.quit(),
                _ => {}
            });
        },
        _ => {}
    }
}

fn tray_open(app: &mut App) {
    if let Some(wid) = app.last_active_window(false) {
        app.show_window(wid);
    } else {
        let p = app.local.last_profile.clone();
        app.new_window(&p, false, vec![], true);
    }
}

pub fn key_down(vk: VIRTUAL_KEY) -> bool {
    unsafe { (GetKeyState(vk.0 as i32) as u16 & 0x8000) != 0 }
}
