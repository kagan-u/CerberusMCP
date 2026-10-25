//! Gemini CLI adapter.
//!
//! Verified against https://github.com/google-gemini/gemini-cli/blob/main/docs/tools/mcp-server.md
//! and docs/cli/settings.md (Oct 2026).
//!
//! Format notes (verified):
//! - User config: `~/.gemini/settings.json`, key `mcpServers`.
//! - stdio servers: `command` (string) + `args` (array) + `env` (object).
//! - Streamable HTTP: `httpUrl` + `headers`. (Plain `url` means SSE — we
//!   never write SSE.)
//! - Enablement is stored in a separate file (`~/.gemini/mcp-server-enablement.json`)
//!   managed by `gemini mcp enable/disable`; settings.json has no per-server
//!   enabled flag. Disabled servers are therefore omitted from the written
//!   `mcpServers` map (documented limitation).
//! - `$VAR` / `${VAR}` expansion works inside `env` values only.
//! - Server names must not contain `_` (Gemini splits tool names on the
//!   first underscore); registry validation allows `_` for other clients, so
//!   this adapter warns when a name contains one.

use std::collections::BTreeMap;
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
pub struct GeminiCliAdapter;

const CLIENT: &str = "gemini-cli";

impl GeminiCliAdapter {
    pub fn new() -> Self {
        Self
    }

    pub fn default_config_path() -> Result<PathBuf> {
        home_path(&[".gemini", "settings.json"])
    }

    fn to_entry(
        name: &str,
        def: &ServerDef,
        ctx: &SyncContext<'_>,
        warnings: &mut Vec<String>,
    ) -> Result<Option<Value>> {
        if name.contains('_') {
            warnings.push(format!(
                "server name `{name}` contains `_`; Gemini CLI splits tool names on the \
                 first underscore — rename the server to avoid tool-name ambiguity"
            ));
        }
        if !def.enabled {
            warnings.push(format!(
                "server `{name}` is disabled in the registry and will not be written to \
                 Gemini CLI settings (no per-server enabled flag exists in settings.json)"
            ));
            return Ok(None);
        }
        match &def.transport {
            Transport::Stdio {
                command,
                args,
                env,
                cwd,
            } => {
                let mut entry = Map::new();
                entry.insert("command".into(), json!(command));
                entry.insert("args".into(), json!(args));
                let mapped = map_env(env, ctx)?;
                if !mapped.is_empty() {
                    entry.insert("env".into(), Value::Object(mapped));
                }
                if let Some(cwd) = cwd {
                    entry.insert("cwd".into(), json!(cwd.display().to_string()));
                }
                if let Some(ms) = def.timeout_ms {
                    entry.insert("timeout".into(), json!(ms));
                }
                Ok(Some(Value::Object(entry)))
            }
            Transport::Http {
                url,
                headers,
                bearer_token,
            } => {
                let mut entry = Map::new();
                // httpUrl selects Streamable HTTP; plain `url` would be SSE.
                entry.insert("httpUrl".into(), json!(url));
                let mut out_headers = Map::new();
                for (k, v) in headers {
                    out_headers.insert(k.clone(), resolve_header(v, ctx)?);
                }
                if let Some(token) = bearer_token {
                    out_headers.insert(
                        "Authorization".into(),
                        json!(format!("Bearer {}", ctx.secrets.resolve_env_value(token)?)),
                    );
                }
                if !out_headers.is_empty() {
                    entry.insert("headers".into(), Value::Object(out_headers));
                }
                Ok(Some(Value::Object(entry)))
            }
        }
    }
}

impl Default for GeminiCliAdapter {
    fn default() -> Self {
        Self::new()
    }
}

fn map_env(env: &BTreeMap<String, EnvValue>, ctx: &SyncContext<'_>) -> Result<Map<String, Value>> {
    let mut out = Map::new();
    for (key, value) in env {
        match value {
            EnvValue::Env { name } => {
                out.insert(key.clone(), json!(format!("${{{name}}}")));
            }
            EnvValue::SecretRef { name } => {
                out.insert(key.clone(), json!(ctx.secrets.resolve(name)?.expose()));
            }
            EnvValue::Literal { value } => {
                out.insert(key.clone(), json!(value));
            }
        }
    }
    Ok(out)
}

