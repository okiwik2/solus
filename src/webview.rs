//! WebView2 engine plumbing: environment creation, engine preferences, controller creation and
//! per-webview settings.

use std::path::{Path, PathBuf};

use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::*;
use std::result::Result;
use windows::core::{Interface, BOOL, PCWSTR, PWSTR};
use windows::Win32::Foundation::*;

use crate::app::{defer, App};
use crate::settings::{LocalState, Settings};
use crate::util::WStr;

pub const SCHEME: &str = "solus";

pub fn browser_arguments(local: &LocalState, block_third_party_cookies: bool) -> String {
    let mut a: Vec<String> = vec![];
    if block_third_party_cookies {
        // Blocks unpartitioned third-party cookies engine-wide, the same behaviour as Chrome's setting
        // (partitioned/CHIPS cookies still work). Measured: 305 -> 0 third-party cookies on an ad-heavy
        // page. The profile Preferences value alone is ignored by the WebView2 runtime.
        a.push("--test-third-party-cookie-phaseout".into());
    }
    if !local.gpu_acceleration {
        a.push("--disable-gpu".into());
        a.push("--disable-gpu-compositing".into());
    }
    // test harness only: extra engine flags (e.g. keep rendering while the screen is locked)
    if let Ok(extra) = std::env::var("SOLUS_ENGINE_ARGS") {
        if std::env::var_os("SOLUS_AUTOMATION_TOKEN").is_some() {
            a.push(extra);
        }
    }
    a.join(" ")
}

/// Chromium preferences that WebView2 has no API for are written into the profile's Preferences
/// file before the browser process starts (Chromium reads them at start-up).
pub fn prepare_engine_prefs(data: &Path, app: &mut App) {
    let ids: Vec<String> = app.local.profiles.iter().map(|p| p.id.clone()).collect();
    for id in ids {
        let s = app.settings(&id).clone();
        write_profile_prefs(&data.join(engine_profile_name(&id)), &s);
    }
}

pub fn engine_profile_name(profile_id: &str) -> String {
    // WebView2 profile names are limited to a safe character set
    let n: String = profile_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(60)
        .collect();
    if n.is_empty() || n == "default" {
        "Default".to_string()
    } else {
        format!("solus-{n}")
    }
}

pub fn write_profile_prefs(dir: &Path, s: &Settings) {
    let path = dir.join("Preferences");
    let mut v: serde_json::Value = std::fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    if !v.is_object() {
        v = serde_json::json!({});
    }
    let set = |v: &mut serde_json::Value, path: &[&str], val: serde_json::Value| {
        let mut cur = v;
        for (i, k) in path.iter().enumerate() {
            if i == path.len() - 1 {
                cur[*k] = val.clone();
            } else {
                if !cur.get(*k).map(|x| x.is_object()).unwrap_or(false) {
                    cur[*k] = serde_json::json!({});
                }
                cur = cur.get_mut(*k).unwrap();
            }
        }
    };
    // 1 = block third-party cookies, 0 = allow
    set(&mut v, &["profile", "cookie_controls_mode"], serde_json::json!(if s.block_third_party_cookies { 1 } else { 0 }));
    set(&mut v, &["enable_do_not_track"], serde_json::json!(s.do_not_track));
    // Solus has its own translate offer and password manager; keep Edge's from double-prompting
    set(&mut v, &["translate", "enabled"], serde_json::json!(false));
    set(&mut v, &["credentials_enable_service"], serde_json::json!(false));
    if let Ok(b) = serde_json::to_vec(&v) {
        let _ = std::fs::create_dir_all(dir);
        let _ = crate::util::write_atomic(&path, &b);
    }
}

pub fn create_environment(
    data: PathBuf,
    args: String,
    cb: impl FnOnce(Result<ICoreWebView2Environment, String>) + 'static,
) {
    let _ = std::fs::create_dir_all(&data);
    let options = CoreWebView2EnvironmentOptions::default();
    unsafe {
        options.set_additional_browser_arguments(args);
        options.set_are_browser_extensions_enabled(true);
        options.set_enable_tracking_prevention(true);
        options.set_scroll_bar_style(COREWEBVIEW2_SCROLLBAR_STYLE_FLUENT_OVERLAY);
        let reg = CoreWebView2CustomSchemeRegistration::new(SCHEME.to_string());
        reg.set_treat_as_secure(true);
        reg.set_has_authority_component(true);
        options.set_scheme_registrations(vec![Some(reg.into())]);
    }
    let options: ICoreWebView2EnvironmentOptions = options.into();
    let data_w = WStr::path(&data);
    let cb = std::cell::Cell::new(Some(cb));
    let handler = CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(move |hr, env| {
        if let Some(cb) = cb.take() {
            match (hr, env) {
                (Ok(()), Some(env)) => cb(Ok(env)),
                (Err(e), _) => cb(Err(format!("{e}"))),
                _ => cb(Err("no environment".into())),
            }
        }
        Ok(())
    }));
    let r = unsafe { CreateCoreWebView2EnvironmentWithOptions(PCWSTR::null(), data_w.pcwstr(), &options, &handler) };
    if let Err(e) = r {
        // completion handler never fires in this case
        let msg = if e.code() == windows::core::HRESULT::from_win32(ERROR_FILE_NOT_FOUND.0) {
            "The Microsoft Edge WebView2 Runtime is not installed.".to_string()
        } else {
            format!("{e}")
        };
        defer(move |app| {
            app.env = crate::app::EnvState::Failed(msg.clone());
            crate::app::show_fatal(&format!("Solus could not start its engine: {msg}"));
            app.quit();
        });
    }
}

