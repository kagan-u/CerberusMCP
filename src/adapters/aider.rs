//! Aider adapter — documented as an unsupported integration.
//!
//! Aider has **no native MCP client support** as of October 2026. Evidence:
//! - Umbrella issue Aider-AI/aider#3314 "MCP SUPPORT" remains open (Feb 2025 → present).
//! - PRs #3672 and #3937 were closed without merge; no MCP code exists in the
//!   main tree.
//! - No MCP documentation page on aider.chat.
//!
//! CerberusMCP does not invent a configuration format. This adapter exists so
//! `cerberus client list` reports the limitation truthfully and so
//! `cerberus client sync aider` fails with an actionable message rather than
//! pretending to succeed. Documented alternatives: AiderDesk (GUI wrapper
//! with MCP client support) or running Aider itself as an MCP server via
//! third-party bridges.

use std::path::PathBuf;

use super::{
    ClientAdapter, ClientInspection, ClientMeta, ClientStatus, ServerSummary, SyncContext,
    SyncPlan, SyncReport, detect_by_config_path,
};
use crate::error::{CerberusError, Result};
use crate::paths::{CerberusPaths, home_path};

#[derive(Debug, Clone, Copy)]
pub struct AiderAdapter;

const CLIENT: &str = "aider";

impl AiderAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for AiderAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl ClientAdapter for AiderAdapter {
    fn meta(&self) -> &ClientMeta {
        static META: std::sync::OnceLock<ClientMeta> = std::sync::OnceLock::new();
        META.get_or_init(|| ClientMeta {
            id: super::ClientId::Aider,
            display_name: "Aider",
            supported: false,
            unsupported_reason: Some(
                "Aider has no native MCP client support (Aider-AI/aider#3314); \
                 CerberusMCP will not write an invented configuration format",
            ),
            config_format: "n/a",
            requires_restart: true,
            limitations: &[
                "No native MCP support in Aider as of October 2026",
                "See docs/client-integrations.md for verified alternatives (AiderDesk, bridges)",
            ],
        })
    }

    fn config_path(&self, _paths: &CerberusPaths) -> Result<PathBuf> {
        // Aider's own config (~/.aider.conf.yml) exists but has no MCP keys;
        // we surface it for detection only.
        home_path(&[".aider.conf.yml"])
    }

    async fn detect(&self, paths: &CerberusPaths) -> Result<ClientStatus> {
        let path = self.config_path(paths)?;
        let mut status = detect_by_config_path(CLIENT, &path).await?;
        // Also detect the binary on PATH.
        let binary_found = ["aider", "aider.exe"].iter().any(|bin| {
            std::env::var_os("PATH")
                .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join(bin).exists()))
                .unwrap_or(false)
        });
        if binary_found {
            status.detected = true;
            status.notes.push("aider binary found on PATH".into());
        }
        status
            .notes
            .push("unsupported integration: Aider has no native MCP client support".into());
        Ok(status)
    }

    async fn inspect(&self, paths: &CerberusPaths) -> Result<ClientInspection> {
        let path = self.config_path(paths)?;
        let status = self.detect(paths).await?;
        Ok(ClientInspection {
            client: CLIENT.into(),
            display_name: self.meta().display_name.into(),
            supported: false,
            detected: status.detected,
            config_path: Some(path),
            servers: Vec::<ServerSummary>::new(),
            limitations: self
                .meta()
                .limitations
                .iter()
                .map(|s| s.to_string())
                .collect(),
            notes: status.notes,
        })
    }

    async fn plan_sync(&self, _ctx: &SyncContext<'_>) -> Result<SyncPlan> {
        Err(CerberusError::UnsupportedClient {
            name: CLIENT.into(),
            reason: self
                .meta()
                .unsupported_reason
                .unwrap_or("unsupported")
                .to_string(),
        })
    }

    async fn apply_sync(&self, _ctx: &SyncContext<'_>, _plan: &SyncPlan) -> Result<SyncReport> {
        Err(CerberusError::UnsupportedClient {
            name: CLIENT.into(),
            reason: self
                .meta()
                .unsupported_reason
                .unwrap_or("unsupported")
                .to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::{ApplyOptions, ClientManagedState};

    #[tokio::test]
    async fn plan_sync_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let paths = CerberusPaths::with_roots(dir.path().join("c"), dir.path().join("d"));
        let secrets = crate::secrets::SecretStore::default();
        let servers = std::collections::BTreeMap::new();
        let managed = ClientManagedState::default();
        let ctx = SyncContext {
            paths: &paths,
            secrets: &secrets,
            servers: &servers,
            managed: &managed,
            options: ApplyOptions::default(),
        };
        let err = AiderAdapter.plan_sync(&ctx).await.unwrap_err();
        assert!(matches!(err, CerberusError::UnsupportedClient { .. }));
        assert_eq!(err.exit_code(), crate::error::ExitCode::Unsupported);
    }

    #[tokio::test]
    async fn reports_unsupported_status() {
        let dir = tempfile::tempdir().unwrap();
        let paths = CerberusPaths::with_roots(dir.path().join("c"), dir.path().join("d"));
        let status = AiderAdapter.detect(&paths).await.unwrap();
        assert!(status.notes.iter().any(|n| n.contains("unsupported")));
    }
}
