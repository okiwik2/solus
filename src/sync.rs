//! Encrypted sync and cloud backups through a folder that the user's cloud drive (OneDrive,
//! Google Drive, Dropbox...) keeps in sync between PCs.
//!
//! <folder>/Solus Sync/
//!   vault.json               salt + KDF parameters + an encrypted check value (no secrets)
//!   devices/<device>.sync    this device's data, AES-256-GCM sealed with the passphrase key
//!   backups/<profile>-<ts>.solusbackup   encrypted full backups (restorable on a new PC)
//!
//! Each device writes only its own file, so cloud drives never see write conflicts. Records merge
//! by "last change wins" using per-record timestamps; deletions travel as soft-deleted records.

use std::path::{Path, PathBuf};

use base64::Engine;
use rusqlite::params;
use serde_json::{json, Value};

use crate::app::App;
use crate::bridge::{self, ok, Ctx, Reply};
use crate::store::Store;
use crate::util::{self, now_ms};

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;
const CHECK: &[u8] = b"solus-sync-check-v1";

fn root(folder: &str) -> PathBuf {
    Path::new(folder).join("Solus Sync")
}

fn sync_key(app: &mut App, profile: &str) -> Option<[u8; 32]> {
    let k = crate::crypto::load_secret(&app.profile(profile).dir, "sync_key")?;
    k.try_into().ok()
}

/// Create the vault (first device) or unlock an existing one (other devices).
pub fn setup(folder: &str, passphrase: &str) -> Result<([u8; 32], bool), String> {
    if passphrase.chars().count() < 10 {
        return Err("Use a passphrase of at least 10 characters".into());
    }
    let r = root(folder);
    std::fs::create_dir_all(r.join("devices")).map_err(|e| format!("Can't use that folder: {e}"))?;
    std::fs::create_dir_all(r.join("backups")).map_err(|e| e.to_string())?;
    let vault = r.join("vault.json");
    if let Ok(b) = std::fs::read(&vault) {
        let v: Value = serde_json::from_slice(&b).map_err(|_| "The sync folder's vault.json is damaged".to_string())?;
        let salt = B64.decode(v["salt"].as_str().unwrap_or("")).map_err(|_| "bad vault")?;
        let key = crate::crypto::passphrase_key(passphrase, &salt);
        let check = B64.decode(v["check"].as_str().unwrap_or("")).map_err(|_| "bad vault")?;
        match crate::crypto::open(&key, &check, b"vault") {
            Some(c) if c == CHECK => Ok((key, false)),
            _ => Err("That passphrase doesn't match the one used on your other device".into()),
        }
    } else {
        let salt = crate::crypto::random_bytes::<16>();
        let key = crate::crypto::passphrase_key(passphrase, &salt);
        let v = json!({
            "version": 1, "kdf": "argon2id", "m": 65536, "t": 3, "p": 1,
            "salt": B64.encode(salt), "check": B64.encode(crate::crypto::seal(&key, CHECK, b"vault")), "created": now_ms(),
        });
        util::write_atomic(&vault, serde_json::to_vec_pretty(&v).unwrap().as_slice()).map_err(|e| e.to_string())?;
        Ok((key, true))
    }
}

fn seal_json(key: &[u8; 32], v: &Value) -> Vec<u8> {
    let mut out = b"SOLUS1".to_vec();
    out.extend(crate::crypto::seal(key, &serde_json::to_vec(v).unwrap(), b"solus-sync"));
    out
}

fn open_json(key: &[u8; 32], b: &[u8]) -> Option<Value> {
    let body = b.strip_prefix(b"SOLUS1")?;
    let plain = crate::crypto::open(key, body, b"solus-sync")?;
    serde_json::from_slice(&plain).ok()
}

// ---------------------------------------------------------------- export / merge

const SYNC_SETTINGS: &[&str] = &[
    "theme", "density", "theme_id", "custom_css", "animations", "show_home_button", "search_engine", "search_suggestions",
    "search_shortcuts", "auto_group_tabs", "duplicate_detection", "blocking_level", "tracking_prevention", "block_third_party_cookies",
    "https_first", "do_not_track", "translate_offer", "translate_target", "translate_always", "translate_never", "shortcuts", "newtab",
    "site_exceptions",
];

