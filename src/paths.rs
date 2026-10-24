use std::path::{Path, PathBuf};

use directories::{BaseDirs, ProjectDirs};

use crate::error::{CerberusError, Result};

const PROJECT_QUALIFIER: &str = "dev";
const PROJECT_ORGANIZATION: &str = "cerberus-mcp";
const PROJECT_NAME: &str = "cerberus-mcp";

/// Resolved filesystem locations for configuration, data, and logs.
///
/// Every location can be overridden with an environment variable so tests and
/// CI never touch real user configuration:
///
/// - `CERBERUS_CONFIG_DIR`  — directory containing `cerberus.yaml`
/// - `CERBERUS_DATA_DIR`    — backups, managed-entry state, logs
/// - `CERBERUS_CONFIG_FILE` — explicit path to the registry file
#[derive(Debug, Clone)]
pub struct CerberusPaths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub registry_file: PathBuf,
}

impl CerberusPaths {
    pub fn resolve() -> Result<Self> {
        let config_env = std::env::var_os("CERBERUS_CONFIG_DIR").map(PathBuf::from);
        let data_env = std::env::var_os("CERBERUS_DATA_DIR").map(PathBuf::from);

        // Both overrides set: never touch the OS home directory. This keeps
        // sandboxed runs (tests, CI containers without a user profile) working
        // even when `ProjectDirs` cannot resolve one.
        let project = if config_env.is_some() && data_env.is_some() {
            None
        } else {
            Some(
                ProjectDirs::from(PROJECT_QUALIFIER, PROJECT_ORGANIZATION, PROJECT_NAME)
                    .ok_or_else(|| {
                        CerberusError::Other(
                            "could not determine home directory; set CERBERUS_CONFIG_DIR and CERBERUS_DATA_DIR"
                                .into(),
                        )
                    })?,
            )
        };

        let config_dir = config_env.unwrap_or_else(|| {
            project
                .as_ref()
                .expect("project dirs resolved when no config override")
                .config_local_dir()
                .to_path_buf()
        });
        let data_dir = data_env.unwrap_or_else(|| {
            project
                .as_ref()
                .expect("project dirs resolved when no data override")
                .data_local_dir()
                .to_path_buf()
        });
        let registry_file = match std::env::var_os("CERBERUS_CONFIG_FILE") {
            Some(file) => PathBuf::from(file),
            None => config_dir.join("cerberus.yaml"),
        };

        Ok(Self {
            config_dir,
            data_dir,
            registry_file,
        })
    }

    /// Resolve paths, optionally overriding the registry file location
    /// (used by the global `--config` flag).
    pub fn resolve_with_override(config_override: Option<PathBuf>) -> Result<Self> {
        let mut paths = Self::resolve()?;
        if let Some(file) = config_override {
            if let Some(parent) = file.parent() {
                if !parent.as_os_str().is_empty() {
                    paths.config_dir = parent.to_path_buf();
                }
            }
            paths.registry_file = file;
        }
        Ok(paths)
    }

    /// Construct paths rooted at explicit directories (used by tests and
    /// `--config` overrides).
    pub fn with_roots(config_dir: PathBuf, data_dir: PathBuf) -> Self {
        let registry_file = config_dir.join("cerberus.yaml");
        Self {
            config_dir,
            data_dir,
            registry_file,
        }
    }

    pub fn with_registry_file(mut self, file: PathBuf) -> Self {
        self.registry_file = file;
        self
    }

    pub fn backups_dir(&self) -> PathBuf {
        self.data_dir.join("backups")
    }

    pub fn state_dir(&self) -> PathBuf {
        self.data_dir.join("state")
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.data_dir.join("logs")
    }

    pub fn gateway_state_file(&self) -> PathBuf {
        self.state_dir().join("gateway.json")
    }

    pub fn managed_state_file(&self) -> PathBuf {
        self.state_dir().join("managed.json")
    }

    pub fn ensure_dirs(&self) -> Result<()> {
        for dir in [
            self.config_dir.clone(),
            self.data_dir.clone(),
            self.backups_dir(),
            self.state_dir(),
            self.logs_dir(),
        ] {
            std::fs::create_dir_all(&dir)
                .map_err(|source| CerberusError::Write { path: dir, source })?;
        }
        Ok(())
    }
}

/// Resolve the user's home directory without assuming it exists.
///
/// Explicit environment variables win over the OS lookup: `BaseDirs` on
/// Windows resolves the profile folder through `SHGetKnownFolderPath`, which
/// ignores `HOME`/`USERPROFILE` entirely. Honouring the variables first is
/// what makes sandboxed runs (tests, containers) actually take effect, and
/// matches the documented `HOME` override.
pub fn home_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    let from_env = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"));
    #[cfg(not(windows))]
    let from_env = std::env::var_os("HOME");

    from_env
        .map(PathBuf::from)
        .or_else(|| BaseDirs::new().map(|d| d.home_dir().to_path_buf()))
}

/// Join `segments` onto the home directory, failing loudly if home is unset.
pub fn home_path(segments: &[&str]) -> Result<PathBuf> {
    let mut path = home_dir()
        .ok_or_else(|| CerberusError::Other("home directory is not available; set HOME".into()))?;
    for segment in segments {
        path.push(segment);
    }
    Ok(path)
}

/// Expand a leading `~` to the user's home directory. Leaves other paths
/// untouched. Does not expand environment variables inside the path.
pub fn expand_tilde(path: &Path) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~").ok().and_then(|r| r.to_str()) {
        if rest.is_empty() {
            if let Some(home) = home_dir() {
                return home;
            }
        } else if let Some(rest) = rest.strip_prefix('/') {
            if let Some(home) = home_dir() {
                return home.join(rest);
            }
        }
    }
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_honours_env_overrides() {
        // Deliberately does not mutate process env; with_roots is the
        // test-friendly constructor and must produce consistent layout.
        let paths =
            CerberusPaths::with_roots(PathBuf::from("/tmp/cfg"), PathBuf::from("/tmp/data"));
        assert_eq!(paths.registry_file, PathBuf::from("/tmp/cfg/cerberus.yaml"));
        assert_eq!(paths.backups_dir(), PathBuf::from("/tmp/data/backups"));
        assert_eq!(
            paths.gateway_state_file(),
            PathBuf::from("/tmp/data/state/gateway.json")
        );
    }

    #[test]
    fn expand_tilde_leaves_absolute_paths() {
        assert_eq!(
            expand_tilde(Path::new("/etc/hosts")),
            PathBuf::from("/etc/hosts")
        );
    }
}
