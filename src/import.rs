//! Chrome data migration (bookmarks, history, search shortcuts, start-up settings) and the guided
//! password import.
//!
//! Safety: Solus never writes to Chrome's profile folder. Files are opened read-only with full
//! sharing (so a running Chrome is not disturbed), copied to a private temp folder, and only the
//! copies are read. Chrome's saved passwords and cookies are protected with Chrome's App-Bound
//! Encryption (Chrome 127+), which only Chrome itself can unlock — so passwords come across through
//! Chrome's own "Export passwords" CSV, never by reading Chrome's database.

use std::io::Read;
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde_json::{json, Value};

use crate::app::App;
use crate::bridge::{self, Ctx, Reply};
use crate::util::{self, now_ms};

pub fn chrome_exe() -> Option<PathBuf> {
    let mut c = vec![];
    for var in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
        if let Some(v) = std::env::var_os(var) {
            c.push(PathBuf::from(v).join("Google").join("Chrome").join("Application").join("chrome.exe"));
        }
    }
    c.into_iter().find(|p| p.exists())
}

pub fn chrome_user_data() -> Option<PathBuf> {
    let p = PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("Google").join("Chrome").join("User Data");
    p.is_dir().then_some(p)
}

/// Read a file that another process may have open for writing.
fn read_shared(p: &Path) -> std::io::Result<Vec<u8>> {
    // FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE
    let mut f = std::fs::OpenOptions::new().read(true).share_mode(7).open(p)?;
    let mut b = Vec::new();
    f.read_to_end(&mut b)?;
    Ok(b)
}

