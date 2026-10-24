//! Backup and restore of managed configuration files.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::error::{CerberusError, Result};
use crate::paths::CerberusPaths;
use crate::registry::atomic_write;

const MAX_BACKUPS: usize = 50;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupMeta {
    pub id: String,
    /// Path of the file that was backed up.
    pub original: PathBuf,
    /// Path of the stored copy.
    pub stored: PathBuf,
    /// Unix timestamp (seconds) of creation.
    pub created_unix: i64,
    /// Optional label supplied by the caller (e.g. "pre-sync-opencode").
    pub label: Option<String>,
}

/// Create a backup of `original` under the backups directory.
pub fn create(paths: &CerberusPaths, original: &Path, label: Option<&str>) -> Result<BackupMeta> {
    if !original.exists() {
        return Err(CerberusError::ConfigNotFound {
            path: original.to_path_buf(),
        });
    }
    let text = std::fs::read_to_string(original).map_err(|source| CerberusError::Io {
        path: original.to_path_buf(),
        source,
    })?;
    create_from_text(paths, original, &text, label)
}

/// Create a backup from known text content (used when the caller already
/// holds the file contents and wants to avoid a TOCTOU gap).
pub fn create_from_text(
    paths: &CerberusPaths,
    original: &Path,
    text: &str,
    label: Option<&str>,
) -> Result<BackupMeta> {
    let id = format!("{}-{}", timestamp_slug(), short_id());
    let dir = paths.backups_dir();
    std::fs::create_dir_all(&dir).map_err(|source| CerberusError::Write {
        path: dir.clone(),
        source,
    })?;
    let stored = dir.join(format!("{id}.bak"));
    atomic_write(&stored, text.as_bytes())?;

    let meta = BackupMeta {
        id: id.clone(),
        original: original.to_path_buf(),
        stored,
        created_unix: OffsetDateTime::now_utc().unix_timestamp(),
        label: label.map(str::to_string),
    };
    write_meta(paths, &meta)?;
    prune_old(paths)?;
    Ok(meta)
}

fn write_meta(paths: &CerberusPaths, meta: &BackupMeta) -> Result<()> {
    let dir = paths.backups_dir();
    let path = dir.join(format!("{}.meta.json", meta.id));
    let text = serde_json::to_string_pretty(meta)
        .map_err(|e| CerberusError::Other(format!("backup metadata serialize: {e}")))?;
    atomic_write(&path, text.as_bytes())
}

fn read_meta(path: &Path) -> Result<BackupMeta> {
    let text = std::fs::read_to_string(path).map_err(|source| CerberusError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    serde_json::from_str(&text).map_err(|e| CerberusError::Other(format!("backup metadata: {e}")))
}

/// List backups, newest first.
pub fn list(paths: &CerberusPaths) -> Result<Vec<BackupMeta>> {
    let dir = paths.backups_dir();
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir).map_err(|source| CerberusError::Io {
        path: dir.clone(),
        source,
    })? {
        let entry = entry.map_err(|source| CerberusError::Io {
            path: dir.clone(),
            source,
        })?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("json")
            && path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with(".meta.json"))
        {
            if let Ok(meta) = read_meta(&path) {
                out.push(meta);
            }
        }
    }
    out.sort_by(|a, b| b.created_unix.cmp(&a.created_unix));
    Ok(out)
}

/// Restore a backup to its original path. The current file (if any) is
/// backed up first under a `pre-restore` label.
pub fn restore(paths: &CerberusPaths, id: &str) -> Result<BackupMeta> {
    let meta = find(paths, id)?;
    let text = std::fs::read_to_string(&meta.stored).map_err(|source| CerberusError::Io {
        path: meta.stored.clone(),
        source,
    })?;

    // Verify the stored copy parses as JSON or YAML before overwriting.
    let looks_json = text.trim_start().starts_with('{');
    let parse_ok = if looks_json {
        serde_json::from_str::<serde_json::Value>(&text).is_ok()
    } else {
        serde_yaml::from_str::<serde_yaml::Value>(&text).is_ok()
    };
    if !parse_ok {
        return Err(CerberusError::Other(format!(
            "backup {id} does not parse as JSON or YAML; refusing to restore"
        )));
    }

    if meta.original.exists() {
        let _ = create_from_text(
            paths,
            &meta.original,
            &text_of(&meta.original)?,
            Some("pre-restore"),
        )?;
    }
    atomic_write(&meta.original, text.as_bytes())?;
    Ok(meta)
}

