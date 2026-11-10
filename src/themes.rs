//! Themes: built-in palettes plus user themes made in the theme editor (stored per profile).
//! A theme is a set of colour tokens for light and dark mode; the UI turns them into CSS variables.

use serde_json::{json, Value};

use crate::app::App;
use crate::util;

fn palette(accent: &str, light: [&str; 6], dark: [&str; 6]) -> Value {
    // [frame, toolbar, tabActive, text, muted, field]
    let mk = |p: [&str; 6]| {
        json!({
            "frame": p[0], "toolbar": p[1], "tabActive": p[2], "text": p[3], "muted": p[4], "field": p[5]
        })
    };
    json!({ "accent": accent, "light": mk(light), "dark": mk(dark) })
}

pub fn builtin() -> Vec<Value> {
    let t = |id: &str, name: &str, p: Value| {
        let mut v = p;
        v["id"] = json!(id);
        v["name"] = json!(name);
        v["builtin"] = json!(true);
        v
    };
    vec![
        t("default", "Solus", palette("#e8590c",
            ["#e8ebf0", "#ffffff", "#ffffff", "#1f2328", "#5f6670", "#eef1f5"],
            ["#1f2023", "#2b2c30", "#2b2c30", "#e8e9ed", "#a3a7b0", "#1c1d20"])),
        t("ocean", "Ocean", palette("#1c7ed6",
            ["#dde8f5", "#f8fbff", "#f8fbff", "#13283f", "#4f6378", "#e9f1fa"],
            ["#16202c", "#1e2b3a", "#1e2b3a", "#e3ecf6", "#9fb1c4", "#131b25"])),
        t("forest", "Forest", palette("#2f9e44",
            ["#e1ece3", "#fbfdfb", "#fbfdfb", "#17291b", "#566b5a", "#ecf3ed"],
            ["#18211a", "#212d23", "#212d23", "#e2ede4", "#a0b3a4", "#141b15"])),
        t("graphite", "Graphite", palette("#495057",
            ["#e9eaec", "#fafafa", "#fafafa", "#1e1f21", "#62666b", "#f0f1f2"],
            ["#191a1c", "#242527", "#242527", "#e6e6e7", "#a1a3a6", "#151617"])),
        t("lavender", "Lavender", palette("#7048e8",
            ["#ebe7f6", "#fcfbff", "#fcfbff", "#241c3b", "#655c7d", "#f3f0fb"],
            ["#1d1a26", "#282433", "#282433", "#ebe7f6", "#aaa3bf", "#17151f"])),
        t("rose", "Rosé", palette("#d6336c",
            ["#f6e6ec", "#fffbfc", "#fffbfc", "#3b1825", "#7a5966", "#fbf0f4"],
            ["#241a1e", "#30232a", "#30232a", "#f5e6ec", "#bba3ad", "#1d1518"])),
    ]
}

fn path(app: &mut App, profile: &str) -> std::path::PathBuf {
    app.profile(profile).dir.join("themes.json")
}

pub fn custom(app: &mut App, profile: &str) -> Vec<Value> {
    std::fs::read(path(app, profile))
        .ok()
        .and_then(|b| serde_json::from_slice::<Vec<Value>>(&b).ok())
        .unwrap_or_default()
}

pub fn list(app: &mut App, profile: &str) -> Vec<Value> {
    let mut v = builtin();
    v.extend(custom(app, profile));
    v
}

fn valid_color(s: &str) -> bool {
    let s = s.trim();
    s.len() == 7 && s.starts_with('#') && s[1..].chars().all(|c| c.is_ascii_hexdigit())
}

/// Save (create or update) a user theme. Returns its id.
pub fn save(app: &mut App, profile: &str, mut theme: Value) -> Result<String, String> {
    let name = theme["name"].as_str().unwrap_or("").trim().to_string();
    if name.is_empty() || name.len() > 40 {
        return Err("Theme name must be 1-40 characters".into());
    }
    if !valid_color(theme["accent"].as_str().unwrap_or("")) {
        return Err("Accent must be a #rrggbb colour".into());
    }
    for mode in ["light", "dark"] {
        for k in ["frame", "toolbar", "tabActive", "text", "muted", "field"] {
            if !valid_color(theme[mode][k].as_str().unwrap_or("")) {
                return Err(format!("{mode}.{k} must be a #rrggbb colour"));
            }
        }
    }
    let mut list = custom(app, profile);
    let id = match theme["id"].as_str() {
        Some(id) if !id.is_empty() && list.iter().any(|t| t["id"] == id) => id.to_string(),
        _ => format!("custom-{}", &util::new_id()[..8]),
    };
    theme["id"] = json!(id);
    theme["builtin"] = json!(false);
    theme["name"] = json!(name);
    list.retain(|t| t["id"] != json!(id));
    list.push(theme);
    let p = path(app, profile);
    util::write_atomic(&p, &serde_json::to_vec_pretty(&list).unwrap()).map_err(|e| e.to_string())?;
    Ok(id)
}

pub fn delete(app: &mut App, profile: &str, id: &str) -> Result<(), String> {
    let mut list = custom(app, profile);
    list.retain(|t| t["id"] != json!(id));
    let p = path(app, profile);
    util::write_atomic(&p, &serde_json::to_vec_pretty(&list).unwrap()).map_err(|e| e.to_string())
}