pub fn export(db: &Store, s: &crate::settings::Settings, what: &crate::settings::SyncSettings, full: bool) -> Value {
    let q = |sql: &str, f: &dyn Fn(&rusqlite::Row) -> rusqlite::Result<Value>| -> Vec<Value> {
        db.conn.prepare(sql).and_then(|mut st| st.query_map([], |r| f(r)).map(|r| r.flatten().collect())).unwrap_or_default()
    };
    let mut data = json!({"version": 1, "exported": now_ms()});
    if what.bookmarks || full {
        data["bookmarks"] = json!(q("SELECT id,parent,kind,title,url,pos,created,updated,deleted FROM bookmarks WHERE id NOT IN ('bar','other')", &|r| Ok(json!({
            "id": r.get::<_, String>(0)?, "parent": r.get::<_, String>(1)?, "kind": r.get::<_, String>(2)?, "title": r.get::<_, String>(3)?,
            "url": r.get::<_, String>(4)?, "pos": r.get::<_, f64>(5)?, "created": r.get::<_, i64>(6)?, "updated": r.get::<_, i64>(7)?, "deleted": r.get::<_, i64>(8)?}))));
    }
    if what.notes || full {
        data["notes"] = json!(q("SELECT id,scope,key,title,body,color,pinned,created,updated,deleted FROM notes", &|r| Ok(json!({
            "id": r.get::<_, String>(0)?, "scope": r.get::<_, String>(1)?, "key": r.get::<_, String>(2)?, "title": r.get::<_, String>(3)?,
            "body": r.get::<_, Option<Vec<u8>>>(4)?.map(|b| db.keys.open_str(&b)).unwrap_or_default(), "color": r.get::<_, String>(5)?,
            "pinned": r.get::<_, i64>(6)?, "created": r.get::<_, i64>(7)?, "updated": r.get::<_, i64>(8)?, "deleted": r.get::<_, i64>(9)?}))));
    }
    if what.tasks || full {
        data["tasks"] = json!(q("SELECT id,list,title,notes,due,done,url,pos,created,updated,deleted FROM tasks", &|r| Ok(json!({
            "id": r.get::<_, String>(0)?, "list": r.get::<_, String>(1)?, "title": r.get::<_, String>(2)?, "notes": r.get::<_, String>(3)?,
            "due": r.get::<_, i64>(4)?, "done": r.get::<_, i64>(5)?, "url": r.get::<_, String>(6)?, "pos": r.get::<_, f64>(7)?,
            "created": r.get::<_, i64>(8)?, "updated": r.get::<_, i64>(9)?, "deleted": r.get::<_, i64>(10)?}))));
        data["reminders"] = json!(q("SELECT id,title,body,at,url,fired,repeat,created,updated,deleted FROM reminders", &|r| Ok(json!({
            "id": r.get::<_, String>(0)?, "title": r.get::<_, String>(1)?, "body": r.get::<_, String>(2)?, "at": r.get::<_, i64>(3)?,
            "url": r.get::<_, String>(4)?, "fired": r.get::<_, i64>(5)?, "repeat": r.get::<_, String>(6)?, "created": r.get::<_, i64>(7)?,
            "updated": r.get::<_, i64>(8)?, "deleted": r.get::<_, i64>(9)?}))));
    }
    if what.workspaces || full {
        data["workspaces"] = json!(q("SELECT id,name,icon,color,pos,created,updated,deleted FROM workspaces", &|r| Ok(json!({
            "id": r.get::<_, String>(0)?, "name": r.get::<_, String>(1)?, "icon": r.get::<_, String>(2)?, "color": r.get::<_, String>(3)?,
            "pos": r.get::<_, f64>(4)?, "created": r.get::<_, i64>(5)?, "updated": r.get::<_, i64>(6)?, "deleted": r.get::<_, i64>(7)?}))));
    }
    if what.passwords || full {
        data["passwords"] = json!(q("SELECT id,origin,username,password,note,created,updated,last_used,deleted FROM passwords", &|r| Ok(json!({
            "id": r.get::<_, String>(0)?, "origin": r.get::<_, String>(1)?,
            "username": r.get::<_, Option<Vec<u8>>>(2)?.map(|b| db.keys.open_str(&b)).unwrap_or_default(),
            "password": db.keys.open_str(&r.get::<_, Vec<u8>>(3)?),
            "note": r.get::<_, Option<Vec<u8>>>(4)?.map(|b| db.keys.open_str(&b)).unwrap_or_default(),
            "created": r.get::<_, i64>(5)?, "updated": r.get::<_, i64>(6)?, "lastUsed": r.get::<_, i64>(7)?, "deleted": r.get::<_, i64>(8)?}))));
    }
    if what.history || full {
        let limit = if full { 100_000 } else { 5_000 };
        data["history"] = json!(q(&format!("SELECT url,title,visits,typed,last_visit,deleted,updated FROM history_urls ORDER BY last_visit DESC LIMIT {limit}"), &|r| Ok(json!({
            "url": db.keys.open_str(&r.get::<_, Vec<u8>>(0)?), "title": r.get::<_, Option<Vec<u8>>>(1)?.map(|b| db.keys.open_str(&b)).unwrap_or_default(),
            "visits": r.get::<_, i64>(2)?, "typed": r.get::<_, i64>(3)?, "last": r.get::<_, i64>(4)?, "deleted": r.get::<_, i64>(5)?, "updated": r.get::<_, i64>(6)?}))));
    }
    if what.settings || full {
        let sv = serde_json::to_value(s).unwrap_or_default();
        let mut sub = serde_json::Map::new();
        for k in SYNC_SETTINGS {
            sub.insert(k.to_string(), sv[*k].clone());
        }
        data["settings"] = json!({"updated": db.kv_get("settings_updated").and_then(|x| x.parse::<i64>().ok()).unwrap_or(0), "values": sub});
    }
    data
}

