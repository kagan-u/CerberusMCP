//! Diagnostics: doctor, health checks, and log inspection.

use std::path::PathBuf;
use std::time::Duration;

use serde::Serialize;

use crate::adapters::{Adapter, ClientStatus};
use crate::error::Result;
use crate::paths::CerberusPaths;
use crate::process;
use crate::registry::{RegistryStore, validate};
use crate::secrets::{SecretStore, collect_literal_secrets, redact_secrets};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Critical,
}

#[derive(Debug, Clone, Serialize)]
pub struct Diagnostic {
    pub severity: Severity,
    pub area: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClientDiagnostic {
    pub client: String,
    pub display_name: String,
    pub status: String,
    pub detected: bool,
    pub supported: bool,
    pub servers: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DoctorReport {
    pub config_path: PathBuf,
    pub config_valid: bool,
    pub schema_version: Option<u32>,
    pub server_count: usize,
    pub clients: Vec<ClientDiagnostic>,
    pub diagnostics: Vec<Diagnostic>,
    pub warnings: usize,
    pub critical: usize,
}

pub struct DoctorOptions {
    /// Restrict client checks to a single client.
    pub client: Option<String>,
    /// Probe server processes (can spawn processes).
    pub probe_servers: bool,
}

/// Run the full doctor sweep.
pub async fn doctor(
    paths: &CerberusPaths,
    opts: &DoctorOptions,
    secrets: &SecretStore,
) -> Result<DoctorReport> {
    let mut diagnostics = Vec::new();
    let mut config_valid = true;
    let mut schema_version = None;
    let mut server_count = 0;

    if !paths.registry_file.exists() {
        diagnostics.push(Diagnostic {
            severity: Severity::Warning,
            area: "config".into(),
            message: format!(
                "registry not found at {}; run `cerberus init`",
                paths.registry_file.display()
            ),
        });
        config_valid = false;
    } else {
        match RegistryStore::load(&paths.registry_file) {
            Ok(store) => {
                schema_version = Some(store.registry.schema_version);
                server_count = store.registry.servers.len();
                if let Err(e) = validate::validate(&store.registry) {
                    config_valid = false;
                    for line in e.to_string().lines() {
                        diagnostics.push(Diagnostic {
                            severity: Severity::Critical,
                            area: "config".into(),
                            message: line.to_string(),
                        });
                    }
                }

                // Missing environment references.
                for (name, def) in &store.registry.servers {
                    if let crate::registry::schema::Transport::Stdio { env, command, .. } =
                        &def.transport
                    {
                        if !process::executable_exists(command) {
                            diagnostics.push(Diagnostic {
                                severity: Severity::Warning,
                                area: "server".into(),
                                message: format!(
                                    "server `{name}`: executable `{command}` not found on PATH"
                                ),
                            });
                        }
                        for (key, value) in env {
                            if let crate::registry::schema::EnvValue::Env { name: var } = value {
                                if std::env::var(var).is_err() {
                                    diagnostics.push(Diagnostic {
                                        severity: Severity::Warning,
                                        area: "server".into(),
                                        message: format!(
                                            "server `{name}`: env `{key}` references unset variable `{var}`"
                                        ),
                                    });
                                }
                            }
                            if let crate::registry::schema::EnvValue::SecretRef { name: s } = value
                            {
                                if secrets.resolve(s).is_err() {
                                    diagnostics.push(Diagnostic {
                                        severity: Severity::Warning,
                                        area: "server".into(),
                                        message: format!(
                                            "server `{name}`: env `{key}` references unresolved secret `{s}`"
                                        ),
                                    });
                                }
                            }
                        }
                    }

                    // Install provenance: variables the server documents as
                    // required. Already-bound ones are covered by the check
                    // above, so this only reports truly unbound requirements.
                    if let Some(source) = &def.source {
                        let bound: Vec<&str> = match &def.transport {
                            crate::registry::schema::Transport::Stdio { env, .. } => {
                                env.keys().map(String::as_str).collect()
                            }
                            crate::registry::schema::Transport::Http { .. } => Vec::new(),
                        };
                        for variable in source.env.iter().filter(|v| v.required) {
                            if std::env::var_os(&variable.name).is_none()
                                && !bound.contains(&variable.name.as_str())
                            {
                                diagnostics.push(Diagnostic {
                                    severity: Severity::Warning,
                                    area: "server".into(),
                                    message: format!(
                                        "server `{name}`: source requires unset variable `{}`",
                                        variable.name
                                    ),
                                });
                            }
                        }
                    }

                    if opts.probe_servers && def.enabled {
                        if let crate::registry::schema::Transport::Stdio { .. } = &def.transport {
                            match process::probe_stdio_liveness(
                                def,
                                secrets,
                                Duration::from_secs(3),
                            )
                            .await
                            {
                                Ok(()) => {}
                                Err(e) => diagnostics.push(Diagnostic {
                                    severity: Severity::Warning,
                                    area: "server".into(),
                                    message: format!(
                                        "server `{name}` health probe failed: {}",
                                        redact_secrets(
                                            &e.to_string(),
                                            &collect_literal_secrets(&store.registry)
                                        )
                                    ),
                                }),
                            }
                        }
                    }
                }

                // Unresolved references in profiles.
                for (pname, profile) in &store.registry.profiles {
                    for s in &profile.servers {
                        if !store.registry.servers.contains_key(s) {
                            diagnostics.push(Diagnostic {
                                severity: Severity::Critical,
                                area: "profile".into(),
                                message: format!(
                                    "profile `{pname}` references unknown server `{s}`"
                                ),
                            });
                        }
                    }
                }
            }
            Err(e) => {
                config_valid = false;
                diagnostics.push(Diagnostic {
                    severity: Severity::Critical,
                    area: "config".into(),
                    message: e.to_string(),
                });
            }
        }
    }

    // Gateway state.
    let gateway_file = paths.gateway_state_file();
    if gateway_file.exists() {
        match std::fs::read_to_string(&gateway_file) {
            Ok(text) => {
                if let Ok(state) = serde_json::from_str::<serde_json::Value>(&text) {
                    let running = state
                        .get("pid")
                        .and_then(|p| p.as_u64())
                        .map(|pid| pid_running(pid as u32))
                        .unwrap_or(false);
                    if !running {
                        diagnostics.push(Diagnostic {
                            severity: Severity::Info,
                            area: "gateway".into(),
                            message: "stale gateway state file found (process not running)".into(),
                        });
                    }
                }
            }
            Err(e) => diagnostics.push(Diagnostic {
                severity: Severity::Warning,
                area: "gateway".into(),
                message: format!("could not read gateway state: {e}"),
            }),
        }
    }

    // Clients.
    let mut clients = Vec::new();
    let adapters: Vec<Adapter> = match &opts.client {
        Some(name) => {
            let id = crate::adapters::ClientId::parse(name).ok_or_else(|| {
                crate::error::CerberusError::ClientNotFound { name: name.clone() }
            })?;
            vec![Adapter::by_id(id)?]
        }
        None => Adapter::all(),
    };

    for adapter in adapters {
        let meta = adapter.meta();
        let status: ClientStatus = adapter.detect(paths).await.unwrap_or(ClientStatus {
            client: meta.id.as_str().into(),
            detected: false,
            config_path: None,
            notes: vec!["detection failed".into()],
        });
        let servers = if meta.supported && status.detected {
            adapter
                .inspect(paths)
                .await
                .map(|i| i.servers.len())
                .unwrap_or(0)
        } else {
            0
        };
        let status_str = if !meta.supported {
            "unsupported integration".to_string()
        } else if status.detected {
            "detected".to_string()
        } else {
            "not detected".to_string()
        };
        clients.push(ClientDiagnostic {
            client: meta.id.as_str().into(),
            display_name: meta.display_name.into(),
            status: status_str,
            detected: status.detected,
            supported: meta.supported,
            servers,
            notes: status.notes,
        });
    }

    let warnings = diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Warning)
        .count();
    let critical = diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Critical)
        .count();

