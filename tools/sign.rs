//! Release signing tool.
//!   solus-sign keygen <private-key-file>             create the release key (keep it secret!)
//!   solus-sign manifest <key> <installer> <version> <download-url> [notes]   -> prints latest.json
use base64::Engine;
use sha2::Digest;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let b64 = base64::engine::general_purpose::STANDARD;
    match a.get(1).map(|s| s.as_str()) {
        Some("keygen") => {
            let path = a.get(2).expect("key path");
            if std::path::Path::new(path).exists() {
                eprintln!("refusing to overwrite {path}");
                std::process::exit(1);
            }
            let mut seed = [0u8; 32];
            getrandom(&mut seed);
            let sk = ed25519_dalek::SigningKey::from_bytes(&seed);
            std::fs::write(path, b64.encode(seed)).unwrap();
            println!("{}", b64.encode(sk.verifying_key().to_bytes()));
        }
        Some("manifest") => {
            let seed: [u8; 32] = b64.decode(std::fs::read_to_string(&a[2]).unwrap().trim()).unwrap().try_into().unwrap();
            let sk = ed25519_dalek::SigningKey::from_bytes(&seed);
            let bytes = std::fs::read(&a[3]).unwrap();
            let hex: String = sha2::Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
            use ed25519_dalek::Signer;
            let sig = sk.sign(hex.as_bytes());
            let v = serde_json::json!({
                "version": a[4], "url": a[5], "sha256": hex, "signature": b64.encode(sig.to_bytes()),
                "notes": a.get(6).cloned().unwrap_or_default(), "size": bytes.len(),
            });
            println!("{}", serde_json::to_string_pretty(&v).unwrap());
        }
        _ => eprintln!("usage: solus-sign keygen <key-file> | manifest <key-file> <installer> <version> <url> [notes]"),
    }
}

fn getrandom(buf: &mut [u8]) {
    use rand::RngCore;
    rand::rngs::OsRng.fill_bytes(buf);
}
