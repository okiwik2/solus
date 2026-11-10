//! Application core: the single-threaded event queue, the WebView2 environment, profiles and windows.
//!
//! All WebView2 objects live on the UI thread. COM event handlers never touch `App` directly: they
//! push a closure with `defer()` and the message loop runs it with `&mut App`. Worker threads use
//! `post()`. This keeps re-entrancy (a WebView2 call raising another event) from ever aliasing state.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::Mutex;

use webview2_com::Microsoft::Web::WebView2::Win32::*;
use windows::core::{w, Interface};
use windows::Win32::Foundation::*;
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::settings::{LocalState, Settings};
use crate::tab::OpenOpts;
use crate::util::{self, WStr};
use crate::window::BrowserWindow;
use crate::Args;

pub const WM_APP_WAKE: u32 = WM_APP + 1;
pub const WM_APP_TRAY: u32 = WM_APP + 2;
pub const TIMER_TICK: usize = 1;
pub const TIMER_SAVE: usize = 2;

type LocalJob = Box<dyn FnOnce(&mut App)>;
type RemoteJob = Box<dyn FnOnce(&mut App) + Send>;

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    static LOCAL_Q: RefCell<VecDeque<LocalJob>> = RefCell::new(VecDeque::new());
}
static REMOTE_Q: Mutex<VecDeque<RemoteJob>> = Mutex::new(VecDeque::new());
static MSG_HWND: AtomicIsize = AtomicIsize::new(0);

/// Run `f` with the app on the next turn of the message loop (UI thread only).
pub fn defer(f: impl FnOnce(&mut App) + 'static) {
    LOCAL_Q.with(|q| q.borrow_mut().push_back(Box::new(f)));
    wake();
}

/// Run `f` with the app on the UI thread; callable from any thread.
pub fn post(f: impl FnOnce(&mut App) + Send + 'static) {
    REMOTE_Q.lock().unwrap().push_back(Box::new(f));
    wake();
}

fn wake() {
    let h = MSG_HWND.load(Ordering::Relaxed);
    if h != 0 {
        unsafe {
            let _ = PostMessageW(Some(HWND(h as _)), WM_APP_WAKE, WPARAM(0), LPARAM(0));
        }
    }
}

pub fn msg_hwnd() -> HWND {
    HWND(MSG_HWND.load(Ordering::Relaxed) as _)
}

/// Borrow the app if nobody else holds it. Returns None when called re-entrantly.
pub fn try_with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    let r = APP.with(|a| match a.try_borrow_mut() {
        Ok(mut g) => g.as_mut().map(f),
        Err(_) => None,
    });
    if r.is_some() && queue_nonempty() {
        wake();
    }
    r
}

fn queue_nonempty() -> bool {
    LOCAL_Q.with(|q| !q.borrow().is_empty()) || !REMOTE_Q.lock().unwrap().is_empty()
}

fn drain_queue() {
    loop {
        if APP.with(|a| a.try_borrow_mut().is_err()) {
            return; // re-entrant (modal loop inside app code); try_with_app re-wakes when done
        }
        let job: Option<LocalJob> = LOCAL_Q.with(|q| q.borrow_mut().pop_front());
        if let Some(j) = job {
            APP.with(|a| {
                if let Some(app) = a.borrow_mut().as_mut() {
                    j(app)
                }
            });
            continue;
        }
        let rjob = REMOTE_Q.lock().unwrap().pop_front();
        if let Some(j) = rjob {
            APP.with(|a| {
                if let Some(app) = a.borrow_mut().as_mut() {
                    j(app)
                }
            });
            continue;
        }
        break;
    }
}

pub struct Profile {
    pub id: String,
    pub dir: PathBuf,
    pub settings: Settings,
    pub settings_dirty: bool,
    pub store: Option<crate::store::Store>,
}

impl Profile {
    pub fn settings_path(&self) -> PathBuf {
        self.dir.join("settings.json")
    }
    pub fn db(&self) -> Option<&crate::store::Store> {
        self.store.as_ref()
    }
}

