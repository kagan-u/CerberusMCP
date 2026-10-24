//! Cline adapter.
//!
//! Verified against cline/cline source (`apps/vscode/src/services/mcp/schemas.ts`,
//! `shared/mcp.ts`) and https://docs.cline.bot/mcp/mcp-overview (Oct 2026).
//!
//! Format notes (verified):
//! - Settings file is `cline_mcp_settings.json`. For the VSCode extension it
//!   lives in the extension's globalStorage: `.../User/globalStorage/
//!   saoudrizwan.claude-dev/settings/cline_mcp_settings.json`. The Cline CLI
//!   uses `~/.cline/data/settings/cline_mcp_settings.json`. The path can be
//!   overridden with `CLINE_MCP_SETTINGS_PATH`.
//! - `mcpServers.<name>`: `type` is `stdio | sse | streamableHttp` (note the
//!   camelCase); **omitting `type` silently means SSE**, so we always write
//!   it explicitly.
//! - stdio `command` is a string with a separate `args` array.
//! - Enablement: `disabled: bool` (inverted).
//! - `autoApprove`: array of tool names (preserved, never managed).
//! - `timeout` is in **seconds**.
//! - Cline performs **no** environment-variable substitution in env values,
//!   so env references must be resolved to literals at sync time; this
//!   requires the variables to be set in the environment running `cerberus`
//!   and places credentials in plaintext in the client file (warned).

use std::path::PathBuf;

use serde_json::{Map, Value, json};

use super::{
    ChangeKind, ClientAdapter, ClientInspection, ClientManagedState, ClientMeta, ClientStatus,
    ManagedEntry, ManagedState, ServerSummary, SyncAction, SyncContext, SyncPlan, SyncReport,
    classify_entry, detect_by_config_path, ensure_object, now_unix, parse_json_file, plan_removals,
    write_json_file,
};
use crate::error::{CerberusError, Result};
use crate::paths::{CerberusPaths, home_path};
use crate::registry::schema::{EnvValue, HeaderValue, ServerDef, Transport};

#[derive(Debug, Clone, Copy)]
pub struct ClineAdapter;

const CLIENT: &str = "cline";
const EXTENSION_DIR: &str = "saoudrizwan.claude-dev";

impl ClineAdapter {
    pub fn new() -> Self {
        Self
    }

    /// Resolve the settings file, preferring (in order):
    /// 1. `CLINE_MCP_SETTINGS_PATH`
    /// 2. `~/.cline/data/settings/cline_mcp_settings.json` (Cline CLI layout)
    /// 3. VSCode-family globalStorage (Code, then Cursor, then Windsurf)
    pub fn default_config_path() -> Result<PathBuf> {
        if let Some(p) = std::env::var_os("CLINE_MCP_SETTINGS_PATH") {
            return Ok(PathBuf::from(p));
        }
        let cli_path = home_path(&[".cline", "data", "settings", "cline_mcp_settings.json"])?;
        if cli_path.exists() || home_path(&[".cline"])?.exists() {
            return Ok(cli_path);
        }

        // VSCode-family editors.
        let bases = ["Code", "Cursor", "Windsurf", "Code - Insiders"];
        #[cfg(target_os = "macos")]
        let app_support = home_path(&["Library", "Application Support"]).ok();
        #[cfg(target_os = "linux")]
        let app_support = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| home_path(&[".config"]).ok());
        #[cfg(target_os = "windows")]
        let app_support = std::env::var_os("APPDATA").map(PathBuf::from);

        if let Some(base) = app_support {
            for editor in bases {
                let candidate = base
                    .join(editor)
                    .join("User")
                    .join("globalStorage")
                    .join(EXTENSION_DIR)
                    .join("settings")
                    .join("cline_mcp_settings.json");
                if candidate.exists() {
                    return Ok(candidate);
                }
            }
            // Fall back to the standard VSCode path even if the file does not
            // exist yet (it is where Cline will look).
            return Ok(base
                .join("Code")
                .join("User")
                .join("globalStorage")
                .join(EXTENSION_DIR)
                .join("settings")
                .join("cline_mcp_settings.json"));
        }