fn newer(db: &Store, table: &str, id: &str, updated: i64) -> bool {
    match db.conn.query_row(&format!("SELECT updated FROM {table} WHERE id=?1"), [id], |r| r.get::<_, i64>(0)) {
        Ok(local) => updated > local,
        Err(_) => true,
    }
}

/// Merge a snapshot into the local database. Returns the number of records changed.
pub fn merge(db: &Store, data: &Value) -> usize {
    let mut n = 0;
    let arr = |k: &str| data[k].as_array().cloned().unwrap_or_default();
    for b in arr("bookmarks") {
        let id = b["id"].as_str().unwrap_or("");
        let up = b["updated"].as_i64().unwrap_or(0);
        if id.is_empty() || !newer(db, "bookmarks", id, up) {
            continue;
        }
        if db
            .conn
            .execute(
                "INSERT OR REPLACE INTO bookmarks(id,parent,kind,title,url,pos,created,updated,deleted) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![id, b["parent"].as_str().unwrap_or("other"), b["kind"].as_str().unwrap_or("url"), b["title"].as_str().unwrap_or(""),
                    b["url"].as_str().unwrap_or(""), b["pos"].as_f64().unwrap_or(0.0), b["created"].as_i64().unwrap_or(up), up, b["deleted"].as_i64().unwrap_or(0)],
            )
            .is_ok()
        {
            n += 1;
        }
    }
    for x in arr("notes") {
        let id = x["id"].as_str().unwrap_or("");
        let up = x["updated"].as_i64().unwrap_or(0);
        if id.is_empty() || !newer(db, "notes", id, up) {
            continue;
        }
        if db.conn.execute(
            "INSERT OR REPLACE INTO notes(id,scope,key,title,body,color,pinned,created,updated,deleted) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![id, x["scope"].as_str().unwrap_or("page"), x["key"].as_str().unwrap_or(""), x["title"].as_str().unwrap_or(""),
                db.keys.seal_str(x["body"].as_str().unwrap_or("")), x["color"].as_str().unwrap_or(""), x["pinned"].as_i64().unwrap_or(0),
                x["created"].as_i64().unwrap_or(up), up, x["deleted"].as_i64().unwrap_or(0)],
        ).is_ok() {
            n += 1;
        }
    }
    for x in arr("tasks") {
        let id = x["id"].as_str().unwrap_or("");
        let up = x["updated"].as_i64().unwrap_or(0);
        if id.is_empty() || !newer(db, "tasks", id, up) {
            continue;
        }
        if db.conn.execute(
            "INSERT OR REPLACE INTO tasks(id,list,title,notes,due,done,url,pos,created,updated,deleted) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            params![id, x["list"].as_str().unwrap_or("Tasks"), x["title"].as_str().unwrap_or(""), x["notes"].as_str().unwrap_or(""),
                x["due"].as_i64().unwrap_or(0), x["done"].as_i64().unwrap_or(0), x["url"].as_str().unwrap_or(""), x["pos"].as_f64().unwrap_or(0.0),
                x["created"].as_i64().unwrap_or(up), up, x["deleted"].as_i64().unwrap_or(0)],
        ).is_ok() {
            n += 1;
        }
    }
    for x in arr("reminders") {
        let id = x["id"].as_str().unwrap_or("");
        let up = x["updated"].as_i64().unwrap_or(0);
        if id.is_empty() || !newer(db, "reminders", id, up) {
            continue;
        }
        if db.conn.execute(
            "INSERT OR REPLACE INTO reminders(id,title,body,at,url,fired,repeat,created,updated,deleted) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![id, x["title"].as_str().unwrap_or(""), x["body"].as_str().unwrap_or(""), x["at"].as_i64().unwrap_or(0), x["url"].as_str().unwrap_or(""),
                x["fired"].as_i64().unwrap_or(0), x["repeat"].as_str().unwrap_or(""), x["created"].as_i64().unwrap_or(up), up, x["deleted"].as_i64().unwrap_or(0)],
        ).is_ok() {
            n += 1;
        }
    }
    for x in arr("workspaces") {
        let id = x["id"].as_str().unwrap_or("");
        let up = x["updated"].as_i64().unwrap_or(0);
        if !newer(db, "workspaces", id, up) {
            continue;
        }
        if db.conn.execute(
            "INSERT OR REPLACE INTO workspaces(id,name,icon,color,pos,created,updated,deleted) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![id, x["name"].as_str().unwrap_or("Workspace"), x["icon"].as_str().unwrap_or(""), x["color"].as_str().unwrap_or(""),
                x["pos"].as_f64().unwrap_or(0.0), x["created"].as_i64().unwrap_or(up), up, x["deleted"].as_i64().unwrap_or(0)],
        ).is_ok() {
            n += 1;
        }
    }
    if let Some(pws) = data["passwords"].as_array() {
        let local = crate::passwords::all(db);
        for x in pws {
            let (origin, user) = (x["origin"].as_str().unwrap_or(""), x["username"].as_str().unwrap_or(""));
            let up = x["updated"].as_i64().unwrap_or(0);
            // the same login may exist under different ids on two devices: match on origin + username
            let existing = local.iter().find(|e| e.origin == origin && e.username == user);
            let id = existing.map(|e| e.id.clone()).unwrap_or_else(|| x["id"].as_str().unwrap_or("").to_string());
            if id.is_empty() || existing.map(|e| e.updated >= up).unwrap_or(false) {
                continue;
            }
            if db.conn.execute(
                "INSERT OR REPLACE INTO passwords(id,origin,username,password,note,created,updated,last_used,deleted) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![id, origin, db.keys.seal_str(user), db.keys.seal_str(x["password"].as_str().unwrap_or("")), db.keys.seal_str(x["note"].as_str().unwrap_or("")),
                    x["created"].as_i64().unwrap_or(up), up, x["lastUsed"].as_i64().unwrap_or(0), x["deleted"].as_i64().unwrap_or(0)],
            ).is_ok() {
                n += 1;
            }
        }
    }
    if let Some(hist) = data["history"].as_array() {
        for h in hist {
            let url = h["url"].as_str().unwrap_or("");
            if url.is_empty() {
                continue;
            }
            let tag = db.keys.tag(url.as_bytes());
            let last = h["last"].as_i64().unwrap_or(0);
            let deleted = h["deleted"].as_i64().unwrap_or(0);
            let local: Option<(i64, i64, i64)> = db
                .conn
                .query_row("SELECT id,last_visit,updated FROM history_urls WHERE tag=?1", [&tag], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                .ok();
            match local {
                Some((id, llast, lup)) => {
                    if deleted == 1 && h["updated"].as_i64().unwrap_or(0) > lup.max(llast) {
                        let _ = db.conn.execute("UPDATE history_urls SET deleted=1, updated=?2 WHERE id=?1", params![id, now_ms()]);
                        let _ = db.conn.execute("DELETE FROM history_visits WHERE url_id=?1", [id]);
                        n += 1;
                    } else if deleted == 0 && last > llast {
                        let _ = db.conn.execute(
                            "UPDATE history_urls SET last_visit=?2, visits=MAX(visits,?3), deleted=0 WHERE id=?1",
                            params![id, last, h["visits"].as_i64().unwrap_or(1)],
                        );
                        let _ = db.conn.execute("INSERT INTO history_visits(url_id,ts,transition) VALUES(?1,?2,'synced')", params![id, last]);
                        n += 1;
                    }
                }
                None if deleted == 0 => {
                    let site_tag = db.keys.tag(util::site_of(url).as_bytes());
                    if db.conn.execute(
                        "INSERT INTO history_urls(uid,tag,url,title,site_tag,visits,typed,last_visit,updated) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                        params![util::new_id(), tag, db.keys.seal_str(url), db.keys.seal_str(h["title"].as_str().unwrap_or("")), site_tag,
                            h["visits"].as_i64().unwrap_or(1), h["typed"].as_i64().unwrap_or(0), last, now_ms()],
                    ).is_ok() {
                        let id = db.conn.last_insert_rowid();
                        let _ = db.conn.execute("INSERT INTO history_visits(url_id,ts,transition) VALUES(?1,?2,'synced')", params![id, last]);
                        n += 1;
                    }
                }
                None => {}
            }
        }
    }
    n
}

