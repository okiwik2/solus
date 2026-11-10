//! Privacy & security: ad/tracker blocking (adblock-rust with EasyList/EasyPrivacy), cosmetic
//! filtering, per-site permissions, clearing site data / browsing data and the dashboard data.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use rusqlite::params;
use serde_json::{json, Value};
use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::*;
use windows::core::{Interface, PWSTR};

use crate::app::{defer, post, App};
use crate::bridge::{self, ok, Ctx, Reply};
use crate::util::{self, now_ms, WStr};

// ---------------------------------------------------------------- filter engine

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Level {
    Off,
    Balanced,
    Strict,
}

impl Level {
    pub fn parse(s: &str) -> Level {
        match s {
            "off" => Level::Off,
            "strict" => Level::Strict,
            _ => Level::Balanced,
        }
    }
    fn lists(self) -> &'static [&'static str] {
        match self {
            Level::Off => &[],
            Level::Balanced => &["easylist.txt", "easyprivacy.txt"],
            Level::Strict => &["easylist.txt", "easyprivacy.txt", "ublock-privacy.txt", "fanboy-annoyance.txt"],
        }
    }
    fn name(self) -> &'static str {
        match self {
            Level::Off => "off",
            Level::Balanced => "balanced",
            Level::Strict => "strict",
        }
    }
}

pub const LIST_URLS: &[(&str, &str)] = &[
    ("easylist.txt", "https://easylist.to/easylist/easylist.txt"),
    ("easyprivacy.txt", "https://easylist.to/easylist/easyprivacy.txt"),
    ("ublock-privacy.txt", "https://raw.githubusercontent.com/uBlockOrigin/uAssets/master/filters/privacy.txt"),
    ("fanboy-annoyance.txt", "https://secure.fanboy.co.nz/fanboy-annoyance.txt"),
];

thread_local! {
    static ENGINES: RefCell<HashMap<Level, adblock::Engine>> = RefCell::new(HashMap::new());
    static BUILDING: RefCell<HashSet<Level>> = RefCell::new(HashSet::new());
    /// webview pointer -> (window, tab, level, incognito)
    static VIEWS: RefCell<HashMap<usize, (u32, u32, Level, bool)>> = RefCell::new(HashMap::new());
    static TAB_URL: RefCell<HashMap<u32, String>> = RefCell::new(HashMap::new());
    static BLOCKED: RefCell<HashMap<u32, u32>> = RefCell::new(HashMap::new());
    static LAST_EMIT: RefCell<HashMap<u32, i64>> = RefCell::new(HashMap::new());
    static STATS: RefCell<HashMap<(String, i64, String), u32>> = RefCell::new(HashMap::new());
    static EXEMPT: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
    static ENV: RefCell<Option<ICoreWebView2Environment>> = const { RefCell::new(None) };
    static ENABLED_ANY: Cell<bool> = const { Cell::new(false) };
}

fn lists_dir_user() -> PathBuf {
    util::user_data_root().join("Filter Lists")
}

fn lists_dir_bundled() -> PathBuf {
    let beside = util::exe_dir().join("lists");
    if beside.is_dir() {
        return beside;
    }
    let mut p = util::exe_dir();
    for _ in 0..4 {
        if p.join("res").join("lists").is_dir() {
            return p.join("res").join("lists");
        }
        if !p.pop() {
            break;
        }
    }
    beside
}

/// The newest copy of a list: downloaded update if present, otherwise the bundled snapshot.
fn list_path(name: &str) -> PathBuf {
    let u = lists_dir_user().join(name);
    if u.exists() {
        u
    } else {
        lists_dir_bundled().join(name)
    }
}

fn engine_cache_key(level: Level) -> String {
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    h.update(b"adblock-0.13-solus-1");
    for l in level.lists() {
        let p = list_path(l);
        let m = std::fs::metadata(&p).ok();
        h.update(p.to_string_lossy().as_bytes());
        h.update(m.as_ref().map(|m| m.len()).unwrap_or(0).to_le_bytes());
        h.update(m.and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0).to_le_bytes());
    }
    h.finalize()[..8].iter().map(|b| format!("{b:02x}")).collect()
}

