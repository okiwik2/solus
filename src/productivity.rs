//! Workspaces, tab groups, website-attached notes, tasks and reminders.

use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};

use crate::app::App;
use crate::bridge::{self, ok, Ctx, Reply};
use crate::store::Store;
use crate::tab::{OpenOpts, NEWTAB};
use crate::util::{self, now_ms};
use crate::window::TabGroup;

pub fn note_key(url: &str) -> String {
    url.split('#').next().unwrap_or(url).to_string()
}

pub fn note_count_for(db: &Store, url: &str) -> i64 {
    if !url.starts_with("http") {
        return 0;
    }
    let site = util::host_of(url).trim_start_matches("www.").to_string();
    db.conn
        .query_row(
            "SELECT COUNT(*) FROM notes WHERE deleted=0 AND ((scope='page' AND key=?1) OR (scope='site' AND key=?2))",
            params![note_key(url), site],
            |r| r.get(0),
        )
        .unwrap_or(0)
}

fn note_row(db: &Store, r: &rusqlite::Row) -> rusqlite::Result<Value> {
    let body: Option<Vec<u8>> = r.get(4)?;
    Ok(json!({
        "id": r.get::<_, String>(0)?, "scope": r.get::<_, String>(1)?, "key": r.get::<_, String>(2)?,
        "title": r.get::<_, String>(3)?, "body": body.map(|b| db.keys.open_str(&b)).unwrap_or_default(),
        "color": r.get::<_, String>(5)?, "updated": r.get::<_, i64>(6)?, "created": r.get::<_, i64>(7)?,
    }))
}

pub fn notes_for_url(db: &Store, url: &str) -> Vec<Value> {
    let site = util::host_of(url).trim_start_matches("www.").to_string();
    let Ok(mut st) = db.conn.prepare(
        "SELECT id,scope,key,title,body,color,updated,created FROM notes WHERE deleted=0 AND ((scope='page' AND key=?1) OR (scope='site' AND key=?2)) ORDER BY updated DESC",
    ) else {
        return vec![];
    };
    st.query_map(params![note_key(url), site], |r| note_row(db, r)).map(|r| r.flatten().collect()).unwrap_or_default()
}

pub fn notes_list(db: &Store, q: &str) -> Vec<Value> {
    let ql = q.to_lowercase();
    let Ok(mut st) = db.conn.prepare("SELECT id,scope,key,title,body,color,updated,created FROM notes WHERE deleted=0 ORDER BY updated DESC LIMIT 1000") else {
        return vec![];
    };
    let rows: Vec<Value> = st.query_map([], |r| note_row(db, r)).map(|r| r.flatten().collect()).unwrap_or_default();
    rows.into_iter()
        .filter(|n| ql.is_empty() || n["body"].as_str().unwrap_or("").to_lowercase().contains(&ql) || n["key"].as_str().unwrap_or("").to_lowercase().contains(&ql))
        .take(200)
        .collect()
}

pub fn note_save(db: &Store, args: &Value) -> Result<String, String> {
    let body = args["body"].as_str().unwrap_or("").trim().to_string();
    if body.is_empty() {
        return Err("Note is empty".into());
    }
    if body.len() > 100_000 {
        return Err("Note is too long".into());
    }
    let scope = if args["scope"] == "site" { "site" } else { "page" };
    let url = args["url"].as_str().unwrap_or("");
    let key = match args["key"].as_str() {
        Some(k) if !k.is_empty() => k.to_string(),
        _ if scope == "site" => util::host_of(url).trim_start_matches("www.").to_string(),
        _ => note_key(url),
    };
    if key.is_empty() {
        return Err("Notes need a web page".into());
    }
    let now = now_ms();
    let sealed = db.keys.seal_str(&body);
    let color = args["color"].as_str().unwrap_or("").to_string();
    match args["id"].as_str() {
        Some(id) if !id.is_empty() => {
            db.conn
                .execute("UPDATE notes SET body=?2, scope=?3, key=?4, updated=?5 WHERE id=?1", params![id, sealed, scope, key, now])
                .map_err(|e| e.to_string())?;
            Ok(id.to_string())
        }
        _ => {
            let id = util::new_id();
            db.conn
                .execute(
                    "INSERT INTO notes(id,scope,key,title,body,color,created,updated) VALUES(?1,?2,?3,'',?4,?5,?6,?6)",
                    params![id, scope, key, sealed, color, now],
                )
                .map_err(|e| e.to_string())?;
            Ok(id)
        }
    }
}