fn apply_settings(app: &mut App, profile: &str, data: &Value) -> bool {
    let remote_up = data["settings"]["updated"].as_i64().unwrap_or(0);
    let local_up = app.profile(profile).db().and_then(|d| d.kv_get("settings_updated")).and_then(|x| x.parse::<i64>().ok()).unwrap_or(0);
    if remote_up <= local_up {
        return false;
    }
    if let Some(vals) = data["settings"]["values"].as_object() {
        let p = app.profile(profile);
        let mut cur = serde_json::to_value(&p.settings).unwrap_or_default();
        for (k, v) in vals {
            if SYNC_SETTINGS.contains(&k.as_str()) && !v.is_null() {
                cur[k] = v.clone();
            }
        }
        if let Ok(s) = serde_json::from_value(cur) {
            p.settings = s;
            p.settings_dirty = true;
        }
        if let Some(db) = p.db() {
            db.kv_set("settings_updated", &remote_up.to_string());
        }
        return true;
    }
    false
}

/// Run one sync pass for a profile. Returns a human summary.
pub fn sync_now(app: &mut App, profile: &str) -> Result<Value, String> {
    let s = app.settings(profile).clone();
    if !s.sync.enabled || s.sync.folder.is_empty() {
        return Err("Sync is not set up".into());
    }
    let key = sync_key(app, profile).ok_or("Sync key missing — set up sync again")?;
    let r = root(&s.sync.folder);
    if !r.join("vault.json").exists() {
        return Err("The sync folder is not available (is your cloud drive running?)".into());
    }
    let device = app.local.device_id.clone();
    let mut merged = 0;
    let mut devices = vec![];
    let mut settings_applied = false;
    if let Ok(rd) = std::fs::read_dir(r.join("devices")) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if !name.ends_with(".sync") || name.starts_with(&device) {
                continue;
            }
            let Ok(b) = std::fs::read(e.path()) else { continue };
            let Some(data) = open_json(&key, &b) else {
                logf!("sync: could not decrypt {name}");
                continue;
            };
            if data["profile"].as_str().unwrap_or("default") != profile && data["profileName"].is_string() && profile != "default" {
                continue;
            }
            devices.push(json!({"name": data["deviceName"], "at": data["exported"]}));
            if let Some(db) = app.profile(profile).db() {
                merged += merge(db, &data);
            }
            if s.sync.settings {
                settings_applied |= apply_settings(app, profile, &data);
            }
        }
    }
    // write our snapshot after merging so it carries everything we know
    let s2 = app.settings(profile).clone();
    let Some(db) = app.profile(profile).db() else { return Err("database unavailable".into()) };
    let mut data = export(db, &s2, &s2.sync, false);
    data["device"] = json!(device);
    data["deviceName"] = json!(app.local.device_name);
    data["profile"] = json!(profile);
    let path = r.join("devices").join(format!("{device}-{profile}.sync"));
    util::write_atomic(&path, &seal_json(&key, &data)).map_err(|e| format!("Couldn't write to the sync folder: {e}"))?;
    let now = now_ms();
    let p = app.profile(profile);
    p.settings.sync.last_sync = now;
    p.settings.sync.last_error.clear();
    p.settings_dirty = true;
    if merged > 0 {
        crate::hooks::bookmarks_changed(app, profile);
    }
    if settings_applied {
        bridge::push_settings(app, profile);
        let ids: Vec<u32> = app.windows.iter().filter(|w| w.profile == profile).map(|w| w.id).collect();
        for id in ids {
            bridge::push_theme(app, id);
        }
    }
    Ok(json!({"merged": merged, "devices": devices, "at": now}))
}

