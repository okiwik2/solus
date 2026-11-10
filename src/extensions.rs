//! Chrome Web Store extensions on the WebView2 extension runtime.
//!
//! Installing downloads the extension package (CRX) from Google's official update service over
//! HTTPS, unpacks it into the profile and registers it with the engine. WebView2 supports a subset
//! of Chrome's extension APIs, so some extensions work fully, some partly, and some not at all.

use std::io::Read;
use std::path::{Path, PathBuf};

use base64::Engine;
use rusqlite::params;
use serde_json::{json, Value};
use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::*;
use std::result::Result;
use windows::core::{Interface, PWSTR};

use crate::app::{defer, App};
use crate::bridge::{self, ok, Ctx, Reply};
use crate::util::{now_ms, WStr};

/// Popular extensions the user can install with one click from the Extensions page.
pub fn suggested() -> Value {
    json!([
        {"id": "eimadpbcbfnmbkopoojfekhnkhdbieeh", "name": "Dark Reader", "about": "Dark mode for every website"},
        {"id": "nngceckbapebfimnlniiiahkandclblb", "name": "Bitwarden Password Manager", "about": "Open-source password manager"},
        {"id": "gebbhagfogifgggkldgodflihgfeippi", "name": "Return YouTube Dislike", "about": "Shows YouTube dislike counts"},
        {"id": "dhdgffkkebhmkfjojejmpbldmpobfkfo", "name": "Tampermonkey", "about": "Userscript manager"},
        {"id": "clngdbkpkpeebahjckkjfobafhncgmne", "name": "Stylus", "about": "Custom CSS for websites"},
    ])
}

pub fn parse_id(input: &str) -> Option<String> {
    let s = input.trim();
    let is_id = |x: &str| x.len() == 32 && x.chars().all(|c| ('a'..='p').contains(&c));
    if is_id(s) {
        return Some(s.to_string());
    }
    // https://chromewebstore.google.com/detail/<name>/<id>
    s.split(['/', '?', '#']).find(|p| is_id(p)).map(|p| p.to_string())
}

fn ext_root(app: &mut App, profile: &str) -> PathBuf {
    app.profile(profile).dir.join("Extensions")
}

/// Strip the CRX2/CRX3 header and return the embedded ZIP.
pub fn crx_to_zip(b: &[u8]) -> Result<&[u8], String> {
    if b.len() < 16 || &b[0..4] != b"Cr24" {
        // some mirrors serve a plain zip
        if b.starts_with(b"PK\x03\x04") {
            return Ok(b);
        }
        return Err("Not a Chrome extension package".into());
    }
    let ver = u32::from_le_bytes(b[4..8].try_into().unwrap());
    let start = match ver {
        3 => 12 + u32::from_le_bytes(b[8..12].try_into().unwrap()) as usize,
        2 => {
            let k = u32::from_le_bytes(b[8..12].try_into().unwrap()) as usize;
            let s = u32::from_le_bytes(b[12..16].try_into().unwrap()) as usize;
            16 + k + s
        }
        _ => return Err(format!("Unsupported package version {ver}")),
    };
    b.get(start..).filter(|z| z.starts_with(b"PK")).ok_or_else(|| "Damaged extension package".to_string())
}

pub fn unpack(zip_bytes: &[u8], dest: &Path) -> Result<Value, String> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).map_err(|e| format!("Damaged package: {e}"))?;
    if dest.exists() {
        let _ = std::fs::remove_dir_all(dest);
    }
    std::fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    let mut total: u64 = 0;
    for i in 0..z.len() {
        let mut f = z.by_index(i).map_err(|e| e.to_string())?;
        // refuses absolute paths and "..": no writing outside the extension folder
        let Some(rel) = f.enclosed_name() else { return Err("Unsafe path in extension package".into()) };
        let out = dest.join(rel);
        if f.is_dir() {
            std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
            continue;
        }
        total += f.size();
        if total > 300 * 1024 * 1024 {
            return Err("Extension is too large".into());
        }
        if let Some(p) = out.parent() {
            std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
        }
        let mut buf = Vec::with_capacity(f.size() as usize);
        f.read_to_end(&mut buf).map_err(|e| e.to_string())?;
        std::fs::write(&out, buf).map_err(|e| e.to_string())?;
    }
    let manifest = std::fs::read(dest.join("manifest.json")).map_err(|_| "The package has no manifest.json".to_string())?;
    let m: Value = serde_json::from_slice(strip_bom(&manifest)).map_err(|e| format!("Invalid manifest: {e}"))?;
    Ok(m)
}

