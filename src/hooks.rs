//! Glue between tab events and feature modules, plus command routing for feature namespaces.

use std::cell::RefCell;
use std::collections::HashSet;

use serde_json::{json, Value};
use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::*;
use windows::core::Interface;

use crate::app::{defer, App};
use crate::bridge::{self, ok, Ctx, Reply};
use crate::util::{self, WStr};

pub fn profile_loaded(app: &mut App, id: &str) {
    let s = app.settings(id).clone();
    crate::scheme::set_https_first(s.https_first);
    crate::tab::SHORTCUTS.with(|m| *m.borrow_mut() = crate::tab::build_shortcuts(&s.shortcuts));
    crate::downloads::load_history(app, id);
    crate::privacy::reconfigure(app);
}

/// After a navigation completes: record history, refresh page indicators, detect duplicates,
/// auto-group tabs.
pub fn page_loaded(app: &mut App, wid: u32, tid: u32, ok: bool) {
    let Some(w) = app.win(wid) else { return };
    let incognito = w.incognito;
    let profile = w.profile.clone();
    let Some(t) = w.tab(tid) else { return };
    let (url, title, transition) = (t.url.clone(), t.title.clone(), t.transition.clone());
    if ok && !incognito {
        if let Some(db) = app.profile(&profile).db() {
            db.add_visit(&url, &title, &transition);
        }
    }
    if let Some(t) = app.win_mut(wid).and_then(|w| w.tab_mut(tid)) {
        t.transition = "link".into();
    }
    page_info(app, wid, tid);
    let settings = app.settings(&profile).clone();
    if settings.duplicate_detection && (url.starts_with("http://") || url.starts_with("https://")) {
        let norm = |u: &str| u.split('#').next().unwrap_or("").trim_end_matches('/').to_string();
        let me = norm(&url);
        if let Some(other) = app
            .win(wid)
            .and_then(|w| w.tabs.iter().find(|x| x.id != tid && norm(&x.url) == me).map(|x| x.id))
        {
            bridge::emit(app, wid, "duplicate", json!({"tab": tid, "other": other}));
        }
    }
    if settings.auto_group_tabs {
        auto_group(app, wid, settings.auto_group_threshold.max(2) as usize);
    }
    crate::privacy::page_loaded(app, wid, tid);
}

pub fn favicon_changed(app: &mut App, wid: u32, tid: u32) {
    let Some(w) = app.win(wid) else { return };
    if w.incognito {
        return;
    }
    let profile = w.profile.clone();
    let Some(t) = w.tab(tid) else { return };
    let (url, fav) = (t.url.clone(), t.favicon.clone());
    if fav.is_empty() || !url.starts_with("http") {
        return;
    }
    if let Some(db) = app.profile(&profile).db() {
        db.favicon_set(&util::host_of(&url), &fav);
    }
}

pub fn title_changed(app: &mut App, wid: u32, tid: u32) {
    let Some(w) = app.win(wid) else { return };
    if w.incognito {
        return;
    }
    let profile = w.profile.clone();
    let Some(t) = w.tab(tid) else { return };
    let (url, title) = (t.url.clone(), t.title.clone());
    if let Some(db) = app.profile(&profile).db() {
        db.update_title(&url, &title);
    }
}

/// Per-page indicators shown in the omnibox: bookmarked, notes, saved passwords.
pub fn page_info(app: &mut App, wid: u32, tid: u32) {
    let Some(w) = app.win(wid) else { return };
    let profile = w.profile.clone();
    let Some(t) = w.tab(tid) else { return };
    let url = t.url.clone();
    let p = app.profile(&profile);
    let bookmark = p.db().and_then(|d| d.bm_find_url(&url)).map(|b| b["id"].clone()).unwrap_or(Value::Null);
    let notes = p.db().map(|d| crate::productivity::note_count_for(d, &url)).unwrap_or(0);
    let passwords = p.db().map(|d| crate::passwords::count_for(d, &url)).unwrap_or(0);
    bridge::emit(app, wid, "pageInfo", json!({"tab": tid, "url": url, "bookmark": bookmark, "notes": notes, "passwords": passwords}));
}

fn group_color(site: &str) -> &'static str {
    const C: [&str; 8] = ["blue", "red", "yellow", "green", "pink", "purple", "cyan", "orange"];
    let h: usize = site.bytes().map(|b| b as usize).sum();
    C[h % C.len()]
}

