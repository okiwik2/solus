//! Encryption helpers.
//!
//! * Local data: each profile has a random 256-bit master key protected with Windows DPAPI (only this
//!   Windows account on this PC can unwrap it). History, notes and passwords are sealed with
//!   AES-256-GCM under keys derived from it.
//! * Cloud sync / backups: keys derived from the user's passphrase with Argon2id, so the files in the
//!   cloud folder are useless without the passphrase.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use rand::RngCore;
use sha2::{Digest, Sha256};
use windows::Win32::Foundation::{LocalFree, HLOCAL};
use windows::Win32::Security::Cryptography::*;

#[derive(Clone)]
pub struct Keys {
    pub enc: [u8; 32],
    pub mac: [u8; 32],
}

impl std::fmt::Debug for Keys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Keys(..)")
    }
}

impl Keys {
    pub fn from_master(master: &[u8]) -> Keys {
        Keys { enc: derive(master, b"solus-enc-v1"), mac: derive(master, b"solus-mac-v1") }
    }
    pub fn seal(&self, plain: &[u8]) -> Vec<u8> {
        seal(&self.enc, plain, b"")
    }
    pub fn open(&self, sealed: &[u8]) -> Option<Vec<u8>> {
        open(&self.enc, sealed, b"")
    }
    pub fn seal_str(&self, s: &str) -> Vec<u8> {
        self.seal(s.as_bytes())
    }
    pub fn open_str(&self, b: &[u8]) -> String {
        self.open(b).map(|v| String::from_utf8_lossy(&v).into_owned()).unwrap_or_default()
    }
    /// Deterministic keyed hash for lookups (e.g. "is this URL already in history?").
    pub fn tag(&self, data: &[u8]) -> Vec<u8> {
        hmac_sha256(&self.mac, data).to_vec()
    }
}

fn derive(master: &[u8], label: &[u8]) -> [u8; 32] {
    hmac_sha256(master, label)
}

pub fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut k = [0u8; 64];
    if key.len() > 64 {
        k[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut ipad = [0x36u8; 64];
    let mut opad = [0x5cu8; 64];
    for i in 0..64 {
        ipad[i] ^= k[i];
        opad[i] ^= k[i];
    }
    let inner = Sha256::new().chain_update(ipad).chain_update(data).finalize();
    let outer = Sha256::new().chain_update(opad).chain_update(inner).finalize();
    outer.into()
}

pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    rand::thread_rng().fill_bytes(&mut b);
    b
}

/// AES-256-GCM: output = 12-byte nonce || ciphertext+tag.
pub fn seal(key: &[u8; 32], plain: &[u8], aad: &[u8]) -> Vec<u8> {
    let cipher = Aes256Gcm::new_from_slice(key).expect("key length");
    let nonce = random_bytes::<12>();
    let ct = cipher
        .encrypt(Nonce::from_slice(&nonce), Payload { msg: plain, aad })
        .expect("encrypt");
    let mut out = Vec::with_capacity(12 + ct.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    out
}

pub fn open(key: &[u8; 32], sealed: &[u8], aad: &[u8]) -> Option<Vec<u8>> {
    if sealed.len() < 12 + 16 {
        return None;
    }
    let cipher = Aes256Gcm::new_from_slice(key).ok()?;
    cipher.decrypt(Nonce::from_slice(&sealed[..12]), Payload { msg: &sealed[12..], aad }).ok()
}

/// Argon2id key from a passphrase (memory-hard; ~64 MB, a few hundred ms).
pub fn passphrase_key(pass: &str, salt: &[u8]) -> [u8; 32] {
    use argon2::{Algorithm, Argon2, Params, Version};
    let params = Params::new(64 * 1024, 3, 1, Some(32)).expect("argon2 params");
    let a = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut out = [0u8; 32];
    a.hash_password_into(pass.as_bytes(), salt, &mut out).expect("argon2");
    out
}

/// Protect bytes with DPAPI for the current Windows user.
pub fn dpapi_protect(data: &[u8]) -> Result<Vec<u8>, String> {
    unsafe {
        let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
        let mut out = CRYPT_INTEGER_BLOB::default();
        CryptProtectData(&input, windows::core::w!("Solus"), None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out)
            .map_err(|e| e.to_string())?;
        let v = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        let _ = LocalFree(Some(HLOCAL(out.pbData as _)));
        Ok(v)
    }
}

pub fn dpapi_unprotect(data: &[u8]) -> Result<Vec<u8>, String> {
    unsafe {
        let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
        let mut out = CRYPT_INTEGER_BLOB::default();
        CryptUnprotectData(&input, None, None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out).map_err(|e| e.to_string())?;
        let v = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        let _ = LocalFree(Some(HLOCAL(out.pbData as _)));
        Ok(v)
    }
}

/// Load (or create) the profile master key stored DPAPI-protected in `path`.
pub fn load_or_create_master(path: &std::path::Path) -> Result<Vec<u8>, String> {
    if let Ok(b) = std::fs::read(path) {
        return dpapi_unprotect(&b).map_err(|e| format!("profile key could not be unlocked: {e}"));
    }
    let master = random_bytes::<32>().to_vec();
    let prot = dpapi_protect(&master)?;
    crate::util::write_atomic(path, &prot).map_err(|e| e.to_string())?;
    Ok(master)
}

/// Secret values (API keys, sync passphrase key) stored DPAPI-protected, one file per name.
pub fn store_secret(dir: &std::path::Path, name: &str, value: &[u8]) -> Result<(), String> {
    let p = dir.join("secrets").join(format!("{name}.bin"));
    if value.is_empty() {
        let _ = std::fs::remove_file(&p);
        return Ok(());
    }
    let prot = dpapi_protect(value)?;
    crate::util::write_atomic(&p, &prot).map_err(|e| e.to_string())
}

pub fn load_secret(dir: &std::path::Path, name: &str) -> Option<Vec<u8>> {
    let p = dir.join("secrets").join(format!("{name}.bin"));
    let b = std::fs::read(p).ok()?;
    dpapi_unprotect(&b).ok()
}

pub fn has_secret(dir: &std::path::Path, name: &str) -> bool {
    dir.join("secrets").join(format!("{name}.bin")).exists()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn seal_open_roundtrip_and_tamper() {
        let k = Keys::from_master(&[7u8; 32]);
        let s = k.seal_str("https://example.com/secret");
        assert_eq!(k.open_str(&s), "https://example.com/secret");
        let mut bad = s.clone();
        let n = bad.len();
        bad[n - 1] ^= 1;
        assert!(k.open(&bad).is_none(), "tampering must be detected");
        assert_ne!(k.seal_str("x"), k.seal_str("x"), "nonces must differ");
    }
    #[test]
    fn hmac_known_vector() {
        // RFC 4231 test case 2
        let h = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
        let hex: String = h.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
    }
    #[test]
    fn dpapi_roundtrip() {
        let p = dpapi_protect(b"hello").unwrap();
        assert_ne!(p, b"hello");
        assert_eq!(dpapi_unprotect(&p).unwrap(), b"hello");
    }
    #[test]
    fn passphrase_kdf_is_deterministic() {
        let a = passphrase_key("correct horse", b"0123456789abcdef");
        let b = passphrase_key("correct horse", b"0123456789abcdef");
        let c = passphrase_key("wrong horse", b"0123456789abcdef");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }
}