fn resolve_header(value: &HeaderValue, ctx: &SyncContext<'_>) -> Result<Value> {
    match value {
        HeaderValue::Resolved(EnvValue::Env { name }) => Ok(json!(format!("${{{name}}}"))),
        HeaderValue::Resolved(other) => Ok(json!(ctx.secrets.resolve_env_value(other)?)),
        HeaderValue::Literal(s) => Ok(json!(s)),
    }
}

impl ClientAdapter for GeminiCliAdapter {
    fn meta(&self) -> &ClientMeta {
        static META: std::sync::OnceLock<ClientMeta> = std::sync::OnceLock::new();
        META.get_or_init(|| ClientMeta {
            id: super::ClientId::GeminiCli,
            display_name: "Gemini CLI",
            supported: true,
            unsupported_reason: None,
            config_format: "JSON (~/.gemini/settings.json)",
            requires_restart: true,
            limitations: &[
                "Disabled servers are omitted because settings.json has no enabled flag \
                 (enablement lives in ~/.gemini/mcp-server-enablement.json)",
                "HTTP servers use `httpUrl` (Streamable HTTP); `url` would mean SSE",
                "Server names should not contain `_` (Gemini tool-name splitting)",
                "Config changes require a CLI restart",
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
            let dir = home_path(&[".gemini"]).unwrap_or_default();
            if dir.exists() {
                status.detected = true;
                status.notes.push(format!(
                    "~/.gemini exists but settings.json does not; config path {}",
                    path.display()
                ));
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
            if let Some(map) = root.get("mcpServers").and_then(Value::as_object) {
                for (name, entry) in map {
                    if !entry.is_object() {
                        continue;
                    }
                    let transport = if entry.get("httpUrl").is_some() {
                        "http"
                    } else if entry.get("url").is_some() {
                        "sse"
                    } else {
                        "stdio"
                    };
                    servers.push(ServerSummary {
                        name: name.clone(),
                        transport: transport.to_string(),
                        managed: false,
                        enabled: true,
                    });
                }
            } else {
                notes.push("no mcpServers key present".into());
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
        let current = root
            .get("mcpServers")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();

        let mut actions = Vec::new();
        let mut warnings = Vec::new();

        for (name, def) in ctx.servers {
            let desired = Self::to_entry(name, def, ctx, &mut warnings)?;
            match desired {
                Some(desired) => {
                    let (change, detail) = classify_entry(
                        CLIENT,
                        name,
                        current.get(name),
                        &desired,
                        ctx.managed.servers.get(name),
                        ctx.options.force,
                    )?;
                    actions.push(SyncAction {
                        server: name.clone(),
                        change,
                        detail,
                    });
                }
                None => {
                    if current.contains_key(name) {
                        let managed = ctx.managed.servers.contains_key(name);
                        actions.push(SyncAction {
                            server: name.clone(),
                            change: if managed || ctx.options.force {
                                ChangeKind::Remove
                            } else {
                                ChangeKind::Conflict
                            },
                            detail: Some(
                                "disabled in registry; omitted from Gemini CLI settings".into(),
                            ),
                        });
                    } else {
                        actions.push(SyncAction {
                            server: name.clone(),
                            change: ChangeKind::Unchanged,
                            detail: Some("disabled in registry; already absent".into()),
                        });
                    }
                }
            }
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

        let mut current = root
            .get("mcpServers")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();

        let mut warnings = plan.warnings.clone();
        let mut applied = Vec::new();
        let mut skipped = Vec::new();
        let mut new_managed = ClientManagedState::default();

        for action in &plan.actions {
            match action.change {
                ChangeKind::Unchanged => {
                    if let Some(def) = ctx.servers.get(&action.server) {
                        if def.enabled {
                            let mut w = Vec::new();
                            let desired =
                                Self::to_entry(&action.server, def, ctx, &mut w).unwrap_or(None);
                            warnings.extend(w);
                            if let Some(desired) = desired {
                                new_managed.servers.insert(
                                    action.server.clone(),
                                    ManagedEntry {
                                        hash: crate::registry::hash_value(&desired),
                                        written_unix: now_unix(),
                                    },
                                );
                            }
                        }
                        applied.push(action.clone());
                    }
                }
                ChangeKind::Add | ChangeKind::Update => {
                    let Some(def) = ctx.servers.get(&action.server) else {
                        skipped.push(action.clone());
                        continue;
                    };
                    let mut w = Vec::new();
                    let desired = match Self::to_entry(&action.server, def, ctx, &mut w)? {
                        Some(v) => v,
                        None => {
                            skipped.push(action.clone());
                            continue;
                        }
                    };
                    warnings.extend(w);
                    current.insert(action.server.clone(), desired.clone());
                    new_managed.servers.insert(
                        action.server.clone(),
                        ManagedEntry {
                            hash: crate::registry::hash_value(&desired),
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

    fn stdio_server(cmd: &str) -> ServerDef {
        ServerDef {
            enabled: true,
            transport: Transport::Stdio {
                command: cmd.into(),
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
            pre_launch: vec![],
        }
    }

    #[tokio::test]
    async fn writes_httpurl_for_http_and_preserves_settings() {
        let _env_guard = crate::adapters::test_lock::env_lock();
        let dir = tempfile::tempdir().unwrap();
        unsafe {
            std::env::set_var("HOME", dir.path());
            // Windows BaseDirs resolves USERPROFILE, not HOME.
            std::env::set_var("USERPROFILE", dir.path());
        };
        let gem = dir.path().join(".gemini");
        std::fs::create_dir_all(&gem).unwrap();
        let cfg = gem.join("settings.json");
        std::fs::write(&cfg, r#"{"theme":"dark"}"#).unwrap();

        let paths = CerberusPaths::with_roots(dir.path().join("cfg"), dir.path().join("data"));
        paths.ensure_dirs().unwrap();
        let secrets = crate::secrets::SecretStore::default();
        let remote = ServerDef {
            enabled: true,
            transport: Transport::Http {
                url: "https://example.com/mcp".into(),
                headers: BTreeMap::new(),
                bearer_token: None,
            },
            mode: None,
            tags: vec![],
            timeout_ms: None,
            startup_timeout_ms: None,
            policy: ServerPolicy::default(),
            description: None,
            source: None,
            pre_launch: vec![],
        };
        let servers = BTreeMap::from([("remote".to_string(), remote)]);
        let managed = ClientManagedState::default();
        let ctx = SyncContext {
            paths: &paths,
            secrets: &secrets,
            servers: &servers,
            managed: &managed,
            options: ApplyOptions::default(),
        };

        let adapter = GeminiCliAdapter;
        let plan = adapter.plan_sync(&ctx).await.unwrap();
        let report = adapter.apply_sync(&ctx, &plan).await.unwrap();
        let written = parse_json_file(&report.config_path).unwrap();
        assert_eq!(written["theme"], "dark");
        assert_eq!(
            written["mcpServers"]["remote"]["httpUrl"],
            "https://example.com/mcp"
        );
        assert!(written["mcpServers"]["remote"].get("url").is_none());
    }

    #[tokio::test]
    async fn warns_on_underscore_names() {
        let dir = tempfile::tempdir().unwrap();
        let paths = CerberusPaths::with_roots(dir.path().join("cfg"), dir.path().join("data"));
        paths.ensure_dirs().unwrap();
        let secrets = crate::secrets::SecretStore::default();
        let servers = BTreeMap::from([("my_server".to_string(), stdio_server("npx"))]);
        let managed = ClientManagedState::default();
        let ctx = SyncContext {
            paths: &paths,
            secrets: &secrets,
            servers: &servers,
            managed: &managed,
            options: ApplyOptions::default(),
        };
        let adapter = GeminiCliAdapter;
        let plan = adapter.plan_sync(&ctx).await.unwrap();
        assert!(plan.warnings.iter().any(|w| w.contains("first underscore")));
    }
}
