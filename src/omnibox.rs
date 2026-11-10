//! Omnibox suggestions: typed URL, open tabs, bookmarks, history and search-engine suggestions.
//! Local results are returned immediately; engine suggestions follow as a `suggestMore` event.

use serde_json::{json, Value};

use crate::app::App;
use crate::bridge::Ctx;

pub fn suggest(app: &mut App, ctx: Ctx, id: Value, args: &Value) {
    let q = args["q"].as_str().unwrap_or("").trim().to_string();
    let Some(w) = app.win(ctx.wid) else { return };
    let profile = w.profile.clone();
    let incognito = w.incognito;
    let settings = app.settings(&profile).clone();
    let mut out: Vec<Value> = vec![];
    if q.is_empty() {
        crate::bridge::respond(app, &ctx, &id, Ok(json!({"items": [], "inline": null})));
        return;
    }
    let ql = q.to_lowercase();
    let page = !ctx.is_ui();

    // 1. what Enter would do
    let target = crate::tab::fixup_with(&settings, &q);
    let is_search = target == settings.search_url(&q);
    if is_search {
        out.push(json!({"type": "search", "text": q, "url": target}));
    } else {
        out.push(json!({"type": "url", "text": target, "url": target}));
    }

    // 2. open tabs (switch to tab) — browser UI only
    if !page {
        let mut tabs: Vec<Value> = vec![];
        for win in app.windows.iter().filter(|x| x.profile == profile && x.incognito == incognito) {
            for t in &win.tabs {
                if t.id == win.active && win.id == ctx.wid {
                    continue;
                }
                if t.title.to_lowercase().contains(&ql) || t.url.to_lowercase().contains(&ql) {
                    tabs.push(json!({"type": "tab", "text": t.title, "url": t.url, "tab": t.id, "win": win.id, "icon": t.favicon}));
                }
            }
        }
        out.extend(tabs.into_iter().take(3));
    }

    // 3. bookmarks + history (local, instant)
    let local = crate::hooks::local_matches(app, &profile, &q, incognito);
    let mut seen: std::collections::HashSet<String> = out.iter().filter_map(|v| v["url"].as_str().map(|s| s.to_string())).collect();
    let mut inline: Option<String> = None;
    for m in local {
        let u = m["url"].as_str().unwrap_or("").to_string();
        if seen.insert(u.clone()) {
            if inline.is_none() && !q.contains(' ') && m["type"] == "history" {
                // inline autocomplete when the typed text is a prefix of a visited host
                let bare = u.trim_start_matches("https://").trim_start_matches("http://").trim_start_matches("www.");
                if bare.to_lowercase().starts_with(&ql) {
                    let host_part = bare.split('/').next().unwrap_or(bare);
                    inline = Some(if host_part.len() >= q.len() { host_part.to_string() } else { bare.to_string() });
                }
            }
            out.push(m);
        }
        if out.len() >= 8 {
            break;
        }
    }

    let first = json!({"items": out.clone(), "inline": inline});
    crate::bridge::respond(app, &ctx, &id, Ok(first));
    if !settings.search_suggestions || q.len() > 200 || q.starts_with("solus:") {
        return;
    }
    // 4. engine suggestions on a worker thread
    let surl = settings.suggest_url(&q);
    std::thread::spawn(move || {
        let sugg = crate::net::suggestions(&surl);
        if sugg.is_empty() {
            return;
        }
        let mut items = out;
        let have: std::collections::HashSet<String> =
            items.iter().filter_map(|v| v["text"].as_str().map(|s| s.to_lowercase())).collect();
        let mut added = 0;
        for s in sugg {
            if have.contains(&s.to_lowercase()) || added >= 5 {
                continue;
            }
            let url = crate::tab::fixup_with(&settings, &s);
            let is_url = url != settings.search_url(&s);
            items.push(json!({"type": if is_url {"url"} else {"suggest"}, "text": s, "url": url}));
            added += 1;
        }
        let data = json!({"q": q, "items": items});
        crate::app::post(move |app| {
            if ctx.is_ui() {
                crate::bridge::emit(app, ctx.wid, "suggestMore", data);
            } else {
                crate::bridge::emit_page(app, ctx.wid, ctx.tid, "suggestMore", data);
            }
        });
    });
}