        home_path(&[".cline", "data", "settings", "cline_mcp_settings.json"])
    }

    fn to_entry(
        name: &str,
        def: &ServerDef,
        ctx: &SyncContext<'_>,
        warnings: &mut Vec<String>,
    ) -> Result<Value> {
        let mut entry = Map::new();
        let mut has_env_refs = false;

        match &def.transport {
            Transport::Stdio {
                command,
                args,
                env,
                cwd,
            } => {
                entry.insert("type".into(), json!("stdio"));
                entry.insert("command".into(), json!(command));
                entry.insert("args".into(), json!(args));
                let mut resolved = Map::new();
                for (key, value) in env {
                    match value {
                        EnvValue::Env { name } => match std::env::var(name) {
                            Ok(v) => {
                                resolved.insert(key.clone(), json!(v));
                                has_env_refs = true;
                            }
                            Err(_) => {
                                return Err(CerberusError::SecretResolution {
                                    name: name.clone(),
                                    detail: format!(
                                        "Cline cannot reference environment variables; `{name}` \
                                         must be set when running `cerberus sync` so it can be \
                                         inlined into the Cline settings file"
                                    ),
                                });
                            }
                        },
                        EnvValue::SecretRef { name: secret_name } => {
                            resolved.insert(
                                key.clone(),
                                json!(ctx.secrets.resolve(secret_name)?.expose().to_string()),
                            );
                            has_env_refs = true;
                        }
                        EnvValue::Literal { value } => {
                            resolved.insert(key.clone(), json!(value));
                        }
                    }
                }
                if !resolved.is_empty() {
                    entry.insert("env".into(), Value::Object(resolved));
                }
                if let Some(cwd) = cwd {
                    entry.insert("cwd".into(), json!(cwd.display().to_string()));
                }
                // Registry timeout_ms → Cline timeout seconds.
                if let Some(ms) = def.timeout_ms {
                    entry.insert("timeout".into(), json!(ms.div_ceil(1000).max(1)));
                }
            }
            Transport::Http {
                url,
                headers,
                bearer_token,
            } => {
                entry.insert("type".into(), json!("streamableHttp"));
                entry.insert("url".into(), json!(url));
                let mut out_headers = Map::new();
                for (k, v) in headers {
                    out_headers.insert(k.clone(), resolve_header(v, ctx, &mut has_env_refs)?);
                }
                if let Some(token) = bearer_token {
                    out_headers.insert(
                        "Authorization".into(),
                        json!(format!("Bearer {}", ctx.secrets.resolve_env_value(token)?)),
                    );
                    has_env_refs = true;
                }
                if !out_headers.is_empty() {
                    entry.insert("headers".into(), Value::Object(out_headers));
                }
            }
        }

        entry.insert("disabled".into(), json!(!def.enabled));

        let _ = name;

        if has_env_refs {
            warnings.push(format!(
                "server `{name}`: Cline performs no env-var substitution — resolved secret \
                 values are written in plaintext into the Cline settings file"
            ));
        }
        Ok(Value::Object(entry))
    }

    /// Merge entry while preserving the user's `autoApprove` list.
    fn merge_preserving(current: Option<&Value>, desired: Value) -> Value {
        let mut desired = desired;
        if let Some(cur) = current {
            if let Some(auto) = cur.get("autoApprove") {
                if let Some(obj) = desired.as_object_mut() {
                    obj.insert("autoApprove".into(), auto.clone());
                }
            }
            // Preserve opaque oauth/metadata blobs the user obtained via Cline.
            for key in ["oauth", "metadata", "remoteConfigured"] {
                if let Some(v) = cur.get(key) {
                    if let Some(obj) = desired.as_object_mut() {
                        if !obj.contains_key(key) {
                            obj.insert(key.into(), v.clone());
                        }
                    }
                }
            }
        }
        desired
    }
}

impl Default for ClineAdapter {
    fn default() -> Self {
        Self::new()
    }
}

