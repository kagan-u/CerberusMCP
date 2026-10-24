//! Synchronization engine.
//!
//! Orchestrates adapter planning and application with transaction-like
//! semantics: dry-run by default, backup before write, atomic replace,
//! post-write parse verification, and ownership tracking.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::adapters::{
    Adapter, ApplyOptions, ChangeKind, ClientManagedState, ManagedState, SyncContext, SyncPlan,
    SyncReport,
};
use crate::error::{CerberusError, Result};
use crate::paths::CerberusPaths;
use crate::registry::RegistryStore;
use crate::registry::schema::ServerDef;
use crate::secrets::SecretStore;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncMode {
    DryRun,
    Apply,
}

pub struct SyncRequest {
    pub clients: Vec<Adapter>,
    pub mode: SyncMode,
    pub profile: Option<String>,
    pub options: ApplyOptions,
}

#[derive(Debug, Serialize)]
pub struct ClientSyncOutcome {
    pub client: String,
    pub plan: SyncPlan,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report: Option<SyncReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SyncOutcome {
    pub mode: SyncMode,
    pub results: Vec<ClientSyncOutcome>,
}

impl SyncOutcome {
    pub fn has_errors(&self) -> bool {
        self.results.iter().any(|r| r.error.is_some())
    }

    pub fn total_changes(&self) -> usize {
        self.results
            .iter()
            .filter_map(|r| r.report.as_ref())
            .map(|rep| {
                rep.applied
                    .iter()
                    .filter(|a| !matches!(a.change, ChangeKind::Unchanged))
                    .count()
            })
            .sum()
    }
}

/// Load the servers selected by the profile, including disabled ones (their
/// enabled flag is part of client state).
pub fn selected_servers(
    store: &RegistryStore,
    profile: Option<&str>,
) -> Result<BTreeMap<String, ServerDef>> {
    // Include ALL servers so enable/disable state can be represented; the
    // profile only filters membership.
    let profile_name = profile
        .map(str::to_string)
        .or_else(|| store.registry.active_profile.clone());

    let Some(name) = profile_name else {
        return Ok(store.registry.servers.clone());
    };

    let profile = store
        .registry
        .profiles
        .get(&name)
        .ok_or_else(|| CerberusError::ProfileNotFound { name: name.clone() })?;

    let mut out = BTreeMap::new();
    for server in &profile.servers {
        let def = store.registry.servers.get(server).ok_or_else(|| {
            CerberusError::Validation(format!(
                "profile `{name}` references unknown server `{server}`"
            ))
        })?;
        out.insert(server.clone(), def.clone());
    }
    Ok(out)
}

pub async fn run(
    paths: &CerberusPaths,
    store: &RegistryStore,
    secrets: &SecretStore,
    request: SyncRequest,
) -> Result<SyncOutcome> {
    let servers = selected_servers(store, request.profile.as_deref())?;
    crate::registry::validate::validate(&store.registry)?;

    let state_path = paths.managed_state_file();
    let managed_state = ManagedState::load(&state_path)?;

    let mut results = Vec::new();
    for adapter in request.clients {
        let client_id = adapter.id().as_str().to_string();
        let managed = managed_state
            .clients
            .get(&client_id)
            .cloned()
            .unwrap_or_default();

        let ctx = SyncContext {
            paths,
            secrets,
            servers: &servers,
            managed: &managed,
            options: request.options.clone(),
        };

        let plan = match adapter.plan_sync(&ctx).await {
            Ok(plan) => plan,
            Err(e) => {
                results.push(ClientSyncOutcome {
                    client: client_id.clone(),
                    plan: SyncPlan {
                        client: client_id.clone(),
                        config_path: adapter.config_path(paths).unwrap_or_default(),
                        actions: Vec::new(),
                        warnings: Vec::new(),
                        has_changes: false,
                    },
                    report: None,
                    error: Some(e.to_string()),
                });
                continue;
            }
        };

        match request.mode {
            SyncMode::DryRun => results.push(ClientSyncOutcome {
                client: client_id,
                plan,
                report: None,
                error: None,
            }),
            SyncMode::Apply => {
                if !plan.has_changes {
                    results.push(ClientSyncOutcome {
                        client: client_id,
                        plan,
                        report: None,
                        error: None,
                    });
                    continue;
                }
                match adapter.apply_sync(&ctx, &plan).await {
                    Ok(report) => results.push(ClientSyncOutcome {
                        client: client_id,
                        plan,
                        report: Some(report),
                        error: None,
                    }),
                    Err(e) => results.push(ClientSyncOutcome {
                        client: client_id,
                        plan,
                        report: None,
                        error: Some(e.to_string()),
                    }),
                }
            }
        }
    }

    Ok(SyncOutcome {
        mode: request.mode,
        results,
    })
}

/// Load ownership state for one client (used by diff/inspect).
pub fn load_managed(paths: &CerberusPaths, client: &str) -> Result<ClientManagedState> {
    Ok(ManagedState::load(&paths.managed_state_file())?
        .clients
        .get(client)
        .cloned()
        .unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::schema::*;

    #[test]
    fn selected_servers_respects_profile() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cerberus.yaml");
        let mut store = RegistryStore {
            path: path.clone(),
            registry: Registry::default(),
            content_hash: 0,
        };
        for name in ["a", "b"] {
            store.registry.servers.insert(
                name.into(),
                ServerDef {
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
                },
            );
        }
        store.registry.profiles.insert(
            "dev".into(),
            Profile {
                servers: vec!["a".into()],
                description: None,
            },
        );
        let selected = selected_servers(&store, Some("dev")).unwrap();
        assert_eq!(selected.keys().collect::<Vec<_>>(), vec!["a"]);

        let all = selected_servers(&store, None).unwrap();
        assert_eq!(all.len(), 2);
    }
}