/// Make sure the engine for `level` is (being) loaded. Loading never blocks the UI thread for long:
/// a cached serialized engine is deserialized directly, otherwise it is compiled on a worker.
pub fn ensure_engine(level: Level) {
    if level == Level::Off || ENGINES.with(|e| e.borrow().contains_key(&level)) {
        return;
    }
    if !BUILDING.with(|b| b.borrow_mut().insert(level)) {
        return;
    }
    let key = engine_cache_key(level);
    let cache = lists_dir_user().join(format!("engine-{}-{key}.dat", level.name()));
    std::thread::spawn(move || {
        let t0 = std::time::Instant::now();
        let bytes = match std::fs::read(&cache) {
            Ok(b) => b,
            Err(_) => {
                let mut text = String::new();
                for l in level.lists() {
                    if let Ok(t) = std::fs::read_to_string(list_path(l)) {
                        text.push_str(&t);
                        text.push('\n');
                    }
                }
                let engine = adblock::Engine::new_with_list_text(text);
                let b = engine.serialize();
                let _ = std::fs::create_dir_all(lists_dir_user());
                // drop stale caches for this level
                if let Ok(rd) = std::fs::read_dir(lists_dir_user()) {
                    for e in rd.flatten() {
                        let n = e.file_name().to_string_lossy().to_string();
                        if n.starts_with(&format!("engine-{}-", level.name())) {
                            let _ = std::fs::remove_file(e.path());
                        }
                    }
                }
                let _ = util::write_atomic(&cache, &b);
                b
            }
        };
        let built = t0.elapsed();
        post(move |_app| {
            let mut engine = adblock::Engine::default();
            match engine.deserialize(&bytes) {
                Ok(()) => {
                    ENGINES.with(|e| e.borrow_mut().insert(level, engine));
                    logf!("blocking engine '{}' ready ({} KB, prepared in {:?})", level.name(), bytes.len() / 1024, built);
                }
                Err(e) => {
                    logf!("blocking engine load failed: {e:?}");
                    let _ = std::fs::remove_file(&cache);
                }
            }
            BUILDING.with(|b| b.borrow_mut().remove(&level));
        });
    });
}

pub fn reconfigure(app: &mut App) {
    EXEMPT.with(|e| {
        let mut e = e.borrow_mut();
        e.clear();
        for p in app.profiles.values() {
            for s in &p.settings.site_exceptions {
                e.insert(util::site_of_host(s));
            }
        }
    });
    let mut levels = HashSet::new();
    for p in app.profiles.values() {
        levels.insert(Level::parse(&p.settings.blocking_level));
    }
    for l in levels {
        ensure_engine(l);
    }
    // update every live tab's level and request filter
    let profiles: HashMap<String, Level> =
        app.profiles.iter().map(|(k, p)| (k.clone(), Level::parse(&p.settings.blocking_level))).collect();
    for w in app.windows.iter() {
        let level = profiles.get(&w.profile).copied().unwrap_or(Level::Balanced);
        for t in w.tabs.iter() {
            if let Some(v) = &t.view {
                set_view_level(&v.wv, w.id, t.id, level, w.incognito);
            }
        }
    }
}

fn set_view_level(wv: &ICoreWebView2, wid: u32, tid: u32, level: Level, incognito: bool) {
    let key = wv.as_raw() as usize;
    let prev = VIEWS.with(|v| v.borrow_mut().insert(key, (wid, tid, level, incognito)).map(|x| x.2));
    let filter = WStr::new("*");
    unsafe {
        let was_on = prev.map(|l| l != Level::Off).unwrap_or(false);
        let on = level != Level::Off;
        if on && !was_on {
            let _ = wv.AddWebResourceRequestedFilter(filter.pcwstr(), COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL);
        } else if !on && was_on {
            let _ = wv.RemoveWebResourceRequestedFilter(filter.pcwstr(), COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL);
        }
    }
    if level != Level::Off {
        ENABLED_ANY.with(|c| c.set(true));
    }
}

/// Called when a content tab's webview is created.
pub fn attach(app: &mut App, wv: &ICoreWebView2, wid: u32, tid: u32) {
    let Some(w) = app.win(wid) else { return };
    let (profile, incognito) = (w.profile.clone(), w.incognito);
    let level = Level::parse(&app.settings(&profile).blocking_level);
    ensure_engine(level);
    if ENV.with(|e| e.borrow().is_none()) {
        ENV.with(|e| *e.borrow_mut() = app.env());
    }
    set_view_level(wv, wid, tid, level, incognito);
    unsafe {
        if let Ok(w2) = wv.cast::<ICoreWebView2_2>() {
            let h = DOMContentLoadedEventHandler::create(Box::new(move |_, _| {
                defer(move |app| cosmetic(app, wid, tid));
                Ok(())
            }));
            let mut tok = Default::default();
            let _ = w2.add_DOMContentLoaded(&h, &mut tok);
        }
    }
    install_permissions(wv, wid, tid);
}

pub fn detach(wv: &ICoreWebView2) {
    VIEWS.with(|v| v.borrow_mut().remove(&(wv.as_raw() as usize)));
}

/// Main-frame navigation began: new page, new counters.
pub fn on_nav_start(tid: u32, uri: &str) {
    TAB_URL.with(|m| m.borrow_mut().insert(tid, uri.to_string()));
    BLOCKED.with(|m| m.borrow_mut().insert(tid, 0));
}

pub fn blocked_count(tid: u32) -> u32 {
    BLOCKED.with(|m| m.borrow().get(&tid).copied().unwrap_or(0))
}