/// Group ungrouped tabs that share a site once there are `threshold` of them.
pub fn auto_group(app: &mut App, wid: u32, threshold: usize) {
    let Some(w) = app.win_mut(wid) else { return };
    let ws = w.workspace.clone();
    let mut counts: std::collections::HashMap<String, Vec<u32>> = Default::default();
    for t in w.tabs.iter().filter(|t| t.workspace == ws && t.group.is_empty() && !t.pinned) {
        if t.url.starts_with("http") {
            counts.entry(util::site_of(&t.url)).or_default().push(t.id);
        }
    }
    let mut changed = false;
    for (site, ids) in counts {
        if ids.len() < threshold || site.is_empty() {
            continue;
        }
        let gid = match w.groups.iter().find(|g| g.auto && g.name == display_site(&site)) {
            Some(g) => g.id.clone(),
            None => {
                let g = crate::window::TabGroup {
                    id: util::new_id(),
                    name: display_site(&site),
                    color: group_color(&site).into(),
                    collapsed: false,
                    auto: true,
                };
                let id = g.id.clone();
                w.groups.push(g);
                id
            }
        };
        for t in w.tabs.iter_mut().filter(|t| ids.contains(&t.id)) {
            t.group = gid.clone();
        }
        // keep a group's tabs together, positioned where its first tab was
        let first = w.tabs.iter().position(|t| t.group == gid).unwrap_or(0);
        let (mut grouped, rest): (Vec<_>, Vec<_>) = w.tabs.drain(..).partition(|t| t.group == gid);
        let mut rest = rest;
        let at = first.min(rest.len());
        let tail = rest.split_off(at);
        rest.append(&mut grouped);
        rest.extend(tail);
        w.tabs = rest;
        changed = true;
    }
    if changed {
        w.session_dirty = true;
        bridge::push_tabs(app, wid);
    }
}

fn display_site(site: &str) -> String {
    let base = site.split('.').next().unwrap_or(site);
    let mut c = base.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => site.to_string(),
    }
}