pub enum EnvState {
    None,
    Creating,
    Ready(ICoreWebView2Environment),
    Failed(String),
}

pub struct App {
    pub env: EnvState,
    env_waiters: Vec<LocalJob>,
    pub windows: Vec<BrowserWindow>,
    pub profiles: HashMap<String, Profile>,
    pub local: LocalState,
    pub local_dirty: bool,
    pub next_id: u32,
    pub quitting: bool,
    pub system_dark: bool,
    pub env_restarts: u32,
    pub restart_on_exit: bool,
    pub ticks: u64,
    pub update_relaunch: bool,
}

impl App {
    pub fn next_id(&mut self) -> u32 {
        self.next_id += 1;
        self.next_id
    }

    pub fn env(&self) -> Option<ICoreWebView2Environment> {
        match &self.env {
            EnvState::Ready(e) => Some(e.clone()),
            _ => None,
        }
    }

    /// Run `f` once the WebView2 environment exists (creating it if needed).
    pub fn with_env(&mut self, f: impl FnOnce(&mut App) + 'static) {
        match self.env {
            EnvState::Ready(_) => f(self),
            EnvState::Creating => self.env_waiters.push(Box::new(f)),
            EnvState::None | EnvState::Failed(_) => {
                self.env_waiters.push(Box::new(f));
                self.create_env();
            }
        }
    }

    fn create_env(&mut self) {
        self.env = EnvState::Creating;
        let data = util::user_data_root().join("EBWebView");
        let last = self.local.last_profile.clone();
        let block3p = self.settings(&last).block_third_party_cookies;
        let args = crate::webview::browser_arguments(&self.local, block3p);
        crate::webview::prepare_engine_prefs(&data, self);
        crate::webview::create_environment(data, args, |res| {
            defer(move |app| match res {
                Ok(env) => {
                    logf!("WebView2 environment ready ({})", crate::webview::runtime_version(&env));
                    crate::webview::on_environment_created(app, &env);
                    app.env = EnvState::Ready(env);
                    let waiters = std::mem::take(&mut app.env_waiters);
                    for w in waiters {
                        w(app);
                    }
                }
                Err(e) => {
                    logf!("WebView2 environment failed: {e}");
                    app.env = EnvState::Failed(e.clone());
                    app.env_waiters.clear();
                    show_fatal(&format!(
                        "Solus could not start its Chromium engine (Microsoft Edge WebView2 Runtime).\n\n{e}\n\nReinstalling Solus will repair the runtime."
                    ));
                    app.quit();
                }
            })
        });
    }

    /// Called when the browser process died. Recreate the environment and reload every window.
    pub fn on_browser_process_exited(&mut self) {
        if self.quitting {
            return;
        }
        self.env_restarts += 1;
        logf!("browser process exited unexpectedly (restart #{})", self.env_restarts);
        for w in self.windows.iter_mut() {
            w.drop_all_views();
        }
        self.env = EnvState::None;
        if self.env_restarts > 5 {
            show_fatal("The Chromium engine keeps crashing. Solus will close; your tabs are saved and will reopen next time.");
            self.quit();
            return;
        }
        let ids: Vec<u32> = self.windows.iter().map(|w| w.id).collect();
        self.with_env(move |app| {
            for id in ids {
                app.attach_ui(id);
            }
        });
    }

    pub fn profile(&mut self, id: &str) -> &mut Profile {
        if !self.profiles.contains_key(id) {
            let dir = util::user_data_root().join("Profiles").join(id);
            let _ = std::fs::create_dir_all(&dir);
            crate::backup::apply_pending_restore(&dir, id);
            let settings = Settings::load(&dir.join("settings.json"));
            let store = match crate::store::Store::open(&dir) {
                Ok(s) => Some(s),
                Err(e) => {
                    logf!("profile {id}: database unavailable: {e}");
                    None
                }
            };
            self.profiles.insert(
                id.to_string(),
                Profile { id: id.to_string(), dir, settings, settings_dirty: false, store },
            );
            crate::hooks::profile_loaded(self, id);
        }
        self.profiles.get_mut(id).unwrap()
    }

