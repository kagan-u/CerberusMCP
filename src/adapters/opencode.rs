//! OpenCode adapter.
//!
//! Verified against https://opencode.ai/docs/mcp-servers/ (v1, stable) and
//! the v2 schema at opencode.ai/v2/docs/mcp-servers/.
//!
//! Two live schema generations exist:
//! - **v1** (stable docs): flat `mcp` map; local servers use
//!   `type: "local"`, `command` as an **array**, `environment` (not `env`);
//!   remote servers use `type: "remote"` + `url`. Enablement via `enabled`.
//! - **v2**: `mcp.servers` map; enablement inverted via `disabled`;
//!   per-server `timeout` is an object.
//!
//! The adapter detects which shape an existing config uses and writes back in
//! that shape. `CERBERUS_OPENCODE_SCHEMA=1|2` forces a shape when no config
//! exists yet. Env references use OpenCode's `{env:NAME}` syntax.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::{Map, Value, json};

use super::{
    ChangeKind, ClientAdapter, ClientInspection, ClientManagedState, ClientMeta, ClientStatus,
    ManagedEntry, ServerSummary, SyncAction, SyncContext, SyncPlan, SyncReport, classify_entry,
    detect_by_config_path, ensure_object, now_unix, parse_json_file, plan_removals,
    write_json_file,
};
use crate::error::{CerberusError, Result};
use crate::paths::{CerberusPaths, home_path};
use crate::registry::schema::{EnvValue, HeaderValue, ServerDef, Transport};

#[derive(Debug, Clone, Copy)]
pub struct OpenCodeAdapter;

const CLIENT: &str = "opencode";

impl OpenCodeAdapter {
    pub fn new() -> Self {
        Self
    }

    /// OpenCode's global config location (`~/.config/opencode/opencode.json`),
    /// overridable via `OPENCODE_CONFIG`.
    pub fn default_config_path() -> Result<PathBuf> {
        if let Some(override_path) = std::env::var_os("OPENCODE_CONFIG") {
            return Ok(PathBuf::from(override_path));
        }
        home_path(&[".config", "opencode", "opencode.json"])
    }

    fn schema_generation(root: &Value) -> u32 {
        if let Some(forced) = forced_generation() {
            return forced;
        }
        // Existing config decides: v2 nests servers under `mcp.servers`.
        if root
            .get("mcp")
            .and_then(|m| m.get("servers"))
            .is_some_and(|v| v.is_object())
        {
            2
        } else {
            1
        }
    }

