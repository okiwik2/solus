//! Page translation: detect the page language, offer to translate, and translate through Google
//! Translate's web proxy (public pages). "Always translate" languages are translated automatically.

use serde_json::{json, Value};

use crate::app::App;
use crate::bridge::{self, ok, Ctx, Reply};

pub const LANGS: &[(&str, &str)] = &[
    ("af", "Afrikaans"), ("ar", "Arabic"), ("bg", "Bulgarian"), ("bn", "Bengali"), ("ca", "Catalan"), ("cs", "Czech"),
    ("da", "Danish"), ("de", "German"), ("el", "Greek"), ("en", "English"), ("es", "Spanish"), ("et", "Estonian"),
    ("fa", "Persian"), ("fi", "Finnish"), ("fil", "Filipino"), ("fr", "French"), ("he", "Hebrew"), ("hi", "Hindi"),
    ("hr", "Croatian"), ("hu", "Hungarian"), ("id", "Indonesian"), ("it", "Italian"), ("ja", "Japanese"), ("ko", "Korean"),
    ("lt", "Lithuanian"), ("lv", "Latvian"), ("mi", "Māori"), ("ms", "Malay"), ("nl", "Dutch"), ("no", "Norwegian"),
    ("pl", "Polish"), ("pt", "Portuguese"), ("ro", "Romanian"), ("ru", "Russian"), ("sk", "Slovak"), ("sl", "Slovenian"),
    ("sm", "Samoan"), ("sr", "Serbian"), ("sv", "Swedish"), ("sw", "Swahili"), ("ta", "Tamil"), ("th", "Thai"),
    ("to", "Tongan"), ("tr", "Turkish"), ("uk", "Ukrainian"), ("ur", "Urdu"), ("vi", "Vietnamese"), ("zh", "Chinese"),
];

pub fn lang_name(code: &str) -> String {
    LANGS.iter().find(|l| l.0 == code).map(|l| l.1.to_string()).unwrap_or_else(|| code.to_uppercase())
}

/// User's Windows display language, e.g. "en".
pub fn system_lang() -> String {
    let mut buf = [0u16; 85];
    let n = unsafe { windows::Win32::Globalization::GetUserDefaultLocaleName(&mut buf) };
    let s = String::from_utf16_lossy(&buf[..(n.max(1) as usize - 1)]);
    s.split('-').next().unwrap_or("en").to_lowercase()
}

pub fn normalize(code: &str) -> String {
    let c = code.trim().to_lowercase().replace('_', "-");
    let p = c.split('-').next().unwrap_or("").to_string();
    match p.as_str() {
        "nb" | "nn" => "no".into(),
        "iw" => "he".into(),
        "in" => "id".into(),
        "tl" => "fil".into(),
        _ => p,
    }
}