fn private_copy(p: &Path) -> Result<PathBuf, String> {
    let bytes = read_shared(p).map_err(|e| format!("could not read {}: {e}", p.display()))?;
    let dir = std::env::temp_dir().join(format!("solus-import-{}", util::new_id()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let dst = dir.join(p.file_name().unwrap());
    std::fs::write(&dst, bytes).map_err(|e| e.to_string())?;
    Ok(dst)
}

fn cleanup(p: &Path) {
    if let Some(d) = p.parent() {
        if d.file_name().map(|n| n.to_string_lossy().starts_with("solus-import-")).unwrap_or(false) {
            let _ = std::fs::remove_dir_all(d);
        }
    }
}

pub fn chrome_running() -> bool {
    let out = std::process::Command::new("tasklist")
        .args(["/FI", "IMAGENAME eq chrome.exe", "/NH"])
        .creation_flags_hidden()
        .output();
    out.map(|o| String::from_utf8_lossy(&o.stdout).to_lowercase().contains("chrome.exe")).unwrap_or(false)
}

trait Hidden {
    fn creation_flags_hidden(&mut self) -> &mut Self;
}
impl Hidden for std::process::Command {
    fn creation_flags_hidden(&mut self) -> &mut Self {
        use std::os::windows::process::CommandExt;
        self.creation_flags(0x0800_0000) // CREATE_NO_WINDOW
    }
}

pub fn list_profiles() -> Vec<Value> {
    let Some(root) = chrome_user_data() else { return vec![] };
    let local: Value = read_shared(&root.join("Local State")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(json!({}));
    let mut out = vec![];
    if let Some(cache) = local["profile"]["info_cache"].as_object() {
        for (dir, info) in cache {
            let p = root.join(dir);
            if !p.is_dir() {
                continue;
            }
            out.push(json!({
                "dir": dir,
                "name": info["name"].as_str().unwrap_or(dir),
                "email": info["user_name"].as_str().unwrap_or(""),
                "hasBookmarks": p.join("Bookmarks").exists(),
                "hasHistory": p.join("History").exists(),
            }));
        }
    }
    if out.is_empty() && root.join("Default").is_dir() {
        out.push(json!({"dir": "Default", "name": "Default", "email": "", "hasBookmarks": root.join("Default/Bookmarks").exists(), "hasHistory": root.join("Default/History").exists()}));
    }
    out
}

fn chrome_time_to_ms(t: i64) -> i64 {
    // microseconds since 1601-01-01 -> milliseconds since 1970-01-01
    if t <= 0 {
        return 0;
    }
    t / 1000 - 11_644_473_600_000
}

/// Convert Chrome's bookmark JSON node into Solus' import tree format.
pub fn convert_bookmark_node(n: &Value) -> Option<Value> {
    match n["type"].as_str()? {
        "url" => Some(json!({"kind": "url", "title": n["name"].as_str().unwrap_or(""), "url": n["url"].as_str().unwrap_or("")})),
        "folder" => Some(json!({
            "kind": "folder",
            "title": n["name"].as_str().unwrap_or("Folder"),
            "children": n["children"].as_array().map(|c| c.iter().filter_map(convert_bookmark_node).collect::<Vec<_>>()).unwrap_or_default(),
        })),
        _ => None,
    }
}

pub fn import_bookmarks(db: &crate::store::Store, json_bytes: &[u8]) -> Result<usize, String> {
    let v: Value = serde_json::from_slice(json_bytes).map_err(|e| format!("Bookmarks file is unreadable: {e}"))?;
    let roots = &v["roots"];
    let bar: Vec<Value> = roots["bookmark_bar"]["children"].as_array().map(|c| c.iter().filter_map(convert_bookmark_node).collect()).unwrap_or_default();
    let other: Vec<Value> = roots["other"]["children"].as_array().map(|c| c.iter().filter_map(convert_bookmark_node).collect()).unwrap_or_default();
    let mobile: Vec<Value> = roots["synced"]["children"].as_array().map(|c| c.iter().filter_map(convert_bookmark_node).collect()).unwrap_or_default();
    let mut n = 0;
    // an empty Solus bookmarks bar takes Chrome's bar as-is; otherwise import into a folder
    let bar_empty = db.bm_children("bar").is_empty();
    if bar_empty {
        n += db.bm_import_tree("bar", &bar);
    } else if !bar.is_empty() {
        n += db.bm_import_tree("bar", &[json!({"kind": "folder", "title": "Imported from Chrome", "children": bar})]);
    }
    n += db.bm_import_tree("other", &other);
    if !mobile.is_empty() {
        n += db.bm_import_tree("other", &[json!({"kind": "folder", "title": "Mobile bookmarks", "children": mobile})]);
    }
    Ok(n)
}

pub fn import_history(db: &crate::store::Store, history_db: &Path) -> Result<usize, String> {
    let c = Connection::open_with_flags(history_db, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)
        .map_err(|e| e.to_string())?;
    let mut st = c
        .prepare("SELECT url, title, visit_count, typed_count, last_visit_time FROM urls WHERE hidden=0 ORDER BY last_visit_time DESC LIMIT 25000")
        .map_err(|e| format!("History is unreadable (close Chrome and try again): {e}"))?;
    let rows: Vec<(String, String, i64, i64, i64)> = st
        .query_map([], |r| Ok((r.get(0)?, r.get::<_, Option<String>>(1)?.unwrap_or_default(), r.get(2)?, r.get(3)?, r.get(4)?)))
        .map_err(|e| e.to_string())?
        .flatten()
        .collect();
    let tx = db.conn.unchecked_transaction().map_err(|e| e.to_string())?;
    let mut n = 0;
    for (url, title, visits, typed, last) in rows {
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            continue;
        }
        let last = chrome_time_to_ms(last);
        let tag = db.keys.tag(url.as_bytes());
        let existing: Option<i64> = tx.query_row("SELECT id FROM history_urls WHERE tag=?1", [&tag], |r| r.get(0)).optional().unwrap_or(None);
        let id = match existing {
            Some(id) => {
                let _ = tx.execute(
                    "UPDATE history_urls SET visits=visits+?2, typed=typed+?3, last_visit=MAX(last_visit,?4), deleted=0 WHERE id=?1",
                    params![id, visits, typed, last],
                );
                id
            }
            None => {
                let site_tag = db.keys.tag(util::site_of(&url).as_bytes());
                tx.execute(
                    "INSERT INTO history_urls(uid,tag,url,title,site_tag,visits,typed,last_visit,updated) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                    params![util::new_id(), tag, db.keys.seal_str(&url), db.keys.seal_str(&title), site_tag, visits.max(1), typed, last, now_ms()],
                )
                .map_err(|e| e.to_string())?;
                tx.last_insert_rowid()
            }
        };
        let _ = tx.execute("INSERT INTO history_visits(url_id, ts, transition) VALUES(?1,?2,'imported')", params![id, last]);
        n += 1;
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(n)
}

pub fn read_search_shortcuts(web_data: &Path) -> Vec<crate::settings::SearchShortcut> {
    let Ok(c) = Connection::open_with_flags(web_data, OpenFlags::SQLITE_OPEN_READ_ONLY) else { return vec![] };
    let Ok(mut st) = c.prepare("SELECT short_name, keyword, url FROM keywords WHERE url LIKE '%{searchTerms}%'") else { return vec![] };
    st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))
        .map(|r| {
            r.flatten()
                .filter(|(_, k, u)| !k.is_empty() && !k.contains('.') && u.starts_with("https://"))
                .map(|(n, k, u)| crate::settings::SearchShortcut { keyword: k, name: n, url: u.replace("{searchTerms}", "%s") })
                .collect()
        })
        .unwrap_or_default()
}

fn import_settings(app: &mut App, profile: &str, prefs: &Value) -> Vec<String> {
    let mut changed = vec![];
    let restore = prefs["session"]["restore_on_startup"].as_i64();
    let urls: Vec<String> = prefs["session"]["startup_urls"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
    match restore {
        Some(1) => {
            let _ = bridge::set_setting(app, profile, "startup", json!("restore"));
            changed.push("start-up: continue where you left off".to_string());
        }
        Some(4) if !urls.is_empty() => {
            let _ = bridge::set_setting(app, profile, "startup", json!("urls"));
            let _ = bridge::set_setting(app, profile, "startup_urls", json!(urls));
            changed.push(format!("start-up pages ({})", urls.len()));
        }
        Some(5) => {
            let _ = bridge::set_setting(app, profile, "startup", json!("newtab"));
            changed.push("start-up: new tab page".to_string());
        }
        _ => {}
    }
    if let Some(home) = prefs["homepage"].as_str().filter(|h| h.starts_with("http")) {
        if prefs["homepage_is_newtabpage"].as_bool() != Some(true) {
            let _ = bridge::set_setting(app, profile, "homepage", json!(home));
            changed.push("home page".into());
        }
    }
    if let Some(show) = prefs["browser"]["show_home_button"].as_bool() {
        let _ = bridge::set_setting(app, profile, "show_home_button", json!(show));
    }
    changed
}

pub fn cmd(app: &mut App, ctx: &Ctx, cmd: &str, a: &Value, _id: &Value) -> Reply {
    let profile = app.win(ctx.wid).map(|w| w.profile.clone()).unwrap_or_default();
    match cmd {
        "import.chromeProfiles" => Reply::Now(json!({
            "installed": chrome_exe().is_some(),
            "profiles": list_profiles(),
            "running": chrome_running(),
        })),
        "import.chrome" => {
            let Some(root) = chrome_user_data() else { return Reply::Err("Google Chrome data was not found on this PC".into()) };
            let dir = a["dir"].as_str().unwrap_or("Default");
            if dir.contains("..") || dir.contains('/') || dir.contains('\\') {
                return Reply::Err("invalid profile".into());
            }
            let src = root.join(dir);
            let what = &a["what"];
            let want = |k: &str| what[k].as_bool().unwrap_or(true);
            // restore point before changing anything
            let _ = crate::backup::backup_profile(app, &profile, "before-import");
            let mut report = serde_json::Map::new();
            let mut errors: Vec<String> = vec![];
            if want("bookmarks") {
                match read_shared(&src.join("Bookmarks")) {
                    Ok(b) => match app.profile(&profile).db().map(|db| import_bookmarks(db, &b)) {
                        Some(Ok(n)) => {
                            report.insert("bookmarks".into(), json!(n));
                        }
                        Some(Err(e)) => errors.push(e),
                        None => errors.push("database unavailable".into()),
                    },
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        report.insert("bookmarks".into(), json!(0));
                    }
                    Err(e) => errors.push(format!("bookmarks: {e}")),
                }
            }
            if want("history") {
                match private_copy(&src.join("History")) {
                    Ok(copy) => {
                        let r = app.profile(&profile).db().map(|db| import_history(db, &copy));
                        cleanup(&copy);
                        match r {
                            Some(Ok(n)) => {
                                report.insert("history".into(), json!(n));
                            }
                            Some(Err(e)) => errors.push(e),
                            None => errors.push("database unavailable".into()),
                        }
                    }
                    Err(e) => errors.push(e),
                }
            }
            if want("searchEngines") {
                if let Ok(copy) = private_copy(&src.join("Web Data")) {
                    let found = read_search_shortcuts(&copy);
                    cleanup(&copy);
                    let mut cur = app.settings(&profile).search_shortcuts.clone();
                    let mut added = 0;
                    for s in found {
                        if !cur.iter().any(|c| c.keyword.eq_ignore_ascii_case(&s.keyword)) {
                            cur.push(s);
                            added += 1;
                        }
                    }
                    let _ = bridge::set_setting(app, &profile, "search_shortcuts", serde_json::to_value(&cur).unwrap());
                    report.insert("searchShortcuts".into(), json!(added));
                }
            }
            if want("settings") {
                if let Ok(b) = read_shared(&src.join("Preferences")) {
                    if let Ok(prefs) = serde_json::from_slice::<Value>(&b) {
                        report.insert("settings".into(), json!(import_settings(app, &profile, &prefs)));
                    }
                }
            }
            crate::hooks::bookmarks_changed(app, &profile);
            logf!("Chrome import: {:?} errors={:?}", report, errors);
            Reply::Now(json!({"imported": report, "errors": errors}))
        }
        _ => Reply::Err("unknown import command".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chrome_time_conversion() {
        // 2024-01-01T00:00:00Z in Chrome time
        assert_eq!(chrome_time_to_ms(13_348_540_800_000_000), 1_704_067_200_000);
    }
    #[test]
    fn bookmark_json_import() {
        let db = crate::store::Store::open(&std::env::temp_dir().join(format!("solus-imp-{}", util::new_id()))).unwrap();
        let j = br#"{"roots":{"bookmark_bar":{"children":[{"type":"url","name":"Rust","url":"https://rust-lang.org/"},
            {"type":"folder","name":"Uni","children":[{"type":"url","name":"Canvas","url":"https://canvas.example/"}]}]},
            "other":{"children":[{"type":"url","name":"Other","url":"https://o.example/"}]},"synced":{"children":[]}}}"#;
        assert_eq!(import_bookmarks(&db, j).unwrap(), 3);
        assert_eq!(db.bm_children("bar").len(), 2);
        // importing again doesn't duplicate (goes into a folder; existing URLs in it are skipped)
        let again = import_bookmarks(&db, j).unwrap();
        assert!(again <= 2);
    }
    #[test]
    fn history_import_from_chrome_schema() {
        let dir = std::env::temp_dir().join(format!("solus-hist-{}", util::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let h = dir.join("History");
        let c = Connection::open(&h).unwrap();
        c.execute_batch("CREATE TABLE urls(id INTEGER PRIMARY KEY, url TEXT, title TEXT, visit_count INTEGER, typed_count INTEGER, last_visit_time INTEGER, hidden INTEGER DEFAULT 0);
            INSERT INTO urls(url,title,visit_count,typed_count,last_visit_time) VALUES('https://example.com/','Example',5,1,13348540800000000),('chrome://settings','x',1,0,1);").unwrap();
        drop(c);
        let db = crate::store::Store::open(&dir.join("profile")).unwrap();
        assert_eq!(import_history(&db, &h).unwrap(), 1);
        assert_eq!(db.history_matches("example", 3)[0].0, "https://example.com/");
    }
}
