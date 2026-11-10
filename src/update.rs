//! Automatic updates for Solus itself (the Chromium engine is kept up to date separately by
//! Windows through the Evergreen WebView2 Runtime).
//!
//! The feed `latest.json` names the installer URL, its SHA-256 and an Ed25519 signature of that
//! hash made with the Solus release key. An update is only installed when both checks pass, so
//! a tampered download or a compromised host can't push code onto the PC.

use std::path::PathBuf;

use base64::Engine;
use serde_json::{json, Value};
use sha2::Digest;

use crate::app::{post, App};
use crate::bridge::{self, ok, Ctx, Reply};
use crate::util::{self, now_ms};

pub const FEED: &str = "https://github.com/okiwik2/solus/releases/latest/download/latest.json";
/// Ed25519 public key of the Solus release signing key (private half never leaves the release PC).
pub const PUBKEY_B64: &str = include_str!("../res/release-pubkey.txt");

#[derive(Clone, Default)]
pub struct Ready {
    pub version: String,
    pub path: PathBuf,
    pub notes: String,
}

thread_local! {
    pub static READY: std::cell::RefCell<Option<Ready>> = const { std::cell::RefCell::new(None) };
    static CHECKING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub fn newer(remote: &str, local: &str) -> bool {
    let p = |s: &str| -> Vec<u64> { s.trim_start_matches('v').split(['.', '-']).take(3).map(|x| x.parse().unwrap_or(0)).collect() };
    p(remote) > p(local)
}

pub fn verify(bytes: &[u8], sha_hex: &str, sig_b64: &str) -> Result<(), String> {
    let digest = sha2::Sha256::digest(bytes);
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    if !hex.eq_ignore_ascii_case(sha_hex.trim()) {
        return Err("The downloaded update is corrupt (checksum mismatch)".into());
    }
    let key_bytes: [u8; 32] = base64::engine::general_purpose::STANDARD
        .decode(PUBKEY_B64.trim())
        .ok()
        .and_then(|v| v.try_into().ok())
        .ok_or("release key missing")?;
    let key = ed25519_dalek::VerifyingKey::from_bytes(&key_bytes).map_err(|_| "release key invalid")?;
    let sig_bytes: [u8; 64] = base64::engine::general_purpose::STANDARD
        .decode(sig_b64.trim())
        .ok()
        .and_then(|v| v.try_into().ok())
        .ok_or("The update's signature is malformed")?;
    let sig = ed25519_dalek::Signature::from_bytes(&sig_bytes);
    key.verify_strict(hex.as_bytes(), &sig).map_err(|_| "The update is not signed by Solus — it was not installed".to_string())
}

/// Test builds can point at a local feed, but only when the automation token is also present.
fn feed_url() -> (String, bool) {
    match (std::env::var("SOLUS_UPDATE_FEED"), std::env::var_os("SOLUS_AUTOMATION_TOKEN")) {
        (Ok(f), Some(_)) if f.starts_with("http://127.0.0.1:") => (f, true),
        _ => (FEED.to_string(), false),
    }
}

fn updates_dir() -> PathBuf {
    util::user_data_root().join("Updates")
}

/// Check (and download) in the background. `manual` reports "up to date" too.
pub fn check(app: &mut App, notify: Option<(Ctx, Value)>) {
    if CHECKING.with(|c| c.replace(true)) {
        if let Some((c, i)) = notify {
            bridge::respond(app, &c, &i, Ok(json!({"status": "checking"})));
        }
        return;
    }
    app.local.update_last_check = now_ms();
    app.local_dirty = true;
    std::thread::spawn(move || {
        let r = (|| -> Result<Option<Ready>, String> {
            let (feed, test_feed) = feed_url();
            let text = crate::net::get_text(&feed).map_err(|e| {
                if e.contains("404") {
                    "No update feed has been published yet".to_string()
                } else {
                    format!("Couldn't check for updates: {e}")
                }
            })?;
            let v: Value = serde_json::from_str(&text).map_err(|_| "The update feed is malformed")?;
            let version = v["version"].as_str().unwrap_or("");
            if !newer(version, crate::VERSION) {
                return Ok(None);
            }
            let url = v["url"].as_str().unwrap_or("");
            let trusted_host = url.starts_with("https://github.com/") || url.starts_with("https://objects.githubusercontent.com/") || (test_feed && url.starts_with("http://127.0.0.1:"));
            if !trusted_host {
                return Err("The update feed points somewhere unexpected; ignoring it".into());
            }
            let bytes = crate::net::get_bytes(url, 600 * 1024 * 1024)?;
            verify(&bytes, v["sha256"].as_str().unwrap_or(""), v["signature"].as_str().unwrap_or(""))?;
            let dir = updates_dir();
            let _ = std::fs::create_dir_all(&dir);
            let path = dir.join(format!("SolusSetup-{version}.exe"));
            std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
            Ok(Some(Ready { version: version.into(), path, notes: v["notes"].as_str().unwrap_or("").into() }))
        })();
        post(move |app| {
            CHECKING.with(|c| c.set(false));
            let status = match &r {
                Ok(Some(rd)) => {
                    logf!("update {} downloaded and verified", rd.version);
                    READY.with(|x| *x.borrow_mut() = Some(rd.clone()));
                    let ids: Vec<u32> = app.windows.iter().map(|w| w.id).collect();
                    for id in ids {
                        bridge::emit(app, id, "infobar", json!({"id": "update", "icon": "download", "text": format!("Solus {} is ready to install.", rd.version),
                            "buttons": [{"label": "Restart now", "primary": true, "cmd": "update.restart"}]}));
                    }
                    json!({"status": "ready", "version": rd.version, "notes": rd.notes})
                }
                Ok(None) => json!({"status": "current", "version": crate::VERSION}),
                Err(e) => {
                    logf!("update check: {e}");
                    json!({"status": "error", "error": e})
                }
            };
            if let Some((c, i)) = notify {
                bridge::respond(app, &c, &i, Ok(status));
            }
        });
    });
}

pub fn maybe_check(app: &mut App) {
    if app.local.update_check && now_ms() - app.local.update_last_check > 86_400_000 {
        check(app, None);
    }
}

/// Run the verified installer silently. Called at exit (and by "Restart now").
pub fn apply_on_exit(relaunch: bool) -> bool {
    let Some(r) = READY.with(|x| x.borrow_mut().take()) else { return false };
    if !r.path.exists() {
        return false;
    }
    let mut cmd = std::process::Command::new(&r.path);
    cmd.args(["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/CLOSEAPPLICATIONS"]);
    if relaunch {
        cmd.arg("/RELAUNCH=1");
    }
    match cmd.spawn() {
        Ok(_) => {
            logf!("starting update installer {}", r.version);
            true
        }
        Err(e) => {
            logf!("couldn't start the update installer: {e}");
            false
        }
    }
}

pub fn cmd(app: &mut App, ctx: &Ctx, cmd: &str, _a: &Value, id: &Value) -> Reply {
    match cmd {
        "update.status" => Reply::Now(json!({
            "version": crate::VERSION,
            "engine": app.env().map(|e| crate::webview::runtime_version(&e)).unwrap_or_default(),
            "ready": READY.with(|r| r.borrow().as_ref().map(|x| json!({"version": x.version, "notes": x.notes}))),
            "lastCheck": app.local.update_last_check, "auto": app.local.update_check, "feed": FEED,
        })),
        "update.check" => {
            check(app, Some((ctx.clone(), id.clone())));
            Reply::Later
        }
        "update.restart" => {
            if READY.with(|r| r.borrow().is_none()) {
                return Reply::Err("No update is ready".into());
            }
            app.update_relaunch = true;
            app.quit();
            ok()
        }
        _ => Reply::Err("unknown update command".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn version_compare() {
        assert!(newer("0.2.0", "0.1.9"));
        assert!(newer("v1.0.0", "0.9.9"));
        assert!(!newer("0.1.0", "0.1.0"));
        assert!(!newer("0.0.9", "0.1.0"));
    }
    #[test]
    fn rejects_bad_checksum_and_signature() {
        assert!(verify(b"abc", "00", "AAAA").unwrap_err().contains("checksum"));
        let hex: String = sha2::Sha256::digest(b"abc").iter().map(|b| format!("{b:02x}")).collect();
        let fake_sig = base64::engine::general_purpose::STANDARD.encode([7u8; 64]);
        assert!(verify(b"abc", &hex, &fake_sig).is_err());
    }
}