    pub fn settings(&mut self, profile: &str) -> &Settings {
        &self.profile(profile).settings
    }

    pub fn win(&self, id: u32) -> Option<&BrowserWindow> {
        self.windows.iter().find(|w| w.id == id)
    }
    pub fn win_mut(&mut self, id: u32) -> Option<&mut BrowserWindow> {
        self.windows.iter_mut().find(|w| w.id == id)
    }
    pub fn win_by_hwnd(&self, hwnd: HWND) -> Option<u32> {
        self.windows.iter().find(|w| w.hwnd == hwnd).map(|w| w.id)
    }

    /// The window that should receive things like links from other apps.
    pub fn last_active_window(&self, incognito: bool) -> Option<u32> {
        self.windows
            .iter()
            .filter(|w| w.incognito == incognito && !w.closing)
            .max_by_key(|w| w.last_focus)
            .map(|w| w.id)
    }

    pub fn new_window(&mut self, profile: &str, incognito: bool, urls: Vec<String>, restore: bool) -> u32 {
        let id = self.next_id();
        let placement = self.local.placement.clone();
        let w = BrowserWindow::create(id, profile, incognito, &placement, self.windows.len());
        self.windows.push(w);
        let profile = profile.to_string();
        self.with_env(move |app| {
            app.attach_ui(id);
            app.initial_tabs(id, &profile, incognito, urls, restore);
        });
        id
    }

    fn initial_tabs(&mut self, wid: u32, profile: &str, incognito: bool, urls: Vec<String>, restore: bool) {
        let mut opened = 0;
        if restore && !incognito {
            opened = crate::tab::restore_session(self, wid, profile);
        }
        for u in urls {
            let url = crate::tab::fixup_input(self, profile, &u);
            self.open_tab(wid, &url, OpenOpts { activate: true, ..Default::default() });
            opened += 1;
        }
        if opened == 0 {
            let start = {
                let s = self.settings(profile);
                if s.startup == "urls" && !s.startup_urls.is_empty() && !incognito {
                    s.startup_urls.clone()
                } else {
                    vec![]
                }
            };
            for u in &start {
                self.open_tab(wid, u, OpenOpts { activate: opened == 0, ..Default::default() });
                opened += 1;
            }
        }
        if opened == 0 {
            let first_run = !self.local.onboarding_done && !incognito;
            let url = if first_run { "solus://welcome/" } else { crate::tab::NEWTAB };
            self.open_tab(wid, url, OpenOpts { activate: true, ..Default::default() });
        }
    }

    /// Handle a launch from another process (e.g. a link clicked in another app).
    pub fn handle_remote_args(&mut self, args: Args) {
        let profile = self.local.last_profile.clone();
        let target = if args.new_window { None } else { self.last_active_window(args.incognito) };
        match target {
            Some(wid) if !args.urls.is_empty() => {
                for u in &args.urls {
                    let url = crate::tab::fixup_input(self, &profile, u);
                    self.open_tab(wid, &url, OpenOpts { activate: true, ..Default::default() });
                }
                self.show_window(wid);
            }
            Some(wid) => self.show_window(wid),
            None => {
                self.new_window(&profile, args.incognito, args.urls, false);
            }
        }
    }

    pub fn show_window(&mut self, wid: u32) {
        if let Some(w) = self.win_mut(wid) {
            w.show_and_focus();
        }
        if self.win(wid).map(|w| w.ui.is_none()).unwrap_or(false) {
            self.with_env(move |app| app.attach_ui(wid));
        }
        self.ensure_window_views(wid);
    }

