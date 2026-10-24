//! Client adapter framework.
//!
//! Every AI CLI integration implements [`ClientAdapter`]. Adapters translate
//! the canonical registry into each client's native configuration format,
//! preserving unrelated user-owned fields.

pub mod aider;
pub mod claude_code;
pub mod cline;
pub mod codex;
pub mod gemini_cli;
pub mod opencode;

use std::collections::BTreeMap;
use std::future::Future;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::{CerberusError, Result};
use crate::paths::CerberusPaths;
use crate::registry::hash_value;
use crate::registry::schema::ServerDef;
use crate::secrets::SecretStore;

/// Identifier for a supported client.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ClientId {
    OpenCode,
    ClaudeCode,
    Codex,
    GeminiCli,
    Aider,
    Cline,
}

impl ClientId {
    pub const ALL: [ClientId; 6] = [
        ClientId::OpenCode,
        ClientId::ClaudeCode,
        ClientId::Codex,
        ClientId::GeminiCli,
        ClientId::Aider,
        ClientId::Cline,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            ClientId::OpenCode => "opencode",
            ClientId::ClaudeCode => "claude-code",
            ClientId::Codex => "codex",
            ClientId::GeminiCli => "gemini-cli",
            ClientId::Aider => "aider",
            ClientId::Cline => "cline",
        }
    }

    pub fn parse(s: &str) -> Option<ClientId> {
        match s {
            "opencode" => Some(ClientId::OpenCode),
            "claude-code" | "claude" => Some(ClientId::ClaudeCode),
            "codex" => Some(ClientId::Codex),
            "gemini-cli" | "gemini" => Some(ClientId::GeminiCli),
            "aider" => Some(ClientId::Aider),
            "cline" => Some(ClientId::Cline),
            _ => None,
        }
    }
}

impl std::fmt::Display for ClientId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Static metadata about a client integration.
#[derive(Debug, Clone)]
pub struct ClientMeta {
    pub id: ClientId,
    pub display_name: &'static str,
    /// False when CerberusMCP cannot provide a legitimate integration path.
    pub supported: bool,
    /// Human-readable reason shown for unsupported clients.
    pub unsupported_reason: Option<&'static str>,
    pub config_format: &'static str,
    /// Whether the client must be restarted to pick up config changes.
    pub requires_restart: bool,
    pub limitations: &'static [&'static str],
}

/// Result of probing whether a client exists on this machine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientStatus {
    pub client: String,
    pub detected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