    Ok(DoctorReport {
        config_path: paths.registry_file.clone(),
        config_valid,
        schema_version,
        server_count,
        clients,
        diagnostics,
        warnings,
        critical,
    })
}

#[cfg(unix)]
fn pid_running(pid: u32) -> bool {
    #[cfg(target_os = "linux")]
    {
        std::path::Path::new(&format!("/proc/{pid}")).exists()
    }
    #[cfg(not(target_os = "linux"))]
    {
        // macOS/BSD: best-effort probe via kill -0.
        std::process::Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

#[cfg(windows)]
fn pid_running(_pid: u32) -> bool {
    false
}

/// `cerberus health` — quick status without spawning client detection.
#[derive(Debug, Serialize)]
pub struct HealthReport {
    pub config_valid: bool,
    pub servers: Vec<ServerHealth>,
}

#[derive(Debug, Serialize)]
pub struct ServerHealth {
    pub name: String,
    pub enabled: bool,
    pub transport: String,
    pub executable_found: Option<bool>,
}

pub fn health(paths: &CerberusPaths) -> Result<HealthReport> {
    if !paths.registry_file.exists() {
        return Ok(HealthReport {
            config_valid: false,
            servers: Vec::new(),
        });
    }
    let store = RegistryStore::load(&paths.registry_file)?;
    let valid = validate::validate(&store.registry).is_ok();
    let servers = store
        .registry
        .servers
        .iter()
        .map(|(name, def)| {
            let executable_found = match &def.transport {
                crate::registry::schema::Transport::Stdio { command, .. } => {
                    Some(process::executable_exists(command))
                }
                crate::registry::schema::Transport::Http { .. } => None,
            };
            ServerHealth {
                name: name.clone(),
                enabled: def.enabled,
                transport: def.transport.kind().to_string(),
                executable_found,
            }
        })
        .collect();
    Ok(HealthReport {
        config_valid: valid,
        servers,
    })
}

/// Read the most recent log lines from the log directory.
pub fn recent_logs(paths: &CerberusPaths, lines: usize) -> Result<Vec<String>> {
    let dir = paths.logs_dir();
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map_err(|e| crate::error::CerberusError::Other(e.to_string()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("log"))
        .collect();
    files.sort();
    let Some(latest) = files.pop() else {
        return Ok(Vec::new());
    };
    let text = std::fs::read_to_string(&latest)
        .map_err(|e| crate::error::CerberusError::Other(e.to_string()))?;
    let all: Vec<&str> = text.lines().collect();
    let start = all.len().saturating_sub(lines);
    Ok(all[start..].iter().map(|s| s.to_string()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_on_missing_config_is_soft() {
        let dir = tempfile::tempdir().unwrap();
        let paths = CerberusPaths::with_roots(dir.path().join("c"), dir.path().join("d"));
        let report = health(&paths).unwrap();
        assert!(!report.config_valid);
    }

    #[tokio::test]
    async fn doctor_runs_on_empty_env() {
        let dir = tempfile::tempdir().unwrap();
        let paths = CerberusPaths::with_roots(dir.path().join("c"), dir.path().join("d"));
        paths.ensure_dirs().unwrap();
        // Write a valid empty registry so config checks pass.
        let mut store = RegistryStore {
            path: paths.registry_file.clone(),
            registry: Default::default(),
            content_hash: 0,
        };
        store.save().unwrap();
        let secrets = SecretStore::default();
        let report = doctor(
            &paths,
            &DoctorOptions {
                client: None,
                probe_servers: false,
            },
            &secrets,
        )
        .await
        .unwrap();
        assert!(report.config_valid);
        assert_eq!(report.clients.len(), 6);
        let aider = report.clients.iter().find(|c| c.client == "aider").unwrap();
        assert_eq!(aider.status, "unsupported integration");
    }
}