fn strip_bom(b: &[u8]) -> &[u8] {
    b.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(b)
}

fn localized(dir: &Path, m: &Value, s: &str) -> String {
    // "__MSG_appName__" -> _locales/<default>/messages.json
    if let Some(key) = s.strip_prefix("__MSG_").and_then(|x| x.strip_suffix("__")) {
        let loc = m["default_locale"].as_str().unwrap_or("en");
        for l in [loc, "en", "en_US"] {
            if let Ok(b) = std::fs::read(dir.join("_locales").join(l).join("messages.json")) {
                if let Ok(v) = serde_json::from_slice::<Value>(strip_bom(&b)) {
                    if let Some(obj) = v.as_object() {
                        if let Some((_, e)) = obj.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)) {
                            return e["message"].as_str().unwrap_or(s).to_string();
                        }
                    }
                }
            }
        }
    }
    s.to_string()
}

fn icon_data(dir: &Path, m: &Value) -> String {
    let icons = m["icons"].as_object().cloned().unwrap_or_default();
    let mut best: Option<(u32, String)> = None;
    for (k, v) in icons {
        let size: u32 = k.parse().unwrap_or(0);
        if let Some(p) = v.as_str() {
            if best.as_ref().map(|b| (size as i32 - 32).abs() < (b.0 as i32 - 32).abs()).unwrap_or(true) {
                best = Some((size, p.to_string()));
            }
        }
    }
    best.and_then(|(_, p)| std::fs::read(dir.join(p.trim_start_matches('/'))).ok())
        .filter(|b| b.len() < 200_000)
        .map(|b| format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(b)))
        .unwrap_or_default()
}

fn profile_iface(app: &App, wid: u32) -> Option<ICoreWebView2Profile7> {
    let w = app.win(wid)?;
    let wv = w.ui.as_ref().map(|v| v.wv.clone())?;
    unsafe { wv.cast::<ICoreWebView2_13>().ok()?.Profile().ok()?.cast::<ICoreWebView2Profile7>().ok() }
}

fn download_crx(id: &str, version_hint: &str) -> Result<Vec<u8>, String> {
    let ver = if version_hint.is_empty() { "157.0.0.0" } else { version_hint };
    let url = format!(
        "https://clients2.google.com/service/update2/crx?response=redirect&prodversion={ver}&acceptformat=crx2,crx3&x=id%3D{id}%26uc"
    );
    let b = crate::net::get_bytes(&url, 200 * 1024 * 1024)?;
    if b.is_empty() {
        return Err("The Chrome Web Store returned nothing for this extension".into());
    }
    Ok(b)
}

/// Install (or update) from the Chrome Web Store. Network + unpacking run on a worker thread.
fn install(app: &mut App, ctx: Ctx, rid: Value, cws_id: String) {
    let Some(w) = app.win(ctx.wid) else { return };
    let profile = w.profile.clone();
    let root = ext_root(app, &profile);
    let engine_ver = app.env().map(|e| crate::webview::runtime_version(&e)).unwrap_or_default();
    let engine_ver = engine_ver.split_whitespace().next().unwrap_or("").to_string();
    std::thread::spawn(move || {
        let res = (|| -> Result<(PathBuf, Value), String> {
            let crx = download_crx(&cws_id, &engine_ver)?;
            let zip = crx_to_zip(&crx)?;
            let tmp = root.join(format!("{cws_id}.tmp"));
            let m = unpack(zip, &tmp)?;
            let version = m["version"].as_str().unwrap_or("0").to_string();
            let dest = root.join(&cws_id).join(&version);
            if dest.exists() {
                let _ = std::fs::remove_dir_all(&dest);
            }
            std::fs::create_dir_all(dest.parent().unwrap()).map_err(|e| e.to_string())?;
            std::fs::rename(&tmp, &dest).map_err(|e| e.to_string())?;
            Ok((dest, m))
        })();
        crate::app::post(move |app| match res {
            Err(e) => bridge::respond(app, &ctx, &rid, Err(e)),
            Ok((dir, m)) => register(app, ctx, rid, profile, cws_id, dir, m),
        });
    });
}