    pub fn close_window(&mut self, wid: u32) {
        let Some(idx) = self.windows.iter().position(|w| w.id == wid) else { return };
        let is_last_normal = !self.windows[idx].incognito
            && self.windows.iter().filter(|w| !w.incognito && !w.closing).count() == 1;
        if is_last_normal && self.local.close_to_tray && !self.quitting {
            // keep running in the tray; tabs are unloaded later by the performance manager
            crate::tab::save_session(self, wid);
            self.windows[idx].hide_to_tray();
            crate::window::ensure_tray(self);
            return;
        }
        if !self.windows[idx].incognito {
            crate::tab::save_session(self, wid);
        }
        let mut w = self.windows.remove(idx);
        w.destroy();
        if self.windows.is_empty() && !self.local.close_to_tray {
            self.quit();
        }
    }

    pub fn quit(&mut self) {
        if self.quitting {
            return;
        }
        self.quitting = true;
        logf!("quitting");
        let ids: Vec<u32> = self.windows.iter().filter(|w| !w.incognito).map(|w| w.id).collect();
        for id in ids {
            crate::tab::save_session(self, id);
        }
        self.flush();
        self.local.clean_exit = true;
        self.local.save();
        for mut w in std::mem::take(&mut self.windows) {
            w.destroy();
        }
        crate::window::remove_tray();
        crate::privacy::flush_stats(self);
        for p in self.profiles.values() {
            if let Some(db) = &p.store {
                db.checkpoint();
            }
        }
        crate::update::apply_on_exit(self.update_relaunch);
        self.env = EnvState::None;
        unsafe { PostQuitMessage(0) };
    }

    /// Persist anything dirty. Called on a timer and at exit.
    pub fn flush(&mut self) {
        for p in self.profiles.values_mut() {
            if p.settings_dirty {
                p.settings.save(&p.settings_path());
                p.settings_dirty = false;
            }
        }
        if self.local_dirty {
            self.local.save();
            self.local_dirty = false;
        }
    }

    fn on_tick(&mut self) {
        self.ticks += 1;
        if self.ticks % 3 == 0 {
            crate::perf::tick(self);
        }
        crate::productivity::check_reminders(self);
        if self.ticks % 12 == 0 {
            crate::privacy::flush_stats(self);
        }
        if self.ticks % 60 == 7 {
            crate::sync::tick(self);
            crate::update::maybe_check(self);
            crate::privacy::maybe_update_lists(self);
        }
        if self.ticks % 120 == 5 {
            crate::backup::maybe_auto_backup(self);
        }
        // periodic work: autosave sessions
        let ids: Vec<u32> = self.windows.iter().filter(|w| !w.incognito && w.session_dirty).map(|w| w.id).collect();
        for id in ids {
            crate::tab::save_session(self, id);
        }
        self.flush();
    }

    pub fn set_system_dark(&mut self, dark: bool) {
        if self.system_dark != dark {
            self.system_dark = dark;
            let ids: Vec<u32> = self.windows.iter().map(|w| w.id).collect();
            for id in ids {
                crate::bridge::push_theme(self, id);
            }
        }
    }
}

pub fn show_fatal(msg: &str) {
    let m = WStr::new(msg);
    unsafe {
        MessageBoxW(None, m.pcwstr(), w!("Solus"), MB_OK | MB_ICONERROR);
    }
}

unsafe extern "system" fn msg_wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_APP_WAKE => {
            drain_queue();
            LRESULT(0)
        }
        WM_TIMER => {
            let id = wp.0;
            defer(move |app| match id {
                TIMER_TICK => app.on_tick(),
                _ => {}
            });
            LRESULT(0)
        }
        WM_COPYDATA => {
            let cds = &*(lp.0 as *const COPYDATASTRUCT);
            if cds.dwData == 0x5301 && !cds.lpData.is_null() {
                let bytes = std::slice::from_raw_parts(cds.lpData as *const u8, cds.cbData as usize);
                if let Ok(args) = serde_json::from_slice::<Args>(bytes) {
                    defer(move |app| app.handle_remote_args(args));
                }
            }
            LRESULT(1)
        }
        WM_APP_TRAY => {
            crate::window::on_tray_message(lp.0 as u32 & 0xffff);
            LRESULT(0)
        }
        WM_SETTINGCHANGE => {
            let dark = crate::window::system_prefers_dark();
            defer(move |app| app.set_system_dark(dark));
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        WM_POWERBROADCAST => DefWindowProcW(hwnd, msg, wp, lp),
        _ => {
            if msg == crate::window::taskbar_created_msg() {
                defer(|app| crate::window::readd_tray(app));
            }
            DefWindowProcW(hwnd, msg, wp, lp)
        }
    }
}