fn request_type(ctx: COREWEBVIEW2_WEB_RESOURCE_CONTEXT) -> &'static str {
    match ctx {
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_DOCUMENT => "subdocument",
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_STYLESHEET => "stylesheet",
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_IMAGE => "image",
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_MEDIA => "media",
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_FONT => "font",
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_SCRIPT => "script",
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_XML_HTTP_REQUEST => "xmlhttprequest",
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_FETCH => "xmlhttprequest",
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_WEBSOCKET => "websocket",
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_PING => "ping",
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_CSP_VIOLATION_REPORT => "csp_report",
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_MANIFEST => "other",
        _ => "other",
    }
}

/// Decide whether to block a web request. Runs synchronously inside WebResourceRequested.
pub fn check_request(wv: &ICoreWebView2, args: &ICoreWebView2WebResourceRequestedEventArgs, uri: &str) -> windows::core::Result<()> {
    let Some((wid, tid, level, incognito)) = VIEWS.with(|v| v.borrow().get(&(wv.as_raw() as usize)).copied()) else { return Ok(()) };
    if level == Level::Off {
        return Ok(());
    }
    let source = TAB_URL.with(|m| m.borrow().get(&tid).cloned()).unwrap_or_default();
    if source.is_empty() || source.starts_with("solus:") {
        return Ok(());
    }
    let site = util::site_of(&source);
    if EXEMPT.with(|e| e.borrow().contains(&site)) {
        return Ok(());
    }
    let mut ctx = COREWEBVIEW2_WEB_RESOURCE_CONTEXT_OTHER;
    unsafe {
        let _ = args.ResourceContext(&mut ctx);
    }
    if ctx == COREWEBVIEW2_WEB_RESOURCE_CONTEXT_DOCUMENT && uri == source {
        return Ok(()); // never block the page itself
    }
    let blocked = ENGINES.with(|e| {
        let e = e.borrow();
        let Some(engine) = e.get(&level) else { return false };
        match adblock::request::Request::new(uri, &source, request_type(ctx), "get") {
            Ok(req) => engine.check_network_request(&req).should_block(),
            Err(_) => false,
        }
    });
    if !blocked {
        return Ok(());
    }
    let env = ENV.with(|e| e.borrow().clone());
    if let Some(env) = env {
        unsafe {
            let reason = WStr::new("Blocked by Solus");
            let hdr = WStr::new("Content-Type: text/plain");
            let resp = env.CreateWebResourceResponse(None, 403, reason.pcwstr(), hdr.pcwstr())?;
            args.SetResponse(&resp)?;
        }
    }
    let n = BLOCKED.with(|m| {
        let mut m = m.borrow_mut();
        let c = m.entry(tid).or_insert(0);
        *c += 1;
        *c
    });
    if !incognito {
        let day = now_ms() / 86_400_000;
        STATS.with(|s| *s.borrow_mut().entry((String::new(), day, site.clone())).or_insert(0) += 1);
    }
    let now = now_ms();
    let due = LAST_EMIT.with(|l| {
        let mut l = l.borrow_mut();
        let last = l.get(&tid).copied().unwrap_or(0);
        if now - last > 400 {
            l.insert(tid, now);
            true
        } else {
            false
        }
    });
    if due {
        defer(move |app| {
            if let Some(t) = app.win_mut(wid).and_then(|w| w.tab_mut(tid)) {
                t.blocked = n;
            }
            bridge::emit(app, wid, "blocked", json!({"tab": tid, "count": blocked_count(tid)}));
        });
    }
    Ok(())
}

/// After load: final blocked count to the UI, cosmetic rules.
pub fn page_loaded(app: &mut App, wid: u32, tid: u32) {
    let n = blocked_count(tid);
    if let Some(t) = app.win_mut(wid).and_then(|w| w.tab_mut(tid)) {
        t.blocked = n;
    }
    bridge::emit(app, wid, "blocked", json!({"tab": tid, "count": n}));
}