    /// Build the client-native entry for one server.
    fn to_entry(
        name: &str,
        def: &ServerDef,
        ctx: &SyncContext<'_>,
        generation: u32,
        warnings: &mut Vec<String>,
    ) -> Result<Value> {
        let enabled = def.enabled;
        match &def.transport {
            Transport::Stdio {
                command,
                args,
                env,
                cwd,
            } => {
                let mut entry = Map::new();
                entry.insert("type".into(), json!("local"));
                entry.insert(
                    "command".into(),
                    Value::Array(
                        std::iter::once(Value::String(command.clone()))
                            .chain(args.iter().cloned().map(Value::String))
                            .collect(),
                    ),
                );
                let environment = map_env(name, env, ctx, warnings)?;
                if !environment.is_empty() {
                    entry.insert("environment".into(), Value::Object(environment));
                }
                if let Some(cwd) = cwd {
                    entry.insert("cwd".into(), json!(cwd.to_string()));
                }
                apply_enablement(&mut entry, enabled, generation);
                if generation == 1 {
                    if let Some(ms) = def.timeout_ms {
                        entry.insert("timeout".into(), json!(ms));
                    }
                }
                Ok(Value::Object(entry))
            }
            Transport::Http {
                url,
                headers,
                bearer_token,
            } => {
                let mut entry = Map::new();
                entry.insert("type".into(), json!("remote"));
                entry.insert("url".into(), json!(url));
                let mut out_headers = Map::new();
                for (k, v) in headers {
                    out_headers.insert(k.clone(), resolve_header(name, k, v, ctx)?);
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
                apply_enablement(&mut entry, enabled, generation);
                Ok(Value::Object(entry))
            }
        }
    }
}

impl Default for OpenCodeAdapter {
    fn default() -> Self {
        Self::new()
    }
}

fn forced_generation() -> Option<u32> {
    match std::env::var("CERBERUS_OPENCODE_SCHEMA").ok()?.as_str() {
        "1" => Some(1),
        "2" => Some(2),
        _ => None,
    }
}

fn apply_enablement(entry: &mut Map<String, Value>, enabled: bool, generation: u32) {
    if generation == 2 {
        // v2 inverts the flag: only write `disabled` when true.
        if !enabled {
            entry.insert("disabled".into(), json!(true));
        }
    } else {
        entry.insert("enabled".into(), json!(enabled));
    }
}

fn map_env(
    server: &str,
    env: &BTreeMap<String, EnvValue>,
    ctx: &SyncContext<'_>,
    warnings: &mut Vec<String>,
) -> Result<Map<String, Value>> {
    let mut out = Map::new();
    for (key, value) in env {
        match value {
            EnvValue::Env { name } => {
                out.insert(key.clone(), json!(format!("{{env:{name}}}")));
            }
            EnvValue::SecretRef { name } => {
                out.insert(key.clone(), json!(ctx.secrets.resolve(name)?.expose()));
            }
            EnvValue::Literal { value } => {
                out.insert(key.clone(), json!(value));
            }
        }
    }
    let _ = server;
    let _ = warnings;
    Ok(out)
}

fn resolve_header(
    server: &str,
    header: &str,
    value: &HeaderValue,
    ctx: &SyncContext<'_>,
) -> Result<Value> {
    match value {
        HeaderValue::Resolved(EnvValue::Env { name }) => Ok(json!(format!("{{env:{name}}}"))),
        HeaderValue::Resolved(other) => Ok(json!(ctx.secrets.resolve_env_value(other)?)),
        HeaderValue::Literal(s) => {
            let _ = (server, header);
            Ok(json!(s))
        }
    }
}

fn existing_servers(root: &Value, generation: u32) -> Option<&Map<String, Value>> {
    let mcp = root.get("mcp")?;
    if generation == 2 || mcp.get("servers").is_some_and(|v| v.is_object()) {
        mcp.get("servers")?.as_object()
    } else {
        mcp.as_object()
    }
}

impl ClientAdapter for OpenCodeAdapter {
    fn meta(&self) -> &ClientMeta {
        static META: std::sync::OnceLock<ClientMeta> = std::sync::OnceLock::new();
        META.get_or_init(|| ClientMeta {
            id: super::ClientId::OpenCode,
            display_name: "OpenCode",
            supported: true,
            unsupported_reason: None,
            config_format: "JSON (opencode.json, v1 or v2 schema)",
            requires_restart: true,
            limitations: &[
                "Config changes require starting a new OpenCode session",
                "v2 per-server timeout objects are not written (numbers are v1-only)",
                "OAuth configuration is preserved but not managed",
            ],
        })
    }

    fn config_path(&self, _paths: &CerberusPaths) -> Result<PathBuf> {
        Self::default_config_path()
    }

