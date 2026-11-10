//! Per-profile settings and app-wide local state. Both are JSON files written atomically.
//! Every field has a default so older files keep loading after upgrades.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::util;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct SearchShortcut {
    pub keyword: String,
    pub name: String,
    pub url: String, // contains %s
}

impl Default for SearchShortcut {
    fn default() -> Self {
        SearchShortcut { keyword: String::new(), name: String::new(), url: String::new() }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct AiPermissions {
    pub read_page: bool,
    pub read_tabs: bool,
    pub read_history: bool,
    pub manage_tabs: bool,
    pub manage_bookmarks: bool,
    pub manage_workspaces: bool,
    pub in_incognito: bool,
}

impl Default for AiPermissions {
    fn default() -> Self {
        AiPermissions {
            read_page: true,
            read_tabs: true,
            read_history: false,
            manage_tabs: true,
            manage_bookmarks: true,
            manage_workspaces: true,
            in_incognito: false,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct AiSettings {
    pub enabled: bool,
    pub provider: String, // "anthropic" | "groq"
    pub model: String,
    pub permissions: AiPermissions,
    pub confirm_actions: bool,
}

impl Default for AiSettings {
    fn default() -> Self {
        AiSettings {
            enabled: true,
            provider: "anthropic".into(),
            model: String::new(),
            permissions: AiPermissions::default(),
            confirm_actions: true,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct SyncSettings {
    pub enabled: bool,
    pub folder: String,
    pub bookmarks: bool,
    pub history: bool,
    pub passwords: bool,
    pub notes: bool,
    pub tasks: bool,
    pub settings: bool,
    pub workspaces: bool,
    pub cloud_backups: bool,
    pub last_sync: i64,
    pub last_error: String,
}

impl Default for SyncSettings {
    fn default() -> Self {
        SyncSettings {
            enabled: false,
            folder: String::new(),
            bookmarks: true,
            history: true,
            passwords: true,
            notes: true,
            tasks: true,
            settings: true,
            workspaces: true,
            cloud_backups: true,
            last_sync: 0,
            last_error: String::new(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Settings {
    // Appearance
    pub theme: String,   // system | light | dark
    pub density: String, // compact | comfortable | spacious
    pub theme_id: String,
    pub custom_css: String,
    pub animations: bool,
    pub show_home_button: bool,
    pub show_bookmarks_bar: bool,
    pub sidebar_open: bool,
    pub sidebar_panel: String,
    pub sidebar_width: u32,
    pub ai_panel_width: u32,
    pub zoom_default: f64,
    // Start-up
    pub startup: String, // restore | newtab | urls
    pub startup_urls: Vec<String>,
    pub homepage: String, // "" = new tab page
    // Search
    pub search_engine: String,
    pub search_suggestions: bool,
    pub search_shortcuts: Vec<SearchShortcut>,
    // Tabs
    pub auto_group_tabs: bool,
    pub auto_group_threshold: u32,
    pub duplicate_detection: bool,
    pub tab_hover_cards: bool,
    // Privacy
    pub blocking_level: String,      // off | balanced | strict
    pub tracking_prevention: String, // basic | balanced | strict
    pub block_third_party_cookies: bool,
    pub https_first: bool,
    pub smartscreen: bool,
    pub do_not_track: bool,
    pub password_manager: bool,
    pub autofill_forms: bool,
    pub site_exceptions: Vec<String>, // sites where blocking is disabled
    // Translation
    pub translate_offer: bool,
    pub translate_target: String,
    pub translate_always: Vec<String>,
    pub translate_never: Vec<String>,
    // Downloads
    pub download_dir: String,
    pub download_ask: bool,
    // Keyboard shortcuts: action -> accelerator ("Ctrl+Shift+A")
    pub shortcuts: BTreeMap<String, String>,
    // New tab page layout (owned by the new tab page)
    pub newtab: serde_json::Value,
    // Developer
    pub devtools: bool,
    // AI, sync, backups
    pub ai: AiSettings,
    pub sync: SyncSettings,
    pub local_backups: bool,
    pub backup_keep: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: "system".into(),
            density: "comfortable".into(),
            theme_id: "default".into(),
            custom_css: String::new(),
            animations: true,
            show_home_button: false,
            show_bookmarks_bar: true,
            sidebar_open: false,
            sidebar_panel: "bookmarks".into(),
            sidebar_width: 320,
            ai_panel_width: 380,
            zoom_default: 1.0,
            startup: "restore".into(),
            startup_urls: vec![],
            homepage: String::new(),
            search_engine: "google".into(),
            search_suggestions: true,
            search_shortcuts: default_shortcuts(),
            auto_group_tabs: true,
            auto_group_threshold: 4,
            duplicate_detection: true,
            tab_hover_cards: true,
            blocking_level: "balanced".into(),
            tracking_prevention: "balanced".into(),
            block_third_party_cookies: true,
            https_first: true,
            smartscreen: true,
            do_not_track: false,
            password_manager: true,
            autofill_forms: true,
            site_exceptions: vec![],
            translate_offer: true,
            translate_target: String::new(),
            translate_always: vec![],
            translate_never: vec![],
            download_dir: String::new(),
            download_ask: false,
            shortcuts: BTreeMap::new(),
            newtab: serde_json::Value::Null,
            devtools: true,
            ai: AiSettings::default(),
            sync: SyncSettings::default(),
            local_backups: true,
            backup_keep: 7,
        }
    }
}

fn default_shortcuts() -> Vec<SearchShortcut> {
    let s = |k: &str, n: &str, u: &str| SearchShortcut { keyword: k.into(), name: n.into(), url: u.into() };
    vec![
        s("yt", "YouTube", "https://www.youtube.com/results?search_query=%s"),
        s("w", "Wikipedia", "https://en.wikipedia.org/wiki/Special:Search?search=%s"),
        s("maps", "Google Maps", "https://www.google.com/maps/search/%s"),
        s("img", "Google Images", "https://www.google.com/search?tbm=isch&q=%s"),
        s("gh", "GitHub", "https://github.com/search?q=%s"),
    ]
}

pub fn search_engines() -> Vec<(&'static str, &'static str, &'static str, &'static str)> {
    // id, name, search url, suggest url
    vec![
        ("google", "Google", "https://www.google.com/search?q=%s", "https://suggestqueries.google.com/complete/search?client=chrome&q=%s"),
        ("bing", "Bing", "https://www.bing.com/search?q=%s", "https://api.bing.com/osjson.aspx?query=%s"),
        ("duckduckgo", "DuckDuckGo", "https://duckduckgo.com/?q=%s", "https://duckduckgo.com/ac/?type=list&q=%s"),
        ("ecosia", "Ecosia", "https://www.ecosia.org/search?q=%s", "https://ac.ecosia.org/autocomplete?type=list&q=%s"),
        ("brave", "Brave Search", "https://search.brave.com/search?q=%s", "https://search.brave.com/api/suggest?q=%s"),
    ]
}

impl Settings {
    pub fn load(path: &Path) -> Settings {
        match std::fs::read(path) {
            Ok(b) => serde_json::from_slice(&b).unwrap_or_else(|e| {
                logf!("settings parse error ({e}); keeping a copy and using defaults");
                let _ = std::fs::copy(path, path.with_extension("corrupt.json"));
                Settings::default()
            }),
            Err(_) => Settings::default(),
        }
    }
    pub fn save(&self, path: &Path) {
        if let Ok(b) = serde_json::to_vec_pretty(self) {
            if let Err(e) = util::write_atomic(path, &b) {
                logf!("settings save failed: {e}");
            }
        }
    }
    pub fn search_url(&self, q: &str) -> String {
        let tmpl = search_engines()
            .into_iter()
            .find(|e| e.0 == self.search_engine)
            .map(|e| e.2)
            .unwrap_or("https://www.google.com/search?q=%s");
        tmpl.replace("%s", &url_encode(q))
    }
    pub fn suggest_url(&self, q: &str) -> String {
        let tmpl = search_engines()
            .into_iter()
            .find(|e| e.0 == self.search_engine)
            .map(|e| e.3)
            .unwrap_or("https://suggestqueries.google.com/complete/search?client=chrome&q=%s");
        tmpl.replace("%s", &url_encode(q))
    }
}

pub fn url_encode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct ProfileInfo {
    pub id: String,
    pub name: String,
    pub color: String,
    pub created: i64,
}

impl Default for ProfileInfo {
    fn default() -> Self {
        ProfileInfo { id: "default".into(), name: "Personal".into(), color: "#ff7a45".into(), created: 0 }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct WindowPlacement {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub maximized: bool,
}

impl Default for WindowPlacement {
    fn default() -> Self {
        WindowPlacement { x: i32::MIN, y: i32::MIN, w: 1280, h: 820, maximized: false }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct SuspendRules {
    /// Minutes before a background tab is frozen (renderer paused, memory trimmed).
    pub freeze_after_min: u32,
    /// Minutes before a background tab is unloaded completely (webview destroyed).
    pub discard_after_min: u32,
    pub never_audio: bool,
    pub never_pinned: bool,
    pub never_forms: bool,
    pub exceptions: Vec<String>, // sites never suspended
}

impl Default for SuspendRules {
    fn default() -> Self {
        SuspendRules { freeze_after_min: 5, discard_after_min: 45, never_audio: true, never_pinned: false, never_forms: true, exceptions: vec![] }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct LocalState {
    pub profiles: Vec<ProfileInfo>,
    pub last_profile: String,
    pub onboarding_done: bool,
    pub clean_exit: bool,
    pub device_id: String,
    pub device_name: String,
    pub perf_profile: String, // performance | balanced | efficiency
    pub auto_battery: bool,
    pub gpu_acceleration: bool,
    pub preload: bool,
    pub custom_rules: bool,
    pub rules: SuspendRules,
    pub close_to_tray: bool,
    pub tray_unload_after_min: u32,
    pub placement: WindowPlacement,
    pub update_check: bool,
    pub update_last_check: i64,
    pub last_backup: i64,
    pub first_run: i64,
}

impl Default for LocalState {
    fn default() -> Self {
        LocalState {
            profiles: vec![ProfileInfo { created: util::now_ms(), ..Default::default() }],
            last_profile: "default".into(),
            onboarding_done: false,
            clean_exit: true,
            device_id: util::new_id(),
            device_name: std::env::var("COMPUTERNAME").unwrap_or_else(|_| "This PC".into()),
            perf_profile: "balanced".into(),
            auto_battery: true,
            gpu_acceleration: true,
            preload: true,
            custom_rules: false,
            rules: SuspendRules::default(),
            close_to_tray: true,
            tray_unload_after_min: 10,
            placement: WindowPlacement::default(),
            update_check: true,
            update_last_check: 0,
            last_backup: 0,
            first_run: util::now_ms(),
        }
    }
}

impl LocalState {
    pub fn path() -> PathBuf {
        util::user_data_root().join("Local State.json")
    }
    pub fn load() -> LocalState {
        match std::fs::read(Self::path()) {
            Ok(b) => serde_json::from_slice(&b).unwrap_or_default(),
            Err(_) => LocalState::default(),
        }
    }
    pub fn save(&self) {
        if let Ok(b) = serde_json::to_vec_pretty(self) {
            let _ = util::write_atomic(&Self::path(), &b);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_roundtrip_and_tolerate_missing_fields() {
        let s: Settings = serde_json::from_str(r#"{"theme":"dark"}"#).unwrap();
        assert_eq!(s.theme, "dark");
        assert_eq!(s.search_engine, "google");
        assert!(s.block_third_party_cookies);
        assert!(!s.ai.permissions.read_history, "history access must be opt-in");
    }
    #[test]
    fn search_urls() {
        let s = Settings::default();
        assert_eq!(s.search_url("a b&c"), "https://www.google.com/search?q=a+b%26c");
    }
}