/// Hide ad placeholders: host-specific selectors plus generic class/id selectors present on the page.
fn cosmetic(app: &mut App, wid: u32, tid: u32) {
    let Some(w) = app.win(wid) else { return };
    let Some(t) = w.tab(tid) else { return };
    let Some(v) = &t.view else { return };
    if !t.url.starts_with("http") {
        return;
    }
    let level = VIEWS.with(|m| m.borrow().get(&(v.wv.as_raw() as usize)).map(|x| x.2)).unwrap_or(Level::Off);
    if level == Level::Off || EXEMPT.with(|e| e.borrow().contains(&util::site_of(&t.url))) {
        return;
    }
    let url = t.url.clone();
    let wv = v.wv.clone();
    let res = ENGINES.with(|e| e.borrow().get(&level).map(|eng| eng.url_cosmetic_resources(&url)));
    let Some(res) = res else { return };
    let mut selectors: Vec<String> = res.hide_selectors.iter().cloned().collect();
    inject_css(&wv, &selectors);
    if res.generichide {
        return;
    }
    let exceptions = res.exceptions.clone();
    const COLLECT: &str = "(() => { const c = new Set(), i = new Set(); for (const el of document.querySelectorAll('[class],[id]')) { \
        if (el.id) i.add(el.id); for (const k of el.classList) c.add(k); if (c.size + i.size > 6000) break; } \
        return JSON.stringify({c: [...c], i: [...i]}); })()";
    let wv2 = wv.clone();
    crate::webview::exec_script(&wv2, COLLECT, move |_app, r| {
        let Some(r) = r else { return };
        let inner: String = serde_json::from_str(&r).unwrap_or_default();
        let Ok(v) = serde_json::from_str::<Value>(&inner) else { return };
        let classes: Vec<String> = v["c"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
        let ids: Vec<String> = v["i"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
        let generic = ENGINES.with(|e| e.borrow().get(&level).map(|eng| eng.hidden_class_id_selectors(&classes, &ids, &exceptions)));
        if let Some(g) = generic {
            selectors.clear();
            selectors.extend(g);
            inject_css(&wv, &selectors);
        }
    });
}

fn inject_css(wv: &ICoreWebView2, selectors: &[String]) {
    if selectors.is_empty() {
        return;
    }
    // invalid selectors would void a whole rule, so emit one rule per selector chunk
    let mut css = String::new();
    for chunk in selectors.chunks(1) {
        css.push_str(&chunk.join(","));
        css.push_str("{display:none !important}\n");
    }
    let script = format!(
        "(() => {{ const s = document.createElement('style'); s.dataset.solus = 'cf'; s.textContent = {}; (document.head || document.documentElement).appendChild(s); }})()",
        serde_json::to_string(&css).unwrap()
    );
    crate::webview::exec_script(wv, &script, |_, _| {});
}

/// Persist blocking statistics (called from the periodic tick).
pub fn flush_stats(app: &mut App) {
    let pending: Vec<((String, i64, String), u32)> = STATS.with(|s| s.borrow_mut().drain().collect());
    if pending.is_empty() {
        return;
    }
    let profile = app.local.last_profile.clone();
    let Some(db) = app.profile(&profile).db() else { return };
    for ((_, day, site), n) in pending {
        let _ = db.conn.execute(
            "INSERT INTO blocked_stats(day,site,count) VALUES(?1,?2,?3) ON CONFLICT(day,site) DO UPDATE SET count=count+?3",
            params![day, site, n],
        );
    }
}

/// Refresh filter lists every 4 days (background thread), then rebuild engines.
pub fn maybe_update_lists(app: &mut App) {
    let last = app.profile(&app.local.last_profile.clone()).db().and_then(|d| d.kv_get("lists_updated")).and_then(|s| s.parse::<i64>().ok()).unwrap_or(0);
    if now_ms() - last < 4 * 86_400_000 {
        return;
    }
    update_lists(app.local.last_profile.clone());
}

pub fn update_lists(profile: String) {
    std::thread::spawn(move || {
        let _ = std::fs::create_dir_all(lists_dir_user());
        let mut updated = 0;
        for (name, url) in LIST_URLS {
            match crate::net::get_bytes(url, 16 * 1024 * 1024) {
                Ok(b) if b.len() > 10_000 && String::from_utf8_lossy(&b[..200.min(b.len())]).contains('!') => {
                    if util::write_atomic(&lists_dir_user().join(name), &b).is_ok() {
                        updated += 1;
                    }
                }
                Ok(_) => logf!("filter list {name}: unexpected content, keeping the previous copy"),
                Err(e) => logf!("filter list {name}: {e}"),
            }
        }
        post(move |app| {
            if let Some(db) = app.profile(&profile).db() {
                db.kv_set("lists_updated", &now_ms().to_string());
            }
            if updated > 0 {
                logf!("updated {updated} filter lists; rebuilding");
                ENGINES.with(|e| e.borrow_mut().clear());
                reconfigure(app);
            }
        });
    });
}

// ---------------------------------------------------------------- permissions

pub const PERMISSIONS: &[(&str, &str)] = &[
    ("geolocation", "Know your location"),
    ("camera", "Use your camera"),
    ("microphone", "Use your microphone"),
    ("notifications", "Show notifications"),
    ("clipboard", "See text and images copied to the clipboard"),
    ("sensors", "Use motion sensors"),
    ("downloads", "Download multiple files automatically"),
    ("files", "Edit files on your device"),
    ("autoplay", "Play media with sound automatically"),
    ("fonts", "Use fonts installed on your computer"),
    ("midi", "Control MIDI devices"),
    ("windows", "Manage windows on your displays"),
];

fn kind_name(k: COREWEBVIEW2_PERMISSION_KIND) -> &'static str {
    match k {
        COREWEBVIEW2_PERMISSION_KIND_GEOLOCATION => "geolocation",
        COREWEBVIEW2_PERMISSION_KIND_CAMERA => "camera",
        COREWEBVIEW2_PERMISSION_KIND_MICROPHONE => "microphone",
        COREWEBVIEW2_PERMISSION_KIND_NOTIFICATIONS => "notifications",
        COREWEBVIEW2_PERMISSION_KIND_CLIPBOARD_READ => "clipboard",
        COREWEBVIEW2_PERMISSION_KIND_OTHER_SENSORS => "sensors",
        COREWEBVIEW2_PERMISSION_KIND_MULTIPLE_AUTOMATIC_DOWNLOADS => "downloads",
        COREWEBVIEW2_PERMISSION_KIND_FILE_READ_WRITE => "files",
        COREWEBVIEW2_PERMISSION_KIND_AUTOPLAY => "autoplay",
        COREWEBVIEW2_PERMISSION_KIND_LOCAL_FONTS => "fonts",
        COREWEBVIEW2_PERMISSION_KIND_MIDI_SYSTEM_EXCLUSIVE_MESSAGES => "midi",
        COREWEBVIEW2_PERMISSION_KIND_WINDOW_MANAGEMENT => "windows",
        _ => "other",
    }
}

fn label(kind: &str) -> &'static str {
    PERMISSIONS.iter().find(|p| p.0 == kind).map(|p| p.1).unwrap_or("Use a device feature")
}

