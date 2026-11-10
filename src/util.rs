//! Small shared helpers: wide strings, paths, time, logging.

use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use windows::core::PCWSTR;

/// Null-terminated UTF-16 buffer that keeps its storage alive while a PCWSTR points into it.
pub struct WStr(Vec<u16>);

impl WStr {
    pub fn new(s: &str) -> Self {
        let mut v: Vec<u16> = OsStr::new(s).encode_wide().collect();
        v.push(0);
        WStr(v)
    }
    pub fn path(p: &Path) -> Self {
        let mut v: Vec<u16> = p.as_os_str().encode_wide().collect();
        v.push(0);
        WStr(v)
    }
    pub fn pcwstr(&self) -> PCWSTR {
        PCWSTR(self.0.as_ptr())
    }
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn new_id() -> String {
    use rand::RngCore;
    let mut b = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut b);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

/// %LOCALAPPDATA%\Solus\User Data (overridable with --user-data-dir for tests).
pub fn user_data_root() -> PathBuf {
    if let Some(p) = crate::ARGS.with(|a| a.borrow().user_data_dir.clone()) {
        return p;
    }
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("Solus").join("User Data")
}

pub fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Folder holding the browser UI (html/css/js). Installed builds ship it beside the exe;
/// development builds fall back to the source tree.
pub fn ui_dir() -> PathBuf {
    let beside = exe_dir().join("ui");
    if beside.join("chrome").is_dir() {
        return beside;
    }
    let mut p = exe_dir();
    for _ in 0..4 {
        if p.join("ui").join("chrome").is_dir() {
            return p.join("ui");
        }
        if !p.pop() {
            break;
        }
    }
    beside
}

/// Write a file atomically (temp file + rename) so a crash never leaves a half-written file.
pub fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp~");
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)
}

static LOG: Mutex<Option<PathBuf>> = Mutex::new(None);

pub fn init_log() {
    let dir = user_data_root().join("Logs");
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("solus.log");
    // keep the log small: rotate at 2 MB
    if fs::metadata(&path).map(|m| m.len() > 2_000_000).unwrap_or(false) {
        let _ = fs::rename(&path, dir.join("solus.old.log"));
    }
    *LOG.lock().unwrap() = Some(path);
}

pub fn log(msg: &str) {
    let line = format!("{} {}\n", now_ms(), msg);
    if cfg!(debug_assertions) {
        eprint!("{line}");
    }
    if let Some(p) = LOG.lock().unwrap().as_ref() {
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(p) {
            let _ = f.write_all(line.as_bytes());
        }
    }
}

#[macro_export]
macro_rules! logf {
    ($($t:tt)*) => { $crate::util::log(&format!($($t)*)) };
}

/// Strip anything that looks like a secret from text before it is logged.
pub fn redact(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for tok in s.split_inclusive(|c: char| c.is_whitespace() || c == '"' || c == '\'') {
        let t = tok.trim_matches(|c: char| c.is_whitespace() || c == '"' || c == '\'');
        if (t.starts_with("sk-") || t.starts_with("gsk_") || t.starts_with("sk-ant-")) && t.len() > 12 {
            out.push_str(&tok.replace(t, "[redacted]"));
        } else {
            out.push_str(tok);
        }
    }
    out
}

pub fn json_str(v: &serde_json::Value, key: &str) -> String {
    v.get(key).and_then(|x| x.as_str()).unwrap_or("").to_string()
}

pub fn host_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_string()))
        .unwrap_or_default()
}

pub fn origin_of(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(u) => u.origin().ascii_serialization(),
        Err(_) => String::new(),
    }
}

/// Registrable domain ("eTLD+1"), e.g. news.bbc.co.uk -> bbc.co.uk.
pub fn site_of_host(host: &str) -> String {
    let h = host.trim_end_matches('.').to_ascii_lowercase();
    if h.parse::<std::net::IpAddr>().is_ok() {
        return h;
    }
    match psl::domain(h.as_bytes()) {
        Some(d) => String::from_utf8_lossy(d.as_bytes()).to_string(),
        None => h,
    }
}

pub fn site_of(url: &str) -> String {
    site_of_host(&host_of(url))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sites() {
        assert_eq!(site_of("https://news.bbc.co.uk/a"), "bbc.co.uk");
        assert_eq!(site_of("https://www.google.com/search?q=1"), "google.com");
        assert_eq!(site_of("http://127.0.0.1:8080/"), "127.0.0.1");
    }
    #[test]
    fn redaction() {
        let r = redact("key sk-ant-api03-abcdefghijklmnop rest");
        assert!(!r.contains("abcdefghijk"));
        assert!(r.contains("rest"));
    }
    #[test]
    fn ids_unique() {
        assert_ne!(new_id(), new_id());
        assert_eq!(new_id().len(), 36);
    }
}
