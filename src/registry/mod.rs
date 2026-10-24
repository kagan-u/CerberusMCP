//! Registry load/save and CRUD operations.

pub mod catalog;
pub mod migrate;
pub mod schema;
pub mod validate;

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use crate::error::{CerberusError, Result};
use crate::paths::CerberusPaths;
use schema::{Profile, Registry, ServerDef, Transport};

pub use schema::{
    EnvValue, ExecutionMode, HeaderValue, SCHEMA_VERSION, ServerPolicy, Settings,
    Transport as ServerTransport,
};

/// Handle to a loaded registry bound to its on-disk location.
#[derive(Debug)]
pub struct RegistryStore {
    pub path: PathBuf,
    pub registry: Registry,
    /// SHA-256-like content fingerprint (fnv-based, 64-bit hex) of the file as
    /// last read/written, used for concurrent-modification detection.
    pub content_hash: u64,
}

impl RegistryStore {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Err(CerberusError::ConfigNotFound {
                path: path.to_path_buf(),
            });
        }
        let text = std::fs::read_to_string(path).map_err(|source| CerberusError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let registry = parse_registry(&text, path)?;
        let content_hash = hash_text(&text);
        Ok(Self {
            path: path.to_path_buf(),
            registry,
            content_hash,
        })
    }

    pub fn load_or_default(paths: &CerberusPaths) -> Result<Self> {
        if paths.registry_file.exists() {
            Self::load(&paths.registry_file)
        } else {
            Ok(Self {
                path: paths.registry_file.clone(),
                registry: Registry::default(),
                content_hash: 0,
            })
        }
    }

    /// Persist to disk atomically: write a sibling temp file, fsync, rename.
    pub fn save(&mut self) -> Result<()> {
        self.save_to(&self.path.clone())
    }

    pub fn save_to(&mut self, path: &Path) -> Result<()> {
        validate::validate(&self.registry)?;
        let text = serde_yaml::to_string(&self.registry)
            .map_err(|e| CerberusError::Validation(format!("failed to serialize registry: {e}")))?;
        atomic_write(path, text.as_bytes())?;
        self.path = path.to_path_buf();
        self.content_hash = hash_text(&text);
        Ok(())
    }

    /// Re-check that the file on disk has not changed since it was loaded.
    pub fn detect_concurrent_modification(&self) -> Result<()> {
        if !self.path.exists() {
            return Ok(());
        }
        let text = std::fs::read_to_string(&self.path).map_err(|source| CerberusError::Io {
            path: self.path.clone(),
            source,
        })?;
        if hash_text(&text) != self.content_hash {
            return Err(CerberusError::SyncConflict {
                client: "registry".into(),
                server: String::new(),
                detail: format!(
                    "{} was modified on disk since it was loaded; re-run the command",
                    self.path.display()
                ),
            });
        }
        Ok(())
    }

    // ---- CRUD ----------------------------------------------------------

    pub fn add_server(&mut self, name: &str, def: ServerDef) -> Result<()> {
        validate::validate_server_name(name)?;
        validate::validate_server_def(name, &def)?;
        if self.registry.servers.contains_key(name) {
            return Err(CerberusError::ServerExists {
                name: name.to_string(),
            });
        }
        self.registry.servers.insert(name.to_string(), def);
        Ok(())
    }

    pub fn remove_server(&mut self, name: &str) -> Result<ServerDef> {
        let def =
            self.registry
                .servers
                .remove(name)
                .ok_or_else(|| CerberusError::ServerNotFound {
                    name: name.to_string(),
                })?;
        for (pname, profile) in self.registry.profiles.iter_mut() {
            if profile.servers.iter().any(|s| s == name) {
                profile.servers.retain(|s| s != name);
                tracing::debug!(profile = %pname, server = %name, "removed server from profile");
            }
        }
        Ok(def)
    }

    pub fn set_enabled(&mut self, name: &str, enabled: bool) -> Result<()> {
        let def =
            self.registry
                .servers
                .get_mut(name)
                .ok_or_else(|| CerberusError::ServerNotFound {
                    name: name.to_string(),
                })?;
        def.enabled = enabled;
        Ok(())
    }

    pub fn upsert_profile(&mut self, name: &str, profile: Profile) -> Result<()> {
        validate::validate_profile_name(name)?;
        for server in &profile.servers {
            if !self.registry.servers.contains_key(server) {
                return Err(CerberusError::Validation(format!(
                    "profile `{name}` references unknown server `{server}`"
                )));
            }
        }
        self.registry.profiles.insert(name.to_string(), profile);
        Ok(())
    }

    pub fn remove_profile(&mut self, name: &str) -> Result<Profile> {
        self.registry
            .profiles
            .remove(name)
            .ok_or_else(|| CerberusError::ProfileNotFound {
                name: name.to_string(),
            })
    }

    pub fn set_active_profile(&mut self, name: Option<String>) -> Result<()> {
        if let Some(n) = &name {
            if !self.registry.profiles.contains_key(n) {
                return Err(CerberusError::ProfileNotFound { name: n.clone() });
            }
        }
        self.registry.active_profile = name;
        Ok(())
    }

    /// Transport summary used by `server list`.
    pub fn transport_kind(name: &str, def: &ServerDef) -> &'static str {
        let _ = name;
        def.transport.kind()
    }

    pub fn all_servers(&self) -> &BTreeMap<String, ServerDef> {
        &self.registry.servers
    }

    pub fn parse_transport(doc: &serde_yaml::Value) -> Result<Transport> {
        serde_yaml::from_value(doc.clone()).map_err(|e| CerberusError::Validation(e.to_string()))
    }
}