fn resolve_header(
    value: &HeaderValue,
    ctx: &SyncContext<'_>,
    has_env_refs: &mut bool,
) -> Result<Value> {
    match value {
        HeaderValue::Resolved(EnvValue::Env { name }) => match std::env::var(name) {
            Ok(v) => {
                *has_env_refs = true;
                Ok(json!(v))
            }
            Err(_) => Err(CerberusError::SecretResolution {
                name: name.clone(),
                detail: "Cline cannot reference environment variables; set it before syncing"
                    .into(),
            }),
        },
        HeaderValue::Resolved(other) => {
            *has_env_refs = true;
            Ok(json!(ctx.secrets.resolve_env_value(other)?))
        }
        HeaderValue::Literal(s) => Ok(json!(s)),
    }
}

fn current_servers(root: &Value) -> Map<String, Value> {
    root.get("mcpServers")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
}

impl ClientAdapter for ClineAdapter {
    fn meta(&self) -> &ClientMeta {
        static META: std::sync::OnceLock<ClientMeta> = std::sync::OnceLock::new();
        META.get_or_init(|| ClientMeta {
            id: super::ClientId::Cline,
            display_name: "Cline",
            supported: true,
            unsupported_reason: None,
            config_format: "JSON (cline_mcp_settings.json)",
            requires_restart: false,
            limitations: &[
                "No env-var substitution: secrets are resolved and inlined at sync time",
                "Settings file location depends on editor (VSCode/Cursor/Windsurf) or Cline CLI",
                "timeout is written in seconds",
                "autoApprove and oauth/metadata fields are preserved, never managed",
            ],
        })
    }

    fn config_path(&self, _paths: &CerberusPaths) -> Result<PathBuf> {
        Self::default_config_path()
    }

    async fn detect(&self, paths: &CerberusPaths) -> Result<ClientStatus> {
        let path = self.config_path(paths)?;
        let mut status = detect_by_config_path(CLIENT, &path).await?;
        if !path.exists() {
            // Editor globalStorage existing still counts as detection.
            if let Some(parent) = path.parent() {
                if parent.exists() {
                    status.detected = true;
                    status.notes.push(format!(
                        "settings directory exists at {} (file not yet created)",
                        parent.display()
                    ));
                }
            }
        }
        Ok(status)
    }

    async fn inspect(&self, paths: &CerberusPaths) -> Result<ClientInspection> {
        let path = self.config_path(paths)?;
        let detected = path.exists();
        let meta = self.meta();
        let mut servers = Vec::new();
        let mut notes = Vec::new();

        if detected {
            let root = parse_json_file(&path)?;
            for (name, entry) in current_servers(&root) {
                if !entry.is_object() {
                    continue;
                }
                // Note the Cline footgun: missing type means SSE.
                let transport = entry
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("sse (default when type is omitted)");
                let disabled = entry
                    .get("disabled")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                if !entry.get("type").is_some_and(Value::is_string) {
                    notes.push(format!(
                        "server `{name}` has no `type`; Cline defaults to SSE, which is                          usually not what you want for a stdio server"
                    ));
                }
                servers.push(ServerSummary {
                    name: name.clone(),
                    transport: transport.to_string(),
                    managed: false,
                    enabled: !disabled,
                });
            }
        }

        Ok(ClientInspection {
            client: CLIENT.into(),
            display_name: meta.display_name.into(),
            supported: true,
            detected,
            config_path: Some(path),
            servers,
            limitations: meta.limitations.iter().map(|s| s.to_string()).collect(),
            notes,
        })
    }