    async fn detect(&self, paths: &CerberusPaths) -> Result<ClientStatus> {
        let path = self.config_path(paths)?;
        let mut status = detect_by_config_path(CLIENT, &path).await?;
        let jsonc = path.with_extension("jsonc");
        if jsonc.exists() {
            status.notes.push(format!(
                "{} also exists; CerberusMCP manages {} and leaves JSONC untouched",
                jsonc.display(),
                path.display()
            ));
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
            let generation = Self::schema_generation(&root);
            notes.push(format!("schema generation: v{generation}"));
            if let Some(map) = existing_servers(&root, generation) {
                for (name, entry) in map {
                    if !entry.is_object() {
                        continue;
                    }
                    // Heuristic: only report entries that look like MCP servers.
                    let is_server = entry
                        .get("type")
                        .is_some_and(|t| t == "local" || t == "remote")
                        || entry.get("command").is_some()
                        || entry.get("url").is_some();
                    if !is_server {
                        continue;
                    }
                    let transport = match entry.get("type").and_then(Value::as_str) {
                        Some("remote") => "http".to_string(),
                        Some("local") => "stdio".to_string(),
                        _ if entry.get("url").is_some() => "http".to_string(),
                        _ => "stdio".to_string(),
                    };
                    let enabled = if generation == 2 {
                        !entry
                            .get("disabled")
                            .and_then(Value::as_bool)
                            .unwrap_or(false)
                    } else {
                        entry
                            .get("enabled")
                            .and_then(Value::as_bool)
                            .unwrap_or(true)
                    };
                    servers.push(ServerSummary {
                        name: name.clone(),
                        transport,
                        managed: false,
                        enabled,
                    });
                }
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
        let (root, detected_gen) = if path.exists() {
            let root = parse_json_file(&path)?;
            let detected = Self::schema_generation(&root);
            (root, detected)
        } else {
            (Value::Object(Map::new()), 1)
        };
        let generation = forced_generation().unwrap_or(detected_gen);
        let current = existing_servers(&root, generation)
            .cloned()
            .unwrap_or_default();

        let mut actions = Vec::new();
        let mut warnings = Vec::new();
        let mut desired_entries: BTreeMap<String, Value> = BTreeMap::new();

        for (name, def) in ctx.servers {
            let entry = Self::to_entry(name, def, ctx, generation, &mut warnings)?;
            desired_entries.insert(name.clone(), entry);
        }

        for (name, desired) in &desired_entries {
            let current = current.get(name);
            let managed = ctx.managed.servers.get(name);
            let (change, detail) =
                classify_entry(CLIENT, name, current, desired, managed, ctx.options.force)?;
            actions.push(SyncAction {
                server: name.clone(),
                change,
                detail,
            });
        }

        // Removals: managed entries no longer desired.
        let removals = plan_removals(ctx.managed, &current, ctx.servers);
        for action in removals {
            if action.change == ChangeKind::Remove || action.change == ChangeKind::Conflict {
                actions.push(action);
            }
        }

        // Note: v1 cannot represent disabled entries for servers the user
        // deleted from the registry — that's fine, removal is removal.
        if generation == 2 {
            warnings.push(
                "OpenCode v2 schema detected; per-server timeout objects are not written".into(),
            );
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
        let generation = Self::schema_generation(&root);

        // Backup before modification.
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
        let mut applied = Vec::new();
        let mut skipped = Vec::new();
        let mut new_managed = ClientManagedState::default();

        // First, snapshot existing entries so removals/updates can mutate.
        let mut servers_container = existing_servers(&root, generation)
            .cloned()
            .unwrap_or_default();

        for action in &plan.actions {
            match action.change {
                ChangeKind::Unchanged => {
                    if let Some(def) = ctx.servers.get(&action.server) {
                        let mut w = Vec::new();
                        let desired = Self::to_entry(&action.server, def, ctx, generation, &mut w)?;
                        warnings.extend(w);
                        new_managed.servers.insert(
                            action.server.clone(),
                            ManagedEntry {
                                hash: crate::registry::hash_value(&desired),
                                written_unix: now_unix(),
                            },
                        );
                        applied.push(action.clone());
                    }
                }
                ChangeKind::Add | ChangeKind::Update => {
                    let Some(def) = ctx.servers.get(&action.server) else {
                        skipped.push(action.clone());
                        continue;
                    };
                    let mut w = Vec::new();
                    let desired = Self::to_entry(&action.server, def, ctx, generation, &mut w)?;
                    warnings.extend(w);
                    servers_container.insert(action.server.clone(), desired.clone());
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
                    servers_container.remove(&action.server);
                    applied.push(action.clone());
                }
                ChangeKind::Conflict => {
                    // Preserve the user's version; keep ownership only if the
                    // entry still matches what we previously wrote.
                    if let Some(cur) = servers_container.get(&action.server) {
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

        // Write back into the correct generation shape.
        if generation == 2 {
            let mcp = ensure_object(&mut root, "mcp");
            mcp.insert("servers".into(), Value::Object(servers_container));
        } else {
            let mcp = ensure_object(&mut root, "mcp");
            // Remove a stale nested `servers` key if we migrated shapes.
            mcp.remove("servers");
            for (k, v) in servers_container {
                mcp.insert(k, v);
            }
        }

        write_json_file(&path, &root)?;

        // Verify the result parses.
        let _ = parse_json_file(&path)?;

        // Persist ownership state.
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

use super::ManagedState;

/// Test helper: sync a server set into an in-memory config root.
#[cfg(test)]
pub(crate) fn test_ctx<'a>(
    paths: &'a CerberusPaths,
    secrets: &'a crate::secrets::SecretStore,
    servers: &'a BTreeMap<String, ServerDef>,
    managed: &'a ClientManagedState,
) -> SyncContext<'a> {
    SyncContext {
        paths,
        secrets,
        servers,
        managed,
        options: crate::adapters::ApplyOptions::default(),
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
            pre_launch: vec![],
        }
    }

    #[tokio::test]
    async fn writes_v1_shape_by_default() {
        let _env_guard = crate::adapters::test_lock::env_lock();
        let dir = tempfile::tempdir().unwrap();
        unsafe {
            std::env::set_var("OPENCODE_CONFIG", dir.path().join("opencode.json"));
        };
        let paths = CerberusPaths::with_roots(dir.path().join("cfg"), dir.path().join("data"));
        paths.ensure_dirs().unwrap();
        let secrets = crate::secrets::SecretStore::default();
        let servers = BTreeMap::from([(
            "demo".to_string(),
            stdio_server("npx", &["-y", "demo-server"]),
        )]);
        let managed = ClientManagedState::default();
        let ctx = test_ctx(&paths, &secrets, &servers, &managed);

        let adapter = OpenCodeAdapter;
        let plan = adapter.plan_sync(&ctx).await.unwrap();
        assert!(plan.has_changes);
        assert_eq!(plan.actions[0].change, ChangeKind::Add);

        let report = adapter.apply_sync(&ctx, &plan).await.unwrap();
        assert_eq!(report.applied.len(), 1);

        let written = parse_json_file(&report.config_path).unwrap();
        let entry = &written["mcp"]["demo"];
        assert_eq!(entry["type"], "local");
        assert_eq!(entry["command"][0], "npx");
        assert_eq!(entry["enabled"], true);

        // Idempotency: second run reports Unchanged.
        let managed2 = load_managed(&paths, CLIENT);
        let ctx2 = test_ctx(&paths, &secrets, &servers, &managed2);
        let plan2 = adapter.plan_sync(&ctx2).await.unwrap();
        assert!(!plan2.has_changes);
        unsafe {
            std::env::remove_var("OPENCODE_CONFIG");
        };
    }

    #[tokio::test]
    async fn preserves_unrelated_fields_and_user_servers() {
        let _env_guard = crate::adapters::test_lock::env_lock();
        let dir = tempfile::tempdir().unwrap();
        unsafe {
            std::env::set_var("OPENCODE_CONFIG", dir.path().join("opencode.json"));
        };
        let cfg = dir.path().join("opencode.json");
        std::fs::write(
            &cfg,
            r#"{"theme":"dark","mcp":{"user-owned":{"type":"local","command":["keep"]}}}"#,
        )
        .unwrap();

        let paths = CerberusPaths::with_roots(dir.path().join("cfg"), dir.path().join("data"));
        paths.ensure_dirs().unwrap();
        let secrets = crate::secrets::SecretStore::default();
        let servers = BTreeMap::from([("demo".to_string(), stdio_server("npx", &["-y", "demo"]))]);
        let managed = ClientManagedState::default();
        let ctx = test_ctx(&paths, &secrets, &servers, &managed);

        let adapter = OpenCodeAdapter;
        let plan = adapter.plan_sync(&ctx).await.unwrap();
        let report = adapter.apply_sync(&ctx, &plan).await.unwrap();

        let written = parse_json_file(&report.config_path).unwrap();
        assert_eq!(written["theme"], "dark");
        assert!(written["mcp"]["user-owned"].is_object());
        assert!(written["mcp"]["demo"].is_object());
        unsafe {
            std::env::remove_var("OPENCODE_CONFIG");
        };
    }

    #[tokio::test]
    async fn detects_v2_and_writes_disabled() {
        let _env_guard = crate::adapters::test_lock::env_lock();
        let dir = tempfile::tempdir().unwrap();
        unsafe {
            std::env::set_var("OPENCODE_CONFIG", dir.path().join("opencode.json"));
        };
        let cfg = dir.path().join("opencode.json");
        std::fs::write(
            &cfg,
            r#"{"mcp":{"servers":{"existing":{"type":"local","command":["x"]}}}}"#,
        )
        .unwrap();

        let paths = CerberusPaths::with_roots(dir.path().join("cfg"), dir.path().join("data"));
        paths.ensure_dirs().unwrap();
        let secrets = crate::secrets::SecretStore::default();
        let mut def = stdio_server("npx", &["-y", "demo"]);
        def.enabled = false;
        let servers = BTreeMap::from([("demo".to_string(), def)]);
        let managed = ClientManagedState::default();
        let ctx = test_ctx(&paths, &secrets, &servers, &managed);

        let adapter = OpenCodeAdapter;
        let plan = adapter.plan_sync(&ctx).await.unwrap();
        let report = adapter.apply_sync(&ctx, &plan).await.unwrap();
        let written = parse_json_file(&report.config_path).unwrap();
        assert_eq!(written["mcp"]["servers"]["demo"]["disabled"], true);
        assert!(written["mcp"]["servers"]["existing"].is_object());
        unsafe {
            std::env::remove_var("OPENCODE_CONFIG");
        };
    }

    fn load_managed(paths: &CerberusPaths, client: &str) -> ClientManagedState {
        ManagedState::load(&paths.managed_state_file())
            .unwrap()
            .clients
            .get(client)
            .cloned()
            .unwrap_or_default()
    }

    #[test]
    fn env_references_use_opencode_syntax() {
        let mut env = BTreeMap::new();
        env.insert("TOKEN".to_string(), EnvValue::env("MY_TOKEN"));
        env.insert("LIT".to_string(), EnvValue::literal("v"));
        let map = map_env("s", &env, &unsafe_dummy_ctx(), &mut Vec::new()).unwrap();
        assert_eq!(map["TOKEN"], "{env:MY_TOKEN}");
        assert_eq!(map["LIT"], "v");
    }

    fn unsafe_dummy_ctx() -> SyncContext<'static> {
        // map_env only touches ctx.secrets for SecretRef, which these tests
        // never use; leak statics for simplicity.
        let paths: &'static CerberusPaths = Box::leak(Box::new(CerberusPaths::with_roots(
            PathBuf::from("/tmp/c"),
            PathBuf::from("/tmp/d"),
        )));
        let secrets: &'static crate::secrets::SecretStore =
            Box::leak(Box::new(crate::secrets::SecretStore::default()));
        let servers: &'static BTreeMap<String, ServerDef> = Box::leak(Box::new(BTreeMap::new()));
        let managed: &'static ClientManagedState =
            Box::leak(Box::new(ClientManagedState::default()));
        SyncContext {
            paths,
            secrets,
            servers,
            managed,
            options: ApplyOptions::default(),
        }
    }
}