/// Bookmarks + history matches for the omnibox.
pub fn local_matches(app: &mut App, profile: &str, q: &str, incognito: bool) -> Vec<Value> {
    let _ = incognito; // incognito windows still see (but never write) the profile's history
    let Some(db) = app.profile(profile).db() else { return vec![] };
    let mut out = vec![];
    for b in db.bm_search(q, 3) {
        out.push(json!({"type": "bookmark", "text": b["title"], "url": b["url"]}));
    }
    for (url, title, _) in db.history_matches(q, 6) {
        out.push(json!({"type": "history", "text": title, "url": url}));
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Find in page (WebView2 Find API, our own find bar)
// ---------------------------------------------------------------------------------------------

thread_local! {
    static FIND_HOOKED: RefCell<HashSet<usize>> = RefCell::new(HashSet::new());
}

fn emit_find_result(find: &ICoreWebView2Find, wid: u32) {
    let mut count = 0;
    let mut idx = -1;
    unsafe {
        let _ = find.MatchCount(&mut count);
        let _ = find.ActiveMatchIndex(&mut idx);
    }
    defer(move |app| bridge::emit(app, wid, "findResult", json!({"count": count, "index": idx})));
}

pub fn find(app: &mut App, wid: u32, args: &Value) {
    let Some(w) = app.win(wid) else { return };
    let Some(t) = w.active_tab() else { return };
    let Some(v) = &t.view else { return };
    let Some(env) = app.env() else { return };
    let action = args["action"].as_str().unwrap_or("start");
    unsafe {
        let Ok(w28) = v.wv.cast::<ICoreWebView2_28>() else { return };
        let Ok(find) = w28.Find() else { return };
        let key = find.as_raw() as usize;
        if FIND_HOOKED.with(|s| s.borrow_mut().insert(key)) {
            let f2 = find.clone();
            let h = FindMatchCountChangedEventHandler::create(Box::new(move |_, _| {
                emit_find_result(&f2, wid);
                Ok(())
            }));
            let mut tok = Default::default();
            let _ = find.add_MatchCountChanged(&h, &mut tok);
            let f3 = find.clone();
            let h = FindActiveMatchIndexChangedEventHandler::create(Box::new(move |_, _| {
                emit_find_result(&f3, wid);
                Ok(())
            }));
            let _ = find.add_ActiveMatchIndexChanged(&h, &mut tok);
        }
        match action {
            "start" => {
                let text = args["text"].as_str().unwrap_or("");
                if text.is_empty() {
                    let _ = find.Stop();
                    bridge::emit(app, wid, "findResult", json!({"count": 0, "index": -1}));
                    return;
                }
                let Ok(env15) = env.cast::<ICoreWebView2Environment15>() else { return };
                let Ok(opts) = env15.CreateFindOptions() else { return };
                let term = WStr::new(text);
                let _ = opts.SetFindTerm(term.pcwstr());
                let _ = opts.SetIsCaseSensitive(args["case"].as_bool().unwrap_or(false));
                let _ = opts.SetShouldMatchWord(args["word"].as_bool().unwrap_or(false));
                let _ = opts.SetShouldHighlightAllMatches(true);
                let _ = opts.SetSuppressDefaultFindDialog(true);
                let f2 = find.clone();
                let h = FindStartCompletedHandler::create(Box::new(move |_| {
                    emit_find_result(&f2, wid);
                    Ok(())
                }));
                let _ = find.Start(&opts, &h);
            }
            "next" => {
                let _ = find.FindNext();
            }
            "prev" => {
                let _ = find.FindPrevious();
            }
            _ => {
                let _ = find.Stop();
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Settings side effects
// ---------------------------------------------------------------------------------------------

pub fn setting_changed(app: &mut App, profile: &str, key: &str, restart: &mut bool) {
    let s = app.settings(profile).clone();
    let root = key.split('.').next().unwrap_or("");
    match root {
        "theme" | "density" | "theme_id" | "custom_css" | "animations" => {
            let ids: Vec<u32> = app.windows.iter().filter(|w| w.profile == profile).map(|w| w.id).collect();
            for id in ids {
                bridge::push_theme(app, id);
            }
            apply_profile_to_engine(app, profile);
        }
        "tracking_prevention" | "autofill_forms" | "smartscreen" | "devtools" => apply_profile_to_engine(app, profile),
        "https_first" => crate::scheme::set_https_first(s.https_first),
        "shortcuts" => crate::tab::SHORTCUTS.with(|m| *m.borrow_mut() = crate::tab::build_shortcuts(&s.shortcuts)),
        "blocking_level" | "site_exceptions" => crate::privacy::reconfigure(app),
        "block_third_party_cookies" | "do_not_track" => {
            // Chromium reads these preferences at start-up; apply at next engine start
            *restart = true;
        }
        _ => {}
    }
}

/// App-wide (local state) setting changed.
pub fn local_changed(app: &mut App, key: &str) {
    if key == "perf_profile" || key == "auto_battery" || key == "rules" || key == "custom_rules" {
        crate::perf::tick(app);
        let ids: Vec<u32> = app.windows.iter().map(|w| w.id).collect();
        let p = app.local.perf_profile.clone();
        for id in ids {
            bridge::emit(app, id, "perf", json!({"profile": p}));
        }
    }
}

fn apply_profile_to_engine(app: &mut App, profile: &str) {
    let s = app.settings(profile).clone();
    for w in app.windows.iter().filter(|w| w.profile == profile) {
        for t in &w.tabs {
            if let Some(v) = &t.view {
                crate::webview::configure_content(&v.wv, &s);
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Command routing for feature namespaces
// ---------------------------------------------------------------------------------------------

fn profile_of(app: &App, ctx: &Ctx) -> (String, bool) {
    app.win(ctx.wid).map(|w| (w.profile.clone(), w.incognito)).unwrap_or_default()
}

pub fn bookmarks_changed(app: &mut App, profile: &str) {
    let ids: Vec<u32> = app.windows.iter().filter(|w| w.profile == profile).map(|w| w.id).collect();
    for id in ids {
        bridge::emit(app, id, "bookmarksChanged", json!({}));
        let active = app.win(id).map(|w| w.active).unwrap_or(0);
        page_info(app, id, active);
    }
    bridge::emit_pages(app, profile, "bookmarksChanged", json!({}));
}

pub fn dispatch(app: &mut App, ctx: &Ctx, cmd: &str, args: &Value, id: &Value) -> Reply {
    let (profile, incognito) = profile_of(app, ctx);
    let ns = cmd.split('.').next().unwrap_or("");
    match ns {
        "history" => history_cmd(app, &profile, incognito, cmd, args),
        "bookmarks" => bookmarks_cmd(app, &profile, cmd, args),
        "themes" => match cmd {
            "themes.list" => Reply::Now(json!(crate::themes::list(app, &profile))),
            "themes.save" => match crate::themes::save(app, &profile, args["theme"].clone()) {
                Ok(id) => {
                    let ids: Vec<u32> = app.windows.iter().filter(|w| w.profile == profile).map(|w| w.id).collect();
                    for w in ids {
                        bridge::push_theme(app, w);
                    }
                    Reply::Now(json!(id))
                }
                Err(e) => Reply::Err(e),
            },
            "themes.delete" => {
                let id = args["id"].as_str().unwrap_or("");
                if let Err(e) = crate::themes::delete(app, &profile, id) {
                    return Reply::Err(e);
                }
                if app.settings(&profile).theme_id == id {
                    let _ = bridge::set_setting(app, &profile, "theme_id", json!("default"));
                }
                ok()
            }
            _ => Reply::Err("unknown theme command".into()),
        },
        "backup" => match cmd {
            "backup.list" => Reply::Now(json!(crate::backup::list(&profile))),
            "backup.create" => match crate::backup::backup_profile(app, &profile, "manual") {
                Ok(_) => Reply::Now(json!(crate::backup::list(&profile))),
                Err(e) => Reply::Err(e),
            },
            "backup.restore" => match crate::backup::schedule_restore(&profile, args["name"].as_str().unwrap_or("")) {
                Ok(()) => Reply::Now(json!({"restartRequired": true})),
                Err(e) => Reply::Err(e),
            },
            "backup.openFolder" => {
                let dir = crate::backup::backups_dir(&profile);
                let _ = std::fs::create_dir_all(&dir);
                crate::platform::open_path(&dir);
                ok()
            }
            _ => Reply::Err("unknown backup command".into()),
        },
        "app" => match cmd {
            "app.restart" => {
                crate::platform::restart(app);
                ok()
            }
            "app.quit" => {
                app.quit();
                ok()
            }
            "app.local" => Reply::Now(serde_json::to_value(&app.local).unwrap_or_default()),
            "app.setLocal" => {
                let key = args["key"].as_str().unwrap_or("");
                match crate::platform::set_local(app, key, args["value"].clone()) {
                    Ok(v) => Reply::Now(v),
                    Err(e) => Reply::Err(e),
                }
            }
            "app.onboardingDone" => {
                app.local.onboarding_done = true;
                app.local_dirty = true;
                ok()
            }
            "app.makeDefault" => {
                crate::platform::open_default_apps();
                ok()
            }
            "app.pickFolder" => match crate::platform::pick_folder(app, ctx.wid, args["title"].as_str().unwrap_or("Choose a folder")) {
                Some(p) => Reply::Now(json!(p)),
                None => Reply::Now(Value::Null),
            },
            "app.pickFile" => match crate::platform::pick_file(app, ctx.wid, args["filter"].as_str().unwrap_or("")) {
                Some(p) => Reply::Now(json!(p)),
                None => Reply::Now(Value::Null),
            },
            "app.clearData" => crate::privacy::clear_data(app, ctx.clone(), id.clone(), args),
            "app.openPath" => {
                crate::platform::open_path(std::path::Path::new(args["path"].as_str().unwrap_or("")));
                ok()
            }
            _ => Reply::Err("unknown app command".into()),
        },
        "profiles" => crate::platform::profiles_cmd(app, ctx, cmd, args),
        "favicons" => {
            let Some(db) = app.profile(&profile).db() else { return Reply::Now(json!({})) };
            let mut out = serde_json::Map::new();
            for u in args["urls"].as_array().cloned().unwrap_or_default() {
                if let Some(u) = u.as_str() {
                    if let Some(i) = db.favicon_for_url(u) {
                        out.insert(u.to_string(), json!(i));
                    }
                }
            }
            Reply::Now(Value::Object(out))
        }
        "perf" => crate::perf::cmd(app, ctx, cmd, args, id),
        "downloads" => crate::downloads::cmd(app, ctx, cmd, args),
        "notes" | "tasks" | "reminders" | "workspaces" | "groups" => crate::productivity::cmd(app, ctx, cmd, args),
        "privacy" | "permissions" | "sites" => crate::privacy::cmd(app, ctx, cmd, args, id),
        "passwords" => crate::passwords::cmd(app, ctx, cmd, args, id),
        "import" => crate::import::cmd(app, ctx, cmd, args, id),
        "ai" => crate::ai::cmd(app, ctx, cmd, args, id),
        "sync" => crate::sync::cmd(app, ctx, cmd, args, id),
        "ext" => crate::extensions::cmd(app, ctx, cmd, args, id),
        "translate" => crate::translate::cmd(app, ctx, cmd, args, id),
        "update" => crate::update::cmd(app, ctx, cmd, args, id),
        _ => Reply::Err(format!("unknown command {cmd}")),
    }
}

fn history_cmd(app: &mut App, profile: &str, incognito: bool, cmd: &str, args: &Value) -> Reply {
    let Some(db) = app.profile(profile).db() else { return Reply::Err("history unavailable".into()) };
    match cmd {
        "history.query" => Reply::Now(json!(db.with_icons(db.history_query(
            args["text"].as_str().unwrap_or(""),
            args["before"].as_i64().unwrap_or(0),
            args["limit"].as_u64().unwrap_or(100).min(1000) as usize
        )))),
        "history.mostVisited" => Reply::Now(json!(db.most_visited(args["limit"].as_u64().unwrap_or(8) as usize))),
        "history.deleteVisits" => {
            let ids: Vec<i64> = args["ids"].as_array().map(|a| a.iter().filter_map(|x| x.as_i64()).collect()).unwrap_or_default();
            db.delete_visits(&ids);
            ok()
        }
        "history.deleteUrl" => {
            db.delete_url(args["url"].as_str().unwrap_or(""));
            ok()
        }
        "history.clear" => {
            if incognito {
                return ok();
            }
            let n = db.clear_history(args["from"].as_i64().unwrap_or(0), args["to"].as_i64().unwrap_or(i64::MAX));
            Reply::Now(json!(n))
        }
        "history.count" => Reply::Now(json!(db.history_count())),
        _ => Reply::Err("unknown history command".into()),
    }
}

fn bookmarks_cmd(app: &mut App, profile: &str, cmd: &str, args: &Value) -> Reply {
    let Some(db) = app.profile(profile).db() else { return Reply::Err("bookmarks unavailable".into()) };
    let s = |k: &str| args[k].as_str().unwrap_or("").to_string();
    let r = match cmd {
        "bookmarks.children" => return Reply::Now(json!(db.with_icons(db.bm_children(&s("parent"))))),
        "bookmarks.tree" => {
            let p = if s("parent").is_empty() { "root".to_string() } else { s("parent") };
            return Reply::Now(json!(db.bm_tree(&p, args["depth"].as_u64().unwrap_or(8) as usize)));
        }
        "bookmarks.search" => return Reply::Now(json!(db.with_icons(db.bm_search(&s("q"), 100)))),
        "bookmarks.forUrl" => return Reply::Now(db.bm_find_url(&s("url")).unwrap_or(Value::Null)),
        "bookmarks.get" => return Reply::Now(db.bm_get(&s("id")).unwrap_or(Value::Null)),
        "bookmarks.add" => {
            let kind = if s("kind").is_empty() { "url".to_string() } else { s("kind") };
            db.bm_add(&s("parent"), &kind, &s("title"), &s("url"), args["index"].as_u64().map(|x| x as usize)).map(|id| json!(id))
        }
        "bookmarks.toggle" => match db.bm_find_url(&s("url")) {
            Some(b) => db.bm_delete(b["id"].as_str().unwrap_or("")).map(|_| Value::Null),
            None => {
                let title = if s("title").is_empty() { s("url") } else { s("title") };
                db.bm_add("bar", "url", &title, &s("url"), None).map(|id| json!(id))
            }
        },
        "bookmarks.update" => {
            db.bm_update(&s("id"), args["title"].as_str(), args["url"].as_str()).map(|_| json!(true))
        }
        "bookmarks.move" => db.bm_move(&s("id"), &s("parent"), args["index"].as_u64().unwrap_or(0) as usize).map(|_| json!(true)),
        "bookmarks.delete" => db.bm_delete(&s("id")).map(|_| json!(true)),
        "bookmarks.openAll" => {
            let kids = db.bm_children(&s("id"));
            let urls: Vec<String> = kids.iter().filter_map(|k| k["url"].as_str().filter(|u| !u.is_empty()).map(|u| u.to_string())).collect();
            return Reply::Now(json!(urls));
        }
        _ => return Reply::Err("unknown bookmarks command".into()),
    };
    match r {
        Ok(v) => {
            bookmarks_changed(app, profile);
            Reply::Now(v)
        }
        Err(e) => Reply::Err(e),
    }
}