/// Very small language identifier for pages without a lang attribute: script ranges first,
/// then common function words for Latin-script languages.
pub fn guess(sample: &str) -> String {
    let mut counts = [0usize; 10]; // han, kana, hangul, cyrillic, arabic, hebrew, greek, thai, devanagari, latin
    for c in sample.chars() {
        let u = c as u32;
        let i = match u {
            0x4E00..=0x9FFF => 0,
            0x3040..=0x30FF => 1,
            0xAC00..=0xD7AF => 2,
            0x0400..=0x04FF => 3,
            0x0600..=0x06FF => 4,
            0x0590..=0x05FF => 5,
            0x0370..=0x03FF => 6,
            0x0E00..=0x0E7F => 7,
            0x0900..=0x097F => 8,
            _ if c.is_ascii_alphabetic() || (0xC0..=0x17F).contains(&u) => 9,
            _ => continue,
        };
        counts[i] += 1;
    }
    let total: usize = counts.iter().sum();
    if total < 20 {
        return String::new();
    }
    if counts[1] > total / 20 {
        return "ja".into();
    }
    let non_latin = [(0, "zh"), (2, "ko"), (3, "ru"), (4, "ar"), (5, "he"), (6, "el"), (7, "th"), (8, "hi")];
    for (i, code) in non_latin {
        if counts[i] * 2 > total {
            if code == "ru" && sample.chars().any(|c| "їєґі".contains(c)) {
                return "uk".into();
            }
            return code.into();
        }
    }
    const WORDS: &[(&str, &[&str])] = &[
        ("en", &["the", "and", "of", "to", "is", "in", "that", "for", "with", "you"]),
        ("es", &["el", "la", "de", "que", "y", "en", "los", "por", "con", "para"]),
        ("fr", &["le", "la", "les", "de", "et", "des", "est", "pour", "dans", "une"]),
        ("de", &["der", "die", "und", "das", "ist", "nicht", "mit", "den", "für", "auf"]),
        ("it", &["il", "di", "che", "e", "la", "per", "non", "una", "sono", "della"]),
        ("pt", &["de", "que", "não", "uma", "para", "com", "os", "do", "da", "em"]),
        ("nl", &["de", "het", "een", "en", "van", "is", "niet", "op", "voor", "met"]),
        ("sv", &["och", "att", "det", "som", "är", "på", "för", "med", "inte", "en"]),
        ("pl", &["i", "w", "nie", "na", "się", "z", "jest", "to", "że", "do"]),
        ("tr", &["ve", "bir", "bu", "için", "ile", "da", "de", "çok", "ne", "olarak"]),
        ("id", &["dan", "yang", "di", "ini", "untuk", "dengan", "tidak", "dari", "itu", "ada"]),
        ("vi", &["và", "của", "là", "có", "các", "không", "được", "cho", "những", "một"]),
        ("mi", &["te", "ki", "ka", "ngā", "me", "i", "o", "ko", "kei", "nei"]),
    ];
    let lower = sample.to_lowercase();
    let words: Vec<&str> = lower.split(|c: char| !c.is_alphabetic()).filter(|w| !w.is_empty()).collect();
    if words.len() < 8 {
        return String::new();
    }
    let mut best = ("", 0usize);
    for (code, list) in WORDS {
        let n = words.iter().filter(|w| list.contains(w)).count();
        if n > best.1 {
            best = (code, n);
        }
    }
    if best.1 * 12 >= words.len() {
        best.0.to_string()
    } else {
        String::new()
    }
}

fn target(app: &mut App, profile: &str) -> String {
    let t = app.settings(profile).translate_target.clone();
    if t.is_empty() {
        system_lang()
    } else {
        normalize(&t)
    }
}

pub fn proxy_url(url: &str, to: &str) -> String {
    format!("https://translate.google.com/translate?sl=auto&tl={}&u={}", to, crate::settings::url_encode(url))
}

/// "www-example-com.translate.goog/path?_x_tr_sl=..." -> "https://www.example.com/path"
pub fn original_of(url: &str) -> Option<String> {
    let u = url::Url::parse(url).ok()?;
    let host = u.host_str()?.strip_suffix(".translate.goog")?;
    let real = host.replace("--", "\u{0}").replace('-', ".").replace('\u{0}', "-");
    let mut q: Vec<(String, String)> = u.query_pairs().filter(|(k, _)| !k.starts_with("_x_tr_")).map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
    let mut out = format!("https://{}{}", real, u.path());
    if !q.is_empty() {
        out.push('?');
        out.push_str(&q.drain(..).map(|(k, v)| format!("{}={}", crate::settings::url_encode(&k), crate::settings::url_encode(&v))).collect::<Vec<_>>().join("&"));
    }
    Some(out)
}