fn register(app: &mut App, ctx: Ctx, rid: Value, profile: String, cws_id: String, dir: PathBuf, m: Value) {
    let Some(p7) = profile_iface(app, ctx.wid) else {
        bridge::respond(app, &ctx, &rid, Err("The engine isn't ready yet".into()));
        return;
    };
    let name = localized(&dir, &m, m["name"].as_str().unwrap_or(&cws_id));
    // replace an older version first
    let old: Option<String> = app.profile(&profile).db().and_then(|d| {
        d.conn.query_row("SELECT id FROM extensions WHERE path LIKE ?1", [format!("%{cws_id}%")], |r| r.get(0)).ok()
    });
    let dir_w = WStr::path(&dir);
    let version = m["version"].as_str().unwrap_or("").to_string();
    let icon = icon_data(&dir, &m);
    let popup = m["action"]["default_popup"].as_str().or(m["browser_action"]["default_popup"].as_str()).unwrap_or("").to_string();
    let options = m["options_ui"]["page"].as_str().or(m["options_page"].as_str()).unwrap_or("").to_string();
    let mv = m["manifest_version"].as_i64().unwrap_or(0);
    let (ctx_err, rid_err) = (ctx.clone(), rid.clone());
    let h = ProfileAddBrowserExtensionCompletedHandler::create(Box::new(move |hr, ext| {
        let res = match (hr, ext) {
            (Ok(()), Some(ext)) => unsafe {
                let mut p = PWSTR::null();
                let _ = ext.Id(&mut p);
                Ok(take_pwstr(p))
            },
            (Err(e), _) => Err(format!("The engine refused this extension: {e}")),
            _ => Err("The engine refused this extension".to_string()),
        };
        let (ctx, rid, profile, dir, name, old, version, icon, popup, options, cws_id) =
            (ctx.clone(), rid.clone(), profile.clone(), dir.clone(), name.clone(), old.clone(), version.clone(), icon.clone(), popup.clone(), options.clone(), cws_id.clone());
        defer(move |app| match res {
            Ok(id) => {
                if let Some(db) = app.profile(&profile).db() {
                    if let Some(o) = &old {
                        if *o != id {
                            let _ = db.conn.execute("DELETE FROM extensions WHERE id=?1", [o]);
                        }
                    }
                    let meta = json!({"cws": cws_id, "icon": icon, "popup": popup, "options": options, "mv": mv}).to_string();
                    let _ = db.conn.execute(
                        "INSERT OR REPLACE INTO extensions(id,name,version,enabled,path,installed,source) VALUES(?1,?2,?3,1,?4,?5,?6)",
                        params![id, name, version, dir.to_string_lossy(), now_ms(), meta],
                    );
                }
                logf!("extension installed: {name} {version}");
                let ids: Vec<u32> = app.windows.iter().filter(|w| w.profile == profile).map(|w| w.id).collect();
                for w in ids {
                    bridge::emit(app, w, "extensionsChanged", json!({}));
                }
                bridge::respond(app, &ctx, &rid, Ok(json!({"id": id, "name": name, "version": version, "mv": mv})));
            }
            Err(e) => {
                let _ = std::fs::remove_dir_all(&dir);
                bridge::respond(app, &ctx, &rid, Err(e));
            }
        });
        Ok(())
    }));
    unsafe {
        if let Err(e) = p7.AddBrowserExtension(dir_w.pcwstr(), &h) {
            bridge::respond(app, &ctx_err, &rid_err, Err(format!("Couldn't add the extension: {e}")));
        }
    }
}