struct Pending {
    args: ICoreWebView2PermissionRequestedEventArgs,
    deferral: ICoreWebView2Deferral,
    wid: u32,
    tid: u32,
    origin: String,
    kind: String,
}

thread_local! {
    static PENDING: RefCell<HashMap<u32, Pending>> = RefCell::new(HashMap::new());
    static NEXT_REQ: Cell<u32> = const { Cell::new(0) };
    /// incognito decisions live only in memory
    static SESSION: RefCell<HashMap<(String, String), String>> = RefCell::new(HashMap::new());
    static USED: RefCell<HashSet<(String, String)>> = RefCell::new(HashSet::new());
}

fn install_permissions(wv: &ICoreWebView2, wid: u32, tid: u32) {
    let h = PermissionRequestedEventHandler::create(Box::new(move |_, args| {
        let Some(args) = args else { return Ok(()) };
        unsafe {
            let mut p = PWSTR::null();
            let _ = args.Uri(&mut p);
            let uri = take_pwstr(p);
            let mut k = COREWEBVIEW2_PERMISSION_KIND_UNKNOWN_PERMISSION;
            let _ = args.PermissionKind(&mut k);
            if let Ok(a3) = args.cast::<ICoreWebView2PermissionRequestedEventArgs3>() {
                let _ = a3.SetSavesInProfile(false);
            }
            let deferral = args.GetDeferral()?;
            let kind = kind_name(k).to_string();
            let origin = util::origin_of(&uri);
            defer(move |app| decide(app, Pending { args, deferral, wid, tid, origin, kind }));
        }
        Ok(())
    }));
    let mut tok = Default::default();
    unsafe {
        let _ = wv.add_PermissionRequested(&h, &mut tok);
    }
}

fn stored_state(app: &mut App, profile: &str, incognito: bool, origin: &str, kind: &str) -> String {
    if incognito {
        if let Some(s) = SESSION.with(|m| m.borrow().get(&(origin.to_string(), kind.to_string())).cloned()) {
            return s;
        }
    }
    app.profile(profile)
        .db()
        .and_then(|d| {
            d.conn
                .query_row("SELECT state FROM permissions WHERE origin=?1 AND kind=?2", params![origin, kind], |r| r.get::<_, String>(0))
                .ok()
        })
        .unwrap_or_else(|| "default".into())
}

fn decide(app: &mut App, p: Pending) {
    let Some(w) = app.win(p.wid) else {
        finish(p, false);
        return;
    };
    let (profile, incognito) = (w.profile.clone(), w.incognito);
    USED.with(|u| u.borrow_mut().insert((p.origin.clone(), p.kind.clone())));
    let state = stored_state(app, &profile, incognito, &p.origin, &p.kind);
    match state.as_str() {
        "allow" => finish(p, true),
        "block" => finish(p, false),
        _ => {
            // autoplay and fonts follow Chrome's defaults rather than prompting
            if p.kind == "autoplay" {
                finish(p, false);
                return;
            }
            let id = NEXT_REQ.with(|n| {
                n.set(n.get() + 1);
                n.get()
            });
            let (wid, tid, origin, kind) = (p.wid, p.tid, p.origin.clone(), p.kind.clone());
            PENDING.with(|m| m.borrow_mut().insert(id, p));
            bridge::emit(
                app,
                wid,
                "permissionPrompt",
                json!({"id": id, "tab": tid, "origin": origin, "kind": kind, "label": label(&kind), "host": util::host_of(&origin)}),
            );
        }
    }
}

fn finish(p: Pending, allow: bool) {
    unsafe {
        let _ = p.args.SetState(if allow { COREWEBVIEW2_PERMISSION_STATE_ALLOW } else { COREWEBVIEW2_PERMISSION_STATE_DENY });
        let _ = p.deferral.Complete();
    }
}

/// Deny prompts that belong to a tab that navigated away or closed.
pub fn cancel_prompts_for_tab(tid: u32) {
    let gone: Vec<Pending> = PENDING.with(|m| {
        let mut m = m.borrow_mut();
        let ids: Vec<u32> = m.iter().filter(|(_, p)| p.tid == tid).map(|(k, _)| *k).collect();
        ids.into_iter().filter_map(|k| m.remove(&k)).collect()
    });
    for p in gone {
        finish(p, false);
    }
}