pub fn detected(app: &mut App, wid: u32, tid: u32, lang_attr: &str, sample: &str) {
    let Some(w) = app.win(wid) else { return };
    let profile = w.profile.clone();
    let Some(t) = w.tab(tid) else { return };
    let url = t.url.clone();
    if !url.starts_with("http") || url.contains(".translate.goog") {
        return;
    }
    let s = app.settings(&profile).clone();
    if !s.translate_offer {
        return;
    }
    let mut lang = normalize(lang_attr);
    // many sites declare lang="en" boilerplate; trust content when it clearly disagrees
    let g = guess(sample);
    if lang.is_empty() || (!g.is_empty() && g != lang && sample.chars().count() > 300) {
        lang = g;
    }
    if lang.is_empty() {
        return;
    }
    let to = target(app, &profile);
    if lang == to || s.translate_never.iter().any(|l| l == &lang) {
        return;
    }
    if s.translate_always.iter().any(|l| l == &lang) {
        app.navigate_tab(wid, tid, &proxy_url(&url, &to));
        return;
    }
    bridge::emit(
        app,
        wid,
        "translateOffer",
        json!({"tab": tid, "lang": lang, "langName": lang_name(&lang), "target": to, "targetName": lang_name(&to), "prompt": true}),
    );
}

pub fn cmd(app: &mut App, ctx: &Ctx, cmd: &str, a: &Value, _id: &Value) -> Reply {
    let Some(w) = app.win(ctx.wid) else { return Reply::Err("no window".into()) };
    let profile = w.profile.clone();
    let tid = a["tab"].as_u64().map(|x| x as u32).unwrap_or(w.active);
    let url = w.tab(tid).map(|t| t.url.clone()).unwrap_or_default();
    match cmd {
        "translate.page" => {
            if let Some(orig) = original_of(&url) {
                app.navigate_tab(ctx.wid, tid, &orig);
                return ok();
            }
            if !url.starts_with("http") {
                return Reply::Err("Only web pages can be translated".into());
            }
            let to = target(app, &profile);
            app.navigate_tab(ctx.wid, tid, &proxy_url(&url, &to));
            ok()
        }
        "translate.original" => match original_of(&url) {
            Some(o) => {
                app.navigate_tab(ctx.wid, tid, &o);
                ok()
            }
            None => ok(),
        },
        "translate.always" | "translate.never" => {
            let lang = normalize(a["lang"].as_str().unwrap_or(""));
            if lang.is_empty() {
                return Reply::Err("no language".into());
            }
            let s = app.settings(&profile).clone();
            let (mut always, mut never) = (s.translate_always.clone(), s.translate_never.clone());
            always.retain(|l| *l != lang);
            never.retain(|l| *l != lang);
            if cmd == "translate.always" {
                always.push(lang);
            } else {
                never.push(lang);
            }
            let _ = bridge::set_setting(app, &profile, "translate_always", json!(always));
            let _ = bridge::set_setting(app, &profile, "translate_never", json!(never));
            if cmd == "translate.always" && url.starts_with("http") && !url.contains(".translate.goog") {
                let to = target(app, &profile);
                app.navigate_tab(ctx.wid, tid, &proxy_url(&url, &to));
            }
            ok()
        }
        "translate.languages" => Reply::Now(json!({"languages": LANGS.iter().map(|l| json!({"code": l.0, "name": l.1})).collect::<Vec<_>>(), "system": system_lang()})),
        _ => Reply::Err("unknown translate command".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn language_guessing() {
        assert_eq!(guess("Der schnelle braune Fuchs springt über den faulen Hund und die Katze ist nicht mit der Maus auf dem Tisch für den Abend."), "de");
        assert_eq!(guess("Le chat est sur la table et les enfants sont dans le jardin pour une heure de jeux avec des amis."), "fr");
        assert_eq!(guess("これは日本語の文章です。ひらがなとカタカナと漢字が含まれています。とても長い文章になります。"), "ja");
        assert_eq!(guess("Это русский текст, который должен быть распознан правильно нашей маленькой функцией."), "ru");
        assert_eq!(guess("hi"), "");
    }
    #[test]
    fn proxy_roundtrip() {
        assert_eq!(original_of("https://www-example-com.translate.goog/a/b?x=1&_x_tr_sl=auto&_x_tr_tl=en").as_deref(), Some("https://www.example.com/a/b?x=1"));
        assert_eq!(original_of("https://my--site-co-nz.translate.goog/").as_deref(), Some("https://my-site.co.nz/"));
        assert_eq!(normalize("pt-BR"), "pt");
        assert_eq!(normalize("nb"), "no");
    }
}