fn task_row(r: &rusqlite::Row) -> rusqlite::Result<Value> {
    Ok(json!({
        "id": r.get::<_, String>(0)?, "list": r.get::<_, String>(1)?, "title": r.get::<_, String>(2)?,
        "notes": r.get::<_, String>(3)?, "due": r.get::<_, i64>(4)?, "done": r.get::<_, i64>(5)? != 0,
        "url": r.get::<_, String>(6)?, "updated": r.get::<_, i64>(7)?,
    }))
}

pub fn tasks_list(db: &Store) -> Vec<Value> {
    let Ok(mut st) = db.conn.prepare(
        "SELECT id,list,title,notes,due,done,url,updated FROM tasks WHERE deleted=0
         ORDER BY done, CASE WHEN due=0 THEN 1 ELSE 0 END, due, pos, created",
    ) else {
        return vec![];
    };
    st.query_map([], task_row).map(|r| r.flatten().collect()).unwrap_or_default()
}

pub fn task_save(db: &Store, a: &Value) -> Result<String, String> {
    let now = now_ms();
    match a["id"].as_str() {
        Some(id) if !id.is_empty() => {
            if let Some(t) = a["title"].as_str() {
                if t.trim().is_empty() {
                    return Err("Task needs a title".into());
                }
                db.conn.execute("UPDATE tasks SET title=?2, updated=?3 WHERE id=?1", params![id, t.trim(), now]).map_err(|e| e.to_string())?;
            }
            if let Some(d) = a["done"].as_bool() {
                db.conn.execute("UPDATE tasks SET done=?2, updated=?3 WHERE id=?1", params![id, d as i64, now]).map_err(|e| e.to_string())?;
            }
            if let Some(d) = a["due"].as_i64() {
                db.conn.execute("UPDATE tasks SET due=?2, updated=?3 WHERE id=?1", params![id, d, now]).map_err(|e| e.to_string())?;
            }
            if let Some(n) = a["notes"].as_str() {
                db.conn.execute("UPDATE tasks SET notes=?2, updated=?3 WHERE id=?1", params![id, n, now]).map_err(|e| e.to_string())?;
            }
            Ok(id.to_string())
        }
        _ => {
            let title = a["title"].as_str().unwrap_or("").trim().to_string();
            if title.is_empty() {
                return Err("Task needs a title".into());
            }
            let id = util::new_id();
            db.conn
                .execute(
                    "INSERT INTO tasks(id,list,title,notes,due,done,url,pos,created,updated) VALUES(?1,?2,?3,?4,?5,0,?6,?7,?7,?7)",
                    params![
                        id,
                        a["list"].as_str().unwrap_or("Tasks"),
                        title,
                        a["notes"].as_str().unwrap_or(""),
                        a["due"].as_i64().unwrap_or(0),
                        a["url"].as_str().unwrap_or(""),
                        now
                    ],
                )
                .map_err(|e| e.to_string())?;
            Ok(id)
        }
    }
}

pub fn reminders_list(db: &Store) -> Vec<Value> {
    let Ok(mut st) = db.conn.prepare("SELECT id,title,body,at,url,fired FROM reminders WHERE deleted=0 ORDER BY at") else {
        return vec![];
    };
    st.query_map([], |r| {
        Ok(json!({"id": r.get::<_, String>(0)?, "title": r.get::<_, String>(1)?, "body": r.get::<_, String>(2)?,
            "at": r.get::<_, i64>(3)?, "url": r.get::<_, String>(4)?, "fired": r.get::<_, i64>(5)? != 0}))
    })
    .map(|r| r.flatten().collect())
    .unwrap_or_default()
}