/// Periodic sync (every 10 minutes) and weekly cloud backups.
pub fn tick(app: &mut App) {
    let ids: Vec<String> = app.local.profiles.iter().map(|p| p.id.clone()).collect();
    for id in ids {
        let s = app.settings(&id).clone();
        if !s.sync.enabled {
            continue;
        }
        if now_ms() - s.sync.last_sync > 10 * 60_000 {
            if let Err(e) = sync_now(app, &id) {
                logf!("sync ({id}): {e}");
                let p = app.profile(&id);
                p.settings.sync.last_error = e;
                p.settings.sync.last_sync = now_ms();
                p.settings_dirty = true;
            }
        }
        if s.sync.cloud_backups {
            let last = app.profile(&id).db().and_then(|d| d.kv_get("cloud_backup")).and_then(|x| x.parse::<i64>().ok()).unwrap_or(0);
            if now_ms() - last > 7 * 86_400_000 {
                if let Err(e) = cloud_backup(app, &id) {
                    logf!("cloud backup ({id}): {e}");
                }
            }
        }
    }
}

pub fn cloud_backup(app: &mut App, profile: &str) -> Result<String, String> {
    let s = app.settings(profile).clone();
    let key = sync_key(app, profile).ok_or("Set up sync first — cloud backups use the same encryption")?;
    let Some(db) = app.profile(profile).db() else { return Err("database unavailable".into()) };
    let mut data = export(db, &s, &s.sync, true);
    let mut sv = serde_json::to_value(&s).unwrap_or_default();
    sv["sync"] = json!(null);
    data["allSettings"] = sv;
    data["deviceName"] = json!(app.local.device_name);
    data["profile"] = json!(profile);
    let dir = root(&s.sync.folder).join("backups");
    let _ = std::fs::create_dir_all(&dir);
    let name = format!("{}-{}-{}.solusbackup", profile, app.local.device_name.replace(|c: char| !c.is_ascii_alphanumeric(), "_"), now_ms());
    util::write_atomic(&dir.join(&name), &seal_json(&key, &data)).map_err(|e| e.to_string())?;
    if let Some(db) = app.profile(profile).db() {
        db.kv_set("cloud_backup", &now_ms().to_string());
    }
    // keep the newest 6
    if let Ok(rd) = std::fs::read_dir(&dir) {
        let mut files: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().map(|e| e == "solusbackup").unwrap_or(false)).collect();
        files.sort();
        while files.len() > 6 {
            let _ = std::fs::remove_file(files.remove(0));
        }
    }
    Ok(name)
}