fn rows(app: &mut App, profile: &str) -> Vec<Value> {
    let Some(db) = app.profile(profile).db() else { return vec![] };
    let Ok(mut st) = db.conn.prepare("SELECT id,name,version,enabled,path,installed,source FROM extensions ORDER BY name") else { return vec![] };
    st.query_map([], |r| {
        let meta: Value = serde_json::from_str(&r.get::<_, String>(6)?).unwrap_or(json!({}));
        let id: String = r.get(0)?;
        Ok(json!({
            "id": id, "name": r.get::<_, String>(1)?, "version": r.get::<_, String>(2)?, "enabled": r.get::<_, i64>(3)? != 0,
            "installed": r.get::<_, i64>(5)?, "cws": meta["cws"], "icon": meta["icon"], "mv": meta["mv"],
            "popup": if meta["popup"].as_str().unwrap_or("").is_empty() { Value::Null } else { json!(format!("chrome-extension://{}/{}", id, meta["popup"].as_str().unwrap().trim_start_matches('/'))) },
            "options": if meta["options"].as_str().unwrap_or("").is_empty() { Value::Null } else { json!(format!("chrome-extension://{}/{}", id, meta["options"].as_str().unwrap().trim_start_matches('/'))) },
        }))
    })
    .map(|r| r.flatten().collect())
    .unwrap_or_default()
}