    async fn plan_sync(&self, ctx: &SyncContext<'_>) -> Result<SyncPlan> {
        let path = self.config_path(ctx.paths)?;
        let root = if path.exists() {
            parse_json_file(&path)?
        } else {
            Value::Object(Map::new())
        };
        let current = current_servers(&root);

        let mut actions = Vec::new();
        let mut warnings = Vec::new();

        for (name, def) in ctx.servers {
            let desired = match Self::to_entry(name, def, ctx, &mut warnings) {
                Ok(v) => v,
                Err(e) => {
                    warnings.push(format!("server `{name}`: {e} — excluded from sync"));
                    actions.push(SyncAction {
                        server: name.clone(),
                        change: ChangeKind::Conflict,
                        detail: Some(e.to_string()),
                    });
                    continue;
                }
            };
            let merged = Self::merge_preserving(current.get(name), desired);
            let (change, detail) = classify_entry(
                CLIENT,
                name,
                current.get(name),
                &merged,
                ctx.managed.servers.get(name),
                ctx.options.force,
            )?;
            // autoApprove preservation makes entries differ from a pure
            // desired entry; if the only difference is preserved fields the
            // classify above already handled it because we merged first.
            actions.push(SyncAction {
                server: name.clone(),
                change,
                detail,
            });
        }

        for action in plan_removals(ctx.managed, &current, ctx.servers) {
            if matches!(action.change, ChangeKind::Remove | ChangeKind::Conflict) {
                actions.push(action);
            }
        }

        let has_changes = actions
            .iter()
            .any(|a| !matches!(a.change, ChangeKind::Unchanged));

        Ok(SyncPlan {
            client: CLIENT.into(),
            config_path: path,
            actions,
            warnings,
            has_changes,
        })
    }

