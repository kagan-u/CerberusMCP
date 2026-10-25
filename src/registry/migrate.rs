//! Versioned schema migrations.
//!
//! Migrations transform a parsed YAML document from an older `schema_version`
//! to the current one. Unknown (future) versions are rejected rather than
//! silently reinterpreted. Before a migration is applied the original file is
//! copied into the backups directory.

use std::path::Path;

use crate::error::{CerberusError, Result};
use crate::paths::CerberusPaths;

/// Bring `doc` (reported at `from_version`) up to the current schema version.
pub fn migrate(
    doc: serde_yaml::Value,
    from_version: u32,
    path: &Path,
) -> Result<serde_yaml::Value> {
    let current = super::schema::SCHEMA_VERSION;
    if from_version == current {
        return Ok(doc);
    }
    if from_version > current {
        return Err(CerberusError::UnsupportedSchemaVersion {
            found: from_version,
            supported: current,
        });
    }

    // Pre-migration backup so no data can be lost by a bad transform.
    if let Ok(paths) = CerberusPaths::resolve() {
        if let Err(e) = backup_before_migration(&paths, path) {
            tracing::warn!(error = %e, "could not create pre-migration backup");
        }
    }

    let mut doc = doc;
    let mut version = from_version;
    while version < current {
        doc = match version {
            0 => migrate_v0_to_v1(doc)?,
            1 => migrate_v1_to_v2(doc)?,
            _ => {
                return Err(CerberusError::UnsupportedSchemaVersion {
                    found: version,
                    supported: current,
                });
            }
        };
        version += 1;
        tracing::info!(to = version, "applied schema migration");
    }
    Ok(doc)
}

/// v0 → v1: accept documents that predate `schema_version` being mandatory
/// (treat missing version as 0 is handled by the caller; this transform
/// normalizes legacy `servers` given as a bare command string).
fn migrate_v0_to_v1(doc: serde_yaml::Value) -> Result<serde_yaml::Value> {
    let mut map = match doc {
        serde_yaml::Value::Mapping(m) => m,
        other => {
            let _ = other;
            return Err(CerberusError::Validation(
                "cannot migrate a non-mapping document".into(),
            ));
        }
    };

    // Legacy: `servers.<name>: "command string"` → stdio transport object.
    if let Some(serde_yaml::Value::Mapping(servers)) = map.get(serde_yaml::Value::from("servers")) {
        let mut new_servers = serde_yaml::Mapping::new();
        let mut changed = false;
        for (k, v) in servers {
            match v {
                serde_yaml::Value::String(cmd) => {
                    let mut entry = serde_yaml::Mapping::new();
                    entry.insert("enabled".into(), true.into());
                    let mut transport = serde_yaml::Mapping::new();
                    transport.insert("type".into(), "stdio".into());
                    transport.insert("command".into(), cmd.clone().into());
                    entry.insert("transport".into(), serde_yaml::Value::Mapping(transport));
                    new_servers.insert(k.clone(), serde_yaml::Value::Mapping(entry));
                    changed = true;
                }
                other => {
                    new_servers.insert(k.clone(), other.clone());
                }
            }
        }
        if changed {
            map.insert("servers".into(), serde_yaml::Value::Mapping(new_servers));
        }
    }

    map.insert("schema_version".into(), 1.into());
    Ok(serde_yaml::Value::Mapping(map))
}

/// v1 → v2: additive schema change (`commands` + `pre_launch`, both
/// optional with serde defaults), so the transform only bumps the version
/// marker — the existing document structure carries over unchanged. The
/// format conversion (YAML → TOML) happens on the next save, not here.
fn migrate_v1_to_v2(mut doc: serde_yaml::Value) -> Result<serde_yaml::Value> {
    match &mut doc {
        serde_yaml::Value::Mapping(map) => {
            map.insert("schema_version".into(), 2.into());
            Ok(doc)
        }
        _ => Err(CerberusError::Validation(
            "cannot migrate a non-mapping document".into(),
        )),
    }
}

fn backup_before_migration(paths: &CerberusPaths, path: &Path) -> Result<()> {
    use std::io::Write as _;
    let text = std::fs::read_to_string(path).map_err(|source| CerberusError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let stamp = crate::backup::timestamp_slug();
    let dir = paths.backups_dir().join("migrations");
    std::fs::create_dir_all(&dir).map_err(|source| CerberusError::Write {
        path: dir.clone(),
        source,
    })?;
    let dest = dir.join(format!("cerberus-{stamp}.yaml.bak"));
    let mut f = std::fs::File::create(&dest).map_err(|source| CerberusError::Write {
        path: dest.clone(),
        source,
    })?;
    f.write_all(text.as_bytes())
        .map_err(|source| CerberusError::Write { path: dest, source })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_version_is_noop() {
        let doc: serde_yaml::Value = serde_yaml::from_str("schema_version: 2").unwrap();
        let out = migrate(doc.clone(), 2, Path::new("/nonexistent")).unwrap();
        assert_eq!(doc, out);
    }

    #[test]
    fn migrates_v1_to_v2_additively() {
        let doc: serde_yaml::Value =
            serde_yaml::from_str("schema_version: 1\nservers: {}\n").unwrap();
        let out = migrate(doc, 1, Path::new("/nonexistent")).unwrap();
        let reg: super::super::schema::Registry = serde_yaml::from_value(out).unwrap();
        assert_eq!(reg.schema_version, 2);
        assert!(reg.servers.is_empty());
        assert!(reg.commands.is_empty());
    }

    #[test]
    fn future_version_rejected() {
        let doc: serde_yaml::Value = serde_yaml::from_str("schema_version: 99").unwrap();
        let err = migrate(doc, 99, Path::new("/nonexistent")).unwrap_err();
        assert!(matches!(
            err,
            CerberusError::UnsupportedSchemaVersion { found: 99, .. }
        ));
    }

    #[test]
    fn migrates_legacy_string_servers() {
        let doc: serde_yaml::Value =
            serde_yaml::from_str("servers:\n  legacy: npx -y something\n").unwrap();
        let out = migrate(doc, 0, Path::new("/nonexistent")).unwrap();
        let reg: super::super::schema::Registry = serde_yaml::from_value(out).unwrap();
        match &reg.servers["legacy"].transport {
            super::super::schema::Transport::Stdio { command, .. } => {
                assert_eq!(command, "npx -y something")
            }
            other => panic!("expected stdio, got {other:?}"),
        }
        assert_eq!(reg.schema_version, 2);
    }
}
