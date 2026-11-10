//! Windows integration: shell, file pickers, restart, default-browser hand-off, profile management
//! and app-wide (local state) settings.

use std::path::Path;

use serde_json::{json, Value};
use windows::core::{w, Interface, PCWSTR};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

use crate::app::App;
use crate::bridge::{ok, Ctx, Reply};
use crate::settings::ProfileInfo;
use crate::util::{self, WStr};

pub fn open_path(p: &Path) {
    let s = WStr::path(p);
    unsafe {
        ShellExecuteW(None, w!("open"), s.pcwstr(), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
    }
}

pub fn open_url_external(url: &str) {
    let s = WStr::new(url);
    unsafe {
        ShellExecuteW(None, w!("open"), s.pcwstr(), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
    }
}

/// Show a file in Explorer with it selected.
pub fn reveal(p: &Path) {
    let args = WStr::new(&format!("/select,\"{}\"", p.display()));
    unsafe {
        ShellExecuteW(None, w!("open"), w!("explorer.exe"), args.pcwstr(), PCWSTR::null(), SW_SHOWNORMAL);
    }
}

pub fn open_default_apps() {
    open_url_external("ms-settings:defaultapps?registeredAppUser=Solus");
}

/// Quit and start again (used after changes that need a fresh engine).
pub fn restart(app: &mut App) {
    app.restart_on_exit = true;
    app.quit();
}

pub fn spawn_self(extra: &[&str]) {
    if let Ok(exe) = std::env::current_exe() {
        let mut cmd = std::process::Command::new(exe);
        if let Some(d) = crate::ARGS.with(|a| a.borrow().user_data_dir.clone()) {
            cmd.arg(format!("--user-data-dir={}", d.display()));
        }
        cmd.args(extra);
        let _ = cmd.spawn();
    }
}

fn hwnd_of(app: &App, wid: u32) -> Option<windows::Win32::Foundation::HWND> {
    app.win(wid).map(|w| w.hwnd)
}

pub fn pick_folder(app: &App, wid: u32, title: &str) -> Option<String> {
    unsafe {
        let dlg: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let opts = dlg.GetOptions().ok()?;
        dlg.SetOptions(opts | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM).ok()?;
        let t = WStr::new(title);
        let _ = dlg.SetTitle(t.pcwstr());
        dlg.Show(hwnd_of(app, wid)).ok()?;
        let item = dlg.GetResult().ok()?;
        let p = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let s = p.to_string().ok();
        windows::Win32::System::Com::CoTaskMemFree(Some(p.0 as _));
        s
    }
}

/// filter: "CSV files|*.csv;HTML files|*.html"
pub fn pick_file(app: &App, wid: u32, filter: &str) -> Option<String> {
    unsafe {
        let dlg: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let mut names: Vec<WStr> = vec![];
        let mut specs: Vec<WStr> = vec![];
        for part in filter.split(';').filter(|p| p.contains('|')) {
            let (n, s) = part.split_once('|').unwrap();
            names.push(WStr::new(n));
            specs.push(WStr::new(s));
        }
        let filters: Vec<windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC> = names
            .iter()
            .zip(specs.iter())
            .map(|(n, s)| windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC { pszName: n.pcwstr(), pszSpec: s.pcwstr() })
            .collect();
        if !filters.is_empty() {
            let _ = dlg.SetFileTypes(&filters);
        }
        dlg.Show(hwnd_of(app, wid)).ok()?;
        let item = dlg.GetResult().ok()?;
        let p = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let s = p.to_string().ok();
        windows::Win32::System::Com::CoTaskMemFree(Some(p.0 as _));
        s
    }
}

pub fn save_file_dialog(app: &App, wid: u32, name: &str, filter: &str) -> Option<String> {
    unsafe {
        let dlg: IFileSaveDialog = CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let n = WStr::new(name);
        let _ = dlg.SetFileName(n.pcwstr());
        let mut names: Vec<WStr> = vec![];
        let mut specs: Vec<WStr> = vec![];
        for part in filter.split(';').filter(|p| p.contains('|')) {
            let (a, b) = part.split_once('|').unwrap();
            names.push(WStr::new(a));
            specs.push(WStr::new(b));
        }
        let filters: Vec<windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC> = names
            .iter()
            .zip(specs.iter())
            .map(|(n, s)| windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC { pszName: n.pcwstr(), pszSpec: s.pcwstr() })
            .collect();
        if !filters.is_empty() {
            let _ = dlg.SetFileTypes(&filters);
        }
        dlg.Show(hwnd_of(app, wid)).ok()?;
        let item = dlg.GetResult().ok()?;
        let p = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let s = p.to_string().ok();
        windows::Win32::System::Com::CoTaskMemFree(Some(p.0 as _));
        s
    }
}

/// Update an app-wide setting (performance profile, GPU, tray...) by dotted key.
pub fn set_local(app: &mut App, key: &str, value: Value) -> Result<Value, String> {
    let mut v = serde_json::to_value(&app.local).map_err(|e| e.to_string())?;
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
    let allowed = [
        "perf_profile", "auto_battery", "gpu_acceleration", "preload", "custom_rules", "rules", "close_to_tray",
        "tray_unload_after_min", "update_check", "device_name",
    ];
    if !allowed.contains(&parts[0]) {
        return Err(format!("{key} cannot be changed here"));
    }
    app.local = serde_json::from_value(v).map_err(|e| format!("invalid value for {key}: {e}"))?;
    app.local_dirty = true;
    let restart = parts[0] == "gpu_acceleration";
    crate::hooks::local_changed(app, parts[0]);
    Ok(json!({"restartRequired": restart}))
}

pub fn profiles_cmd(app: &mut App, ctx: &Ctx, cmd: &str, args: &Value) -> Reply {
    match cmd {
        "profiles.list" => Reply::Now(json!({"profiles": app.local.profiles, "current": app.win(ctx.wid).map(|w| w.profile.clone())})),
        "profiles.create" => {
            let name = args["name"].as_str().unwrap_or("").trim().to_string();
            if name.is_empty() || name.len() > 40 {
                return Reply::Err("Profile name must be 1-40 characters".into());
            }
            let id = util::new_id()[..8].to_string();
            let color = args["color"].as_str().unwrap_or("#1c7ed6").to_string();
            app.local.profiles.push(ProfileInfo { id: id.clone(), name, color, created: util::now_ms() });
            app.local_dirty = true;
            app.flush();
            // engine preferences (third-party cookies, Do Not Track) must exist before the engine first loads the profile
            let settings = app.settings(&id).clone();
            let engine_dir = util::user_data_root().join("EBWebView").join(crate::webview::engine_profile_name(&id));
            crate::webview::write_profile_prefs(&engine_dir, &settings);
            app.new_window(&id, false, vec!["solus://welcome/".into()], false);
            Reply::Now(json!(id))
        }
        "profiles.rename" => {
            let id = args["id"].as_str().unwrap_or("");
            let name = args["name"].as_str().unwrap_or("").trim().to_string();
            if name.is_empty() {
                return Reply::Err("name required".into());
            }
            if let Some(p) = app.local.profiles.iter_mut().find(|p| p.id == id) {
                p.name = name;
                if let Some(c) = args["color"].as_str() {
                    p.color = c.to_string();
                }
            }
            app.local_dirty = true;
            ok()
        }
        "profiles.open" => {
            let id = args["id"].as_str().unwrap_or("").to_string();
            if !app.local.profiles.iter().any(|p| p.id == id) {
                return Reply::Err("no such profile".into());
            }
            match app.windows.iter().find(|w| w.profile == id && !w.incognito).map(|w| w.id) {
                Some(w) => app.show_window(w),
                None => {
                    let restore = app.settings(&id).startup == "restore";
                    app.new_window(&id, false, vec![], restore);
                }
            }
            app.local.last_profile = id;
            app.local_dirty = true;
            ok()
        }
        "profiles.delete" => {
            let id = args["id"].as_str().unwrap_or("").to_string();
            if id == "default" {
                return Reply::Err("The main profile cannot be deleted".into());
            }
            if app.windows.iter().any(|w| w.profile == id) {
                return Reply::Err("Close this profile's windows first".into());
            }
            app.local.profiles.retain(|p| p.id != id);
            app.profiles.remove(&id);
            app.local_dirty = true;
            let dir = util::user_data_root().join("Profiles").join(&id);
            // move to a trash folder instead of deleting outright
            let trash = util::user_data_root().join("Deleted Profiles").join(format!("{id}-{}", util::now_ms()));
            let _ = std::fs::create_dir_all(trash.parent().unwrap());
            let _ = std::fs::rename(&dir, &trash);
            ok()
        }
        _ => Reply::Err("unknown profiles command".into()),
    }
}

pub fn file_url_to_path(u: &str) -> Option<std::path::PathBuf> {
    url::Url::parse(u).ok()?.to_file_path().ok()
}

#[allow(dead_code)]
fn _iface_check(x: &IFileOpenDialog) -> windows::core::Result<IShellItem> {
    unsafe { x.GetResult() }.and_then(|i| i.cast())
}