pub fn runtime_version(env: &ICoreWebView2Environment) -> String {
    unsafe {
        let mut p = PWSTR::null();
        if env.BrowserVersionString(&mut p).is_ok() {
            return take_pwstr(p);
        }
    }
    String::new()
}

pub fn on_environment_created(_app: &mut App, env: &ICoreWebView2Environment) {
    unsafe {
        if let Ok(env5) = env.cast::<ICoreWebView2Environment5>() {
            let h = BrowserProcessExitedEventHandler::create(Box::new(|_, args| {
                let mut kind = COREWEBVIEW2_BROWSER_PROCESS_EXIT_KIND_NORMAL;
                if let Some(a) = args {
                    let _ = a.BrowserProcessExitKind(&mut kind);
                }
                if kind == COREWEBVIEW2_BROWSER_PROCESS_EXIT_KIND_FAILED {
                    defer(|app| app.on_browser_process_exited());
                }
                Ok(())
            }));
            let mut tok = Default::default();
            let _ = env5.add_BrowserProcessExited(&h, &mut tok);
        }
    }
}

pub struct ControllerReq {
    pub parent: HWND,
    pub profile: String,
    pub incognito: bool,
}

/// Create a windowed controller in the given parent window. `cb` runs on the UI thread with the app.
pub fn create_controller(
    env: &ICoreWebView2Environment,
    req: ControllerReq,
    cb: impl FnOnce(&mut App, Result<ICoreWebView2Controller, String>) + 'static,
) {
    let cb = std::cell::Cell::new(Some(cb));
    let handler = CreateCoreWebView2ControllerCompletedHandler::create(Box::new(move |hr, ctl| {
        if let Some(cb) = cb.take() {
            let res = match (hr, ctl) {
                (Ok(()), Some(c)) => Ok(c),
                (Err(e), _) => Err(format!("{e}")),
                _ => Err("no controller".into()),
            };
            defer(move |app| cb(app, res));
        }
        Ok(())
    }));
    unsafe {
        let r = (|| -> windows::core::Result<()> {
            let env10: ICoreWebView2Environment10 = env.cast()?;
            let opts = env10.CreateCoreWebView2ControllerOptions()?;
            let name = WStr::new(&engine_profile_name(&req.profile));
            opts.SetProfileName(name.pcwstr())?;
            opts.SetIsInPrivateModeEnabled(req.incognito)?;
            env10.CreateCoreWebView2ControllerWithOptions(req.parent, &opts, &handler)
        })();
        if let Err(e) = r {
            crate::logf!("CreateCoreWebView2ControllerWithOptions failed: {e}");
        }
    }
}

/// Settings for web content tabs.
pub fn configure_content(wv: &ICoreWebView2, s: &Settings) {
    unsafe {
        if let Ok(st) = wv.Settings() {
            let _ = st.SetAreDevToolsEnabled(s.devtools);
            let _ = st.SetAreDefaultContextMenusEnabled(true);
            let _ = st.SetIsStatusBarEnabled(true);
            let _ = st.SetIsZoomControlEnabled(true);
            let _ = st.SetIsBuiltInErrorPageEnabled(true);
            let _ = st.SetAreDefaultScriptDialogsEnabled(true);
            let _ = st.SetIsWebMessageEnabled(true);
            if let Ok(s3) = st.cast::<ICoreWebView2Settings3>() {
                let _ = s3.SetAreBrowserAcceleratorKeysEnabled(true);
            }
            if let Ok(s4) = st.cast::<ICoreWebView2Settings4>() {
                // Solus stores passwords itself (encrypted, importable from Chrome)
                let _ = s4.SetIsPasswordAutosaveEnabled(false);
                let _ = s4.SetIsGeneralAutofillEnabled(s.autofill_forms);
            }
            if let Ok(s5) = st.cast::<ICoreWebView2Settings5>() {
                let _ = s5.SetIsPinchZoomEnabled(true);
            }
            if let Ok(s6) = st.cast::<ICoreWebView2Settings6>() {
                let _ = s6.SetIsSwipeNavigationEnabled(true);
            }
            if let Ok(s8) = st.cast::<ICoreWebView2Settings8>() {
                let _ = s8.SetIsReputationCheckingRequired(s.smartscreen);
            }
        }
        if let Ok(w13) = wv.cast::<ICoreWebView2_13>() {
            if let Ok(p) = w13.Profile() {
                apply_profile_settings(&p, s);
            }
        }
    }
}

