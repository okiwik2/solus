//! Password manager: encrypted vault (DPAPI-protected profile key + AES-GCM), save prompts,
//! fill-on-request, CSV import (Chrome / Edge / Firefox / Bitwarden exports) and export,
//! health report. Revealing or exporting passwords asks Windows to verify the user first.

use std::cell::RefCell;

use rusqlite::params;
use serde_json::{json, Value};

use crate::app::App;
use crate::bridge::{self, ok, Ctx, Reply};
use crate::store::Store;
use crate::util::{self, now_ms};

pub fn origin_key(url: &str) -> String {
    util::origin_of(url)
}

pub fn count_for(db: &Store, url: &str) -> i64 {
    let o = origin_key(url);
    if o.is_empty() || o == "null" {
        return 0;
    }
    db.conn
        .query_row("SELECT COUNT(*) FROM passwords WHERE deleted=0 AND origin=?1", [o], |r| r.get(0))
        .unwrap_or(0)
}

pub struct Entry {
    pub id: String,
    pub origin: String,
    pub username: String,
    pub password: String,
    pub note: String,
    pub updated: i64,
    pub last_used: i64,
}

pub fn all(db: &Store) -> Vec<Entry> {
    let Ok(mut st) = db.conn.prepare("SELECT id,origin,username,password,note,updated,last_used FROM passwords WHERE deleted=0 ORDER BY origin, updated DESC") else {
        return vec![];
    };
    st.query_map([], |r| {
        let u: Option<Vec<u8>> = r.get(2)?;
        let p: Vec<u8> = r.get(3)?;
        let n: Option<Vec<u8>> = r.get(4)?;
        Ok(Entry {
            id: r.get(0)?,
            origin: r.get(1)?,
            username: u.map(|b| db.keys.open_str(&b)).unwrap_or_default(),
            password: db.keys.open_str(&p),
            note: n.map(|b| db.keys.open_str(&b)).unwrap_or_default(),
            updated: r.get(5)?,
            last_used: r.get(6)?,
        })
    })
    .map(|r| r.flatten().collect())
    .unwrap_or_default()
}

/// Insert or update (same origin + username). Returns (id, created).
pub fn upsert(db: &Store, origin: &str, username: &str, password: &str, note: &str) -> Result<(String, bool), String> {
    if origin.is_empty() || password.is_empty() {
        return Err("origin and password are required".into());
    }
    let now = now_ms();
    let existing = all(db).into_iter().find(|e| e.origin == origin && e.username == username);
    match existing {
        Some(e) => {
            db.conn
                .execute(
                    "UPDATE passwords SET password=?2, note=?3, updated=?4 WHERE id=?1",
                    params![e.id, db.keys.seal_str(password), db.keys.seal_str(if note.is_empty() { &e.note } else { note }), now],
                )
                .map_err(|e| e.to_string())?;
            Ok((e.id, false))
        }
        None => {
            let id = util::new_id();
            db.conn
                .execute(
                    "INSERT INTO passwords(id,origin,username,password,note,created,updated) VALUES(?1,?2,?3,?4,?5,?6,?6)",
                    params![id, origin, db.keys.seal_str(username), db.keys.seal_str(password), db.keys.seal_str(note), now],
                )
                .map_err(|e| e.to_string())?;
            Ok((id, true))
        }
    }
}