fn set_permission(app: &mut App, profile: &str, incognito: bool, origin: &str, kind: &str, state: &str) {
    if incognito {
        SESSION.with(|m| m.borrow_mut().insert((origin.to_string(), kind.to_string()), state.to_string()));
        return;
    }
    if let Some(db) = app.profile(profile).db() {
        if state == "default" {
            let _ = db.conn.execute("DELETE FROM permissions WHERE origin=?1 AND kind=?2", params![origin, kind]);
        } else {
            let _ = db.conn.execute(
                "INSERT OR REPLACE INTO permissions(origin,kind,state,updated) VALUES(?1,?2,?3,?4)",
                params![origin, kind, state, now_ms()],
            );
        }
    }
}

// ---------------------------------------------------------------- data clearing

fn profile_iface(app: &App, wid: u32) -> Option<(ICoreWebView2, ICoreWebView2Profile)> {
    let w = app.win(wid)?;
    let wv = w.ui.as_ref().map(|v| v.wv.clone()).or_else(|| w.tabs.iter().find_map(|t| t.view.as_ref().map(|v| v.wv.clone())))?;
    let p = unsafe { wv.cast::<ICoreWebView2_13>().ok()?.Profile().ok()? };
    Some((wv, p))
}

pub fn clear_data(app: &mut App, ctx: Ctx, id: Value, a: &Value) -> Reply {
    let Some(w) = app.win(ctx.wid) else { return Reply::Err("no window".into()) };
    let (profile, incognito) = (w.profile.clone(), w.incognito);
    let types: HashSet<String> = a["types"].as_array().map(|x| x.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default();
    let range_ms: i64 = match a["range"].as_str().unwrap_or("all") {
        "hour" => 3_600_000,
        "day" => 86_400_000,
        "week" => 7 * 86_400_000,
        "month" => 28 * 86_400_000,
        _ => 0,
    };
    let to = now_ms();
    let from = if range_ms == 0 { 0 } else { to - range_ms };
    let mut cleared = vec![];
    if !incognito {
        if let Some(db) = app.profile(&profile).db() {
            if types.contains("history") {
                db.clear_history(from, to);
                let _ = db.conn.execute("DELETE FROM blocked_stats WHERE day*86400000 >= ?1", [from]);
                cleared.push("history");
            }
            if types.contains("downloads") {
                let _ = db.conn.execute("DELETE FROM downloads WHERE started >= ?1", [from]);
                cleared.push("downloads");
            }
            if types.contains("site_settings") {
                let _ = db.conn.execute("DELETE FROM permissions WHERE updated >= ?1", [from]);
                cleared.push("site settings");
            }
            if types.contains("passwords") {
                let _ = db.conn.execute("UPDATE passwords SET deleted=1, updated=?2 WHERE updated >= ?1 AND deleted=0", params![from, to]);
                cleared.push("passwords");
            }
        }
    }
    let mut kinds = COREWEBVIEW2_BROWSING_DATA_KINDS(0);
    if types.contains("cookies") {
        kinds.0 |= COREWEBVIEW2_BROWSING_DATA_KINDS_ALL_SITE.0;
    }
    if types.contains("cache") {
        kinds.0 |= COREWEBVIEW2_BROWSING_DATA_KINDS_DISK_CACHE.0 | COREWEBVIEW2_BROWSING_DATA_KINDS_CACHE_STORAGE.0;
    }
    if types.contains("history") {
        kinds.0 |= COREWEBVIEW2_BROWSING_DATA_KINDS_BROWSING_HISTORY.0 | COREWEBVIEW2_BROWSING_DATA_KINDS_DOWNLOAD_HISTORY.0;
    }
    if types.contains("autofill") {
        kinds.0 |= COREWEBVIEW2_BROWSING_DATA_KINDS_GENERAL_AUTOFILL.0;
    }
    if kinds.0 == 0 {
        return Reply::Now(json!({"cleared": cleared}));
    }
    let Some((_, p)) = profile_iface(app, ctx.wid) else { return Reply::Err("engine not ready".into()) };
    let cleared: Vec<String> = cleared.into_iter().map(String::from).collect();
    unsafe {
        let Ok(p2) = p.cast::<ICoreWebView2Profile2>() else { return Reply::Err("unsupported".into()) };
        let h = ClearBrowsingDataCompletedHandler::create(Box::new(move |hr| {
            let r = if hr.is_ok() { Ok(json!({"cleared": cleared, "engine": true})) } else { Err("The engine could not clear some data".to_string()) };
            bridge::respond_later(ctx.clone(), id.clone(), r);
            Ok(())
        }));
        let _ = p2.ClearBrowsingDataInTimeRange(kinds, from as f64 / 1000.0, to as f64 / 1000.0, &h);
    }
    Reply::Later
}

fn clear_origin(app: &App, wid: u32, origin: &str) -> bool {
    let Some((wv, _)) = profile_iface(app, wid) else { return false };
    let params = json!({"origin": origin, "storageTypes": "all"}).to_string();
    let m = WStr::new("Storage.clearDataForOrigin");
    let p = WStr::new(&params);
    unsafe { wv.CallDevToolsProtocolMethod(m.pcwstr(), p.pcwstr(), None).is_ok() }
}

// ---------------------------------------------------------------- commands & dashboard

pub fn cmd(app: &mut App, ctx: &Ctx, cmd: &str, a: &Value, id: &Value) -> Reply {
    let Some(w) = app.win(ctx.wid) else { return Reply::Err("no window".into()) };
    let (profile, incognito) = (w.profile.clone(), w.incognito);
    let s = |k: &str| a[k].as_str().unwrap_or("").to_string();
    match cmd {
        "permissions.respond" => {
            let rid = a["id"].as_u64().unwrap_or(0) as u32;
            let Some(p) = PENDING.with(|m| m.borrow_mut().remove(&rid)) else { return ok() };
            let allow = a["allow"].as_bool().unwrap_or(false);
            if a["remember"].as_bool().unwrap_or(true) {
                let (o, k) = (p.origin.clone(), p.kind.clone());
                set_permission(app, &profile, incognito, &o, &k, if allow { "allow" } else { "block" });
            }
            finish(p, allow);
            ok()
        }
        "permissions.forOrigin" => {
            let origin = s("origin");
            let list: Vec<Value> = PERMISSIONS
                .iter()
                .map(|(k, l)| {
                    let st = stored_state(app, &profile, incognito, &origin, k);
                    let used = USED.with(|u| u.borrow().contains(&(origin.clone(), k.to_string())));
                    json!({"kind": k, "label": l, "state": st, "used": used})
                })
                .collect();
            Reply::Now(json!(list))
        }
        "permissions.set" => {
            let st = s("state");
            if !["allow", "block", "default"].contains(&st.as_str()) {
                return Reply::Err("invalid state".into());
            }
            set_permission(app, &profile, incognito, &s("origin"), &s("kind"), &st);
            ok()
        }
        "permissions.list" => {
            let Some(db) = app.profile(&profile).db() else { return Reply::Now(json!([])) };
            let Ok(mut st) = db.conn.prepare("SELECT origin,kind,state,updated FROM permissions ORDER BY origin, kind") else { return Reply::Now(json!([])) };
            let rows: Vec<Value> = st
                .query_map([], |r| Ok(json!({"origin": r.get::<_, String>(0)?, "kind": r.get::<_, String>(1)?, "label": label(&r.get::<_, String>(1)?), "state": r.get::<_, String>(2)?, "updated": r.get::<_, i64>(3)?})))
                .map(|r| r.flatten().collect())
                .unwrap_or_default();
            Reply::Now(json!(rows))
        }
        "permissions.reset" => {
            if let Some(db) = app.profile(&profile).db() {
                let _ = db.conn.execute("DELETE FROM permissions WHERE origin=?1", [s("origin")]);
            }
            ok()
        }
        "sites.clearData" => {
            let origin = s("origin");
            if !origin.starts_with("http") {
                return Reply::Err("not a web site".into());
            }
            if clear_origin(app, ctx.wid, &origin) {
                ok()
            } else {
                Reply::Err("could not clear site data".into())
            }
        }
        "privacy.siteException" => {
            let site = util::site_of_host(&s("site"));
            let exempt = a["exempt"].as_bool().unwrap_or(false);
            let mut list = app.settings(&profile).site_exceptions.clone();
            list.retain(|x| util::site_of_host(x) != site);
            if exempt {
                list.push(site);
            }
            let _ = bridge::set_setting(app, &profile, "site_exceptions", json!(list));
            reconfigure(app);
            ok()
        }
        "privacy.updateLists" => {
            update_lists(profile);
            ok()
        }
        "privacy.lists" => {
            let level = Level::parse(&app.settings(&profile).blocking_level);
            let lists: Vec<Value> = LIST_URLS
                .iter()
                .map(|(n, u)| {
                    let p = list_path(n);
                    let m = std::fs::metadata(&p).ok();
                    let modified = m.as_ref().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as i64).unwrap_or(0);
                    json!({"name": n, "url": u, "size": m.map(|m| m.len()).unwrap_or(0), "modified": modified,
                        "updated": p.starts_with(lists_dir_user()), "active": level.lists().contains(n)})
                })
                .collect();
            Reply::Now(json!({"lists": lists, "engineReady": ENGINES.with(|e| e.borrow().contains_key(&level))}))
        }
        "privacy.dashboard" => Reply::Now(dashboard(app, &profile)),
        _ => {
            let _ = id;
            Reply::Err(format!("unknown command {cmd}"))
        }
    }
}