fn text_of(path: &Path) -> Result<String> {
    std::fs::read_to_string(path).map_err(|source| CerberusError::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn find(paths: &CerberusPaths, id: &str) -> Result<BackupMeta> {
    list(paths)?
        .into_iter()
        .find(|m| m.id == id || m.id.starts_with(id))
        .ok_or_else(|| CerberusError::BackupNotFound { id: id.to_string() })
}

fn prune_old(paths: &CerberusPaths) -> Result<()> {
    let mut metas = list(paths)?;
    if metas.len() <= MAX_BACKUPS {
        return Ok(());
    }
    metas.sort_by(|a, b| a.created_unix.cmp(&b.created_unix));
    let excess = metas.len() - MAX_BACKUPS;
    for meta in metas.into_iter().take(excess) {
        let _ = std::fs::remove_file(&meta.stored);
        let _ = std::fs::remove_file(paths.backups_dir().join(format!("{}.meta.json", meta.id)));
    }
    Ok(())
}

/// Timestamp slug used in backup and migration filenames: YYYYMMDD-HHMMSS (UTC).
pub fn timestamp_slug() -> String {
    let now = OffsetDateTime::now_utc();
    format!(
        "{:04}{:02}{:02}-{:02}{:02}{:02}",
        now.year(),
        now.month() as u8,
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    )
}

fn short_id() -> String {
    let mut bytes = [0u8; 4];
    getrandom_fill(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn getrandom_fill(buf: &mut [u8]) {
    // Best-effort randomness from the OS; falls back to time+pid mix.
    #[cfg(unix)]
    {
        use std::io::Read;
        if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
            if f.read_exact(buf).is_ok() {
                return;
            }
        }
    }
    let t = OffsetDateTime::now_utc().unix_timestamp_nanos() as u64;
    for (i, b) in buf.iter_mut().enumerate() {
        *b = ((t >> (i * 8)) ^ (std::process::id() as u64)) as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_list_restore_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let paths = CerberusPaths::with_roots(dir.path().join("cfg"), dir.path().join("data"));
        paths.ensure_dirs().unwrap();

        let target = dir.path().join("client.json");
        std::fs::write(&target, r#"{"a":1}"#).unwrap();

        let meta = create(&paths, &target, Some("test")).unwrap();
        assert_eq!(list(&paths).unwrap().len(), 1);

        std::fs::write(&target, r#"{"a":2}"#).unwrap();
        restore(&paths, &meta.id).unwrap();
        let restored = std::fs::read_to_string(&target).unwrap();
        assert_eq!(restored, r#"{"a":1}"#);
        // pre-restore backup exists too
        assert_eq!(list(&paths).unwrap().len(), 2);
    }

    #[test]
    fn restore_unknown_id_fails() {
        let dir = tempfile::tempdir().unwrap();
        let paths = CerberusPaths::with_roots(dir.path().join("cfg"), dir.path().join("data"));
        paths.ensure_dirs().unwrap();
        assert!(restore(&paths, "missing").is_err());
    }

    #[test]
    fn corrupt_backup_refused() {
        let dir = tempfile::tempdir().unwrap();
        let paths = CerberusPaths::with_roots(dir.path().join("cfg"), dir.path().join("data"));
        paths.ensure_dirs().unwrap();
        let target = dir.path().join("client.json");
        std::fs::write(&target, r#"{"a":1}"#).unwrap();
        let meta = create(&paths, &target, None).unwrap();
        std::fs::write(&meta.stored, "}{ not json or yaml :::: ").unwrap();
        assert!(restore(&paths, &meta.id).is_err());
        // original untouched
        assert_eq!(std::fs::read_to_string(&target).unwrap(), r#"{"a":1}"#);
    }
}