pub fn apply_profile_settings(p: &ICoreWebView2Profile, s: &Settings) {
    unsafe {
        if let Ok(p3) = p.cast::<ICoreWebView2Profile3>() {
            let lvl = match s.tracking_prevention.as_str() {
                "basic" => COREWEBVIEW2_TRACKING_PREVENTION_LEVEL_BASIC,
                "strict" => COREWEBVIEW2_TRACKING_PREVENTION_LEVEL_STRICT,
                "off" => COREWEBVIEW2_TRACKING_PREVENTION_LEVEL_NONE,
                _ => COREWEBVIEW2_TRACKING_PREVENTION_LEVEL_BALANCED,
            };
            let _ = p3.SetPreferredTrackingPreventionLevel(lvl);
        }
        if let Ok(p6) = p.cast::<ICoreWebView2Profile6>() {
            let _ = p6.SetIsPasswordAutosaveEnabled(false);
            let _ = p6.SetIsGeneralAutofillEnabled(s.autofill_forms);
        }
        let scheme = match s.theme.as_str() {
            "light" => COREWEBVIEW2_PREFERRED_COLOR_SCHEME_LIGHT,
            "dark" => COREWEBVIEW2_PREFERRED_COLOR_SCHEME_DARK,
            _ => COREWEBVIEW2_PREFERRED_COLOR_SCHEME_AUTO,
        };
        let _ = p.SetPreferredColorScheme(scheme);
    }
}

/// Settings for the browser-chrome webview (tab strip, toolbar, panels).
pub fn configure_ui(wv: &ICoreWebView2) {
    unsafe {
        if let Ok(st) = wv.Settings() {
            let _ = st.SetAreDevToolsEnabled(cfg!(debug_assertions));
            let _ = st.SetAreDefaultContextMenusEnabled(cfg!(debug_assertions));
            let _ = st.SetIsStatusBarEnabled(false);
            let _ = st.SetIsZoomControlEnabled(false);
            let _ = st.SetIsWebMessageEnabled(true);
            if let Ok(s3) = st.cast::<ICoreWebView2Settings3>() {
                let _ = s3.SetAreBrowserAcceleratorKeysEnabled(false);
            }
            if let Ok(s4) = st.cast::<ICoreWebView2Settings4>() {
                let _ = s4.SetIsPasswordAutosaveEnabled(false);
                let _ = s4.SetIsGeneralAutofillEnabled(false);
            }
            if let Ok(s5) = st.cast::<ICoreWebView2Settings5>() {
                let _ = s5.SetIsPinchZoomEnabled(false);
            }
            if let Ok(s6) = st.cast::<ICoreWebView2Settings6>() {
                let _ = s6.SetIsSwipeNavigationEnabled(false);
            }
            if let Ok(s9) = st.cast::<ICoreWebView2Settings9>() {
                let _ = s9.SetIsNonClientRegionSupportEnabled(true);
            }
        }
    }
}

pub fn set_background(ctl: &ICoreWebView2Controller, r: u8, g: u8, b: u8) {
    unsafe {
        if let Ok(c2) = ctl.cast::<ICoreWebView2Controller2>() {
            let _ = c2.SetDefaultBackgroundColor(COREWEBVIEW2_COLOR { A: 255, R: r, G: g, B: b });
        }
    }
}

pub fn get_source(wv: &ICoreWebView2) -> String {
    unsafe {
        let mut p = PWSTR::null();
        if wv.Source(&mut p).is_ok() {
            return take_pwstr(p);
        }
    }
    String::new()
}

pub fn get_title(wv: &ICoreWebView2) -> String {
    unsafe {
        let mut p = PWSTR::null();
        if wv.DocumentTitle(&mut p).is_ok() {
            return take_pwstr(p);
        }
    }
    String::new()
}

pub fn navigate(wv: &ICoreWebView2, url: &str) {
    if url.starts_with("solus:") {
        crate::scheme::allow_internal_once(wv);
    }
    let u = WStr::new(url);
    unsafe {
        if let Err(e) = wv.Navigate(u.pcwstr()) {
            crate::logf!("navigate failed: {e}");
        }
    }
}

pub fn post_json(wv: &ICoreWebView2, json: &str) {
    let s = WStr::new(json);
    unsafe {
        let _ = wv.PostWebMessageAsJson(s.pcwstr());
    }
}

/// Run script and deliver the JSON result (or None on failure) to `cb` on the UI thread.
pub fn exec_script(wv: &ICoreWebView2, script: &str, cb: impl FnOnce(&mut App, Option<String>) + 'static) {
    let s = WStr::new(script);
    let cb = std::cell::Cell::new(Some(cb));
    let h = ExecuteScriptCompletedHandler::create(Box::new(move |hr, result| {
        if let Some(cb) = cb.take() {
            let r = if hr.is_ok() { Some(result) } else { None };
            defer(move |app| cb(app, r));
        }
        Ok(())
    }));
    unsafe {
        let _ = wv.ExecuteScript(s.pcwstr(), &h);
    }
}

pub fn bool_of(b: BOOL) -> bool {
    b.as_bool()
}