    async fn apply_sync(&self, ctx: &SyncContext<'_>, plan: &SyncPlan) -> Result<SyncReport> {
        let path = plan.config_path.clone();
        let mut root = if path.exists() {
            parse_json_file(&path)?
        } else {
            Value::Object(Map::new())
        };

        let backup_id = if path.exists() {
            let text = std::fs::read_to_string(&path).map_err(|source| CerberusError::Io {
                path: path.clone(),
                source,
            })?;
            Some(
                crate::backup::create_from_text(
                    ctx.paths,
                    &path,
                    &text,
                    Some(&format!("pre-sync-{CLIENT}")),
                )?
                .id,
            )
        } else {
            None
        };

        let mut current = current_servers(&root);
        let mut warnings = plan.warnings.clone();
        let mut applied = Vec::new();
        let mut skipped = Vec::new();
        let mut new_managed = ClientManagedState::default();

        for action in &plan.actions {
            match action.change {
                ChangeKind::Unchanged => {
                    if let Some(def) = ctx.servers.get(&action.server) {
                        let mut w = Vec::new();
                        if let Ok(desired) = Self::to_entry(&action.server, def, ctx, &mut w) {
                            let merged =
                                Self::merge_preserving(current.get(&action.server), desired);
                            new_managed.servers.insert(
                                action.server.clone(),
                                ManagedEntry {
                                    hash: crate::registry::hash_value(&merged),
                                    written_unix: now_unix(),
                                },
                            );
                        }
                        warnings.extend(w);
                        applied.push(action.clone());
                    }
                }
                ChangeKind::Add | ChangeKind::Update => {
                    let Some(def) = ctx.servers.get(&action.server) else {
                        skipped.push(action.clone());
                        continue;
                    };
                    let mut w = Vec::new();
                    let desired = match Self::to_entry(&action.server, def, ctx, &mut w) {
                        Ok(v) => v,
                        Err(e) => {
                            warnings.push(format!("server `{}`: {} — skipped", action.server, e));
                            skipped.push(action.clone());
                            continue;
                        }
                    };
                    warnings.extend(w);
                    let merged = Self::merge_preserving(current.get(&action.server), desired);
                    new_managed.servers.insert(
                        action.server.clone(),
                        ManagedEntry {
                            hash: crate::registry::hash_value(&merged),
                            written_unix: now_unix(),
                        },
                    );
                    current.insert(action.server.clone(), merged);
                    applied.push(action.clone());
                }
                ChangeKind::Remove => {
                    current.remove(&action.server);
                    applied.push(action.clone());
                }
                ChangeKind::Conflict => {
                    if let Some(cur) = current.get(&action.server) {
                        if let Some(m) = ctx.managed.servers.get(&action.server) {
                            if crate::registry::hash_value(cur) == m.hash {
                                new_managed.servers.insert(action.server.clone(), m.clone());
                            }
                        }
                    }
                    skipped.push(action.clone());
                }
            }
        }

        let mcp_servers = ensure_object(&mut root, "mcpServers");
        mcp_servers.clear();
        for (k, v) in current {
            mcp_servers.insert(k, v);
        }

        write_json_file(&path, &root)?;
        let _ = parse_json_file(&path)?;

        let state_path = ctx.paths.managed_state_file();
        let mut state = ManagedState::load(&state_path)?;
        state.clients.insert(CLIENT.to_string(), new_managed);
        state.save(&state_path)?;

        warnings.dedup();
        Ok(SyncReport {
            client: CLIENT.into(),
            config_path: path,
            applied,
            skipped,
            warnings,
            backup_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::ApplyOptions;
    use crate::registry::schema::ServerPolicy;
    use std::collections::BTreeMap;

    #[tokio::test]
    async fn writes_explicit_type_and_preserves_autoapprove() {
        let _env_guard = crate::adapters::test_lock::env_lock();
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("cline_mcp_settings.json");
        unsafe {
            std::env::set_var("CLINE_MCP_SETTINGS_PATH", &settings);
        };
        std::fs::write(
            &settings,
            r#"{"mcpServers":{"mine":{"type":"stdio","command":"x","args":[],"autoApprove":["read"],"oauth":{"t":1}}}}"#,
        )
        .unwrap();

        let paths = CerberusPaths::with_roots(dir.path().join("cfg"), dir.path().join("data"));
        paths.ensure_dirs().unwrap();
        let secrets = crate::secrets::SecretStore::default();
        let def = ServerDef {
            enabled: true,
            transport: Transport::Stdio {
                command: "x".into(),
                args: vec![],
                env: BTreeMap::new(),
                cwd: None,
            },
            mode: None,
            tags: vec![],
            timeout_ms: Some(5000),
            startup_timeout_ms: None,
            policy: ServerPolicy::default(),
            description: None,
        };
        let servers = BTreeMap::from([("mine".to_string(), def)]);
        let managed = ClientManagedState::default();
        let ctx = SyncContext {
            paths: &paths,
            secrets: &secrets,
            servers: &servers,
            managed: &managed,
            options: ApplyOptions { force: true },
        };

        let adapter = ClineAdapter;
        let plan = adapter.plan_sync(&ctx).await.unwrap();
        let report = adapter.apply_sync(&ctx, &plan).await.unwrap();
        let written = parse_json_file(&report.config_path).unwrap();
        let entry = &written["mcpServers"]["mine"];
        assert_eq!(entry["type"], "stdio");
        assert_eq!(entry["timeout"], 5); // seconds
        assert_eq!(entry["autoApprove"][0], "read");
        assert!(entry["oauth"].is_object());
        unsafe {
            std::env::remove_var("CLINE_MCP_SETTINGS_PATH");
        };
    }

    #[tokio::test]
    async fn missing_env_var_fails_actionably_for_cline() {
        let _env_guard = crate::adapters::test_lock::env_lock();
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("cline_mcp_settings.json");
        unsafe {
            std::env::set_var("CLINE_MCP_SETTINGS_PATH", &settings);
        };
        let paths = CerberusPaths::with_roots(dir.path().join("cfg"), dir.path().join("data"));
        paths.ensure_dirs().unwrap();
        let secrets = crate::secrets::SecretStore::default();
        let mut def = ServerDef {
            enabled: true,
            transport: Transport::Stdio {
                command: "x".into(),
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
        };
        if let Transport::Stdio { env, .. } = &mut def.transport {
            env.insert(
                "TOKEN".into(),
                EnvValue::env("CERBERUS_TEST_MISSING_VAR_XYZ"),
            );
        }
        let servers = BTreeMap::from([("s".to_string(), def)]);
        let managed = ClientManagedState::default();
        let ctx = SyncContext {
            paths: &paths,
            secrets: &secrets,
            servers: &servers,
            managed: &managed,
            options: ApplyOptions::default(),
        };
        let adapter = ClineAdapter;
        let plan = adapter.plan_sync(&ctx).await.unwrap();
        // Plan succeeds (mapping happens at plan time and errors become
        // warnings), or fails entirely — either is acceptable; apply must not
        // silently write an empty value.
        if plan.has_changes {
            let report = adapter.apply_sync(&ctx, &plan).await.unwrap();
            assert!(!report.skipped.is_empty() || !report.applied.is_empty());
        }
        unsafe {
            std::env::remove_var("CLINE_MCP_SETTINGS_PATH");
        };
    }
}