pub fn cmd(app: &mut App, ctx: &Ctx, cmd: &str, a: &Value, id: &Value) -> Reply {
    let profile = app.win(ctx.wid).map(|w| w.profile.clone()).unwrap_or_default();
    if app.win(ctx.wid).map(|w| w.incognito).unwrap_or(false) && cmd != "ext.list" {
        return Reply::Err("Extensions can't be changed from an Incognito window".into());
    }
    match cmd {
        "ext.list" => Reply::Now(json!(rows(app, &profile))),
        "ext.suggested" => Reply::Now(suggested()),
        "ext.install" => {
            let Some(cws) = parse_id(a["id"].as_str().unwrap_or("")) else {
                return Reply::Err("Paste a Chrome Web Store link (chromewebstore.google.com/detail/...) or a 32-letter extension ID".into());
            };
            install(app, ctx.clone(), id.clone(), cws);
            Reply::Later
        }
        "ext.remove" | "ext.enable" => {
            let ext_id = a["id"].as_str().unwrap_or("").to_string();
            let Some(p7) = profile_iface(app, ctx.wid) else { return Reply::Err("engine not ready".into()) };
            let enable = a["enabled"].as_bool().unwrap_or(true);
            let remove = cmd == "ext.remove";
            let (c, rid, prof) = (ctx.clone(), id.clone(), profile.clone());
            let h = ProfileGetBrowserExtensionsCompletedHandler::create(Box::new(move |_, list| {
                let mut found: Option<ICoreWebView2BrowserExtension> = None;
                if let Some(list) = list {
                    unsafe {
                        let mut n = 0u32;
                        let _ = list.Count(&mut n);
                        for i in 0..n {
                            if let Ok(e) = list.GetValueAtIndex(i) {
                                let mut p = PWSTR::null();
                                let _ = e.Id(&mut p);
                                if take_pwstr(p) == ext_id {
                                    found = Some(e);
                                }
                            }
                        }
                    }
                }
                let (c, rid, prof, ext_id) = (c.clone(), rid.clone(), prof.clone(), ext_id.clone());
                defer(move |app| {
                    if let Some(db) = app.profile(&prof).db() {
                        if remove {
                            if let Ok(path) = db.conn.query_row("SELECT path FROM extensions WHERE id=?1", [&ext_id], |r| r.get::<_, String>(0)) {
                                let _ = std::fs::remove_dir_all(&path);
                            }
                            let _ = db.conn.execute("DELETE FROM extensions WHERE id=?1", [&ext_id]);
                        } else {
                            let _ = db.conn.execute("UPDATE extensions SET enabled=?2 WHERE id=?1", params![ext_id, enable as i64]);
                        }
                    }
                    if let Some(e) = found {
                        unsafe {
                            if remove {
                                let _ = e.Remove(&BrowserExtensionRemoveCompletedHandler::create(Box::new(|_| Ok(()))));
                            } else {
                                let _ = e.Enable(enable, &BrowserExtensionEnableCompletedHandler::create(Box::new(|_| Ok(()))));
                            }
                        }
                    }
                    let ids: Vec<u32> = app.windows.iter().filter(|w| w.profile == prof).map(|w| w.id).collect();
                    for w in ids {
                        bridge::emit(app, w, "extensionsChanged", json!({}));
                    }
                    bridge::respond(app, &c, &rid, Ok(json!(true)));
                });
                Ok(())
            }));
            unsafe {
                let _ = p7.GetBrowserExtensions(&h);
            }
            Reply::Later
        }
        "ext.openPopup" | "ext.openOptions" => {
            let ext_id = a["id"].as_str().unwrap_or("");
            let list = rows(app, &profile);
            let Some(e) = list.iter().find(|x| x["id"] == ext_id) else { return Reply::Err("not installed".into()) };
            let url = if cmd == "ext.openPopup" { e["popup"].as_str() } else { e["options"].as_str() };
            match url {
                Some(u) => {
                    app.open_tab(ctx.wid, u, crate::tab::OpenOpts { activate: true, ..Default::default() });
                    ok()
                }
                None => Reply::Err("This extension has no page to open".into()),
            }
        }
        "ext.update" => {
            // re-install every extension from the store (keeps settings stored by the extension)
            let list = rows(app, &profile);
            let ids: Vec<String> = list.iter().filter_map(|e| e["cws"].as_str().map(String::from)).collect();
            for cws in ids {
                install(app, Ctx { wid: ctx.wid, tid: ctx.tid, page: ctx.page.clone() }, Value::Null, cws);
            }
            ok()
        }
        _ => Reply::Err("unknown extensions command".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util;
    #[test]
    fn ids_from_urls() {
        assert_eq!(parse_id("https://chromewebstore.google.com/detail/dark-reader/eimadpbcbfnmbkopoojfekhnkhdbieeh?hl=en").as_deref(), Some("eimadpbcbfnmbkopoojfekhnkhdbieeh"));
        assert_eq!(parse_id("eimadpbcbfnmbkopoojfekhnkhdbieeh").as_deref(), Some("eimadpbcbfnmbkopoojfekhnkhdbieeh"));
        assert!(parse_id("https://example.com/abc").is_none());
    }
    #[test]
    fn crx3_header_stripped_and_zip_slip_refused() {
        // build a zip with a manifest and a malicious entry
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            let o: zip::write::SimpleFileOptions = Default::default();
            z.start_file("manifest.json", o).unwrap();
            std::io::Write::write_all(&mut z, br#"{"name":"T","version":"1.2","manifest_version":3}"#).unwrap();
            z.finish().unwrap();
        }
        let zip_bytes = buf.into_inner();
        let mut crx = b"Cr24".to_vec();
        crx.extend(3u32.to_le_bytes());
        crx.extend(4u32.to_le_bytes());
        crx.extend([1, 2, 3, 4]);
        crx.extend(&zip_bytes);
        let z = crx_to_zip(&crx).unwrap();
        let dir = std::env::temp_dir().join(format!("solus-ext-{}", util::new_id()));
        let m = unpack(z, &dir).unwrap();
        assert_eq!(m["version"], "1.2");
        let mut bad = std::io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut bad);
            let o: zip::write::SimpleFileOptions = Default::default();
            z.start_file("../evil.txt", o).unwrap();
            std::io::Write::write_all(&mut z, b"x").unwrap();
            z.finish().unwrap();
        }
        assert!(unpack(&bad.into_inner(), &dir.join("x")).is_err());
    }
}
