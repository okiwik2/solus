//! The `solus://` scheme (internal pages and the browser UI) and navigation policy:
//! web pages may not navigate or frame internal pages, and HTTPS-First upgrades plain http.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::*;
use windows::core::{Interface, PWSTR};
use windows::Win32::UI::Shell::SHCreateMemStream;

use crate::util::{self, WStr};

thread_local! {
    static ALLOW_INTERNAL: RefCell<HashSet<usize>> = RefCell::new(HashSet::new());
    static HTTPS_FIRST: Cell<bool> = const { Cell::new(true) };
    static HTTP_OK_HOSTS: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
    static UI_ROOT: PathBuf = util::ui_dir();
}

pub fn set_https_first(on: bool) {
    HTTPS_FIRST.with(|c| c.set(on));
}

pub fn allow_http_host(host: &str) {
    HTTP_OK_HOSTS.with(|s| {
        s.borrow_mut().insert(host.to_ascii_lowercase());
    });
}

/// Host-initiated navigations to internal pages are recorded here so NavigationStarting can tell
/// them apart from a web page trying to open `solus://...`.
pub fn allow_internal_once(wv: &ICoreWebView2) {
    ALLOW_INTERNAL.with(|s| {
        s.borrow_mut().insert(wv.as_raw() as usize);
    });
}

pub enum NavDecision {
    Allow,
    Block,
}

pub fn navigation_policy_for(wv: &ICoreWebView2, current: &str, target: &str, history_nav: bool) -> NavDecision {
    if !target.starts_with("solus:") {
        return NavDecision::Allow;
    }
    let allowed = ALLOW_INTERNAL.with(|s| s.borrow_mut().remove(&(wv.as_raw() as usize)));
    if allowed || history_nav || current.is_empty() || current.starts_with("solus:") || current == "about:blank" {
        NavDecision::Allow
    } else {
        NavDecision::Block
    }
}

pub fn https_first_upgrade(uri: &str) -> bool {
    if !HTTPS_FIRST.with(|c| c.get()) || !uri.starts_with("http://") {
        return false;
    }
    let host = util::host_of(uri).to_ascii_lowercase();
    if host.is_empty()
        || host == "localhost"
        || host.parse::<std::net::IpAddr>().is_ok()
        || host.trim_matches(|c| c == '[' || c == ']').parse::<std::net::Ipv6Addr>().is_ok()
        || !host.contains('.')
        || [".local", ".lan", ".internal", ".home", ".localhost", ".test", ".corp"].iter().any(|s| host.ends_with(s))
    {
        return false;
    }
    if url::Url::parse(uri).map(|u| u.port().is_some()).unwrap_or(false) {
        return false; // explicit ports are usually dev servers
    }
    !HTTP_OK_HOSTS.with(|s| s.borrow().contains(&host))
}

/// Serve `solus://` requests for this webview.
pub fn install(wv: &ICoreWebView2) {
    unsafe {
        let filter = WStr::new("solus://*");
        let _ = wv.AddWebResourceRequestedFilter(filter.pcwstr(), COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL);
        let h = WebResourceRequestedEventHandler::create(Box::new(|sender, args| {
            let (Some(wv), Some(args)) = (sender, args) else { return Ok(()) };
            let req = args.Request()?;
            let mut p = PWSTR::null();
            req.Uri(&mut p)?;
            let uri = take_pwstr(p);
            if !uri.starts_with("solus:") {
                return crate::privacy::check_request(&wv, &args, &uri);
            }
            let (status, ctype, body) = resolve(&uri);
            let env = wv.cast::<ICoreWebView2_2>()?.Environment()?;
            let stream = SHCreateMemStream(Some(&body));
            let mut headers = format!("Content-Type: {ctype}\r\nCache-Control: no-cache\r\nX-Content-Type-Options: nosniff");
            if ctype.starts_with("text/html") {
                headers.push_str("\r\nContent-Security-Policy: frame-ancestors 'self'\r\nX-Frame-Options: SAMEORIGIN");
            }
            let reason = WStr::new(if status == 200 { "OK" } else { "Not Found" });
            let hdr = WStr::new(&headers);
            let resp = env.CreateWebResourceResponse(stream.as_ref(), status, reason.pcwstr(), hdr.pcwstr())?;
            args.SetResponse(&resp)?;
            Ok(())
        }));
        let mut tok = Default::default();
        let _ = wv.add_WebResourceRequested(&h, &mut tok);
    }
}

fn mime(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase().as_str() {
        "html" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
}

/// Map a solus:// URL to a file under the UI folder.
pub fn map_path(uri: &str) -> Option<PathBuf> {
    let u = url::Url::parse(uri).ok()?;
    let host = u.host_str().unwrap_or("").to_ascii_lowercase();
    let path = u.path();
    let root = UI_ROOT.with(|r| r.clone());
    let rel: PathBuf = if path == "/" || path.is_empty() {
        if host == "chrome" {
            PathBuf::from("chrome/index.html")
        } else {
            PathBuf::from(format!("pages/{host}.html"))
        }
    } else {
        let p = PathBuf::from(path.trim_start_matches('/'));
        if p.components().any(|c| !matches!(c, Component::Normal(_))) {
            return None;
        }
        p
    };
    if rel.components().any(|c| !matches!(c, Component::Normal(_))) {
        return None;
    }
    Some(root.join(rel))
}

pub fn resolve(uri: &str) -> (i32, &'static str, Vec<u8>) {
    if let Some(p) = map_path(uri) {
        if let Ok(b) = std::fs::read(&p) {
            return (200, mime(&p), b);
        }
    }
    let page = format!(
        "<!doctype html><meta charset=utf-8><title>Not found</title><link rel=stylesheet href=\"/shared/base.css\">\
         <body class=page><main class=narrow><h1>Page not found</h1><p>{} is not a Solus page.</p>\
         <p><a href=\"solus://newtab/\">Go to the new tab page</a></p></main></body>",
        html_escape(uri)
    );
    (404, "text/html; charset=utf-8", page.into_bytes())
}

pub fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn path_mapping_rejects_traversal() {
        assert!(map_path("solus://newtab/../../secret").map(|p| !p.to_string_lossy().contains("..")).unwrap_or(true));
        assert!(map_path("solus://chrome/").unwrap().ends_with("chrome/index.html"));
        assert!(map_path("solus://settings/").unwrap().ends_with("pages/settings.html"));
        assert!(map_path("solus://settings/shared/base.css").unwrap().ends_with("shared/base.css"));
    }
    #[test]
    fn https_first_rules() {
        assert!(https_first_upgrade("http://example.com/a"));
        assert!(!https_first_upgrade("http://localhost/"));
        assert!(!https_first_upgrade("http://192.168.1.10/"));
        assert!(!https_first_upgrade("http://router/"));
        assert!(!https_first_upgrade("http://example.com:8080/"));
        assert!(!https_first_upgrade("https://example.com/"));
        allow_http_host("plain.example.org");
        assert!(!https_first_upgrade("http://plain.example.org/"));
    }
}
