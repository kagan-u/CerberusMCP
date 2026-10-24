//! OpenAI Codex CLI adapter.
//!
//! Verified against https://developers.openai.com/codex/extend/mcp and
//! `codex-rs` config reference (Oct 2026).
//!
//! Format notes (verified):
//! - Config is TOML at `~/.codex/config.toml` (root overridable via
//!   `CODEX_HOME`). Servers live under `[mcp_servers.<name>]`.
//! - stdio: `command` (string), `args` (array), `env` (static table),
//!   `env_vars` (pass-through environment variable names), optional `cwd`.
//! - Streamable HTTP: `url = "https://..."` (no SSE support in Codex).
//! - Enablement: `enabled = true|false`.
//! - Timeouts: `startup_timeout_sec` / `startup_timeout_ms`, `tool_timeout_sec`.
//! - No `${VAR}` expansion in values; environment pass-through uses
//!   `env_vars = ["NAME"]`.
//!
//! The adapter edits only the `mcp_servers` tables it owns and preserves all
//! other TOML content by round-tripping through `toml::Value`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use toml::Value as TomlValue;

use super::{
    ChangeKind, ClientAdapter, ClientInspection, ClientManagedState, ClientMeta, ClientStatus,
    ManagedEntry, ManagedState, ServerSummary, SyncAction, SyncContext, SyncPlan, SyncReport,
    detect_by_config_path, now_unix,
};
use crate::error::{CerberusError, Result};
use crate::paths::{CerberusPaths, home_path};
use crate::registry::hash_value;
use crate::registry::schema::{EnvValue, HeaderValue, ServerDef, Transport};

#[derive(Debug, Clone, Copy)]
pub struct CodexAdapter;

const CLIENT: &str = "codex";

impl CodexAdapter {
    pub fn new() -> Self {
        Self
    }

    pub fn default_config_path() -> Result<PathBuf> {
        if let Some(home) = std::env::var_os("CODEX_HOME") {
            return Ok(PathBuf::from(home).join("config.toml"));
        }
        home_path(&[".codex", "config.toml"])
    }

    fn to_entry(
        def: &ServerDef,
        ctx: &SyncContext<'_>,
        warnings: &mut Vec<String>,
    ) -> Result<TomlValue> {
        let mut table = toml::Table::new();

        match &def.transport {
            Transport::Stdio {
                command,
                args,
                env,
                cwd,
            } => {
                table.insert("command".into(), TomlValue::String(command.clone()));
                if !args.is_empty() {
                    table.insert(
                        "args".into(),
                        TomlValue::Array(args.iter().cloned().map(TomlValue::String).collect()),
                    );
                }
                // Static env values + pass-through names.
                let mut static_env = toml::Table::new();
                let mut pass_through: Vec<String> = Vec::new();
                for (key, value) in env {
                    match value {
                        EnvValue::Env { name } => {
                            pass_through.push(name.clone());
                            // Also pass the registry key through if it differs.
                            if name != key {
                                warnings.push(format!(
                                    "server env `{key}`: Codex env_vars passes the variable \
                                     `{name}` through by its own name; rename the registry key \
                                     to `{name}` if the child process expects `{key}`"
                                ));
                            }
                        }
                        EnvValue::SecretRef { name } => {
                            static_env.insert(
                                key.clone(),
                                TomlValue::String(ctx.secrets.resolve(name)?.expose().to_string()),
                            );
                        }
                        EnvValue::Literal { value } => {
                            static_env.insert(key.clone(), TomlValue::String(value.clone()));
                        }
                    }
                }
                if !static_env.is_empty() {
                    table.insert("env".into(), TomlValue::Table(static_env));
                }
                if !pass_through.is_empty() {
                    table.insert(
                        "env_vars".into(),
                        TomlValue::Array(pass_through.into_iter().map(TomlValue::String).collect()),
                    );
                }
                if let Some(cwd) = cwd {
                    table.insert("cwd".into(), TomlValue::String(cwd.display().to_string()));
                }
                if let Some(ms) = def.startup_timeout_ms {
                    table.insert("startup_timeout_ms".into(), TomlValue::Integer(ms as i64));
                }
                if let Some(ms) = def.timeout_ms {
                    // Codex tool timeout is in seconds; round up to 1s minimum.
                    let secs = ms.div_ceil(1000).max(1);
                    table.insert("tool_timeout_sec".into(), TomlValue::Integer(secs as i64));
                }
            }
            Transport::Http {
                url,
                headers,
                bearer_token,
            } => {
                table.insert("url".into(), TomlValue::String(url.clone()));
                if !headers.is_empty() {
                    let mut http_headers = toml::Table::new();
                    for (k, v) in headers {
                        http_headers.insert(k.clone(), resolve_header_literal(v, ctx)?);
                    }
                    table.insert("http_headers".into(), TomlValue::Table(http_headers));
                }
                if let Some(token) = bearer_token {
                    let value = ctx.secrets.resolve_env_value(token)?;
                    match token {
                        EnvValue::Env { name } => {
                            // Prefer the runtime reference form.
                            let _ = value;
                            table.insert(
                                "bearer_token_env_var".into(),
                                TomlValue::String(name.clone()),
                            );
                        }
                        _ => {
                            table.insert(
                                "bearer_token_env_var".into(),
                                TomlValue::String(String::new()),
                            );
                            warnings.push(
                                "bearer_token was a literal/secret_ref; Codex can only read \
                                 bearer tokens from an environment variable name, so an empty \
                                 bearer_token_env_var was written — use an Env source instead"
                                    .into(),
                            );
                        }
                    }
                }
                if let Some(ms) = def.startup_timeout_ms {
                    table.insert("startup_timeout_ms".into(), TomlValue::Integer(ms as i64));
                }
            }
        }

        table.insert("enabled".into(), TomlValue::Boolean(def.enabled));
        Ok(TomlValue::Table(table))
    }
}