/// Minimal RFC 4180 CSV parser (quoted fields, embedded commas/quotes/newlines).
pub fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let mut rows = vec![];
    let mut row = vec![];
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.trim_start_matches('\u{feff}').chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    quoted = false;
                }
            } else {
                field.push(c);
            }
        } else {
            match c {
                '"' => quoted = true,
                ',' => row.push(std::mem::take(&mut field)),
                '\r' => {}
                '\n' => {
                    row.push(std::mem::take(&mut field));
                    if row.iter().any(|f| !f.is_empty()) {
                        rows.push(std::mem::take(&mut row));
                    } else {
                        row.clear();
                    }
                }
                _ => field.push(c),
            }
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

pub struct ImportResult {
    pub added: usize,
    pub updated: usize,
    pub skipped: usize,
}

/// Import a password CSV exported from Chrome/Edge (name,url,username,password,note),
/// Firefox (url,username,password,...) or Bitwarden (login_uri,login_username,login_password).
pub fn import_csv(db: &Store, text: &str) -> Result<ImportResult, String> {
    let rows = parse_csv(text);
    let Some(header) = rows.first() else { return Err("The file is empty".into()) };
    let h: Vec<String> = header.iter().map(|x| x.trim().to_ascii_lowercase()).collect();
    let col = |names: &[&str]| names.iter().find_map(|n| h.iter().position(|x| x == n));
    let (Some(u), Some(p)) = (col(&["url", "login_uri", "origin", "website"]), col(&["password", "login_password"])) else {
        return Err("This doesn't look like a password export (no url/password columns)".into());
    };
    let user = col(&["username", "login_username", "user", "login"]);
    let note = col(&["note", "notes", "extra"]);
    let mut r = ImportResult { added: 0, updated: 0, skipped: 0 };
    for row in rows.iter().skip(1) {
        let url = row.get(u).map(|s| s.trim()).unwrap_or("");
        let pw = row.get(p).map(|s| s.as_str()).unwrap_or("");
        let origin = origin_key(url);
        if origin.is_empty() || origin == "null" || pw.is_empty() {
            r.skipped += 1;
            continue;
        }
        let un = user.and_then(|i| row.get(i)).map(|s| s.trim()).unwrap_or("");
        let nt = note.and_then(|i| row.get(i)).map(|s| s.as_str()).unwrap_or("");
        match upsert(db, &origin, un, pw, nt) {
            Ok((_, true)) => r.added += 1,
            Ok((_, false)) => r.updated += 1,
            Err(_) => r.skipped += 1,
        }
    }
    Ok(r)
}

fn csv_field(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

pub fn export_csv(db: &Store) -> String {
    let mut out = String::from("name,url,username,password,note\n");
    for e in all(db) {
        out.push_str(&[csv_field(&util::host_of(&e.origin)), csv_field(&e.origin), csv_field(&e.username), csv_field(&e.password), csv_field(&e.note)].join(","));
        out.push('\n');
    }
    out
}

pub fn strength(p: &str) -> u32 {
    let mut classes = 0;
    if p.chars().any(|c| c.is_ascii_lowercase()) {
        classes += 1;
    }
    if p.chars().any(|c| c.is_ascii_uppercase()) {
        classes += 1;
    }
    if p.chars().any(|c| c.is_ascii_digit()) {
        classes += 1;
    }
    if p.chars().any(|c| !c.is_ascii_alphanumeric()) {
        classes += 1;
    }
    const COMMON: &[&str] = &["password", "123456", "qwerty", "letmein", "iloveyou", "welcome", "admin", "monkey", "dragon", "abc123"];
    let lower = p.to_lowercase();
    if COMMON.iter().any(|c| lower.contains(c)) || p.len() < 8 {
        return 0;
    }
    match (p.len(), classes) {
        (l, c) if l >= 14 && c >= 2 => 3,
        (l, c) if l >= 10 && c >= 3 => 2,
        _ => 1,
    }
}

/// (count, weak, reused)
pub fn health(db: &Store) -> (i64, i64, i64) {
    let list = all(db);
    let mut counts: std::collections::HashMap<&str, i64> = Default::default();
    for e in &list {
        *counts.entry(e.password.as_str()).or_default() += 1;
    }
    let weak = list.iter().filter(|e| strength(&e.password) == 0).count() as i64;
    let reused = list.iter().filter(|e| counts.get(e.password.as_str()).copied().unwrap_or(0) > 1).count() as i64;
    (list.len() as i64, weak, reused)
}

pub fn generate(len: usize) -> String {
    use rand::Rng;
    const SETS: [&str; 4] = ["abcdefghijkmnopqrstuvwxyz", "ABCDEFGHJKLMNPQRSTUVWXYZ", "23456789", "-_!@#$%*+=?"];
    let mut rng = rand::thread_rng();
    let all: String = SETS.concat();
    let a: Vec<char> = all.chars().collect();
    loop {
        let p: String = (0..len.max(12)).map(|_| a[rng.gen_range(0..a.len())]).collect();
        if SETS.iter().all(|s| p.chars().any(|c| s.contains(c))) {
            return p;
        }
    }
}

// ---------------------------------------------------------------- capture & fill

struct Captured {
    wid: u32,
    tid: u32,
    origin: String,
    user: String,
    pw: String,
    at: i64,
}

thread_local! {
    static PENDING: RefCell<Option<Captured>> = const { RefCell::new(None) };
    static VERIFIED_UNTIL: RefCell<i64> = const { RefCell::new(0) };
}

pub fn captured(app: &mut App, wid: u32, tid: u32, origin: &str, user: &str, pw: &str) {
    let Some(w) = app.win(wid) else { return };
    let (profile, incognito) = (w.profile.clone(), w.incognito);
    let s = app.settings(&profile).clone();
    if !s.password_manager || incognito || pw.is_empty() || !(origin.starts_with("https://") || origin.starts_with("http://localhost")) {
        return;
    }
    let Some(db) = app.profile(&profile).db() else { return };
    let never: Vec<String> = db.kv_get("pw_never").and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    if never.iter().any(|n| n == origin) {
        return;
    }
    let existing = all(db).into_iter().find(|e| e.origin == origin && e.username == user);
    if let Some(e) = &existing {
        if e.password == pw {
            let _ = db.conn.execute("UPDATE passwords SET last_used=?2 WHERE id=?1", params![e.id, now_ms()]);
            return; // nothing new
        }
    }
    let update = existing.is_some();
    PENDING.with(|p| *p.borrow_mut() = Some(Captured { wid, tid, origin: origin.to_string(), user: user.to_string(), pw: pw.to_string(), at: now_ms() }));
    let host = util::host_of(origin);
    bridge::emit(
        app,
        wid,
        "passwordPrompt",
        json!({"tab": tid, "host": host, "user": user, "update": update}),
    );
}

const FILL_JS: &str = r#"((u, p) => {
  const vis = (e) => !!(e.offsetWidth || e.offsetHeight || e.getClientRects().length);
  const pw = [...document.querySelectorAll('input[type=password]')].filter(vis)[0];
  const setVal = (el, v) => { const d = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value'); d.set.call(el, v); el.dispatchEvent(new Event('input', { bubbles: true })); el.dispatchEvent(new Event('change', { bubbles: true })); };
  let filled = 0;
  const scope = (pw && pw.form) || document;
  const user = scope.querySelector('input[autocomplete~=username],input[autocomplete~=email]') ||
    [...scope.querySelectorAll('input')].filter((i) => ['text', 'email', 'tel'].includes(i.type) && vis(i)).pop();
  if (user && u) { setVal(user, u); filled++; }
  if (pw) { setVal(pw, p); filled++; }
  return filled;
})"#;

fn fill(app: &mut App, wid: u32, tid: u32, id: &str) -> Result<(), String> {
    let Some(w) = app.win(wid) else { return Err("no window".into()) };
    let profile = w.profile.clone();
    let Some(t) = w.tab(tid) else { return Err("no tab".into()) };
    let page_origin = origin_key(&t.url);
    let Some(v) = &t.view else { return Err("page not loaded".into()) };
    let wv = v.wv.clone();
    let Some(db) = app.profile(&profile).db() else { return Err("vault unavailable".into()) };
    let e = all(db).into_iter().find(|e| e.id == id).ok_or("not found")?;
    // only ever fill into the site the password was saved for
    if e.origin != page_origin {
        return Err("This password belongs to a different site".into());
    }
    let _ = db.conn.execute("UPDATE passwords SET last_used=?2 WHERE id=?1", params![e.id, now_ms()]);
    let script = format!("{}({}, {})", FILL_JS, serde_json::to_string(&e.username).unwrap(), serde_json::to_string(&e.password).unwrap());
    crate::webview::exec_script(&wv, &script, |_, _| {});
    Ok(())
}

/// Ask Windows to confirm it's the user (Windows Hello / PIN / password) before revealing secrets.
/// A successful check is remembered for 2 minutes.
pub fn verify_user(app: &App, wid: u32, reason: &str) -> bool {
    if VERIFIED_UNTIL.with(|v| *v.borrow()) > now_ms() {
        return true;
    }
    let hwnd = app.win(wid).map(|w| w.hwnd);
    let ok = crate::winauth::verify(hwnd, reason);
    if ok {
        VERIFIED_UNTIL.with(|v| *v.borrow_mut() = now_ms() + 120_000);
    }
    ok
}

pub fn cmd(app: &mut App, ctx: &Ctx, cmd: &str, a: &Value, _id: &Value) -> Reply {
    let Some(w) = app.win(ctx.wid) else { return Reply::Err("no window".into()) };
    let (profile, incognito) = (w.profile.clone(), w.incognito);
    let s = |k: &str| a[k].as_str().unwrap_or("").to_string();
    match cmd {
        "passwords.respond" => {
            let Some(c) = PENDING.with(|p| p.borrow_mut().take()) else { return ok() };
            if now_ms() - c.at > 10 * 60_000 {
                return ok();
            }
            let Some(db) = app.profile(&profile).db() else { return Reply::Err("vault unavailable".into()) };
            match a["choice"].as_str().unwrap_or("") {
                "save" => {
                    let user = a["user"].as_str().unwrap_or(&c.user).to_string();
                    if let Err(e) = upsert(db, &c.origin, &user, &c.pw, "") {
                        return Reply::Err(e);
                    }
                }
                "never" => {
                    let mut never: Vec<String> = db.kv_get("pw_never").and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
                    never.push(c.origin.clone());
                    db.kv_set("pw_never", &serde_json::to_string(&never).unwrap());
                }
                _ => {}
            }
            crate::hooks::page_info(app, c.wid, c.tid);
            ok()
        }
        "passwords.forPage" => {
            let tid = a["tab"].as_u64().unwrap_or(0) as u32;
            let url = app.win(ctx.wid).and_then(|w| w.tab(tid)).map(|t| t.url.clone()).unwrap_or_default();
            let o = origin_key(&url);
            let Some(db) = app.profile(&profile).db() else { return Reply::Now(json!([])) };
            let list: Vec<Value> = all(db).into_iter().filter(|e| e.origin == o).map(|e| json!({"id": e.id, "username": e.username})).collect();
            Reply::Now(json!(list))
        }
        "passwords.fill" => match fill(app, ctx.wid, a["tab"].as_u64().unwrap_or(0) as u32, &s("id")) {
            Ok(()) => ok(),
            Err(e) => Reply::Err(e),
        },
        "passwords.list" => {
            if incognito {
                return Reply::Now(json!([]));
            }
            let Some(db) = app.profile(&profile).db() else { return Reply::Now(json!([])) };
            let mut counts: std::collections::HashMap<String, i64> = Default::default();
            let list = all(db);
            for e in &list {
                *counts.entry(e.password.clone()).or_default() += 1;
            }
            let rows: Vec<Value> = list
                .iter()
                .map(|e| {
                    json!({"id": e.id, "origin": e.origin, "host": util::host_of(&e.origin), "username": e.username, "note": !e.note.is_empty(),
                        "updated": e.updated, "lastUsed": e.last_used, "weak": strength(&e.password) == 0, "reused": counts.get(&e.password).copied().unwrap_or(0) > 1})
                })
                .collect();
            Reply::Now(json!(rows))
        }
        "passwords.reveal" => {
            if !verify_user(app, ctx.wid, "Solus wants to show a saved password") {
                return Reply::Err("Windows could not verify it's you".into());
            }
            let Some(db) = app.profile(&profile).db() else { return Reply::Err("vault unavailable".into()) };
            let id = s("id");
            match all(db).into_iter().find(|e| e.id == id) {
                Some(e) => Reply::Now(json!({"password": e.password, "note": e.note})),
                None => Reply::Err("not found".into()),
            }
        }
        "passwords.save" => {
            let Some(db) = app.profile(&profile).db() else { return Reply::Err("vault unavailable".into()) };
            let url = s("url");
            let origin = origin_key(if url.contains("://") { &url } else { "" });
            let origin = if origin.is_empty() || origin == "null" { origin_key(&format!("https://{url}")) } else { origin };
            if !s("id").is_empty() {
                // editing an existing entry changes it in place
                let id = s("id");
                let _ = db.conn.execute(
                    "UPDATE passwords SET origin=?2, username=?3, password=?4, note=?5, updated=?6 WHERE id=?1",
                    params![id, origin, db.keys.seal_str(&s("username")), db.keys.seal_str(&s("password")), db.keys.seal_str(&s("note")), now_ms()],
                );
                return Reply::Now(json!(id));
            }
            match upsert(db, &origin, &s("username"), &s("password"), &s("note")) {
                Ok((id, _)) => Reply::Now(json!(id)),
                Err(e) => Reply::Err(e),
            }
        }
        "passwords.delete" => {
            let Some(db) = app.profile(&profile).db() else { return Reply::Err("vault unavailable".into()) };
            let _ = db.conn.execute("UPDATE passwords SET deleted=1, updated=?2 WHERE id=?1", params![s("id"), now_ms()]);
            db.tombstone("password", &s("id"));
            ok()
        }
        "passwords.importCsv" => {
            let path = s("path");
            let text = match std::fs::read(&path) {
                Ok(b) => String::from_utf8_lossy(&b).into_owned(),
                Err(e) => return Reply::Err(format!("Couldn't read the file: {e}")),
            };
            let Some(db) = app.profile(&profile).db() else { return Reply::Err("vault unavailable".into()) };
            match import_csv(db, &text) {
                Ok(r) => {
                    let mut deleted = false;
                    if a["deleteFile"].as_bool().unwrap_or(false) {
                        // overwrite before deleting so the plain-text export doesn't linger on disk
                        let len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0) as usize;
                        let _ = std::fs::write(&path, vec![0u8; len]);
                        deleted = std::fs::remove_file(&path).is_ok();
                    }
                    Reply::Now(json!({"added": r.added, "updated": r.updated, "skipped": r.skipped, "fileDeleted": deleted}))
                }
                Err(e) => Reply::Err(e),
            }
        }
        "passwords.exportCsv" => {
            if !verify_user(app, ctx.wid, "Solus wants to export your saved passwords") {
                return Reply::Err("Windows could not verify it's you".into());
            }
            let Some(path) = crate::platform::save_file_dialog(app, ctx.wid, "Solus Passwords.csv", "CSV file|*.csv") else { return Reply::Now(Value::Null) };
            let Some(db) = app.profile(&profile).db() else { return Reply::Err("vault unavailable".into()) };
            match std::fs::write(&path, export_csv(db)) {
                Ok(()) => Reply::Now(json!(path)),
                Err(e) => Reply::Err(e.to_string()),
            }
        }
        "passwords.neverList" => {
            let Some(db) = app.profile(&profile).db() else { return Reply::Now(json!([])) };
            Reply::Now(db.kv_get("pw_never").and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(json!([])))
        }
        "passwords.removeNever" => {
            let Some(db) = app.profile(&profile).db() else { return ok() };
            let mut never: Vec<String> = db.kv_get("pw_never").and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
            never.retain(|x| *x != s("origin"));
            db.kv_set("pw_never", &serde_json::to_string(&never).unwrap());
            ok()
        }
        "passwords.generate" => Reply::Now(json!(generate(a["length"].as_u64().unwrap_or(18) as usize))),
        "passwords.openChromeExport" => {
            // Chrome's own exporter is the only supported way to get passwords out of Chrome
            let chrome = crate::import::chrome_exe();
            match chrome {
                Some(exe) => {
                    let _ = std::process::Command::new(exe).arg("chrome://password-manager/settings").spawn();
                    ok()
                }
                None => Reply::Err("Google Chrome is not installed".into()),
            }
        }
        _ => Reply::Err("unknown passwords command".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn store() -> Store {
        Store::open(&std::env::temp_dir().join(format!("solus-pw-{}", util::new_id()))).unwrap()
    }
    #[test]
    fn csv_parsing_handles_quotes() {
        let rows = parse_csv("name,url,username,password,note\r\nex,https://ex.com/login,\"a,b\",\"p\"\"w\",\"line1\nline2\"\n");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1][2], "a,b");
        assert_eq!(rows[1][3], "p\"w");
        assert_eq!(rows[1][4], "line1\nline2");
    }
    #[test]
    fn chrome_csv_import_and_dedupe() {
        let db = store();
        let csv = "name,url,username,password,note\nex,https://ex.com/login,alice,S3cret!pass,\nex,https://ex.com/other,alice,N3w-pass-word,\nbad,,x,y,\n";
        let r = import_csv(&db, csv).unwrap();
        assert_eq!((r.added, r.updated, r.skipped), (1, 1, 1));
        let e = all(&db);
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].origin, "https://ex.com");
        assert_eq!(e[0].password, "N3w-pass-word");
        let raw: Vec<u8> = db.conn.query_row("SELECT password FROM passwords", [], |r| r.get(0)).unwrap();
        assert!(!String::from_utf8_lossy(&raw).contains("pass"), "stored encrypted");
        assert_eq!(count_for(&db, "https://ex.com/anything"), 1);
        assert!(export_csv(&db).contains("https://ex.com,alice,N3w-pass-word"));
    }
    #[test]
    fn firefox_and_bitwarden_formats() {
        let db = store();
        assert_eq!(import_csv(&db, "\"url\",\"username\",\"password\",\"httpRealm\"\n\"https://a.org\",\"u\",\"p1234567\",\"\"\n").unwrap().added, 1);
        assert_eq!(import_csv(&db, "folder,favorite,type,name,notes,fields,reprompt,login_uri,login_username,login_password,login_totp\n,,login,b,,,,https://b.org/x,bob,pw-123456,\n").unwrap().added, 1);
        assert!(import_csv(&db, "a,b\n1,2\n").is_err());
    }
    #[test]
    fn strength_and_generator() {
        assert_eq!(strength("password1"), 0);
        assert_eq!(strength("short"), 0);
        assert!(strength("Correct-Horse-Battery-9") >= 2);
        let g = generate(18);
        assert_eq!(g.len(), 18);
        assert!(strength(&g) >= 2);
    }
}
