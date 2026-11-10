//! Per-profile SQLite database (WAL). Sensitive columns (history URLs/titles, note bodies, passwords)
//! are AES-GCM encrypted with the profile key; lookups use keyed hashes.

use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

use crate::crypto::Keys;
use crate::util::{self, now_ms};

pub struct Store {
    pub conn: Connection,
    pub keys: Keys,
    pub path: PathBuf,
}

const SCHEMA_VERSION: i32 = 4;

impl Store {
    pub fn open(dir: &Path) -> Result<Store, String> {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let master = crate::crypto::load_or_create_master(&dir.join("key.bin"))?;
        let keys = Keys::from_master(&master);
        let path = dir.join("solus.db");
        let conn = match Self::open_conn(&path) {
            Ok(c) => c,
            Err(e) => {
                logf!("database open failed ({e}); trying latest backup");
                crate::backup::restore_db_from_backup(dir, &path);
                Self::open_conn(&path)?
            }
        };
        let s = Store { conn, keys, path };
        s.migrate()?;
        Ok(s)
    }

    fn open_conn(path: &Path) -> Result<Connection, String> {
        let c = Connection::open(path).map_err(|e| e.to_string())?;
        c.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON; \
             PRAGMA cache_size=-1024; PRAGMA temp_store=MEMORY; PRAGMA busy_timeout=3000;",
        )
        .map_err(|e| e.to_string())?;
        let ok: String = c.query_row("PRAGMA quick_check", [], |r| r.get(0)).map_err(|e| e.to_string())?;
        if ok != "ok" {
            return Err(format!("database check failed: {ok}"));
        }
        Ok(c)
    }

    fn migrate(&self) -> Result<(), String> {
        let v: i32 = self.conn.query_row("PRAGMA user_version", [], |r| r.get(0)).map_err(|e| e.to_string())?;
        if v < 1 {
            self.conn
                .execute_batch(
                    "
                CREATE TABLE IF NOT EXISTS history_urls(
                    id INTEGER PRIMARY KEY, uid TEXT UNIQUE NOT NULL, tag BLOB UNIQUE NOT NULL,
                    url BLOB NOT NULL, title BLOB, site_tag BLOB,
                    visits INTEGER NOT NULL DEFAULT 0, typed INTEGER NOT NULL DEFAULT 0,
                    last_visit INTEGER NOT NULL DEFAULT 0, updated INTEGER NOT NULL DEFAULT 0,
                    deleted INTEGER NOT NULL DEFAULT 0);
                CREATE INDEX IF NOT EXISTS hu_last ON history_urls(last_visit);
                CREATE INDEX IF NOT EXISTS hu_frecency ON history_urls(visits, last_visit);
                CREATE TABLE IF NOT EXISTS history_visits(
                    id INTEGER PRIMARY KEY, url_id INTEGER NOT NULL REFERENCES history_urls(id) ON DELETE CASCADE,
                    ts INTEGER NOT NULL, transition TEXT NOT NULL DEFAULT 'link');
                CREATE INDEX IF NOT EXISTS hv_ts ON history_visits(ts);
                CREATE INDEX IF NOT EXISTS hv_url ON history_visits(url_id);
                CREATE TABLE IF NOT EXISTS bookmarks(
                    id TEXT PRIMARY KEY, parent TEXT NOT NULL, kind TEXT NOT NULL, title TEXT NOT NULL DEFAULT '',
                    url TEXT NOT NULL DEFAULT '', pos REAL NOT NULL DEFAULT 0, created INTEGER NOT NULL,
                    updated INTEGER NOT NULL, deleted INTEGER NOT NULL DEFAULT 0);
                CREATE INDEX IF NOT EXISTS bm_parent ON bookmarks(parent, pos);
                CREATE INDEX IF NOT EXISTS bm_url ON bookmarks(url);
                CREATE TABLE IF NOT EXISTS notes(
                    id TEXT PRIMARY KEY, scope TEXT NOT NULL, key TEXT NOT NULL, title TEXT NOT NULL DEFAULT '',
                    body BLOB, color TEXT NOT NULL DEFAULT '', pinned INTEGER NOT NULL DEFAULT 0,
                    created INTEGER NOT NULL, updated INTEGER NOT NULL, deleted INTEGER NOT NULL DEFAULT 0);
                CREATE INDEX IF NOT EXISTS notes_key ON notes(scope, key);
                CREATE TABLE IF NOT EXISTS tasks(
                    id TEXT PRIMARY KEY, list TEXT NOT NULL DEFAULT 'Tasks', title TEXT NOT NULL, notes TEXT NOT NULL DEFAULT '',
                    due INTEGER NOT NULL DEFAULT 0, done INTEGER NOT NULL DEFAULT 0, url TEXT NOT NULL DEFAULT '',
                    pos REAL NOT NULL DEFAULT 0, created INTEGER NOT NULL, updated INTEGER NOT NULL, deleted INTEGER NOT NULL DEFAULT 0);
                CREATE TABLE IF NOT EXISTS reminders(
                    id TEXT PRIMARY KEY, title TEXT NOT NULL, body TEXT NOT NULL DEFAULT '', at INTEGER NOT NULL,
                    url TEXT NOT NULL DEFAULT '', fired INTEGER NOT NULL DEFAULT 0, repeat TEXT NOT NULL DEFAULT '',
                    created INTEGER NOT NULL, updated INTEGER NOT NULL, deleted INTEGER NOT NULL DEFAULT 0);
                CREATE TABLE IF NOT EXISTS passwords(
                    id TEXT PRIMARY KEY, origin TEXT NOT NULL, username BLOB, password BLOB NOT NULL, note BLOB,
                    created INTEGER NOT NULL, updated INTEGER NOT NULL, last_used INTEGER NOT NULL DEFAULT 0,
                    deleted INTEGER NOT NULL DEFAULT 0);
                CREATE INDEX IF NOT EXISTS pw_origin ON passwords(origin);
                CREATE TABLE IF NOT EXISTS permissions(
                    origin TEXT NOT NULL, kind TEXT NOT NULL, state TEXT NOT NULL, updated INTEGER NOT NULL,
                    PRIMARY KEY(origin, kind));
                CREATE TABLE IF NOT EXISTS workspaces(
                    id TEXT PRIMARY KEY, name TEXT NOT NULL, icon TEXT NOT NULL DEFAULT '', color TEXT NOT NULL DEFAULT '',
                    pos REAL NOT NULL DEFAULT 0, created INTEGER NOT NULL, updated INTEGER NOT NULL, deleted INTEGER NOT NULL DEFAULT 0);
                CREATE TABLE IF NOT EXISTS downloads(
                    id INTEGER PRIMARY KEY, url TEXT NOT NULL, path TEXT NOT NULL, mime TEXT NOT NULL DEFAULT '',
                    total INTEGER NOT NULL DEFAULT 0, received INTEGER NOT NULL DEFAULT 0, state TEXT NOT NULL,
                    started INTEGER NOT NULL, ended INTEGER NOT NULL DEFAULT 0);
                CREATE TABLE IF NOT EXISTS blocked_stats(
                    day INTEGER NOT NULL, site TEXT NOT NULL, count INTEGER NOT NULL DEFAULT 0, PRIMARY KEY(day, site));
                CREATE TABLE IF NOT EXISTS kv(key TEXT PRIMARY KEY, value TEXT NOT NULL);
                ",
                )
                .map_err(|e| e.to_string())?;
        }
        if v < 2 {
            let _ = self.conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS tombstones(kind TEXT NOT NULL, id TEXT NOT NULL, ts INTEGER NOT NULL, PRIMARY KEY(kind, id));",
            );
        }
        if v < 3 {
            let _ = self.conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS extensions(id TEXT PRIMARY KEY, name TEXT NOT NULL, version TEXT NOT NULL,
                 enabled INTEGER NOT NULL DEFAULT 1, path TEXT NOT NULL, installed INTEGER NOT NULL, source TEXT NOT NULL DEFAULT 'cws');",
            );
        }
        if v < 4 {
            let _ = self.conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS favicons(host TEXT PRIMARY KEY, data TEXT NOT NULL, updated INTEGER NOT NULL);",
            );
        }
        self.conn
            .execute_batch(&format!("PRAGMA user_version={SCHEMA_VERSION}"))
            .map_err(|e| e.to_string())?;
        self.ensure_bookmark_roots();
        Ok(())
    }

    pub fn kv_get(&self, key: &str) -> Option<String> {
        self.conn.query_row("SELECT value FROM kv WHERE key=?1", [key], |r| r.get(0)).optional().ok().flatten()
    }
    pub fn kv_set(&self, key: &str, value: &str) {
        let _ = self.conn.execute("INSERT OR REPLACE INTO kv(key,value) VALUES(?1,?2)", params![key, value]);
    }

    pub fn tombstone(&self, kind: &str, id: &str) {
        let _ = self.conn.execute("INSERT OR REPLACE INTO tombstones(kind,id,ts) VALUES(?1,?2,?3)", params![kind, id, now_ms()]);
    }

    // ------------------------------------------------------------------ history

    pub fn add_visit(&self, url: &str, title: &str, transition: &str) {
        if url.is_empty() || url.starts_with("solus:") || url.starts_with("about:") || url.starts_with("data:") {
            return;
        }
        let tag = self.keys.tag(url.as_bytes());
        let now = now_ms();
        let typed = if transition == "typed" { 1 } else { 0 };
        let existing: Option<i64> =
            self.conn.query_row("SELECT id FROM history_urls WHERE tag=?1", [&tag], |r| r.get(0)).optional().ok().flatten();
        let id = match existing {
            Some(id) => {
                let _ = self.conn.execute(
                    "UPDATE history_urls SET visits=visits+1, typed=typed+?2, last_visit=?3, updated=?3, deleted=0,
                     title=CASE WHEN ?4 IS NULL THEN title ELSE ?4 END WHERE id=?1",
                    params![id, typed, now, if title.is_empty() { None } else { Some(self.keys.seal_str(title)) }],
                );
                id
            }
            None => {
                let site_tag = self.keys.tag(util::site_of(url).as_bytes());
                let _ = self.conn.execute(
                    "INSERT INTO history_urls(uid,tag,url,title,site_tag,visits,typed,last_visit,updated) VALUES(?1,?2,?3,?4,?5,1,?6,?7,?7)",
                    params![util::new_id(), tag, self.keys.seal_str(url), self.keys.seal_str(title), site_tag, typed, now],
                );
                self.conn.last_insert_rowid()
            }
        };
        // collapse rapid repeat visits (reloads) into one entry
        let recent: Option<i64> = self
            .conn
            .query_row("SELECT ts FROM history_visits WHERE url_id=?1 ORDER BY ts DESC LIMIT 1", [id], |r| r.get(0))
            .optional()
            .ok()
            .flatten();
        if recent.map(|t| now - t > 60_000).unwrap_or(true) {
            let _ = self.conn.execute(
                "INSERT INTO history_visits(url_id, ts, transition) VALUES(?1,?2,?3)",
                params![id, now, transition],
            );
        }
    }

    pub fn update_title(&self, url: &str, title: &str) {
        if title.is_empty() {
            return;
        }
        let tag = self.keys.tag(url.as_bytes());
        let _ = self.conn.execute("UPDATE history_urls SET title=?2 WHERE tag=?1", params![tag, self.keys.seal_str(title)]);
    }

    /// History page: visits newest first, optionally filtered by text, before a timestamp.
    pub fn history_query(&self, text: &str, before: i64, limit: usize) -> Vec<Value> {
        let text = text.to_lowercase();
        let mut out = vec![];
        let before = if before <= 0 { i64::MAX } else { before };
        let Ok(mut st) = self.conn.prepare(
            "SELECT v.id, v.ts, u.url, u.title, u.visits FROM history_visits v JOIN history_urls u ON u.id=v.url_id
             WHERE v.ts < ?1 AND u.deleted=0 ORDER BY v.ts DESC LIMIT ?2",
        ) else {
            return out;
        };
        // scan in pages so a text filter can still return `limit` matches
        let scan = if text.is_empty() { limit } else { 20_000 };
        let rows = st.query_map(params![before, scan as i64], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, Vec<u8>>(2)?, r.get::<_, Option<Vec<u8>>>(3)?, r.get::<_, i64>(4)?))
        });
        if let Ok(rows) = rows {
            for (vid, ts, u, t, visits) in rows.flatten() {
                let url = self.keys.open_str(&u);
                let title = t.map(|t| self.keys.open_str(&t)).unwrap_or_default();
                if !text.is_empty() && !url.to_lowercase().contains(&text) && !title.to_lowercase().contains(&text) {
                    continue;
                }
                out.push(json!({"id": vid, "ts": ts, "url": url, "title": title, "visits": visits}));
                if out.len() >= limit {
                    break;
                }
            }
        }
        out
    }

    /// Omnibox matches scored by frecency.
    pub fn history_matches(&self, q: &str, limit: usize) -> Vec<(String, String, f64)> {
        let q = q.to_lowercase();
        let now = now_ms();
        let mut out: Vec<(String, String, f64)> = vec![];
        let Ok(mut st) = self.conn.prepare(
            "SELECT url, title, visits, typed, last_visit FROM history_urls WHERE deleted=0
             ORDER BY (visits + typed*3) DESC, last_visit DESC LIMIT 4000",
        ) else {
            return out;
        };
        let rows = st.query_map([], |r| {
            Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, Option<Vec<u8>>>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?, r.get::<_, i64>(4)?))
        });
        if let Ok(rows) = rows {
            for (u, t, visits, typed, last) in rows.flatten() {
                let url = self.keys.open_str(&u);
                let title = t.map(|t| self.keys.open_str(&t)).unwrap_or_default();
                let bare = url
                    .trim_start_matches("https://")
                    .trim_start_matches("http://")
                    .trim_start_matches("www.")
                    .to_lowercase();
                let tl = title.to_lowercase();
                let mut score = 0.0;
                if bare.starts_with(&q) {
                    score += 10.0;
                } else if bare.contains(&q) {
                    score += 4.0;
                }
                if tl.contains(&q) {
                    score += 3.0;
                }
                if score == 0.0 {
                    continue;
                }
                let age_days = ((now - last).max(0) as f64) / 86_400_000.0;
                score += ((visits + typed * 3) as f64).ln_1p() * 2.0 - age_days.min(90.0) / 30.0;
                // prefer shorter URLs (home pages) for equal matches
                score -= bare.len() as f64 / 200.0;
                out.push((url, title, score));
            }
        }
        out.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
        out.truncate(limit);
        out
    }

    pub fn most_visited(&self, limit: usize) -> Vec<Value> {
        let mut out = vec![];
        let Ok(mut st) = self.conn.prepare(
            "SELECT url, title, visits FROM history_urls WHERE deleted=0 AND last_visit > ?1 ORDER BY (visits + typed*3) DESC LIMIT ?2",
        ) else {
            return out;
        };
        let since = now_ms() - 60 * 86_400_000;
        let mut seen_sites = std::collections::HashSet::new();
        if let Ok(rows) = st.query_map(params![since, (limit * 4) as i64], |r| {
            Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, Option<Vec<u8>>>(1)?, r.get::<_, i64>(2)?))
        }) {
            for (u, t, visits) in rows.flatten() {
                let url = self.keys.open_str(&u);
                // one shortcut per site
                if !seen_sites.insert(util::host_of(&url)) {
                    continue;
                }
                let title = t.map(|t| self.keys.open_str(&t)).unwrap_or_default();
                out.push(json!({"url": url, "title": title, "visits": visits}));
                if out.len() >= limit {
                    break;
                }
            }
        }
        out
    }

    pub fn delete_visits(&self, ids: &[i64]) {
        for id in ids {
            let _ = self.conn.execute("DELETE FROM history_visits WHERE id=?1", [id]);
        }
        let _ = self.conn.execute(
            "UPDATE history_urls SET deleted=1, updated=?1 WHERE id NOT IN (SELECT DISTINCT url_id FROM history_visits) AND deleted=0",
            [now_ms()],
        );
    }

    pub fn delete_url(&self, url: &str) {
        let tag = self.keys.tag(url.as_bytes());
        if let Ok(Some(id)) =
            self.conn.query_row("SELECT id FROM history_urls WHERE tag=?1", [&tag], |r| r.get::<_, i64>(0)).optional()
        {
            let _ = self.conn.execute("DELETE FROM history_visits WHERE url_id=?1", [id]);
            let _ = self.conn.execute("UPDATE history_urls SET deleted=1, updated=?2 WHERE id=?1", params![id, now_ms()]);
        }
    }

    /// Clear history visited in [from, to] (ms). Pass 0, i64::MAX for everything.
    pub fn clear_history(&self, from: i64, to: i64) -> usize {
        let n = self
            .conn
            .execute("DELETE FROM history_visits WHERE ts BETWEEN ?1 AND ?2", params![from, to])
            .unwrap_or(0);
        let _ = self.conn.execute(
            "UPDATE history_urls SET deleted=1, updated=?1 WHERE id NOT IN (SELECT DISTINCT url_id FROM history_visits)",
            [now_ms()],
        );
        if from == 0 {
            let _ = self.conn.execute("DELETE FROM history_urls WHERE deleted=1", []);
            let _ = self.conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); VACUUM;");
        }
        n
    }

    pub fn history_count(&self) -> i64 {
        self.conn.query_row("SELECT COUNT(*) FROM history_urls WHERE deleted=0", [], |r| r.get(0)).unwrap_or(0)
    }

    // ------------------------------------------------------------------ bookmarks

    fn ensure_bookmark_roots(&self) {
        let now = now_ms();
        for (id, title, pos) in [("bar", "Bookmarks bar", 0.0), ("other", "Other bookmarks", 1.0)] {
            let _ = self.conn.execute(
                "INSERT OR IGNORE INTO bookmarks(id,parent,kind,title,pos,created,updated) VALUES(?1,'root','folder',?2,?3,?4,?4)",
                params![id, title, pos, now],
            );
        }
    }

    fn bm_row(r: &rusqlite::Row) -> rusqlite::Result<Value> {
        Ok(json!({
            "id": r.get::<_, String>(0)?, "parent": r.get::<_, String>(1)?, "kind": r.get::<_, String>(2)?,
            "title": r.get::<_, String>(3)?, "url": r.get::<_, String>(4)?, "pos": r.get::<_, f64>(5)?,
            "created": r.get::<_, i64>(6)?,
        }))
    }

    pub fn bm_children(&self, parent: &str) -> Vec<Value> {
        let Ok(mut st) = self.conn.prepare(
            "SELECT id,parent,kind,title,url,pos,created FROM bookmarks WHERE parent=?1 AND deleted=0 ORDER BY pos, created",
        ) else {
            return vec![];
        };
        st.query_map([parent], Self::bm_row).map(|r| r.flatten().collect()).unwrap_or_default()
    }

    pub fn bm_tree(&self, parent: &str, depth: usize) -> Vec<Value> {
        let mut kids = self.bm_children(parent);
        if depth > 0 {
            for k in kids.iter_mut() {
                if k["kind"] == "folder" {
                    let id = k["id"].as_str().unwrap_or("").to_string();
                    k["children"] = json!(self.bm_tree(&id, depth - 1));
                }
            }
        }
        kids
    }

    pub fn bm_get(&self, id: &str) -> Option<Value> {
        self.conn
            .query_row("SELECT id,parent,kind,title,url,pos,created FROM bookmarks WHERE id=?1 AND deleted=0", [id], Self::bm_row)
            .optional()
            .ok()
            .flatten()
    }

    pub fn bm_find_url(&self, url: &str) -> Option<Value> {
        self.conn
            .query_row(
                "SELECT id,parent,kind,title,url,pos,created FROM bookmarks WHERE url=?1 AND deleted=0 AND kind='url' LIMIT 1",
                [url],
                Self::bm_row,
            )
            .optional()
            .ok()
            .flatten()
    }

    fn next_pos(&self, parent: &str) -> f64 {
        self.conn
            .query_row("SELECT COALESCE(MAX(pos),0)+1 FROM bookmarks WHERE parent=?1 AND deleted=0", [parent], |r| r.get(0))
            .unwrap_or(1.0)
    }

    pub fn bm_add(&self, parent: &str, kind: &str, title: &str, url: &str, index: Option<usize>) -> Result<String, String> {
        let parent = if parent.is_empty() { "bar" } else { parent };
        if self.bm_get(parent).map(|p| p["kind"] != "folder").unwrap_or(true) {
            return Err("parent folder not found".into());
        }
        if kind == "url" {
            let u = url::Url::parse(url).map_err(|_| "invalid URL".to_string())?;
            if u.scheme() == "javascript" {
                return Err("javascript: bookmarks are not supported".into());
            }
        }
        let id = util::new_id();
        let now = now_ms();
        let pos = match index {
            Some(i) => self.pos_for_index(parent, i),
            None => self.next_pos(parent),
        };
        self.conn
            .execute(
                "INSERT INTO bookmarks(id,parent,kind,title,url,pos,created,updated) VALUES(?1,?2,?3,?4,?5,?6,?7,?7)",
                params![id, parent, kind, title, url, pos, now],
            )
            .map_err(|e| e.to_string())?;
        Ok(id)
    }

    fn pos_for_index(&self, parent: &str, index: usize) -> f64 {
        let kids = self.bm_children(parent);
        let before = if index == 0 { kids.first().and_then(|k| k["pos"].as_f64()).map(|p| p - 1.0) } else { None };
        if let Some(p) = before {
            return p;
        }
        let a = kids.get(index.wrapping_sub(1)).and_then(|k| k["pos"].as_f64());
        let b = kids.get(index).and_then(|k| k["pos"].as_f64());
        match (a, b) {
            (Some(a), Some(b)) => (a + b) / 2.0,
            (Some(a), None) => a + 1.0,
            (None, Some(b)) => b - 1.0,
            (None, None) => 1.0,
        }
    }

    pub fn bm_update(&self, id: &str, title: Option<&str>, url: Option<&str>) -> Result<(), String> {
        if ["bar", "other", "root"].contains(&id) {
            if url.is_some() {
                return Err("cannot change a root folder".into());
            }
        }
        if let Some(u) = url {
            url::Url::parse(u).map_err(|_| "invalid URL".to_string())?;
        }
        let now = now_ms();
        if let Some(t) = title {
            self.conn.execute("UPDATE bookmarks SET title=?2, updated=?3 WHERE id=?1", params![id, t, now]).map_err(|e| e.to_string())?;
        }
        if let Some(u) = url {
            self.conn.execute("UPDATE bookmarks SET url=?2, updated=?3 WHERE id=?1", params![id, u, now]).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn bm_move(&self, id: &str, parent: &str, index: usize) -> Result<(), String> {
        if ["bar", "other"].contains(&id) {
            return Err("cannot move a root folder".into());
        }
        // refuse moving a folder into its own subtree
        let mut cur = parent.to_string();
        for _ in 0..64 {
            if cur == id {
                return Err("cannot move a folder into itself".into());
            }
            match self.bm_get(&cur) {
                Some(p) => cur = p["parent"].as_str().unwrap_or("root").to_string(),
                None => break,
            }
            if cur == "root" {
                break;
            }
        }
        let others: Vec<Value> = self.bm_children(parent).into_iter().filter(|k| k["id"] != id).collect();
        let pos = {
            let a = if index == 0 { None } else { others.get(index - 1).and_then(|k| k["pos"].as_f64()) };
            let b = others.get(index).and_then(|k| k["pos"].as_f64());
            match (a, b) {
                (Some(a), Some(b)) => (a + b) / 2.0,
                (Some(a), None) => a + 1.0,
                (None, Some(b)) => b - 1.0,
                (None, None) => 1.0,
            }
        };
        self.conn
            .execute("UPDATE bookmarks SET parent=?2, pos=?3, updated=?4 WHERE id=?1", params![id, parent, pos, now_ms()])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn bm_delete(&self, id: &str) -> Result<(), String> {
        if ["bar", "other", "root"].contains(&id) {
            return Err("cannot delete a root folder".into());
        }
        let now = now_ms();
        let mut stack = vec![id.to_string()];
        while let Some(cur) = stack.pop() {
            for k in self.bm_children(&cur) {
                stack.push(k["id"].as_str().unwrap_or("").to_string());
            }
            let _ = self.conn.execute("UPDATE bookmarks SET deleted=1, updated=?2 WHERE id=?1", params![cur, now]);
            self.tombstone("bookmark", &cur);
        }
        Ok(())
    }

    pub fn bm_search(&self, q: &str, limit: usize) -> Vec<Value> {
        let like = format!("%{}%", q.replace('%', "\\%").replace('_', "\\_"));
        let Ok(mut st) = self.conn.prepare(
            "SELECT id,parent,kind,title,url,pos,created FROM bookmarks WHERE deleted=0 AND kind='url'
             AND (title LIKE ?1 ESCAPE '\\' OR url LIKE ?1 ESCAPE '\\') ORDER BY updated DESC LIMIT ?2",
        ) else {
            return vec![];
        };
        st.query_map(params![like, limit as i64], Self::bm_row).map(|r| r.flatten().collect()).unwrap_or_default()
    }

    pub fn bm_count(&self) -> i64 {
        self.conn.query_row("SELECT COUNT(*) FROM bookmarks WHERE deleted=0 AND kind='url'", [], |r| r.get(0)).unwrap_or(0)
    }

    /// Insert a folder/url tree (import). Returns number of bookmarks added; skips URL duplicates
    /// within the same folder.
    pub fn bm_import_tree(&self, parent: &str, nodes: &[Value]) -> usize {
        let mut n = 0;
        let existing: std::collections::HashSet<String> =
            self.bm_children(parent).iter().map(|k| format!("{}|{}", k["kind"], k["url"].as_str().unwrap_or(k["title"].as_str().unwrap_or("")))).collect();
        for node in nodes {
            let kind = node["kind"].as_str().unwrap_or("url");
            let title = node["title"].as_str().unwrap_or("");
            if kind == "folder" {
                let key = format!("\"folder\"|{title}");
                let fid = if existing.contains(&key) {
                    self.bm_children(parent)
                        .into_iter()
                        .find(|k| k["kind"] == "folder" && k["title"] == title)
                        .and_then(|k| k["id"].as_str().map(|s| s.to_string()))
                } else {
                    self.bm_add(parent, "folder", title, "", None).ok()
                };
                if let Some(fid) = fid {
                    n += self.bm_import_tree(&fid, node["children"].as_array().map(|a| a.as_slice()).unwrap_or(&[]));
                }
            } else {
                let url = node["url"].as_str().unwrap_or("");
                if url.is_empty() || existing.contains(&format!("\"url\"|{url}")) {
                    continue;
                }
                if self.bm_add(parent, "url", title, url, None).is_ok() {
                    n += 1;
                }
            }
        }
        n
    }

    pub fn favicon_set(&self, host: &str, data: &str) {
        if host.is_empty() || data.len() > 64 * 1024 {
            return;
        }
        let _ = self.conn.execute(
            "INSERT OR REPLACE INTO favicons(host,data,updated) VALUES(?1,?2,?3)",
            params![host, data, now_ms()],
        );
    }

    pub fn favicon_for_url(&self, url: &str) -> Option<String> {
        let host = util::host_of(url);
        if host.is_empty() {
            return None;
        }
        self.conn.query_row("SELECT data FROM favicons WHERE host=?1", [host], |r| r.get(0)).optional().ok().flatten()
    }

    /// Attach an `icon` field to rows that have a `url`.
    pub fn with_icons(&self, mut rows: Vec<Value>) -> Vec<Value> {
        let mut cache: std::collections::HashMap<String, Option<String>> = Default::default();
        for r in rows.iter_mut() {
            if let Some(u) = r["url"].as_str() {
                if u.is_empty() {
                    continue;
                }
                let host = util::host_of(u);
                let icon = cache.entry(host).or_insert_with(|| self.favicon_for_url(u)).clone();
                if let Some(i) = icon {
                    r["icon"] = json!(i);
                }
            }
        }
        rows
    }

    pub fn checkpoint(&self) {
        let _ = self.conn.execute_batch("PRAGMA wal_checkpoint(PASSIVE);");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> Store {
        let dir = std::env::temp_dir().join(format!("solus-test-{}", util::new_id()));
        Store::open(&dir).unwrap()
    }

    #[test]
    fn history_is_encrypted_and_searchable() {
        let s = temp_store();
        s.add_visit("https://www.rust-lang.org/learn", "Learn Rust", "typed");
        s.add_visit("https://example.com/", "Example Domain", "link");
        // ciphertext on disk
        let raw: Vec<u8> = s.conn.query_row("SELECT url FROM history_urls LIMIT 1", [], |r| r.get(0)).unwrap();
        assert!(!String::from_utf8_lossy(&raw).contains("rust-lang"));
        let m = s.history_matches("rust", 5);
        assert_eq!(m[0].0, "https://www.rust-lang.org/learn");
        let q = s.history_query("example", 0, 10);
        assert_eq!(q.len(), 1);
        assert_eq!(q[0]["title"], "Example Domain");
        s.clear_history(0, i64::MAX);
        assert_eq!(s.history_count(), 0);
    }

    #[test]
    fn bookmarks_tree_ops() {
        let s = temp_store();
        let f = s.bm_add("bar", "folder", "Work", "", None).unwrap();
        let a = s.bm_add(&f, "url", "A", "https://a.example/", None).unwrap();
        let _b = s.bm_add(&f, "url", "B", "https://b.example/", Some(0)).unwrap();
        let kids = s.bm_children(&f);
        assert_eq!(kids[0]["title"], "B");
        s.bm_move(&a, "bar", 0).unwrap();
        assert_eq!(s.bm_children("bar")[0]["id"], json!(a));
        assert!(s.bm_move(&f, &f, 0).is_err());
        s.bm_delete(&f).unwrap();
        assert!(s.bm_get(&f).is_none());
        assert!(s.bm_add("bar", "url", "x", "javascript:alert(1)", None).is_err());
        assert_eq!(s.bm_find_url("https://a.example/").unwrap()["id"], json!(a));
    }
}