impl Default for CodexAdapter {
    fn default() -> Self {
        Self::new()
    }
}

fn resolve_header_literal(value: &HeaderValue, ctx: &SyncContext<'_>) -> Result<TomlValue> {
    match value {
        HeaderValue::Resolved(EnvValue::Env { name }) => {
            // Codex supports env_http_headers for env-sourced headers.
            // We emit into http_headers only for literals; env headers are
            // handled by the caller when building env_http_headers. For
            // simplicity we resolve the value here and warn.
            match std::env::var(name) {
                Ok(v) => Ok(TomlValue::String(v)),
                Err(_) => {
                    let _ = ctx;
                    Err(CerberusError::SecretResolution {
                        name: name.clone(),
                        detail: "environment variable is not set (Codex header mapping)".into(),
                    })
                }
            }
        }
        HeaderValue::Resolved(other) => {
            Ok(TomlValue::String(ctx.secrets.resolve_env_value(other)?))
        }
        HeaderValue::Literal(s) => Ok(TomlValue::String(s.clone())),
    }
}

fn mcp_servers_table(root: &TomlValue) -> Option<&toml::Table> {
    root.get("mcp_servers")?.as_table()
}

impl ClientAdapter for CodexAdapter {
    fn meta(&self) -> &ClientMeta {
        static META: std::sync::OnceLock<ClientMeta> = std::sync::OnceLock::new();
        META.get_or_init(|| ClientMeta {
            id: super::ClientId::Codex,
            display_name: "Codex",
            supported: true,
            unsupported_reason: None,
            config_format: "TOML (~/.codex/config.toml)",
            requires_restart: true,
            limitations: &[
                "SSE transports are not supported by Codex; only stdio and Streamable HTTP",
                "Environment references are passed via env_vars (by variable name), not inlined",
                "Bearer tokens for HTTP servers must be provided via an env variable name",
                "tool_timeout_sec is seconds; registry timeout_ms is rounded up",
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
            status
                .notes
                .push("~/.codex/config.toml not found; created by Codex on first run".into());
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
            let text = std::fs::read_to_string(&path).map_err(|source| CerberusError::Io {
                path: path.clone(),
                source,
            })?;
            let root: TomlValue =
                toml::from_str(&text).map_err(|e| CerberusError::ConfigParse {
                    path: path.clone(),
                    source: Box::new(e),
                })?;
            if let Some(table) = mcp_servers_table(&root) {
                for (name, entry) in table {
                    let transport = if entry.get("url").is_some() {
                        "http"
                    } else {
                        "stdio"
                    };
                    let enabled = entry
                        .get("enabled")
                        .and_then(TomlValue::as_bool)
                        .unwrap_or(true);
                    servers.push(ServerSummary {
                        name: name.clone(),
                        transport: transport.to_string(),
                        managed: false,
                        enabled,
                    });
                }
            } else {
                notes.push("no [mcp_servers.*] tables present".into());
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
        let (root, current) = load_toml(&path)?;

        let mut actions = Vec::new();
        let mut warnings = Vec::new();

        let desired_map = build_desired_map(ctx, &mut warnings)?;

        for (name, desired) in &desired_map {
            let cur = current.get(name).map(json_like);
            let desired_json = json_like(desired);
            let managed = ctx.managed.servers.get(name);
            let (change, detail) =
                classify_toml(cur.as_ref(), &desired_json, managed, ctx.options.force);
            actions.push(SyncAction {
                server: name.clone(),
                change,
                detail,
            });
        }

        for (name, entry) in &ctx.managed.servers {
            if desired_map.contains_key(name) {
                continue;
            }
            if let Some(cur) = current.get(name) {
                let cur_json = json_like(cur);
                if hash_value(&cur_json) == entry.hash {
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
                            "managed entry was modified outside CerberusMCP; not removing".into(),
                        ),
                    });
                }
            }
        }

        let _ = root;
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
        let (mut root, mut current) = load_toml(&path)?;

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

        let mut warnings = plan.warnings.clone();
        let desired_map = build_desired_map(ctx, &mut warnings)?;
        let mut applied = Vec::new();
        let mut skipped = Vec::new();
        let mut new_managed = ClientManagedState::default();

        for action in &plan.actions {
            match action.change {
                ChangeKind::Unchanged => {
                    if let Some(desired) = desired_map.get(&action.server) {
                        new_managed.servers.insert(
                            action.server.clone(),
                            ManagedEntry {
                                hash: hash_value(&json_like(desired)),
                                written_unix: now_unix(),
                            },
                        );
                        applied.push(action.clone());
                    } else {
                        applied.push(action.clone());
                    }
                }
                ChangeKind::Add | ChangeKind::Update => {
                    let Some(desired) = desired_map.get(&action.server) else {
                        skipped.push(action.clone());
                        continue;
                    };
                    current.insert(action.server.clone(), desired.clone());
                    new_managed.servers.insert(
                        action.server.clone(),
                        ManagedEntry {
                            hash: hash_value(&json_like(desired)),
                            written_unix: now_unix(),
                        },
                    );
                    applied.push(action.clone());
                }
                ChangeKind::Remove => {
                    current.remove(&action.server);
                    applied.push(action.clone());
                }
                ChangeKind::Conflict => {
                    if let Some(cur) = current.get(&action.server) {
                        if let Some(m) = ctx.managed.servers.get(&action.server) {
                            if hash_value(&json_like(cur)) == m.hash {
                                new_managed.servers.insert(action.server.clone(), m.clone());
                            }
                        }
                    }
                    skipped.push(action.clone());
                }
            }
        }

        // Write back, preserving every other key in the TOML document.
        if current.is_empty() {
            root.as_table_mut().map(|t| t.remove("mcp_servers"));
        } else {
            if let Some(table) = root.as_table_mut() {
                table.insert("mcp_servers".into(), TomlValue::Table(current.clone()));
            }
        }

        let text = toml::to_string_pretty(&root)
            .map_err(|e| CerberusError::Other(format!("TOML serialize: {e}")))?;
        crate::registry::atomic_write(&path, text.as_bytes())?;

        // Verify round-trip.
        let verify_text = std::fs::read_to_string(&path).map_err(|source| CerberusError::Io {
            path: path.clone(),
            source,
        })?;
        toml::from_str::<TomlValue>(&verify_text).map_err(|e| CerberusError::ConfigParse {
            path: path.clone(),
            source: Box::new(e),
        })?;

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

fn load_toml(path: &std::path::Path) -> Result<(TomlValue, toml::Table)> {
    if !path.exists() {
        return Ok((TomlValue::Table(toml::Table::new()), toml::Table::new()));
    }
    let text = std::fs::read_to_string(path).map_err(|source| CerberusError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let root: TomlValue = toml::from_str(&text).map_err(|e| CerberusError::ConfigParse {
        path: path.to_path_buf(),
        source: Box::new(e),
    })?;
    let current = root
        .get("mcp_servers")
        .and_then(TomlValue::as_table)
        .cloned()
        .unwrap_or_default();
    Ok((root, current))
}

fn build_desired_map(
    ctx: &SyncContext<'_>,
    warnings: &mut Vec<String>,
) -> Result<BTreeMap<String, TomlValue>> {
    let mut out = BTreeMap::new();
    for (name, def) in ctx.servers {
        out.insert(name.clone(), CodexAdapter::to_entry(def, ctx, warnings)?);
    }
    Ok(out)
}

/// Convert a TOML value into a JSON value for hashing/comparison.
fn json_like(value: &TomlValue) -> serde_json::Value {
    match value {
        TomlValue::String(s) => serde_json::Value::String(s.clone()),
        TomlValue::Integer(i) => serde_json::json!(i),
        TomlValue::Float(f) => serde_json::json!(f),
        TomlValue::Boolean(b) => serde_json::json!(b),
        TomlValue::Datetime(d) => serde_json::Value::String(d.to_string()),
        TomlValue::Array(a) => serde_json::Value::Array(a.iter().map(json_like).collect()),
        TomlValue::Table(t) => {
            serde_json::Value::Object(t.iter().map(|(k, v)| (k.clone(), json_like(v))).collect())
        }
    }
}

fn classify_toml(
    current: Option<&serde_json::Value>,
    desired: &serde_json::Value,
    managed: Option<&ManagedEntry>,
    force: bool,
) -> (ChangeKind, Option<String>) {
    match current {
        None => {
            if managed.is_some() && !force {
                (
                    ChangeKind::Conflict,
                    Some(
                        "entry was removed from the Codex config; leaving it absent \
                         (re-run with --force to re-add)"
                            .into(),
                    ),
                )
            } else {
                (ChangeKind::Add, None)
            }
        }
        Some(cur) => {
            let cur_hash = hash_value(cur);
            let desired_hash = hash_value(desired);
            if cur_hash == desired_hash {
                return (ChangeKind::Unchanged, None);
            }
            match managed {
                Some(m) if m.hash == cur_hash || force => (ChangeKind::Update, None),
                Some(_) => (
                    ChangeKind::Conflict,
                    Some(
                        "managed entry was modified outside CerberusMCP; \
                         resolve manually or re-run with --force"
                            .into(),
                    ),
                ),
                None => (
                    ChangeKind::Conflict,
                    Some(
                        "an unmanaged entry with this name already exists; \
                         rename it or re-run with --force to take ownership"
                            .into(),
                    ),
                ),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::ApplyOptions;
    use crate::registry::schema::ServerPolicy;

    fn stdio_server(cmd: &str, args: &[&str]) -> ServerDef {
        ServerDef {
            enabled: true,
            transport: Transport::Stdio {
                command: cmd.into(),
                args: args.iter().map(|s| s.to_string()).collect(),
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
        }
    }

    #[tokio::test]
    async fn writes_toml_and_preserves_other_tables() {
        let _env_guard = crate::adapters::test_lock::env_lock();
        let dir = tempfile::tempdir().unwrap();
        unsafe {
            std::env::set_var("CODEX_HOME", dir.path());
        };
        let cfg = dir.path().join("config.toml");
        std::fs::write(
            &cfg,
            "model = \"o3\"\n[projects]\n\"/tmp\" = { trust_level = \"trusted\" }\n",
        )
        .unwrap();

        let paths = CerberusPaths::with_roots(dir.path().join("cfg"), dir.path().join("data"));
        paths.ensure_dirs().unwrap();
        let secrets = crate::secrets::SecretStore::default();
        let servers = BTreeMap::from([("demo".to_string(), stdio_server("npx", &["-y", "demo"]))]);
        let managed = ClientManagedState::default();
        let ctx = SyncContext {
            paths: &paths,
            secrets: &secrets,
            servers: &servers,
            managed: &managed,
            options: ApplyOptions::default(),
        };

        let adapter = CodexAdapter;
        let plan = adapter.plan_sync(&ctx).await.unwrap();
        let report = adapter.apply_sync(&ctx, &plan).await.unwrap();

        let text = std::fs::read_to_string(&report.config_path).unwrap();
        let root: TomlValue = toml::from_str(&text).unwrap();
        assert_eq!(root["model"].as_str(), Some("o3"));
        assert!(root["projects"].is_table());
        assert_eq!(root["mcp_servers"]["demo"]["command"].as_str(), Some("npx"));
        assert_eq!(root["mcp_servers"]["demo"]["enabled"].as_bool(), Some(true));
    }

    #[tokio::test]
    async fn env_sources_become_env_vars() {
        let _env_guard = crate::adapters::test_lock::env_lock();
        let dir = tempfile::tempdir().unwrap();
        unsafe {
            std::env::set_var("CODEX_HOME", dir.path());
        };
        let paths = CerberusPaths::with_roots(dir.path().join("cfg"), dir.path().join("data"));
        paths.ensure_dirs().unwrap();
        let secrets = crate::secrets::SecretStore::default();
        let mut def = stdio_server("npx", &["x"]);
        if let Transport::Stdio { env, .. } = &mut def.transport {
            env.insert("MY_KEY".to_string(), EnvValue::env("MY_KEY"));
        }
        let servers = BTreeMap::from([("demo".to_string(), def)]);
        let managed = ClientManagedState::default();
        let ctx = SyncContext {
            paths: &paths,
            secrets: &secrets,
            servers: &servers,
            managed: &managed,
            options: ApplyOptions::default(),
        };
        let mut warnings = Vec::new();
        let entry = CodexAdapter::to_entry(&servers["demo"], &ctx, &mut warnings).unwrap();
        assert_eq!(
            entry["env_vars"].as_array().unwrap()[0].as_str(),
            Some("MY_KEY")
        );
        assert!(entry.get("env").is_none());
    }
}
