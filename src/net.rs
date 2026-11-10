//! Outbound HTTP for host-side features (search suggestions, AI, filter lists, updates).
//! Uses Windows' own TLS stack (SChannel) and certificate store.

use std::sync::OnceLock;
use std::time::Duration;

use ureq::tls::{RootCerts, TlsConfig, TlsProvider};

pub const UA: &str = concat!("Solus/", env!("CARGO_PKG_VERSION"), " (Windows NT 10.0; Win64; x64)");

fn build(timeout: u64) -> ureq::Agent {
    let cfg = ureq::Agent::config_builder()
        .tls_config(TlsConfig::builder().provider(TlsProvider::NativeTls).root_certs(RootCerts::PlatformVerifier).build())
        .timeout_global(Some(Duration::from_secs(timeout)))
        .http_status_as_error(false)
        .user_agent(UA)
        .build();
    ureq::Agent::new_with_config(cfg)
}

pub fn agent() -> &'static ureq::Agent {
    static A: OnceLock<ureq::Agent> = OnceLock::new();
    A.get_or_init(|| build(15))
}

/// Agent for long-running requests (streaming AI replies, downloads of filter lists).
pub fn long_agent() -> &'static ureq::Agent {
    static A: OnceLock<ureq::Agent> = OnceLock::new();
    A.get_or_init(|| build(300))
}

pub fn get_text(url: &str) -> Result<String, String> {
    let mut r = agent().get(url).call().map_err(|e| e.to_string())?;
    if r.status().as_u16() >= 400 {
        return Err(format!("HTTP {}", r.status()));
    }
    r.body_mut().with_config().limit(32 * 1024 * 1024).read_to_string().map_err(|e| e.to_string())
}

pub fn get_bytes(url: &str, limit: u64) -> Result<Vec<u8>, String> {
    let mut r = long_agent().get(url).call().map_err(|e| e.to_string())?;
    if r.status().as_u16() >= 400 {
        return Err(format!("HTTP {}", r.status()));
    }
    r.body_mut().with_config().limit(limit).read_to_vec().map_err(|e| e.to_string())
}

/// Search suggestions in the OpenSearch JSON format: ["query", ["s1", "s2", ...]].
pub fn suggestions(url: &str) -> Vec<String> {
    let Ok(text) = get_text(url) else { return vec![] };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { return vec![] };
    v.get(1)
        .and_then(|a| a.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).take(8).collect())
        .unwrap_or_default()
}