pub fn cmd(app: &mut App, ctx: &Ctx, cmd: &str, a: &Value, id: &Value) -> Reply {
    let profile = app.win(ctx.wid).map(|w| w.profile.clone()).unwrap_or_default();
    match cmd {
        "sync.status" => {
            let s = app.settings(&profile).clone();
            let mut devices = vec![];
            if s.sync.enabled {
                if let Ok(rd) = std::fs::read_dir(root(&s.sync.folder).join("devices")) {
                    for e in rd.flatten() {
                        let m = e.metadata().ok();
                        devices.push(json!({"file": e.file_name().to_string_lossy(), "modified": m.and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as i64)}));
                    }
                }
            }
            Reply::Now(json!({"sync": s.sync, "keySaved": sync_key(app, &profile).is_some(), "devices": devices, "deviceName": app.local.device_name, "suggested": suggest_folders()}))
        }
        "sync.setup" => {
            let folder = a["folder"].as_str().unwrap_or("").to_string();
            let pass = a["passphrase"].as_str().unwrap_or("").to_string();
            if folder.is_empty() || !Path::new(&folder).is_dir() {
                return Reply::Err("Choose an existing folder (for example your OneDrive folder)".into());
            }
            let (c, i) = (ctx.clone(), id.clone());
            // Argon2 takes a moment; keep the UI responsive
            std::thread::spawn(move || {
                let r = setup(&folder, &pass);
                crate::app::post(move |app| {
                    let res = match r {
                        Ok((key, created)) => {
                            let dir = app.profile(&profile).dir.clone();
                            match crate::crypto::store_secret(&dir, "sync_key", &key) {
                                Ok(()) => {
                                    let p = app.profile(&profile);
                                    p.settings.sync.enabled = true;
                                    p.settings.sync.folder = folder.clone();
                                    p.settings_dirty = true;
                                    let first = sync_now(app, &profile);
                                    Ok(json!({"created": created, "firstSync": first.ok()}))
                                }
                                Err(e) => Err(e),
                            }
                        }
                        Err(e) => Err(e),
                    };
                    bridge::respond(app, &c, &i, res);
                });
            });
            Reply::Later
        }
        "sync.now" => match sync_now(app, &profile) {
            Ok(v) => Reply::Now(v),
            Err(e) => Reply::Err(e),
        },
        "sync.disable" => {
            let dir = app.profile(&profile).dir.clone();
            let _ = crate::crypto::store_secret(&dir, "sync_key", b"");
            let p = app.profile(&profile);
            p.settings.sync.enabled = false;
            p.settings_dirty = true;
            ok()
        }
        "sync.backupNow" => match cloud_backup(app, &profile) {
            Ok(n) => Reply::Now(json!(n)),
            Err(e) => Reply::Err(e),
        },
        "sync.listBackups" => {
            let s = app.settings(&profile).clone();
            let folder = a["folder"].as_str().map(String::from).unwrap_or(s.sync.folder);
            let mut out = vec![];
            if let Ok(rd) = std::fs::read_dir(root(&folder).join("backups")) {
                for e in rd.flatten() {
                    let m = e.metadata().ok();
                    out.push(json!({"name": e.file_name().to_string_lossy(), "size": m.as_ref().map(|m| m.len()), "modified": m.and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as i64)}));
                }
            }
            out.sort_by(|a, b| b["name"].as_str().cmp(&a["name"].as_str()));
            Reply::Now(json!(out))
        }
        "sync.restoreBackup" => {
            // restore an encrypted cloud backup into this profile (merge, newest wins)
            let s = app.settings(&profile).clone();
            let folder = a["folder"].as_str().map(String::from).unwrap_or(s.sync.folder.clone());
            let name = a["name"].as_str().unwrap_or("");
            if name.contains("..") || name.contains('/') || name.contains('\\') {
                return Reply::Err("invalid backup name".into());
            }
            let key = match a["passphrase"].as_str() {
                Some(p) if !p.is_empty() => match setup(&folder, p) {
                    Ok((k, _)) => k,
                    Err(e) => return Reply::Err(e),
                },
                _ => match sync_key(app, &profile) {
                    Some(k) => k,
                    None => return Reply::Err("Enter your sync passphrase".into()),
                },
            };
            let Ok(b) = std::fs::read(root(&folder).join("backups").join(name)) else { return Reply::Err("backup not found".into()) };
            let Some(data) = open_json(&key, &b) else { return Reply::Err("Couldn't decrypt this backup with that passphrase".into()) };
            let _ = crate::backup::backup_profile(app, &profile, "before-cloud-restore");
            let n = app.profile(&profile).db().map(|db| merge(db, &data)).unwrap_or(0);
            if let Ok(all) = serde_json::from_value::<crate::settings::Settings>(data["allSettings"].clone()) {
                let p = app.profile(&profile);
                let sync = p.settings.sync.clone();
                p.settings = all;
                p.settings.sync = sync;
                p.settings_dirty = true;
            }
            crate::hooks::bookmarks_changed(app, &profile);
            bridge::push_settings(app, &profile);
            Reply::Now(json!({"restored": n}))
        }
        _ => Reply::Err("unknown sync command".into()),
    }
}