pub fn reminder_save(db: &Store, a: &Value) -> Result<String, String> {
    let title = a["title"].as_str().unwrap_or("").trim().to_string();
    let at = a["at"].as_i64().unwrap_or(0);
    if title.is_empty() || at <= 0 {
        return Err("A reminder needs a title and a time".into());
    }
    let now = now_ms();
    let id = a["id"].as_str().filter(|s| !s.is_empty()).map(|s| s.to_string()).unwrap_or_else(util::new_id);
    db.conn
        .execute(
            "INSERT INTO reminders(id,title,body,at,url,fired,created,updated) VALUES(?1,?2,?3,?4,?5,0,?6,?6)
             ON CONFLICT(id) DO UPDATE SET title=?2, body=?3, at=?4, url=?5, fired=0, updated=?6",
            params![id, title, a["body"].as_str().unwrap_or(""), at, a["url"].as_str().unwrap_or(""), now],
        )
        .map_err(|e| e.to_string())?;
    Ok(id)
}

thread_local! {
    static LAST_REMINDER_URL: std::cell::RefCell<(String, String)> = std::cell::RefCell::new((String::new(), String::new()));
}

/// Fire due reminders (runs from the periodic tick, also while Solus is in the tray).
pub fn check_reminders(app: &mut App) {
    let now = now_ms();
    let ids: Vec<String> = app.local.profiles.iter().map(|p| p.id.clone()).collect();
    for pid in ids {
        let due: Vec<(String, String, String)> = {
            let Some(db) = app.profile(&pid).db() else { continue };
            let Ok(mut st) = db.conn.prepare("SELECT id,title,url FROM reminders WHERE deleted=0 AND fired=0 AND at<=?1") else { continue };
            let v: Vec<(String, String, String)> = st
                .query_map([now], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                .map(|r| r.flatten().collect())
                .unwrap_or_default();
            for (id, _, _) in &v {
                let _ = db.conn.execute("UPDATE reminders SET fired=1, updated=?2 WHERE id=?1", params![id, now]);
            }
            v
        };
        for (_, title, url) in due {
            let body = if url.is_empty() { "Reminder".to_string() } else { format!("Click to open {}", util::host_of(&url)) };
            crate::window::tray_notify(&title, &body);
            LAST_REMINDER_URL.with(|l| *l.borrow_mut() = (pid.clone(), url.clone()));
            let wins: Vec<u32> = app.windows.iter().filter(|w| w.profile == pid && !w.incognito).map(|w| w.id).collect();
            for w in wins {
                let mut buttons = vec![];
                if !url.is_empty() {
                    buttons.push(json!({"label": "Open page", "primary": true, "cmd": "newTab", "args": {"url": url}}));
                }
                bridge::emit(app, w, "infobar", json!({"id": format!("rem-{now}"), "icon": "bell", "text": format!("Reminder: {title}"), "buttons": buttons}));
                bridge::emit(app, w, "tasksChanged", json!({}));
            }
        }
    }
}

/// The user clicked the reminder notification.
pub fn on_notification_click(app: &mut App) {
    let (pid, url) = LAST_REMINDER_URL.with(|l| l.borrow().clone());
    if pid.is_empty() {
        return;
    }
    let wid = app.windows.iter().find(|w| w.profile == pid && !w.incognito).map(|w| w.id);
    let wid = match wid {
        Some(w) => w,
        None => app.new_window(&pid, false, vec![], false),
    };
    if !url.is_empty() {
        app.open_tab(wid, &url, OpenOpts { activate: true, ..Default::default() });
    }
    app.show_window(wid);
}

// ---------------------------------------------------------------- workspaces

pub fn workspaces_list(db: &Store) -> Vec<Value> {
    let now = now_ms();
    let _ = db.conn.execute(
        "INSERT OR IGNORE INTO workspaces(id,name,icon,color,pos,created,updated) VALUES('', 'Personal', '', '', 0, ?1, ?1)",
        [now],
    );
    let Ok(mut st) = db.conn.prepare("SELECT id,name,icon,color,pos FROM workspaces WHERE deleted=0 ORDER BY pos, created") else {
        return vec![];
    };
    st.query_map([], |r| {
        Ok(json!({"id": r.get::<_, String>(0)?, "name": r.get::<_, String>(1)?, "icon": r.get::<_, String>(2)?, "color": r.get::<_, String>(3)?}))
    })
    .map(|r| r.flatten().collect())
    .unwrap_or_default()
}

fn push_workspaces(app: &mut App, profile: &str) {
    let list = match app.profile(profile).db() {
        Some(db) => workspaces_list(db),
        None => return,
    };
    let ids: Vec<u32> = app.windows.iter().filter(|w| w.profile == profile).map(|w| w.id).collect();
    for id in ids {
        bridge::emit(app, id, "workspaces", json!(list));
    }
    bridge::emit_pages(app, profile, "workspaces", json!(list));
}

pub fn switch_workspace(app: &mut App, wid: u32, ws: &str) {
    let Some(w) = app.win_mut(wid) else { return };
    if w.workspace == ws {
        return;
    }
    w.workspace = ws.to_string();
    w.split = None;
    let target = w.tabs.iter().filter(|t| t.workspace == ws && !t.prerender).max_by_key(|t| t.last_active).map(|t| t.id);
    w.session_dirty = true;
    match target {
        Some(t) => app.activate_tab(wid, t),
        None => {
            app.open_tab(wid, NEWTAB, OpenOpts { activate: true, workspace: Some(ws.to_string()), ..Default::default() });
        }
    }
    bridge::push_tabs(app, wid);
}

pub fn move_tabs_to_workspace(app: &mut App, wid: u32, ids: &[u32], ws: &str) {
    let Some(w) = app.win_mut(wid) else { return };
    let mut active_moved = false;
    for t in w.tabs.iter_mut().filter(|t| ids.contains(&t.id)) {
        t.workspace = ws.to_string();
        t.group.clear();
        if t.id == w.active {
            active_moved = true;
        }
    }
    w.session_dirty = true;
    if active_moved {
        let cur = w.workspace.clone();
        let next = w.tabs.iter().filter(|t| t.workspace == cur && !t.prerender).max_by_key(|t| t.last_active).map(|t| t.id);
        match next {
            Some(n) => app.activate_tab(wid, n),
            None => {
                app.open_tab(wid, NEWTAB, OpenOpts { activate: true, workspace: Some(cur), ..Default::default() });
            }
        }
    }
    prune_groups(app, wid);
    bridge::push_tabs(app, wid);
}

pub fn prune_groups(app: &mut App, wid: u32) {
    if let Some(w) = app.win_mut(wid) {
        let used: std::collections::HashSet<String> = w.tabs.iter().map(|t| t.group.clone()).collect();
        w.groups.retain(|g| used.contains(&g.id));
    }
}

/// Keep a group's tabs next to each other.
fn gather_group(w: &mut crate::window::BrowserWindow, gid: &str) {
    let Some(first) = w.tabs.iter().position(|t| t.group == gid) else { return };
    let (grouped, rest): (Vec<_>, Vec<_>) = w.tabs.drain(..).partition(|t| t.group == gid);
    let mut rest = rest;
    let at = first.min(rest.len());
    let tail = rest.split_off(at);
    rest.extend(grouped);
    rest.extend(tail);
    w.tabs = rest;
}

pub fn cmd(app: &mut App, ctx: &Ctx, cmd: &str, a: &Value) -> Reply {
    let (profile, incognito) = app.win(ctx.wid).map(|w| (w.profile.clone(), w.incognito)).unwrap_or_default();
    let ids_of = |a: &Value| -> Vec<u32> { a["ids"].as_array().map(|x| x.iter().filter_map(|v| v.as_u64().map(|n| n as u32)).collect()).unwrap_or_default() };
    let ns = cmd.split('.').next().unwrap_or("");
    // tab groups live in the window, not the database
    if ns == "groups" {
        let wid = ctx.wid;
        let r = match cmd {
            "groups.create" => {
                let gid = util::new_id();
                let ids = ids_of(a);
                if let Some(w) = app.win_mut(wid) {
                    w.groups.push(TabGroup {
                        id: gid.clone(),
                        name: a["name"].as_str().unwrap_or("").chars().take(40).collect(),
                        color: a["color"].as_str().unwrap_or("blue").to_string(),
                        collapsed: false,
                        auto: false,
                    });
                    for t in w.tabs.iter_mut().filter(|t| ids.contains(&t.id)) {
                        t.group = gid.clone();
                    }
                    gather_group(w, &gid);
                    w.session_dirty = true;
                }
                Reply::Now(json!(gid))
            }
            "groups.update" => {
                let gid = a["id"].as_str().unwrap_or("");
                if let Some(w) = app.win_mut(wid) {
                    let active_group = w.active_tab().map(|t| t.group.clone()).unwrap_or_default();
                    if let Some(g) = w.groups.iter_mut().find(|g| g.id == gid) {
                        if let Some(n) = a["name"].as_str() {
                            g.name = n.chars().take(40).collect();
                            g.auto = false;
                        }
                        if let Some(c) = a["color"].as_str() {
                            g.color = c.to_string();
                            g.auto = false;
                        }
                        if let Some(c) = a["collapsed"].as_bool() {
                            g.collapsed = c;
                        }
                    }
                    w.session_dirty = true;
                    // collapsing the group that holds the active tab moves focus out of it
                    if a["collapsed"].as_bool() == Some(true) && active_group == gid {
                        let ws = w.workspace.clone();
                        if let Some(n) = w.tabs.iter().filter(|t| t.group != gid && t.workspace == ws && !t.prerender).max_by_key(|t| t.last_active).map(|t| t.id) {
                            app.activate_tab(wid, n);
                        }
                    }
                }
                ok()
            }
            "groups.assign" => {
                let gid = a["group"].as_str().unwrap_or("").to_string();
                let ids = ids_of(a);
                if let Some(w) = app.win_mut(wid) {
                    for t in w.tabs.iter_mut().filter(|t| ids.contains(&t.id)) {
                        t.group = gid.clone();
                    }
                    if !gid.is_empty() {
                        gather_group(w, &gid);
                    }
                    w.session_dirty = true;
                }
                prune_groups(app, wid);
                ok()
            }
            "groups.remove" => {
                let gid = a["id"].as_str().unwrap_or("");
                if let Some(w) = app.win_mut(wid) {
                    for t in w.tabs.iter_mut().filter(|t| t.group == gid) {
                        t.group.clear();
                    }
                    w.groups.retain(|g| g.id != gid);
                    w.session_dirty = true;
                }
                ok()
            }
            _ => Reply::Err("unknown groups command".into()),
        };
        bridge::push_tabs(app, wid);
        return r;
    }
    if ns == "workspaces" {
        let r = match cmd {
            "workspaces.list" => {
                let Some(db) = app.profile(&profile).db() else { return Reply::Now(json!([{"id": "", "name": "Personal", "color": ""}])) };
                return Reply::Now(json!(workspaces_list(db)));
            }
            "workspaces.create" => {
                if incognito {
                    return Reply::Err("Workspaces are not available in Incognito".into());
                }
                let name: String = a["name"].as_str().unwrap_or("").trim().chars().take(30).collect();
                if name.is_empty() {
                    return Reply::Err("Workspace needs a name".into());
                }
                let id = util::new_id()[..8].to_string();
                let Some(db) = app.profile(&profile).db() else { return Reply::Err("unavailable".into()) };
                let now = now_ms();
                let pos: f64 = db.conn.query_row("SELECT COALESCE(MAX(pos),0)+1 FROM workspaces", [], |r| r.get(0)).unwrap_or(1.0);
                db.conn
                    .execute(
                        "INSERT INTO workspaces(id,name,icon,color,pos,created,updated) VALUES(?1,?2,?3,?4,?5,?6,?6)",
                        params![id, name, a["icon"].as_str().unwrap_or(""), a["color"].as_str().unwrap_or(""), pos, now],
                    )
                    .map_err(|e| e.to_string())
                    .ok();
                let ids = ids_of(&json!({"ids": a["moveTabs"]}));
                if !ids.is_empty() {
                    move_tabs_to_workspace(app, ctx.wid, &ids, &id);
                }
                Reply::Now(json!(id))
            }
            "workspaces.update" => {
                let Some(db) = app.profile(&profile).db() else { return Reply::Err("unavailable".into()) };
                let id = a["id"].as_str().unwrap_or("");
                if let Some(n) = a["name"].as_str().map(|s| s.trim()).filter(|s| !s.is_empty()) {
                    let _ = db.conn.execute("UPDATE workspaces SET name=?2, updated=?3 WHERE id=?1", params![id, n, now_ms()]);
                }
                if let Some(c) = a["color"].as_str() {
                    let _ = db.conn.execute("UPDATE workspaces SET color=?2, updated=?3 WHERE id=?1", params![id, c, now_ms()]);
                }
                if let Some(p) = a["pos"].as_f64() {
                    let _ = db.conn.execute("UPDATE workspaces SET pos=?2, updated=?3 WHERE id=?1", params![id, p, now_ms()]);
                }
                ok()
            }
            "workspaces.delete" => {
                let id = a["id"].as_str().unwrap_or("").to_string();
                if id.is_empty() {
                    return Reply::Err("The default workspace can't be deleted".into());
                }
                if let Some(db) = app.profile(&profile).db() {
                    let _ = db.conn.execute("UPDATE workspaces SET deleted=1, updated=?2 WHERE id=?1", params![id, now_ms()]);
                    db.tombstone("workspace", &id);
                }
                // tabs of a deleted workspace move to the default one rather than closing
                let wins: Vec<u32> = app.windows.iter().filter(|w| w.profile == profile).map(|w| w.id).collect();
                for w in wins {
                    let ids: Vec<u32> = app.win(w).map(|x| x.tabs.iter().filter(|t| t.workspace == id).map(|t| t.id).collect()).unwrap_or_default();
                    if app.win(w).map(|x| x.workspace == id).unwrap_or(false) {
                        switch_workspace(app, w, "");
                    }
                    move_tabs_to_workspace(app, w, &ids, "");
                }
                ok()
            }
            "workspaces.switch" => {
                switch_workspace(app, ctx.wid, a["id"].as_str().unwrap_or(""));
                return ok();
            }
            "workspaces.moveTabs" => {
                move_tabs_to_workspace(app, ctx.wid, &ids_of(a), a["workspace"].as_str().unwrap_or(""));
                return ok();
            }
            _ => Reply::Err("unknown workspaces command".into()),
        };
        push_workspaces(app, &profile);
        return r;
    }
    let Some(db) = app.profile(&profile).db() else { return Reply::Err("data unavailable".into()) };
    let r: Result<Value, String> = match cmd {
        "notes.forUrl" => return Reply::Now(json!(notes_for_url(db, a["url"].as_str().unwrap_or("")))),
        "notes.list" => return Reply::Now(json!(notes_list(db, a["q"].as_str().unwrap_or("")))),
        "notes.save" => {
            if incognito {
                return Reply::Err("Notes can't be saved in Incognito".into());
            }
            note_save(db, a).map(|id| json!(id))
        }
        "notes.delete" => {
            let id = a["id"].as_str().unwrap_or("");
            db.conn.execute("UPDATE notes SET deleted=1, updated=?2 WHERE id=?1", params![id, now_ms()]).map_err(|e| e.to_string()).map(|_| {
                db.tombstone("note", id);
                json!(true)
            })
        }
        "tasks.list" => return Reply::Now(json!(tasks_list(db))),
        "tasks.save" => task_save(db, a).map(|id| json!(id)),
        "tasks.delete" => {
            let id = a["id"].as_str().unwrap_or("");
            db.conn.execute("UPDATE tasks SET deleted=1, updated=?2 WHERE id=?1", params![id, now_ms()]).map_err(|e| e.to_string()).map(|_| {
                db.tombstone("task", id);
                json!(true)
            })
        }
        "reminders.list" => return Reply::Now(json!(reminders_list(db))),
        "reminders.save" => reminder_save(db, a).map(|id| json!(id)),
        "reminders.delete" => {
            let id = a["id"].as_str().unwrap_or("");
            db.conn.execute("UPDATE reminders SET deleted=1, updated=?2 WHERE id=?1", params![id, now_ms()]).map_err(|e| e.to_string()).map(|_| {
                db.tombstone("reminder", id);
                json!(true)
            })
        }
        _ => Err(format!("unknown command {cmd}")),
    };
    let r = match r {
        Ok(v) => v,
        Err(e) => return Reply::Err(e),
    };
    let ev = if ns == "notes" { "notesChanged" } else { "tasksChanged" };
    let ids: Vec<u32> = app.windows.iter().filter(|w| w.profile == profile).map(|w| w.id).collect();
    for w in ids {
        bridge::emit(app, w, ev, json!({}));
        if ns == "notes" {
            let active = app.win(w).map(|x| x.active).unwrap_or(0);
            crate::hooks::page_info(app, w, active);
        }
    }
    bridge::emit_pages(app, &profile, ev, json!({}));
    Reply::Now(r)
}

#[allow(dead_code)]
fn exists(db: &Store, table: &str, id: &str) -> bool {
    db.conn
        .query_row(&format!("SELECT 1 FROM {table} WHERE id=?1"), [id], |_| Ok(()))
        .optional()
        .ok()
        .flatten()
        .is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn store() -> Store {
        Store::open(&std::env::temp_dir().join(format!("solus-prod-{}", util::new_id()))).unwrap()
    }
    #[test]
    fn notes_attach_to_pages_and_sites() {
        let db = store();
        note_save(&db, &json!({"scope": "page", "url": "https://ex.com/a#x", "body": "page note"})).unwrap();
        note_save(&db, &json!({"scope": "site", "url": "https://www.ex.com/b", "body": "site note"})).unwrap();
        assert_eq!(note_count_for(&db, "https://ex.com/a"), 2);
        assert_eq!(note_count_for(&db, "https://ex.com/c"), 1);
        let raw: Vec<u8> = db.conn.query_row("SELECT body FROM notes LIMIT 1", [], |r| r.get(0)).unwrap();
        assert!(!String::from_utf8_lossy(&raw).contains("note"), "note bodies are encrypted");
        assert_eq!(notes_list(&db, "site").len(), 1);
    }
    #[test]
    fn tasks_and_reminders() {
        let db = store();
        let id = task_save(&db, &json!({"title": "Write essay", "due": 5})).unwrap();
        task_save(&db, &json!({"id": id, "done": true})).unwrap();
        assert_eq!(tasks_list(&db)[0]["done"], json!(true));
        assert!(task_save(&db, &json!({"title": " "})).is_err());
        reminder_save(&db, &json!({"title": "Call", "at": 10})).unwrap();
        assert_eq!(reminders_list(&db).len(), 1);
        assert_eq!(workspaces_list(&db)[0]["name"], "Personal");
    }
}
