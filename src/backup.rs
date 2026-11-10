//! Local profile backups (automatic, daily) and restore. Cloud backups live in sync.rs.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::app::App;
use crate::util;

const FILES: &[&str] = &["settings.json", "themes.json", "session.json", "key.bin", "newtab.json"];

pub fn backups_dir(profile: &str) -> PathBuf {
    util::user_data_root().join("Backups").join(profile)
}

fn stamp() -> String {
    format!("{}", util::now_ms())
}

/// Make a consistent snapshot of a profile. Uses SQLite's VACUUM INTO so the copy is valid
/// even while the database is in use.
pub fn backup_profile(app: &mut App, profile: &str, reason: &str) -> Result<PathBuf, String> {
    let p = app.profile(profile);
    let dir = backups_dir(profile).join(format!("{}-{}", stamp(), reason));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    if let Some(db) = &p.store {
        let target = dir.join("solus.db");
        db.conn
            .execute("VACUUM INTO ?1", [target.to_string_lossy().to_string()])
            .map_err(|e| format!("database snapshot failed: {e}"))?;
    }
    for f in FILES {
        let src = p.dir.join(f);
        if src.exists() {
            let _ = std::fs::copy(&src, dir.join(f));
        }
    }
    let meta = json!({"created": util::now_ms(), "reason": reason, "version": crate::VERSION});
    let _ = std::fs::write(dir.join("backup.json"), meta.to_string());
    let keep = p.settings.backup_keep.max(2) as usize;
    prune(profile, keep);
    logf!("backup created: {}", dir.display());
    Ok(dir)
}

pub fn list(profile: &str) -> Vec<Value> {
    let mut out = vec![];
    if let Ok(rd) = std::fs::read_dir(backups_dir(profile)) {
        for e in rd.flatten() {
            let p = e.path();
            if !p.is_dir() {
                continue;
            }
            let meta: Value = std::fs::read(p.join("backup.json"))
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or(json!({}));
            let size: u64 = std::fs::read_dir(&p).map(|r| r.flatten().filter_map(|f| f.metadata().ok()).map(|m| m.len()).sum()).unwrap_or(0);
            out.push(json!({
                "name": e.file_name().to_string_lossy(),
                "created": meta["created"],
                "reason": meta["reason"],
                "size": size,
            }));
        }
    }
    out.sort_by(|a, b| b["created"].as_i64().cmp(&a["created"].as_i64()));
    out
}

fn prune(profile: &str, keep: usize) {
    let all = list(profile);
    for b in all.iter().skip(keep) {
        if let Some(n) = b["name"].as_str() {
            let _ = std::fs::remove_dir_all(backups_dir(profile).join(n));
        }
    }
}

/// Schedule a restore; it is applied at the next start, before the database is opened.
pub fn schedule_restore(profile: &str, name: &str) -> Result<(), String> {
    let src = backups_dir(profile).join(name);
    if !src.join("backup.json").exists() {
        return Err("backup not found".into());
    }
    let marker = util::user_data_root().join("Profiles").join(profile).join("restore.pending");
    std::fs::write(marker, name).map_err(|e| e.to_string())
}

/// Apply a scheduled restore (called when a profile is first opened).
pub fn apply_pending_restore(profile_dir: &Path, profile: &str) {
    let marker = profile_dir.join("restore.pending");
    let Ok(name) = std::fs::read_to_string(&marker) else { return };
    let src = backups_dir(profile).join(name.trim());
    let _ = std::fs::remove_file(&marker);
    if !src.is_dir() {
        return;
    }
    // keep the current state aside in case the user changes their mind
    let aside = profile_dir.join(format!("before-restore-{}", stamp()));
    let _ = std::fs::create_dir_all(&aside);
    for f in FILES.iter().chain(["solus.db"].iter()) {
        let cur = profile_dir.join(f);
        if cur.exists() {
            let _ = std::fs::copy(&cur, aside.join(f));
        }
        let from = src.join(f);
        if from.exists() {
            let _ = std::fs::copy(&from, &cur);
        }
    }
    let _ = std::fs::remove_file(profile_dir.join("solus.db-wal"));
    let _ = std::fs::remove_file(profile_dir.join("solus.db-shm"));
    logf!("restored profile {profile} from backup {}", src.display());
}

/// Crash recovery for a corrupt database: move it aside and use the newest backup copy.
pub fn restore_db_from_backup(profile_dir: &Path, db_path: &Path) {
    let profile = profile_dir.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let corrupt = db_path.with_extension(format!("corrupt-{}", stamp()));
    let _ = std::fs::rename(db_path, &corrupt);
    let _ = std::fs::remove_file(db_path.with_extension("db-wal"));
    let _ = std::fs::remove_file(db_path.with_extension("db-shm"));
    for b in list(&profile) {
        let src = backups_dir(&profile).join(b["name"].as_str().unwrap_or("")).join("solus.db");
        if src.exists() && std::fs::copy(&src, db_path).is_ok() {
            logf!("database recovered from backup {}", src.display());
            return;
        }
    }
    logf!("no backup available; starting with an empty database (old file kept at {})", corrupt.display());
}

/// Daily automatic backup check (called from the periodic tick).
pub fn maybe_auto_backup(app: &mut App) {
    let day = 86_400_000;
    if util::now_ms() - app.local.last_backup < day {
        return;
    }
    let ids: Vec<String> = app.local.profiles.iter().map(|p| p.id.clone()).collect();
    for id in ids {
        if app.settings(&id).local_backups {
            if let Err(e) = backup_profile(app, &id, "auto") {
                logf!("auto backup failed for {id}: {e}");
            }
        }
    }
    app.local.last_backup = util::now_ms();
    app.local_dirty = true;
}