/// Cloud-drive folders found on this PC (OneDrive, Google Drive, Dropbox, iCloud).
pub fn suggest_folders() -> Vec<Value> {
    let mut out = vec![];
    for var in ["OneDrive", "OneDriveConsumer", "OneDriveCommercial"] {
        if let Ok(p) = std::env::var(var) {
            if Path::new(&p).is_dir() && !out.iter().any(|x: &Value| x["path"] == p) {
                out.push(json!({"name": "OneDrive", "path": p}));
            }
        }
    }
    if let Ok(home) = std::env::var("USERPROFILE") {
        for (n, sub) in [("Google Drive", "Google Drive"), ("Google Drive", "My Drive"), ("Dropbox", "Dropbox"), ("iCloud Drive", "iCloudDrive")] {
            let p = Path::new(&home).join(sub);
            if p.is_dir() {
                out.push(json!({"name": n, "path": p}));
            }
        }
    }
    for d in ['G', 'H', 'I'] {
        let p = PathBuf::from(format!("{d}:\\My Drive"));
        if p.is_dir() {
            out.push(json!({"name": "Google Drive", "path": p}));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    fn store(n: &str) -> Store {
        Store::open(&std::env::temp_dir().join(format!("solus-sync-{n}-{}", util::new_id()))).unwrap()
    }
    #[test]
    fn vault_setup_and_wrong_passphrase() {
        let dir = std::env::temp_dir().join(format!("solus-vault-{}", util::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.to_string_lossy().to_string();
        let (k1, created) = setup(&f, "correct horse battery").unwrap();
        assert!(created);
        let (k2, created2) = setup(&f, "correct horse battery").unwrap();
        assert!(!created2);
        assert_eq!(k1, k2);
        assert!(setup(&f, "wrong passphrase!!").is_err());
        assert!(setup(&f, "short").is_err());
        let sealed = seal_json(&k1, &json!({"a": 1}));
        assert!(!String::from_utf8_lossy(&sealed).contains("\"a\""));
        assert_eq!(open_json(&k1, &sealed).unwrap()["a"], 1);
    }
    #[test]
    fn two_devices_merge_last_change_wins() {
        let a = store("a");
        let b = store("b");
        let all = crate::settings::SyncSettings::default();
        let s = crate::settings::Settings::default();
        let id = a.bm_add("bar", "url", "Rust", "https://rust-lang.org/", None).unwrap();
        a.add_visit("https://example.com/", "Example", "typed");
        crate::passwords::upsert(&a, "https://ex.com", "me", "pw-1-long-enough", "").unwrap();
        // device B receives A's data
        assert!(merge(&b, &export(&a, &s, &all, false)) >= 3);
        assert_eq!(b.bm_get(&id).unwrap()["title"], "Rust");
        assert_eq!(b.history_matches("example", 1)[0].0, "https://example.com/");
        assert_eq!(crate::passwords::all(&b)[0].password, "pw-1-long-enough");
        // B renames later; A deletes earlier -> rename wins on both
        std::thread::sleep(std::time::Duration::from_millis(5));
        b.bm_update(&id, Some("Rust lang"), None).unwrap();
        merge(&a, &export(&b, &s, &all, false));
        assert_eq!(a.bm_get(&id).unwrap()["title"], "Rust lang");
        // a later delete on A propagates
        std::thread::sleep(std::time::Duration::from_millis(5));
        a.bm_delete(&id).unwrap();
        merge(&b, &export(&a, &s, &all, false));
        assert!(b.bm_get(&id).is_none());
    }
}