/// Deeper look at a client's current configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientInspection {
    pub client: String,
    pub display_name: String,
    pub supported: bool,
    pub detected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_path: Option<PathBuf>,
    /// Names of MCP servers currently present in the client config.
    pub servers: Vec<ServerSummary>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub limitations: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerSummary {
    pub name: String,
    pub transport: String,
    /// True when this entry is managed by CerberusMCP.
    pub managed: bool,
    pub enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Add,
    Update,
    Remove,
    Unchanged,
    /// Entry exists but is owned/edited by the user and conflicts.
    Conflict,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncAction {
    pub server: String,
    pub change: ChangeKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncPlan {
    pub client: String,
    pub config_path: PathBuf,
    pub actions: Vec<SyncAction>,
    pub warnings: Vec<String>,
    /// True when applying would change the file.
    pub has_changes: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncReport {
    pub client: String,
    pub config_path: PathBuf,
    pub applied: Vec<SyncAction>,
    pub skipped: Vec<SyncAction>,
    pub warnings: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup_id: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ApplyOptions {
    /// Proceed even when managed entries were edited by hand.
    pub force: bool,
}

/// Everything an adapter needs to compute a sync plan.
pub struct SyncContext<'a> {
    pub paths: &'a CerberusPaths,
    pub secrets: &'a SecretStore,
    /// Servers to synchronize, keyed by registry name (already filtered by
    /// profile selection; disabled servers included so their `enabled` flag
    /// can be represented).
    pub servers: &'a BTreeMap<String, ServerDef>,
    /// Ownership records for this client.
    pub managed: &'a ClientManagedState,
    pub options: ApplyOptions,
}

/// Ownership record for a single managed client entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedEntry {
    /// Canonical hash of the entry we last wrote.
    pub hash: u64,
    pub written_unix: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClientManagedState {
    #[serde(default)]
    pub servers: BTreeMap<String, ManagedEntry>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ManagedState {
    #[serde(default)]
    pub clients: BTreeMap<String, ClientManagedState>,
}

impl ManagedState {
    pub fn load(path: &std::path::Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(path).map_err(|source| CerberusError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        serde_json::from_str(&text).map_err(|e| CerberusError::Other(format!("managed state: {e}")))
    }

    pub fn save(&self, path: &std::path::Path) -> Result<()> {
        let text = serde_json::to_string_pretty(self)
            .map_err(|e| CerberusError::Other(format!("managed state: {e}")))?;
        crate::registry::atomic_write(path, text.as_bytes())
    }
}

/// Common adapter behavior. Implemented per client; dispatched through
/// [`Adapter`].
pub trait ClientAdapter {
    fn meta(&self) -> &ClientMeta;

    /// Location of the configuration file this adapter manages.
    fn config_path(&self, paths: &CerberusPaths) -> Result<PathBuf>;

    fn detect(&self, paths: &CerberusPaths) -> impl Future<Output = Result<ClientStatus>> + Send;

    fn inspect(
        &self,
        paths: &CerberusPaths,
    ) -> impl Future<Output = Result<ClientInspection>> + Send;

    fn plan_sync(&self, ctx: &SyncContext<'_>) -> impl Future<Output = Result<SyncPlan>> + Send;

    fn apply_sync(
        &self,
        ctx: &SyncContext<'_>,
        plan: &SyncPlan,
    ) -> impl Future<Output = Result<SyncReport>> + Send;
}

/// Enum dispatch over all known adapters.
#[derive(Debug)]
pub enum Adapter {
    OpenCode(opencode::OpenCodeAdapter),
    ClaudeCode(claude_code::ClaudeCodeAdapter),
    Codex(codex::CodexAdapter),
    GeminiCli(gemini_cli::GeminiCliAdapter),
    Aider(aider::AiderAdapter),
    Cline(cline::ClineAdapter),
}

impl Adapter {
    pub fn by_id(id: ClientId) -> Result<Self> {
        Ok(match id {
            ClientId::OpenCode => Adapter::OpenCode(opencode::OpenCodeAdapter),
            ClientId::ClaudeCode => Adapter::ClaudeCode(claude_code::ClaudeCodeAdapter),
            ClientId::Codex => Adapter::Codex(codex::CodexAdapter),
            ClientId::GeminiCli => Adapter::GeminiCli(gemini_cli::GeminiCliAdapter),
            ClientId::Aider => Adapter::Aider(aider::AiderAdapter),
            ClientId::Cline => Adapter::Cline(cline::ClineAdapter),
        })
    }

    pub fn all() -> Vec<Adapter> {
        ClientId::ALL
            .iter()
            .filter_map(|id| Self::by_id(*id).ok())
            .collect()
    }

    /// Adapters that can actually synchronize configuration.
    pub fn supported() -> Vec<Adapter> {
        Self::all()
            .into_iter()
            .filter(|a| a.meta().supported)
            .collect()
    }

    pub fn meta(&self) -> &ClientMeta {
        match self {
            Adapter::OpenCode(a) => a.meta(),
            Adapter::ClaudeCode(a) => a.meta(),
            Adapter::Codex(a) => a.meta(),
            Adapter::GeminiCli(a) => a.meta(),
            Adapter::Aider(a) => a.meta(),
            Adapter::Cline(a) => a.meta(),
        }
    }

    pub fn id(&self) -> ClientId {
        self.meta().id
    }

    pub fn config_path(&self, paths: &CerberusPaths) -> Result<PathBuf> {
        match self {
            Adapter::OpenCode(a) => a.config_path(paths),
            Adapter::ClaudeCode(a) => a.config_path(paths),
            Adapter::Codex(a) => a.config_path(paths),
            Adapter::GeminiCli(a) => a.config_path(paths),
            Adapter::Aider(a) => a.config_path(paths),
            Adapter::Cline(a) => a.config_path(paths),
        }
    }

    pub async fn detect(&self, paths: &CerberusPaths) -> Result<ClientStatus> {
        match self {
            Adapter::OpenCode(a) => a.detect(paths).await,
            Adapter::ClaudeCode(a) => a.detect(paths).await,
            Adapter::Codex(a) => a.detect(paths).await,
            Adapter::GeminiCli(a) => a.detect(paths).await,
            Adapter::Aider(a) => a.detect(paths).await,
            Adapter::Cline(a) => a.detect(paths).await,
        }
    }

    pub async fn inspect(&self, paths: &CerberusPaths) -> Result<ClientInspection> {
        match self {
            Adapter::OpenCode(a) => a.inspect(paths).await,
            Adapter::ClaudeCode(a) => a.inspect(paths).await,
            Adapter::Codex(a) => a.inspect(paths).await,
            Adapter::GeminiCli(a) => a.inspect(paths).await,
            Adapter::Aider(a) => a.inspect(paths).await,
            Adapter::Cline(a) => a.inspect(paths).await,
        }
    }

    pub async fn plan_sync(&self, ctx: &SyncContext<'_>) -> Result<SyncPlan> {
        match self {
            Adapter::OpenCode(a) => a.plan_sync(ctx).await,
            Adapter::ClaudeCode(a) => a.plan_sync(ctx).await,
            Adapter::Codex(a) => a.plan_sync(ctx).await,
            Adapter::GeminiCli(a) => a.plan_sync(ctx).await,
            Adapter::Aider(a) => a.plan_sync(ctx).await,
            Adapter::Cline(a) => a.plan_sync(ctx).await,
        }
    }

    pub async fn apply_sync(&self, ctx: &SyncContext<'_>, plan: &SyncPlan) -> Result<SyncReport> {
        match self {
            Adapter::OpenCode(a) => a.apply_sync(ctx, plan).await,
            Adapter::ClaudeCode(a) => a.apply_sync(ctx, plan).await,
            Adapter::Codex(a) => a.apply_sync(ctx, plan).await,
            Adapter::GeminiCli(a) => a.apply_sync(ctx, plan).await,
            Adapter::Aider(a) => a.apply_sync(ctx, plan).await,
            Adapter::Cline(a) => a.apply_sync(ctx, plan).await,
        }
    }
}

// ---------------------------------------------------------------------------
// Shared helpers used by multiple adapters
// ---------------------------------------------------------------------------

pub(crate) fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Default detection: the config file exists.
pub(crate) async fn detect_by_config_path(
    client: &str,
    path: &std::path::Path,
) -> Result<ClientStatus> {
    Ok(ClientStatus {
        client: client.to_string(),
        detected: path.exists(),
        config_path: Some(path.to_path_buf()),
        notes: Vec::new(),
    })
}

/// Classify a client config entry relative to our desired state and ownership
/// records. Shared by the JSON-based adapters.
pub(crate) fn classify_entry(
    _client: &str,
    server: &str,
    current: Option<&serde_json::Value>,
    desired: &serde_json::Value,
    managed: Option<&ManagedEntry>,
    force: bool,
) -> Result<(ChangeKind, Option<String>)> {
    let _ = server;
    let desired_hash = hash_value(desired);
    match current {
        None => {
            if managed.is_some() && !force {
                // Entry vanished from the client config while we still manage it:
                // the user deleted it. Do not silently re-add.
                Ok((
                    ChangeKind::Conflict,
                    Some(
                        "entry was removed from the client config; leaving it absent \
                         (re-run with --force to re-add)"
                            .into(),
                    ),
                ))
            } else {
                Ok((ChangeKind::Add, None))
            }
        }
        Some(cur) => {
            let cur_hash = hash_value(cur);
            if cur_hash == desired_hash {
                return Ok((ChangeKind::Unchanged, None));
            }
            match managed {
                Some(m) if m.hash == cur_hash || force => Ok((ChangeKind::Update, None)),
                Some(m) => Ok((
                    ChangeKind::Conflict,
                    Some(format!(
                        "managed entry was modified outside CerberusMCP \
                         (recorded hash {:#x}, current {:#x}); resolve manually or re-run with --force",
                        m.hash, cur_hash
                    )),
                )),
                None if force => Ok((ChangeKind::Update, None)),
                None => Ok((
                    ChangeKind::Conflict,
                    Some(
                        "an unmanaged entry with this name already exists; \
                         rename it or re-run with --force to take ownership"
                            .into(),
                    ),
                )),
            }
        }
    }
}

/// Entries in the client config that we manage but that are no longer in the
/// desired set: candidates for removal.
pub(crate) fn plan_removals(
    managed: &ClientManagedState,
    current_servers: &serde_json::Map<String, serde_json::Value>,
    desired: &BTreeMap<String, ServerDef>,
) -> Vec<SyncAction> {
    let mut actions = Vec::new();
    for (name, entry) in &managed.servers {
        if desired.contains_key(name) {
            continue;
        }
        match current_servers.get(name) {
            None => {
                // Already gone; drop the stale ownership record silently.
            }
            Some(cur) => {
                let cur_hash = hash_value(cur);
                if cur_hash == entry.hash {
                    actions.push(SyncAction {
                        server: name.clone(),
                        change: ChangeKind::Remove,
                        detail: Some("no longer in registry".into()),
                    });
                } else {
                    actions.push(SyncAction {
                        server: name.clone(),
                        change: ChangeKind::Conflict,
                        detail: Some(
                            "managed entry was modified and is no longer in the registry; \
                             not removing (edit the client config manually or restore it)"
                                .into(),
                        ),
                    });
                }
            }
        }
    }
    actions
}

/// JSON merge helper: get `map[key]` as an object, creating it if needed.
pub(crate) fn ensure_object<'a>(
    root: &'a mut serde_json::Value,
    key: &str,
) -> &'a mut serde_json::Map<String, serde_json::Value> {
    let obj = root.as_object_mut().expect("root must be object");
    obj.entry(key.to_string())
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    root.get_mut(key)
        .and_then(serde_json::Value::as_object_mut)
        .expect("just created object")
}

pub(crate) fn parse_json_file(path: &std::path::Path) -> Result<serde_json::Value> {
    let text = std::fs::read_to_string(path).map_err(|source| CerberusError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    serde_json::from_str(&text).map_err(|e| CerberusError::ConfigParse {
        path: path.to_path_buf(),
        source: Box::new(e),
    })
}

pub(crate) fn write_json_file(path: &std::path::Path, value: &serde_json::Value) -> Result<()> {
    let text = serde_json::to_string_pretty(value)
        .map_err(|e| CerberusError::Other(format!("serialize: {e}")))?;
    crate::registry::atomic_write(path, text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_add_update_unchanged() {
        let desired = serde_json::json!({"a": 1});
        let (kind, _) = classify_entry("c", "s", None, &desired, None, false).unwrap();
        assert_eq!(kind, ChangeKind::Add);

        let (kind, _) = classify_entry(
            "c",
            "s",
            Some(&desired),
            &desired,
            Some(&ManagedEntry {
                hash: hash_value(&desired),
                written_unix: 0,
            }),
            false,
        )
        .unwrap();
        assert_eq!(kind, ChangeKind::Unchanged);

        // Managed entry we last wrote is `other`; desired now differs → Update.
        let other = serde_json::json!({"a": 2});
        let (kind, _) = classify_entry(
            "c",
            "s",
            Some(&other),
            &desired,
            Some(&ManagedEntry {
                hash: hash_value(&other),
                written_unix: 0,
            }),
            false,
        )
        .unwrap();
        assert_eq!(kind, ChangeKind::Update);
    }

    #[test]
    fn classify_conflicts() {
        let desired = serde_json::json!({"a": 1});
        // User deleted a managed entry.
        let (kind, msg) = classify_entry(
            "c",
            "s",
            None,
            &desired,
            Some(&ManagedEntry {
                hash: 1,
                written_unix: 0,
            }),
            false,
        )
        .unwrap();
        assert_eq!(kind, ChangeKind::Conflict);
        assert!(msg.unwrap().contains("--force"));

        // Unmanaged entry occupying the name.
        let cur = serde_json::json!({"a": 9});
        let (kind, _) = classify_entry("c", "s", Some(&cur), &desired, None, false).unwrap();
        assert_eq!(kind, ChangeKind::Conflict);

        // Manually edited managed entry.
        let (kind, _) = classify_entry(
            "c",
            "s",
            Some(&cur),
            &desired,
            Some(&ManagedEntry {
                hash: hash_value(&desired),
                written_unix: 0,
            }),
            false,
        )
        .unwrap();
        assert_eq!(kind, ChangeKind::Conflict);

        // Force overrides.
        let (kind, _) = classify_entry(
            "c",
            "s",
            Some(&cur),
            &desired,
            Some(&ManagedEntry {
                hash: hash_value(&desired),
                written_unix: 0,
            }),
            true,
        )
        .unwrap();
        assert_eq!(kind, ChangeKind::Update);
    }

    #[test]
    fn plan_removals_only_matching_hash() {
        let managed = ClientManagedState {
            servers: BTreeMap::from([
                (
                    "keep-hash".to_string(),
                    ManagedEntry {
                        hash: hash_value(&serde_json::json!({"x": 1})),
                        written_unix: 0,
                    },
                ),
                (
                    "edited".to_string(),
                    ManagedEntry {
                        hash: 2,
                        written_unix: 0,
                    },
                ),
            ]),
        };
        let current = serde_json::json!({
            "keep-hash": {"x": 1},
            "edited": {"x": 99},
        })
        .as_object()
        .cloned()
        .unwrap();
        let actions = plan_removals(&managed, &current, &BTreeMap::new());
        let names: Vec<_> = actions.iter().map(|a| a.server.as_str()).collect();
        assert!(names.contains(&"keep-hash"));
        assert!(names.contains(&"edited"));
        let edited = actions.iter().find(|a| a.server == "edited").unwrap();
        assert_eq!(edited.change, ChangeKind::Conflict);
        let keep = actions.iter().find(|a| a.server == "keep-hash").unwrap();
        assert_eq!(keep.change, ChangeKind::Remove);
    }
}

#[cfg(test)]
pub(crate) mod test_lock {
    use std::sync::{Mutex, MutexGuard, OnceLock};

    /// Serializes tests that mutate process-global environment variables
    /// (`HOME`, `OPENCODE_CONFIG`, `CODEX_HOME`, `CLINE_MCP_SETTINGS_PATH`).
    pub(crate) fn env_lock() -> MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}