/// Parse registry text, routing through migrations when schema versions differ.
pub fn parse_registry(text: &str, path: &Path) -> Result<Registry> {
    let doc: serde_yaml::Value =
        serde_yaml::from_str(text).map_err(|e| CerberusError::ConfigParse {
            path: path.to_path_buf(),
            source: Box::new(e),
        })?;

    let version = doc
        .get("schema_version")
        .and_then(serde_yaml::Value::as_u64)
        .ok_or_else(|| CerberusError::Validation("missing `schema_version`".into()))?
        as u32;

    let migrated = migrate::migrate(doc, version, path)?;
    serde_yaml::from_value(migrated).map_err(|e| CerberusError::ConfigParse {
        path: path.to_path_buf(),
        source: Box::new(e),
    })
}

/// Write bytes to `path` atomically via a temp file in the same directory.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir).map_err(|source| CerberusError::Write {
        path: dir.to_path_buf(),
        source,
    })?;

    // Refuse to follow a symlink when replacing a managed file.
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        if meta.file_type().is_symlink() {
            return Err(CerberusError::Other(format!(
                "refusing to replace symlink at {}",
                path.display()
            )));
        }
    }

    let tmp = dir.join(format!(
        ".{}.cerberus-tmp-{}",
        path.file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "file".into()),
        std::process::id()
    ));
    {
        let mut f = std::fs::File::create(&tmp).map_err(|source| CerberusError::Write {
            path: tmp.clone(),
            source,
        })?;
        f.write_all(bytes).map_err(|source| CerberusError::Write {
            path: tmp.clone(),
            source,
        })?;
        f.sync_all().ok();
    }

    // Preserve existing permissions where possible.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(path) {
            let _ = std::fs::set_permissions(&tmp, meta.permissions());
        } else {
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
        }
    }

    std::fs::rename(&tmp, path).map_err(|source| {
        let _ = std::fs::remove_file(&tmp);
        CerberusError::Write {
            path: path.to_path_buf(),
            source,
        }
    })?;
    Ok(())
}

/// Stable content fingerprint used for change detection (FNV-1a, 64-bit).
pub fn hash_text(text: &str) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET;
    for b in text.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

/// Fingerprint of a structured value, independent of key order (JSON with
/// sorted keys).
pub fn hash_value(value: &serde_json::Value) -> u64 {
    let canonical = canonical_json(value);
    hash_text(&canonical)
}

fn canonical_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(map) => {
            let mut keys: Vec<_> = map.keys().collect();
            keys.sort();
            let mut out = String::from("{");
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(k).unwrap_or_default());
                out.push(':');
                out.push_str(&canonical_json(&map[*k]));
            }
            out.push('}');
            out
        }
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_remove_server_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cerberus.yaml");
        let mut store = RegistryStore {
            path: path.clone(),
            registry: Registry::default(),
            content_hash: 0,
        };
        let def = ServerDef {
            enabled: true,
            transport: Transport::Stdio {
                command: "echo".into(),
                args: vec![],
                env: BTreeMap::new(),
                cwd: None,
            },
            mode: None,
            tags: vec![],
            timeout_ms: None,
            startup_timeout_ms: None,
            policy: ServerPolicy::default(),
            description: None,
            source: None,
        };
        store.add_server("test", def.clone()).unwrap();
        store.save().unwrap();

        let loaded = RegistryStore::load(&path).unwrap();
        assert_eq!(loaded.registry.servers["test"], def);

        let mut store = loaded;
        store.remove_server("test").unwrap();
        store.save().unwrap();
        let loaded = RegistryStore::load(&path).unwrap();
        assert!(loaded.registry.servers.is_empty());
    }

    #[test]
    fn detects_concurrent_modification() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cerberus.yaml");
        let mut store = RegistryStore {
            path: path.clone(),
            registry: Registry::default(),
            content_hash: 0,
        };
        store.save().unwrap();
        store.detect_concurrent_modification().unwrap();
        std::fs::write(&path, "schema_version: 1\nservers: {}\n").unwrap();
        assert!(store.detect_concurrent_modification().is_err());
    }

    #[test]
    fn atomic_write_refuses_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("real.yaml");
        std::fs::write(&target, "x").unwrap();
        let link = dir.path().join("link.yaml");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &link).unwrap();
        #[cfg(unix)]
        assert!(atomic_write(&link, b"y").is_err());
    }

    #[test]
    fn hash_is_order_independent_for_json() {
        let a: serde_json::Value = serde_json::from_str(r#"{"a":1,"b":2}"#).unwrap();
        let b: serde_json::Value = serde_json::from_str(r#"{"b":2,"a":1}"#).unwrap();
        assert_eq!(hash_value(&a), hash_value(&b));
    }
}