fn dashboard(app: &mut App, profile: &str) -> Value {
    let s = app.settings(profile).clone();
    let today = now_ms() / 86_400_000;
    let mut blocked_today = 0i64;
    let mut blocked_week = 0i64;
    let mut total = 0i64;
    let mut top: Vec<Value> = vec![];
    let mut daily: Vec<Value> = vec![];
    let mut perm_summary: Vec<Value> = vec![];
    let mut history_count = 0;
    let (mut pw_count, mut pw_weak, mut pw_reused) = (0, 0, 0);
    if let Some(db) = app.profile(profile).db() {
        let q = |sql: &str, p: i64| -> i64 { db.conn.query_row(sql, [p], |r| r.get(0)).unwrap_or(0) };
        blocked_today = q("SELECT COALESCE(SUM(count),0) FROM blocked_stats WHERE day=?1", today);
        blocked_week = q("SELECT COALESCE(SUM(count),0) FROM blocked_stats WHERE day>?1", today - 7);
        total = q("SELECT COALESCE(SUM(count),0) FROM blocked_stats WHERE day>?1", 0);
        if let Ok(mut st) = db.conn.prepare("SELECT site, SUM(count) c FROM blocked_stats WHERE day>?1 GROUP BY site ORDER BY c DESC LIMIT 10") {
            top = st.query_map([today - 30], |r| Ok(json!({"site": r.get::<_, String>(0)?, "count": r.get::<_, i64>(1)?}))).map(|r| r.flatten().collect()).unwrap_or_default();
        }
        if let Ok(mut st) = db.conn.prepare("SELECT day, SUM(count) FROM blocked_stats WHERE day>?1 GROUP BY day ORDER BY day") {
            daily = st.query_map([today - 14], |r| Ok(json!({"day": r.get::<_, i64>(0)?, "count": r.get::<_, i64>(1)?}))).map(|r| r.flatten().collect()).unwrap_or_default();
        }
        if let Ok(mut st) = db.conn.prepare("SELECT kind, state, COUNT(*) FROM permissions GROUP BY kind, state") {
            perm_summary = st
                .query_map([], |r| Ok(json!({"kind": r.get::<_, String>(0)?, "label": label(&r.get::<_, String>(0)?), "state": r.get::<_, String>(1)?, "count": r.get::<_, i64>(2)?})))
                .map(|r| r.flatten().collect())
                .unwrap_or_default();
        }
        history_count = db.history_count();
        let h = crate::passwords::health(db);
        pw_count = h.0;
        pw_weak = h.1;
        pw_reused = h.2;
    }
    let level = Level::parse(&s.blocking_level);
    json!({
        "blocking": {"level": s.blocking_level, "ready": ENGINES.with(|e| e.borrow().contains_key(&level)), "today": blocked_today, "week": blocked_week, "total": total, "topSites": top, "daily": daily, "exceptions": s.site_exceptions},
        "trackingPrevention": s.tracking_prevention,
        "thirdPartyCookies": if s.block_third_party_cookies { "blocked" } else { "allowed" },
        "httpsFirst": s.https_first,
        "smartscreen": s.smartscreen,
        "doNotTrack": s.do_not_track,
        "permissions": perm_summary,
        "passwords": {"count": pw_count, "weak": pw_weak, "reused": pw_reused, "enabled": s.password_manager},
        "history": {"count": history_count, "encrypted": true},
        "ai": {"enabled": s.ai.enabled, "provider": s.ai.provider, "keySet": crate::ai::has_key(app, profile), "permissions": s.ai.permissions},
        "sync": {"enabled": s.sync.enabled, "folder": s.sync.folder, "lastSync": s.sync.last_sync, "encrypted": true},
        "telemetry": "none",
        "engine": app.env().map(|e| crate::webview::runtime_version(&e)).unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn engine_blocks_known_trackers_and_spares_first_party() {
        let lists = lists_dir_bundled();
        let text = std::fs::read_to_string(lists.join("easylist.txt")).unwrap() + "\n" + &std::fs::read_to_string(lists.join("easyprivacy.txt")).unwrap();
        let e = adblock::Engine::new_with_list_text(text);
        let block = |u: &str, s: &str, t: &str| e.check_network_request(&adblock::request::Request::new(u, s, t, "get").unwrap()).should_block();
        assert!(block("https://www.google-analytics.com/analytics.js", "https://example.com/", "script"));
        assert!(block("https://securepubads.g.doubleclick.net/tag/js/gpt.js", "https://news.example.com/", "script"));
        assert!(!block("https://example.com/app.js", "https://example.com/", "script"));
        assert!(!block("https://cdn.jsdelivr.net/npm/react@18/umd/react.production.min.js", "https://example.com/", "script"));
        // serialized engines round-trip (used for the fast start-up cache)
        let bytes = e.serialize();
        let mut e2 = adblock::Engine::default();
        e2.deserialize(&bytes).unwrap();
        assert!(e2.check_network_request(&adblock::request::Request::new("https://www.google-analytics.com/analytics.js", "https://example.com/", "script", "get").unwrap()).should_block());
    }
}