fn create_msg_window() -> HWND {
    unsafe {
        let hinst = GetModuleHandleW(None).unwrap();
        let wc = WNDCLASSW {
            lpfnWndProc: Some(msg_wndproc),
            hInstance: hinst.into(),
            lpszClassName: w!("SolusMsgWindow"),
            ..Default::default()
        };
        RegisterClassW(&wc);
        let title = WStr::new(&format!("SolusMsg-{}", crate::instance_key()));
        // A hidden top-level window (not message-only) so FindWindow and broadcast messages reach it.
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("SolusMsgWindow"),
            title.pcwstr(),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(hinst.into()),
            None,
        )
        .unwrap()
    }
}

pub fn run(args: Args) {
    crate::window::register_classes();
    let msg = create_msg_window();
    MSG_HWND.store(msg.0 as isize, Ordering::Relaxed);

    let mut local = LocalState::load();
    let crashed = !local.clean_exit;
    if crashed {
        logf!("previous session did not exit cleanly; restoring tabs");
    }
    local.clean_exit = false;
    local.save();

    let app = App {
        env: EnvState::None,
        env_waiters: vec![],
        windows: vec![],
        profiles: HashMap::new(),
        local,
        local_dirty: false,
        next_id: 0,
        quitting: false,
        system_dark: crate::window::system_prefers_dark(),
        env_restarts: 0,
        restart_on_exit: false,
        ticks: 0,
        update_relaunch: false,
    };
    APP.with(|a| *a.borrow_mut() = Some(app));

    unsafe {
        SetTimer(Some(msg), TIMER_TICK, 5000, None);
    }
    if let Some(port) = args.automation_port {
        crate::automation::start(port);
    }

    try_with_app(|app| {
        let profile = args.profile.clone().unwrap_or_else(|| app.local.last_profile.clone());
        app.profile(&profile);
        if args.background {
            crate::window::ensure_tray(app);
        } else {
            let restore = !args.no_restore && (crashed || app.settings(&profile).startup == "restore");
            app.new_window(&profile, args.incognito, args.urls.clone(), restore);
        }
        // warm the engine early so the first tab appears quickly
        app.with_env(|_| {});
    });

    unsafe {
        let mut m = MSG::default();
        while GetMessageW(&mut m, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&m);
            DispatchMessageW(&m);
        }
    }
    let mut restart = false;
    APP.with(|a| {
        if let Some(app) = a.borrow_mut().as_mut() {
            if !app.quitting {
                app.quit();
            }
            restart = app.restart_on_exit;
        }
        *a.borrow_mut() = None;
    });
    if restart {
        crate::platform::spawn_self(&[]);
    }
    logf!("exited");
    // Everything is saved. Exit now rather than letting thread-local WebView2 objects be released
    // after the message loop has stopped, which can block process shutdown.
    // ExitProcess would run DLL detach code (WebView2 loader, COM) that can deadlock now that the
    // message loop is gone, so end the process directly.
    unsafe {
        let _ = windows::Win32::System::Threading::TerminateProcess(windows::Win32::System::Threading::GetCurrentProcess(), 0);
    }
    std::process::exit(0);
}

/// Convenience used by COM callbacks: cast and log failures instead of panicking.
pub fn cast<T: Interface>(x: &impl Interface) -> Option<T> {
    x.cast::<T>().ok()
}
