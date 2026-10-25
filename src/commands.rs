//! CLI command implementations.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Serialize;

use crate::adapters::{Adapter, ApplyOptions, ChangeKind, ClientId};
use crate::cli::output::{self, Ctx, bold, check_mark, cyan, dim, green, red, yellow};
use crate::cli::{
    BackupCommand, ClientCommand, Command, ConfigCommand, GatewayCommand, ProfileCommand,
    ServerCommand,
};
use crate::error::{CerberusError, ExitCode, Result};
use crate::registry::schema::{
    EnvValue, ExecutionMode, Profile, Registry, SCHEMA_VERSION, ServerDef, ServerPolicy, SourceDef,
    Transport,
};
use crate::registry::{RegistryStore, atomic_write, validate};

pub async fn dispatch(ctx: &Ctx, command: Command) -> Result<i32> {
    match command {
        Command::Init { force } => cmd_init(ctx, force).await,
        Command::Config(c) => cmd_config(ctx, c).await,
        Command::Server(c) => cmd_server(ctx, c).await,
        Command::Client(c) => cmd_client(ctx, c).await,
        Command::Sync {
            clients,
            apply,
            dry_run: _,
            profile,
            force,
        } => cmd_sync(ctx, clients, apply, profile, force).await,
        Command::Diff { clients, profile } => cmd_diff(ctx, clients, profile).await,
        Command::Backup(c) => cmd_backup(ctx, c).await,
        Command::Profile(c) => cmd_profile(ctx, c).await,
        Command::Gateway(c) => cmd_gateway(ctx, c).await,
        Command::Search {
            query,
            limit,
            offline,
            registry,
        } => cmd_search(ctx, query, limit, offline, registry).await,
        Command::Install {
            source,
            rename,
            check,
            dry_run,
            disabled,
            tags,
            registry,
        } => {
            cmd_install(
                ctx, source, rename, check, dry_run, disabled, tags, registry,
            )
            .await
        }
        Command::Doctor { client, probe } => cmd_doctor(ctx, client, probe).await,
        Command::Health => cmd_health(ctx).await,
        Command::Logs { lines } => cmd_logs(ctx, lines).await,
        Command::Version => cmd_version(ctx),
        Command::Run { name, args } => cmd_run(ctx, name, args).await,
        Command::Completions { shell } => cmd_completions(ctx, &shell),
    }
}

fn load_store(ctx: &Ctx) -> Result<RegistryStore> {
    RegistryStore::load_with_migration(&ctx.paths)
}

fn save_store(_ctx: &Ctx, store: &mut RegistryStore) -> Result<()> {
    store.detect_concurrent_modification()?;
    store.save()
}

// ---------------------------------------------------------------------------
// init
// ---------------------------------------------------------------------------

async fn cmd_init(ctx: &Ctx, force: bool) -> Result<i32> {
    ctx.paths.ensure_dirs()?;
    let path = ctx.paths.registry_file.clone();
    if path.exists() && !force {
        return Err(CerberusError::Other(format!(
            "{} already exists (use --force to overwrite; a backup will be created)",
            path.display()
        )));
    }
    let legacy = ctx
        .paths
        .config_dir
        .join(crate::paths::LEGACY_REGISTRY_FILE_NAME);
    if !path.exists() && legacy.is_file() {
        if !force {
            return Err(CerberusError::Other(format!(
                "a legacy {} exists next to {}; run any read command \
                 (e.g. `cerberus server list`) to migrate it, or pass --force \
                 to start fresh (a backup of the legacy file will be kept)",
                crate::paths::LEGACY_REGISTRY_FILE_NAME,
                path.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "conf.cerberus".into())
            )));
        }
        if let Err(e) = crate::backup::create(&ctx.paths, &legacy, Some("pre-init-legacy")) {
            tracing::warn!(error = %e, "could not back up legacy registry before init --force");
        }
    }
    if path.exists() {
        crate::backup::create(&ctx.paths, &path, Some("pre-init"))?;
    }
    let mut store = RegistryStore {
        path: path.clone(),
        registry: Registry::default(),
        content_hash: 0,
    };
    store.save()?;

    if ctx.json {
        #[derive(Serialize)]
        struct InitReport {
            config: PathBuf,
            data: PathBuf,
            schema_version: u32,
        }
        ctx.emit_json(&InitReport {
            config: path,
            data: ctx.paths.data_dir.clone(),
            schema_version: SCHEMA_VERSION,
        });
    } else {
        ctx.out(&bold("CerberusMCP initialized"));
        ctx.status("Registry", &path.display().to_string());
        ctx.status("Data directory", &ctx.paths.data_dir.display().to_string());
        ctx.status("Schema", &format!("v{SCHEMA_VERSION}"));
        ctx.out(&dim(
            "Add a server: cerberus server add <name> --command npx --arg -y --arg <pkg>",
        ));
    }
    Ok(ExitCode::Success as i32)
}

// ---------------------------------------------------------------------------
// config
// ---------------------------------------------------------------------------

async fn cmd_config(ctx: &Ctx, command: ConfigCommand) -> Result<i32> {
    match command {
        ConfigCommand::Path => {
            #[derive(Serialize)]
            struct Paths {
                registry: PathBuf,
                config_dir: PathBuf,
                data_dir: PathBuf,
                backups: PathBuf,
                logs: PathBuf,
            }
            let paths = Paths {
                registry: ctx.paths.registry_file.clone(),
                config_dir: ctx.paths.config_dir.clone(),
                data_dir: ctx.paths.data_dir.clone(),
                backups: ctx.paths.backups_dir(),
                logs: ctx.paths.logs_dir(),
            };
            if ctx.json {
                ctx.emit_json(&paths);
            } else {
                ctx.status("Registry", &paths.registry.display().to_string());
                ctx.status("Config dir", &paths.config_dir.display().to_string());
                ctx.status("Data dir", &paths.data_dir.display().to_string());
                ctx.status("Backups", &paths.backups.display().to_string());
                ctx.status("Logs", &paths.logs.display().to_string());
            }
            Ok(ExitCode::Success as i32)
        }
        ConfigCommand::Validate => {
            let store = load_store(ctx)?;
            match validate::validate(&store.registry) {
                Ok(()) => {
                    if ctx.json {
                        ctx.emit_json(&serde_json::json!({
                            "valid": true,
                            "schema_version": store.registry.schema_version,
                            "servers": store.registry.servers.len(),
                        }));
                    } else {
                        ctx.out(&format!(
                            "{} registry is valid (schema v{}, {} server{})",
                            green("✓"),
                            store.registry.schema_version,
                            store.registry.servers.len(),
                            if store.registry.servers.len() == 1 {
                                ""
                            } else {
                                "s"
                            }
                        ));
                    }
                    Ok(ExitCode::Success as i32)
                }
                Err(e) => {
                    if ctx.json {
                        ctx.emit_json(
                            &serde_json::json!({"valid": false, "errors": e.to_string()}),
                        );
                    } else {
                        ctx.error(&e.to_string());
                    }
                    Ok(ExitCode::ConfigInvalid as i32)
                }
            }
        }
        ConfigCommand::Show => {
            let store = load_store(ctx)?;
            validate::validate(&store.registry)?;
            if ctx.json {
                ctx.emit_json(&store.registry);
            } else {
                // Redacted human rendering, in the same format as the file.
                let format = crate::registry::toml_io::format_for_path(&store.path);
                let text = match format {
                    crate::registry::RegistryFormat::Toml => {
                        crate::registry::toml_io::registry_to_toml(&store.registry)
                            .map_err(|e| CerberusError::Other(e.to_string()))?
                    }
                    crate::registry::RegistryFormat::Yaml => serde_yaml::to_string(&store.registry)
                        .map_err(|e| CerberusError::Other(e.to_string()))?,
                };
                let redacted = match format {
                    crate::registry::RegistryFormat::Toml => redact_toml(&text),
                    crate::registry::RegistryFormat::Yaml => redact_yaml(&text),
                };
                ctx.out(&redacted);
            }
            Ok(ExitCode::Success as i32)
        }
    }
}

/// Redact literal values in a YAML rendering for display.
fn redact_yaml(text: &str) -> String {
    // Replace `value: <something>` under literal sources and header literals.
    // Conservative: redact any `value:` line where the source line above did
    // not clearly mark it as non-secret; simplest safe rule — redact all
    // `value:` scalars and Authorization headers.
    let mut out = String::new();
    let mut redact_next_header = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("Authorization:") {
            out.push_str(&line.replace(
                line.split_once(':').map(|(_, v)| v).unwrap_or(""),
                " <redacted>",
            ));
            out.push('\n');
            continue;
        }
        if trimmed == "source: literal" {
            out.push_str(line);
            out.push('\n');
            redact_next_header = true;
            continue;
        }
        if redact_next_header && trimmed.starts_with("value:") {
            let indent = &line[..line.len() - trimmed.len()];
            out.push_str(&format!("{indent}value: <redacted>\n"));
            redact_next_header = false;
            continue;
        }
        redact_next_header = false;
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Redact literal values and Authorization headers in a TOML rendering.
///
/// Only `EnvValue::Literal` emits a `value = "..."` key (commands, hooks,
/// and transports have no other field named `value`), so redacting every
/// `value` scalar is both conservative and complete.
fn redact_toml(text: &str) -> String {
    text.lines()
        .map(|line| {
            let trimmed = line.trim_start();
            let lower = trimmed.to_ascii_lowercase();
            let is_auth = lower.starts_with("authorization")
                && lower["authorization".len()..].trim_start().starts_with('=');
            if is_auth {
                return redact_quoted_after(line, line.find('=').unwrap_or(0));
            }
            if let Some(idx) = line.find("value = \"") {
                return redact_quoted_after(line, idx);
            }
            line.to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Replace the quoted string starting at (or after) `from` with `<redacted>`,
/// keeping everything outside the first…last quote intact.
fn redact_quoted_after(line: &str, from: usize) -> String {
    let bytes = line.as_bytes();
    let Some(open_rel) = bytes[from.min(bytes.len())..]
        .iter()
        .position(|b| *b == b'"')
    else {
        return line.to_string();
    };
    let open = from + open_rel;
    let Some(close_rel) = bytes[open + 1..].iter().rposition(|b| *b == b'"') else {
        return line.to_string();
    };
    let close = open + 1 + close_rel;
    format!("{}<redacted>{}", &line[..open + 1], &line[close..])
}

// ---------------------------------------------------------------------------
// run
// ---------------------------------------------------------------------------

/// Run a named `[commands.*]` entry, or list available commands when no name
/// is given. The child's environment is the full process environment with the
/// command's registry bindings overlaid; its exit code is passed through.
async fn cmd_run(ctx: &Ctx, name: Option<String>, extra: Vec<String>) -> Result<i32> {
    let store = load_store(ctx)?;
    validate::validate(&store.registry)?;

    let Some(name) = name else {
        if ctx.json {
            let entries: Vec<_> = store
                .registry
                .commands
                .iter()
                .map(|(n, c)| {
                    serde_json::json!({
                        "name": n,
                        "run": c.run,
                        "description": c.description,
                        "timeout_ms": c.timeout_ms,
                    })
                })
                .collect();
            ctx.emit_json(&entries);
        } else if store.registry.commands.is_empty() {
            ctx.out("no commands defined ([commands.<name>] in the registry)");
        } else {
            for (n, c) in &store.registry.commands {
                let value = c.description.as_deref().unwrap_or("");
                ctx.status(n, value);
            }
        }
        return Ok(ExitCode::Success as i32);
    };

    let command = store.registry.commands.get(&name).ok_or_else(|| {
        CerberusError::NotFound(format!(
            "command `{name}` is not defined (add it under [commands.{name}])"
        ))
    })?;

    let mut argv = command.run.clone();
    argv.extend(extra);

    let secrets = crate::secrets::SecretStore::default();
    let mut child = tokio::process::Command::new(&argv[0]);
    child.args(&argv[1..]);
    for (key, value) in &command.env {
        child.env(key, secrets.resolve_env_value(value)?);
    }
    if let Some(cwd) = &command.cwd {
        child.current_dir(crate::paths::expand_tilde(cwd));
    }

    // `--json` captures the child's output so the final JSON document stays
    // machine-parseable; otherwise the child shares this process's stdio.
    let capture = ctx.json;
    if capture {
        child.stdin(std::process::Stdio::null());
        child.stdout(std::process::Stdio::piped());
        child.stderr(std::process::Stdio::piped());
    }

    let started = std::time::Instant::now();
    let mut proc = child
        .spawn()
        .map_err(|source| CerberusError::ProcessSpawn {
            command: argv[0].clone(),
            source,
        })?;

    let (stdout_pipe, stderr_pipe) = if capture {
        (proc.stdout.take(), proc.stderr.take())
    } else {
        (None, None)
    };
    let read_stdout = tokio::spawn(async move {
        use tokio::io::AsyncReadExt;
        let mut buf = Vec::new();
        if let Some(mut pipe) = stdout_pipe {
            let _ = pipe.read_to_end(&mut buf).await;
        }
        buf
    });
    let read_stderr = tokio::spawn(async move {
        use tokio::io::AsyncReadExt;
        let mut buf = Vec::new();
        if let Some(mut pipe) = stderr_pipe {
            let _ = pipe.read_to_end(&mut buf).await;
        }
        buf
    });

    let to_spawn_err = |source: std::io::Error| CerberusError::ProcessSpawn {
        command: argv[0].clone(),
        source,
    };
    let wait = proc.wait();
    let status = if let Some(ms) = command.timeout_ms {
        match tokio::time::timeout(std::time::Duration::from_millis(ms), wait).await {
            Ok(res) => res.map_err(to_spawn_err)?,
            Err(_) => {
                let _ = proc.start_kill();
                let _ = proc.wait().await;
                return Err(CerberusError::Timeout { timeout_ms: ms });
            }
        }
    } else {
        wait.await.map_err(to_spawn_err)?
    };
    let code = status.code().unwrap_or(1);
    let duration_ms = started.elapsed().as_millis() as u64;

    if capture {
        let stdout = String::from_utf8_lossy(&read_stdout.await.unwrap_or_default()).into_owned();
        let stderr = String::from_utf8_lossy(&read_stderr.await.unwrap_or_default()).into_owned();
        ctx.emit_json(&serde_json::json!({
            "command": name,
            "argv": argv,
            "exit_code": code,
            "duration_ms": duration_ms,
            "stdout": stdout,
            "stderr": stderr,
        }));
    }
    Ok(code)
}

// ---------------------------------------------------------------------------
// server
// ---------------------------------------------------------------------------

async fn cmd_server(ctx: &Ctx, command: ServerCommand) -> Result<i32> {
    match command {
        ServerCommand::List { tag } => {
            let store = load_store(ctx)?;
            let rows: Vec<_> = store
                .registry
                .servers
                .iter()
                .filter(|(_, s)| tag.as_deref().is_none_or(|t| s.tags.iter().any(|x| x == t)))
                .map(|(name, s)| {
                    let mode = s.effective_mode(&store.registry.settings);
                    ServerRow {
                        name: name.clone(),
                        enabled: s.enabled,
                        transport: s.transport.kind().to_string(),
                        mode: format!("{mode:?}").to_lowercase(),
                        command: s.transport.command_display(),
                        tags: s.tags.clone(),
                    }
                })
                .collect();
            if ctx.json {
                ctx.emit_json(&rows);
            } else if rows.is_empty() {
                ctx.out(&dim(
                    "no servers registered (cerberus server add <name> --command ...)",
                ));
            } else {
                ctx.out(&format!(
                    "{} {}",
                    bold("Servers"),
                    dim(&format!("({})", rows.len()))
                ));
                ctx.out("");
                for row in &rows {
                    let state = if row.enabled {
                        green("enabled")
                    } else {
                        yellow("disabled")
                    };
                    ctx.out(&format!(
                        "  {}  {}  {}  {}  {}",
                        bold(&row.name),
                        state,
                        cyan(&row.transport),
                        dim(&row.mode),
                        dim(&row.command)
                    ));
                }
            }
            Ok(ExitCode::Success as i32)
        }
        ServerCommand::Add {
            name,
            transport,
            command,
            args,
            url,
            env,
            tags,
            timeout_ms,
            startup_timeout_ms,
            mode,
            description,
        } => {
            let mut store = load_store(ctx)?;
            let transport = match transport.as_str() {
                "stdio" => {
                    let command = command.ok_or_else(|| {
                        CerberusError::Validation(
                            "--command is required for stdio transport".into(),
                        )
                    })?;
                    Transport::Stdio {
                        command,
                        args,
                        env: parse_env_bindings(&env)?,
                        cwd: None,
                    }
                }
                "http" => {
                    let url = url.ok_or_else(|| {
                        CerberusError::Validation("--url is required for http transport".into())
                    })?;
                    Transport::Http {
                        url,
                        headers: BTreeMap::new(),
                        bearer_token: None,
                    }
                }
                other => {
                    return Err(CerberusError::Validation(format!(
                        "unknown transport `{other}` (expected stdio or http)"
                    )));
                }
            };
            let mode = match mode.as_deref() {
                None => None,
                Some("direct") => Some(ExecutionMode::Direct),
                Some("gateway") => Some(ExecutionMode::Gateway),
                Some(other) => {
                    return Err(CerberusError::Validation(format!(
                        "unknown mode `{other}` (expected direct or gateway)"
                    )));
                }
            };
            let tags: Vec<String> = tags
                .iter()
                .flat_map(|t| t.split(','))
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect();
            let def = ServerDef {
                enabled: true,
                transport,
                mode,
                tags,
                timeout_ms,
                startup_timeout_ms,
                policy: ServerPolicy::default(),
                description,
                source: None,
                pre_launch: vec![],
            };
            store.add_server(&name, def.clone())?;
            save_store(ctx, &mut store)?;
            if ctx.json {
                ctx.emit_json(&serde_json::json!({"added": name}));
            } else {
                ctx.out(&format!("{} server `{name}`", green("✓ added"),));
            }
            Ok(ExitCode::Success as i32)
        }
        ServerCommand::Inspect { name } => {
            let store = load_store(ctx)?;
            let def = store
                .registry
                .servers
                .get(&name)
                .ok_or_else(|| CerberusError::ServerNotFound { name: name.clone() })?;
            #[derive(Serialize)]
            struct Detail<'a> {
                name: &'a str,
                enabled: bool,
                transport: &'a str,
                command: String,
                mode: String,
                tags: &'a [String],
                timeout_ms: Option<u64>,
                startup_timeout_ms: Option<u64>,
                policy: &'a ServerPolicy,
                description: Option<&'a str>,
                env_keys: Vec<String>,
                source: Option<&'a SourceDef>,
            }
            let env_keys = match &def.transport {
                Transport::Stdio { env, .. } => env.keys().cloned().collect(),
                Transport::Http { headers, .. } => headers.keys().cloned().collect(),
            };
            let detail = Detail {
                name: &name,
                enabled: def.enabled,
                transport: def.transport.kind(),
                command: def.transport.command_display(),
                mode: format!("{:?}", def.effective_mode(&store.registry.settings)).to_lowercase(),
                tags: &def.tags,
                timeout_ms: def.timeout_ms,
                startup_timeout_ms: def.startup_timeout_ms,
                policy: &def.policy,
                description: def.description.as_deref(),
                env_keys,
                source: def.source.as_ref(),
            };
            if ctx.json {
                ctx.emit_json(&detail);
            } else {
                ctx.out(&bold(&format!("Server: {name}")));
                ctx.status("Enabled", if def.enabled { "yes" } else { "no" });
                ctx.status("Transport", def.transport.kind());
                match def.transport.kind() {
                    "http" => ctx.status("URL", &def.transport.command_display()),
                    _ => ctx.status("Command", &def.transport.command_display()),
                }
                ctx.status("Mode", &detail.mode);
                if !def.tags.is_empty() {
                    ctx.status("Tags", &def.tags.join(", "));
                }
                if !detail.env_keys.is_empty() {
                    ctx.status("Env keys", &detail.env_keys.join(", "));
                }
                if !def.policy.allow_tools.is_empty() || !def.policy.deny_tools.is_empty() {
                    ctx.status("Allow tools", &def.policy.allow_tools.join(", "));
                    ctx.status("Deny tools", &def.policy.deny_tools.join(", "));
                }
                if let Some(source) = &def.source {
                    let kind = format!("{:?}", source.kind).to_lowercase();
                    let package = source.package.as_deref().unwrap_or("-");
                    ctx.status("Source", &format!("{kind} {package}"));
                }
            }
            Ok(ExitCode::Success as i32)
        }
        ServerCommand::Enable { name } => set_enabled(ctx, &name, true).await,
        ServerCommand::Disable { name } => set_enabled(ctx, &name, false).await,
        ServerCommand::Remove { name, yes } => {
            let mut store = load_store(ctx)?;
            if !yes && !ctx.json {
                if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
                    return Err(CerberusError::Validation(
                        "refusing to remove without --yes in non-interactive mode".into(),
                    ));
                }
                eprint!("Remove server `{name}`? [y/N] ");
                use std::io::Write as _;
                std::io::stderr().flush().ok();
                let mut answer = String::new();
                std::io::stdin().read_line(&mut answer).ok();
                if !matches!(answer.trim(), "y" | "Y" | "yes") {
                    ctx.out("aborted");
                    return Ok(ExitCode::Success as i32);
                }
            }
            store.remove_server(&name)?;
            save_store(ctx, &mut store)?;
            if ctx.json {
                ctx.emit_json(&serde_json::json!({"removed": name}));
            } else {
                ctx.out(&format!("{} removed `{name}` (client configs still hold it; run `cerberus sync --apply` to clean up)", green("✓")));
            }
            Ok(ExitCode::Success as i32)
        }
    }
}

#[derive(Serialize)]
struct ServerRow {
    name: String,
    enabled: bool,
    transport: String,
    mode: String,
    command: String,
    tags: Vec<String>,
}

fn parse_env_bindings(bindings: &[String]) -> Result<BTreeMap<String, EnvValue>> {
    let mut out = BTreeMap::new();
    for b in bindings {
        let (k, v) = output::parse_env_binding(b).map_err(CerberusError::Validation)?;
        out.insert(k, v);
    }
    Ok(out)
}

async fn set_enabled(ctx: &Ctx, name: &str, enable: bool) -> Result<i32> {
    let mut store = load_store(ctx)?;
    store.set_enabled(name, enable)?;
    save_store(ctx, &mut store)?;
    if ctx.json {
        ctx.emit_json(&serde_json::json!({"server": name, "enabled": enable}));
    } else {
        ctx.out(&format!(
            "{} server `{name}` {}",
            green("✓"),
            if enable { "enabled" } else { "disabled" }
        ));
    }
    Ok(ExitCode::Success as i32)
}

// ---------------------------------------------------------------------------
// search / install (official MCP Registry)
// ---------------------------------------------------------------------------

/// Short descriptor used in listings: package type, `remote`, or `-`.
fn catalog_kind(server: &crate::registry::catalog::RemoteServer) -> String {
    if let Some(package) = server.packages.first() {
        return package.registry_type.clone();
    }
    if !server.remotes.is_empty() {
        return "remote".to_string();
    }
    "-".to_string()
}

async fn cmd_search(
    ctx: &Ctx,
    query: String,
    limit: usize,
    offline: bool,
    registry: Option<String>,
) -> Result<i32> {
    let limit = limit.clamp(1, 100);
    let cache_path = crate::registry::catalog::SearchCache::path(&ctx.paths.data_dir);

    let results = if offline {
        let cache = crate::registry::catalog::SearchCache::load(&cache_path)?;
        let entry = cache.get(&query).ok_or_else(|| {
            CerberusError::NotFound(format!(
                "no cached results for `{query}`; run `cerberus search \"{query}\"` once with \
                 network access, then retry with --offline"
            ))
        })?;
        entry.results.clone()
    } else {
        let client = crate::registry::catalog::RegistryClient::new(registry)?;
        // Ask for extra rows: one server can occupy several of them, one per
        // published version, and they collapse during deduplication below.
        let page = client.search(&query, (limit * 4).min(100), None).await?;
        let results = page.into_servers();
        let mut cache = crate::registry::catalog::SearchCache::load(&cache_path)?;
        cache.save(&cache_path, &query, results.clone());
        results
    };
    // The registry returns one row per published version: collapse them.
    let results = crate::registry::catalog::dedup_latest(results);
    let results = {
        let mut r = results;
        r.truncate(limit);
        r
    };

    if ctx.json {
        let rows: Vec<serde_json::Value> = results
            .iter()
            .map(|server| {
                serde_json::json!({
                    "name": server.name,
                    "kind": catalog_kind(server),
                    "version": server.version,
                    "description": server.description,
                    "homepage": server.repository.as_ref().map(|r| &r.url),
                    "packages": server.packages.iter().map(|p| p.identifier.clone()).collect::<Vec<_>>(),
                })
            })
            .collect();
        ctx.emit_json(&serde_json::json!({
            "query": query,
            "count": rows.len(),
            "offline": offline,
            "results": rows,
        }));
        return Ok(ExitCode::Success as i32);
    }

    if results.is_empty() {
        ctx.out(&yellow("no results"));
        ctx.out(&dim(
            "try a different query, or check https://registry.modelcontextprotocol.io",
        ));
        return Ok(ExitCode::Success as i32);
    }

    ctx.out(&format!(
        "{} {}",
        bold(&format!("{} result(s)", results.len())),
        dim(&format!("for `{query}`"))
    ));
    for server in &results {
        let version = server.version.as_deref().unwrap_or("-");
        ctx.out(&format!(
            "  {}  {}  {}",
            cyan(&server.name),
            dim(&catalog_kind(server)),
            dim(version)
        ));
        if let Some(description) = &server.description {
            let flat: String = description.split_whitespace().collect::<Vec<_>>().join(" ");
            let short: String = flat.chars().take(100).collect();
            let suffix = if flat.chars().count() > 100 {
                "..."
            } else {
                ""
            };
            ctx.out(&format!("      {short}{suffix}"));
        }
    }
    ctx.out(&dim("\n  install with: cerberus install <name>"));
    Ok(ExitCode::Success as i32)
}

/// Load a registry record from a local JSON file or from the registry.
async fn resolve_registry_source(
    source: &str,
    registry: Option<String>,
) -> Result<crate::registry::catalog::RemoteServer> {
    use crate::registry::catalog::RemoteServer;

    let path = PathBuf::from(source);
    if path.is_file() {
        let text = std::fs::read_to_string(&path).map_err(|source_err| CerberusError::Io {
            path: path.clone(),
            source: source_err,
        })?;
        #[derive(serde::Deserialize)]
        struct Envelope {
            server: RemoteServer,
        }
        if let Ok(envelope) = serde_json::from_str::<Envelope>(&text) {
            return Ok(envelope.server);
        }
        return serde_json::from_str(&text).map_err(|e| {
            CerberusError::Validation(format!(
                "failed to parse `{}` as a registry record: {e}",
                path.display()
            ))
        });
    }

    let client = crate::registry::catalog::RegistryClient::new(registry)?;
    client.fetch_exact(source).await
}

#[allow(clippy::too_many_arguments)]
async fn cmd_install(
    ctx: &Ctx,
    source: String,
    rename: Option<String>,
    check: bool,
    dry_run: bool,
    disabled: bool,
    tags: Vec<String>,
    registry: Option<String>,
) -> Result<i32> {
    let mut store = load_store(ctx)?;
    let record = resolve_registry_source(&source, registry).await?;
    let planned = crate::registry::catalog::materialize(&record)?;

    let name = match rename {
        Some(name) => name,
        None => crate::registry::catalog::derive_name(&record.name).ok_or_else(|| {
            CerberusError::Validation(format!(
                "could not derive a local name from `{}`; pass --name <NAME>",
                record.name
            ))
        })?,
    };
    if store.registry.servers.contains_key(&name) {
        return Err(CerberusError::ServerExists { name });
    }

    if check {
        if let Transport::Stdio { command, .. } = &planned.transport {
            if !crate::process::executable_exists(command) {
                return Err(CerberusError::Other(format!(
                    "launcher `{command}` was not found on PATH; install it (e.g. \
                     `npm install -g {command}`) and retry, or omit --check"
                )));
            }
        }
    }

    let tags: Vec<String> = tags
        .iter()
        .flat_map(|t| t.split(','))
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();

    let def = ServerDef {
        enabled: !disabled,
        transport: planned.transport,
        mode: None,
        tags,
        timeout_ms: None,
        startup_timeout_ms: None,
        policy: ServerPolicy::default(),
        description: planned.description.clone(),
        source: planned.source.clone(),
        pre_launch: vec![],
    };
    validate::validate_server_def(&name, &def)?;

    if dry_run {
        if ctx.json {
            ctx.emit_json(&serde_json::json!({
                "name": name,
                "registry_name": record.name,
                "server": def,
                "warnings": planned.warnings,
            }));
        } else {
            ctx.out(&dim(&format!("# would install as `{name}`")));
            match serde_yaml::to_string(&def) {
                Ok(yaml) => ctx.out(yaml.trim_end()),
                Err(e) => ctx.out(&format!("# entry unavailable: {e}")),
            }
            for warning in &planned.warnings {
                ctx.warn(warning);
            }
        }
        return Ok(ExitCode::Success as i32);
    }

    store.add_server(&name, def)?;
    save_store(ctx, &mut store)?;

    if ctx.json {
        ctx.emit_json(&serde_json::json!({
            "installed": name,
            "registry_name": record.name,
            "enabled": !disabled,
            "warnings": planned.warnings,
            "next": ["cerberus sync --apply"],
        }));
        return Ok(ExitCode::Success as i32);
    }

    ctx.out(&format!("{} server `{name}`", green("✓ installed")));
    ctx.status("registry name", &record.name);
    if let Some(kind) = planned
        .source
        .as_ref()
        .map(|s| format!("{:?}", s.kind).to_lowercase())
    {
        ctx.status("source", &kind);
    }
    ctx.status("transport", if disabled { "disabled" } else { "enabled" });
    for warning in &planned.warnings {
        ctx.warn(warning);
    }
    ctx.out(&dim(
        "next: cerberus sync --apply   (write client configurations)",
    ));
    Ok(ExitCode::Success as i32)
}

// ---------------------------------------------------------------------------
// client
// ---------------------------------------------------------------------------

async fn cmd_client(ctx: &Ctx, command: ClientCommand) -> Result<i32> {
    match command {
        ClientCommand::List => {
            let rows: Vec<_> = Adapter::all()
                .into_iter()
                .map(|a| {
                    let m = a.meta();
                    ClientRow {
                        id: m.id.as_str().to_string(),
                        display_name: m.display_name.to_string(),
                        supported: m.supported,
                        config_format: m.config_format.to_string(),
                        requires_restart: m.requires_restart,
                        reason: m.unsupported_reason.map(str::to_string),
                    }
                })
                .collect();
            if ctx.json {
                ctx.emit_json(&rows);
            } else {
                ctx.out(&bold("AI Clients"));
                ctx.out("");
                for row in &rows {
                    let support = if row.supported {
                        green("supported")
                    } else {
                        red("unsupported")
                    };
                    ctx.out(&format!(
                        "  {}  {}  {}",
                        bold(&row.id),
                        support,
                        dim(&row.config_format)
                    ));
                    if let Some(reason) = &row.reason {
                        ctx.out(&format!("      {}", dim(reason)));
                    }
                }
            }
            Ok(ExitCode::Success as i32)
        }
        ClientCommand::Detect => {
            let mut reports = Vec::new();
            for adapter in Adapter::all() {
                let status = adapter.detect(&ctx.paths).await?;
                reports.push(status);
            }
            if ctx.json {
                ctx.emit_json(&reports);
            } else {
                ctx.out(&bold("Client detection"));
                ctx.out("");
                for status in &reports {
                    let mark = if status.detected {
                        green("detected")
                    } else {
                        dim("not detected")
                    };
                    ctx.out(&format!("  {}  {}", bold(&status.client), mark));
                    for note in &status.notes {
                        ctx.out(&format!("      {}", dim(note)));
                    }
                }
            }
            Ok(ExitCode::Success as i32)
        }
        ClientCommand::Inspect { name } => {
            let id = ClientId::parse(&name)
                .ok_or_else(|| CerberusError::ClientNotFound { name: name.clone() })?;
            let adapter = Adapter::by_id(id)?;
            let inspection = adapter.inspect(&ctx.paths).await?;
            if ctx.json {
                ctx.emit_json(&inspection);
            } else {
                ctx.out(&bold(&format!(
                    "{} ({})",
                    inspection.display_name, inspection.client
                )));
                ctx.status(
                    "Support",
                    if inspection.supported {
                        "supported"
                    } else {
                        "unsupported"
                    },
                );
                ctx.status("Detected", if inspection.detected { "yes" } else { "no" });
                if let Some(p) = &inspection.config_path {
                    ctx.status("Config", &p.display().to_string());
                }
                if !inspection.servers.is_empty() {
                    ctx.out("");
                    ctx.out(&bold("  MCP servers:"));
                    let managed_names =
                        crate::adapters::ManagedState::load(&ctx.paths.managed_state_file())?
                            .clients
                            .get(inspection.client.as_str())
                            .map(|m| m.servers.keys().cloned().collect::<Vec<_>>())
                            .unwrap_or_default();
                    for s in &inspection.servers {
                        let tag = if managed_names.contains(&s.name) {
                            cyan("[cerberus]")
                        } else {
                            dim("[user]")
                        };
                        ctx.out(&format!(
                            "    {} {} {} {}",
                            s.name,
                            tag,
                            dim(&s.transport),
                            if s.enabled {
                                String::new()
                            } else {
                                yellow("disabled")
                            }
                        ));
                    }
                }
                for note in &inspection.notes {
                    ctx.out(&format!("  {}", dim(note)));
                }
                for l in &inspection.limitations {
                    ctx.out(&format!("  {}", yellow(l)));
                }
            }
            Ok(ExitCode::Success as i32)
        }
        ClientCommand::Sync {
            name,
            apply,
            force,
            profile,
        } => {
            let id = ClientId::parse(&name)
                .ok_or_else(|| CerberusError::ClientNotFound { name: name.clone() })?;
            let adapter = Adapter::by_id(id)?;
            let mode = if apply {
                crate::sync::SyncMode::Apply
            } else {
                crate::sync::SyncMode::DryRun
            };
            let outcome = crate::sync::run(
                &ctx.paths,
                &load_store(ctx)?,
                &crate::secrets::SecretStore::default(),
                crate::sync::SyncRequest {
                    clients: vec![adapter],
                    mode,
                    profile,
                    options: ApplyOptions { force },
                },
            )
            .await?;
            emit_sync_outcome(ctx, &outcome)?;
            Ok(sync_exit_code(&outcome))
        }
    }
}

#[derive(Serialize)]
struct ClientRow {
    id: String,
    display_name: String,
    supported: bool,
    config_format: String,
    requires_restart: bool,
    reason: Option<String>,
}

// ---------------------------------------------------------------------------
// sync / diff
// ---------------------------------------------------------------------------

async fn cmd_sync(
    ctx: &Ctx,
    clients: Vec<String>,
    apply: bool,
    profile: Option<String>,
    force: bool,
) -> Result<i32> {
    let adapters = resolve_adapters(&clients)?;
    let mode = if apply {
        crate::sync::SyncMode::Apply
    } else {
        crate::sync::SyncMode::DryRun
    };
    let outcome = crate::sync::run(
        &ctx.paths,
        &load_store(ctx)?,
        &crate::secrets::SecretStore::default(),
        crate::sync::SyncRequest {
            clients: adapters,
            mode,
            profile,
            options: ApplyOptions { force },
        },
    )
    .await?;
    emit_sync_outcome(ctx, &outcome)?;
    Ok(sync_exit_code(&outcome))
}

async fn cmd_diff(ctx: &Ctx, clients: Vec<String>, profile: Option<String>) -> Result<i32> {
    let adapters = resolve_adapters(&clients)?;
    let outcome = crate::sync::run(
        &ctx.paths,
        &load_store(ctx)?,
        &crate::secrets::SecretStore::default(),
        crate::sync::SyncRequest {
            clients: adapters,
            mode: crate::sync::SyncMode::DryRun,
            profile,
            options: ApplyOptions::default(),
        },
    )
    .await?;
    if ctx.json {
        ctx.emit_json(&outcome);
    } else {
        for result in &outcome.results {
            ctx.out(&bold(&result.client));
            if let Some(err) = &result.error {
                ctx.out(&format!("  {}", red(err)));
                continue;
            }
            let changes: Vec<_> = result
                .plan
                .actions
                .iter()
                .filter(|a| a.change != ChangeKind::Unchanged)
                .collect();
            if changes.is_empty() {
                ctx.out(&format!("  {} {}", green("✓"), dim("in sync")));
            } else {
                for action in changes {
                    let symbol = match action.change {
                        ChangeKind::Add => green("+"),
                        ChangeKind::Update => yellow("~"),
                        ChangeKind::Remove => red("-"),
                        ChangeKind::Conflict => red("!"),
                        ChangeKind::Unchanged => continue,
                    };
                    ctx.out(&format!(
                        "  {} {} {}",
                        symbol,
                        action.server,
                        action.detail.as_deref().map(dim).unwrap_or_default()
                    ));
                }
            }
            ctx.out("");
        }
    }
    Ok(ExitCode::Success as i32)
}

fn resolve_adapters(clients: &[String]) -> Result<Vec<Adapter>> {
    if clients.is_empty() {
        return Ok(Adapter::supported());
    }
    clients
        .iter()
        .map(|name| {
            let id = ClientId::parse(name)
                .ok_or_else(|| CerberusError::ClientNotFound { name: name.clone() })?;
            let adapter = Adapter::by_id(id)?;
            if !adapter.meta().supported {
                return Err(CerberusError::UnsupportedClient {
                    name: name.clone(),
                    reason: adapter
                        .meta()
                        .unsupported_reason
                        .unwrap_or("unsupported")
                        .to_string(),
                });
            }
            Ok(adapter)
        })
        .collect()
}

fn emit_sync_outcome(ctx: &Ctx, outcome: &crate::sync::SyncOutcome) -> Result<()> {
    if ctx.json {
        ctx.emit_json(outcome);
        return Ok(());
    }
    let dry = matches!(outcome.mode, crate::sync::SyncMode::DryRun);
    for result in &outcome.results {
        ctx.out(&bold(&result.client));
        if let Some(err) = &result.error {
            ctx.out(&format!("  {}", red(err)));
            continue;
        }
        for action in &result.plan.actions {
            let (symbol, label) = match action.change {
                ChangeKind::Add => (green("+"), "add"),
                ChangeKind::Update => (yellow("~"), "update"),
                ChangeKind::Remove => (red("-"), "remove"),
                ChangeKind::Conflict => (red("!"), "conflict"),
                ChangeKind::Unchanged => (dim("="), "unchanged"),
            };
            if action.change == ChangeKind::Unchanged && ctx.quiet {
                continue;
            }
            ctx.out(&format!(
                "  {} {label:<9} {} {}",
                symbol,
                action.server,
                action.detail.as_deref().map(dim).unwrap_or_default()
            ));
        }
        if let Some(report) = &result.report {
            if let Some(id) = &report.backup_id {
                ctx.out(&format!("  {}", dim(&format!("backup: {id}"))));
            }
        }
        for w in &result.plan.warnings {
            ctx.out(&format!("  {}", yellow(&format!("warning: {w}"))));
        }
        if dry && result.plan.has_changes {
            ctx.out(&format!(
                "  {}",
                dim("dry-run: re-run with --apply to write these changes")
            ));
        }
        ctx.out("");
    }
    Ok(())
}

fn sync_exit_code(outcome: &crate::sync::SyncOutcome) -> i32 {
    if outcome.results.iter().any(|r| r.error.is_some()) {
        ExitCode::Failure as i32
    } else if outcome.results.iter().any(|r| {
        r.plan
            .actions
            .iter()
            .any(|a| a.change == ChangeKind::Conflict)
    }) {
        ExitCode::Conflict as i32
    } else {
        ExitCode::Success as i32
    }
}

// ---------------------------------------------------------------------------
// backup / profile
// ---------------------------------------------------------------------------

async fn cmd_backup(ctx: &Ctx, command: BackupCommand) -> Result<i32> {
    match command {
        BackupCommand::Create { file, label } => {
            let target = file.unwrap_or_else(|| ctx.paths.registry_file.clone());
            let meta = crate::backup::create(&ctx.paths, &target, label.as_deref())?;
            if ctx.json {
                ctx.emit_json(&meta);
            } else {
                ctx.out(&format!("{} backup {}", green("✓ created"), bold(&meta.id)));
                ctx.status("File", &target.display().to_string());
            }
            Ok(ExitCode::Success as i32)
        }
        BackupCommand::List => {
            let metas = crate::backup::list(&ctx.paths)?;
            if ctx.json {
                ctx.emit_json(&metas);
            } else if metas.is_empty() {
                ctx.out(&dim("no backups"));
            } else {
                ctx.out(&bold("Backups"));
                for m in &metas {
                    ctx.out(&format!(
                        "  {}  {}  {}",
                        bold(&m.id),
                        dim(&m.original.display().to_string()),
                        m.label.as_deref().unwrap_or("")
                    ));
                }
            }
            Ok(ExitCode::Success as i32)
        }
        BackupCommand::Restore { id } => {
            let meta = crate::backup::restore(&ctx.paths, &id)?;
            if ctx.json {
                ctx.emit_json(&meta);
            } else {
                ctx.out(&format!(
                    "{} restored {}",
                    green("✓"),
                    meta.original.display()
                ));
            }
            Ok(ExitCode::Success as i32)
        }
    }
}

async fn cmd_profile(ctx: &Ctx, command: ProfileCommand) -> Result<i32> {
    match command {
        ProfileCommand::List => {
            let store = load_store(ctx)?;
            #[derive(Serialize)]
            struct Row {
                name: String,
                active: bool,
                servers: Vec<String>,
            }
            let rows: Vec<Row> = store
                .registry
                .profiles
                .iter()
                .map(|(n, p)| Row {
                    name: n.clone(),
                    active: store.registry.active_profile.as_deref() == Some(n.as_str()),
                    servers: p.servers.clone(),
                })
                .collect();
            if ctx.json {
                ctx.emit_json(&rows);
            } else if rows.is_empty() {
                ctx.out(&dim(
                    "no profiles (cerberus profile create <name> --server <srv>)",
                ));
            } else {
                ctx.out(&bold("Profiles"));
                for r in &rows {
                    let mark = if r.active {
                        green("*")
                    } else {
                        " ".to_string()
                    };
                    ctx.out(&format!(
                        "  {mark} {}  {}",
                        bold(&r.name),
                        dim(&r.servers.join(", "))
                    ));
                }
            }
            Ok(ExitCode::Success as i32)
        }
        ProfileCommand::Show { name } => {
            let store = load_store(ctx)?;
            let profile = store
                .registry
                .profiles
                .get(&name)
                .ok_or_else(|| CerberusError::ProfileNotFound { name: name.clone() })?;
            if ctx.json {
                ctx.emit_json(profile);
            } else {
                ctx.out(&bold(&format!("Profile: {name}")));
                if let Some(d) = &profile.description {
                    ctx.out(&format!("  {d}"));
                }
                for s in &profile.servers {
                    let exists = store.registry.servers.contains_key(s);
                    ctx.out(&format!(
                        "    {} {}",
                        if exists { green("✓") } else { red("✗") },
                        s
                    ));
                }
            }
            Ok(ExitCode::Success as i32)
        }
        ProfileCommand::Use { name } => {
            let mut store = load_store(ctx)?;
            store.set_active_profile(Some(name.clone()))?;
            save_store(ctx, &mut store)?;
            if ctx.json {
                ctx.emit_json(&serde_json::json!({"active_profile": name}));
            } else {
                ctx.out(&format!("{} active profile is now `{name}`", green("✓")));
            }
            Ok(ExitCode::Success as i32)
        }
        ProfileCommand::Create {
            name,
            servers,
            description,
        } => {
            let mut store = load_store(ctx)?;
            let servers: Vec<String> = servers
                .iter()
                .flat_map(|s| s.split(','))
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            store.upsert_profile(
                &name,
                Profile {
                    servers,
                    description,
                },
            )?;
            save_store(ctx, &mut store)?;
            if ctx.json {
                ctx.emit_json(&serde_json::json!({"created": name}));
            } else {
                ctx.out(&format!("{} profile `{name}` created", green("✓")));
            }
            Ok(ExitCode::Success as i32)
        }
        ProfileCommand::Validate { name } => {
            let store = load_store(ctx)?;
            let profile = store
                .registry
                .profiles
                .get(&name)
                .ok_or_else(|| CerberusError::ProfileNotFound { name: name.clone() })?;
            let mut errors = Vec::new();
            for s in &profile.servers {
                if !store.registry.servers.contains_key(s) {
                    errors.push(format!("unknown server `{s}`"));
                }
            }
            if errors.is_empty() {
                if ctx.json {
                    ctx.emit_json(&serde_json::json!({"valid": true}));
                } else {
                    ctx.out(&format!("{} profile `{name}` is valid", green("✓")));
                }
                Ok(ExitCode::Success as i32)
            } else {
                if ctx.json {
                    ctx.emit_json(&serde_json::json!({"valid": false, "errors": errors}));
                } else {
                    ctx.error(&format!("profile `{name}`: {}", errors.join("; ")));
                }
                Ok(ExitCode::ConfigInvalid as i32)
            }
        }
        ProfileCommand::Remove { name, yes } => {
            let mut store = load_store(ctx)?;
            if !yes && !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
                return Err(CerberusError::Validation(
                    "refusing to remove profile without --yes".into(),
                ));
            }
            if store.registry.active_profile.as_deref() == Some(name.as_str()) {
                store.set_active_profile(None)?;
            }
            store.remove_profile(&name)?;
            save_store(ctx, &mut store)?;
            if ctx.json {
                ctx.emit_json(&serde_json::json!({"removed": name}));
            } else {
                ctx.out(&format!("{} profile `{name}` removed", green("✓")));
            }
            Ok(ExitCode::Success as i32)
        }
    }
}

// ---------------------------------------------------------------------------
// gateway
// ---------------------------------------------------------------------------

async fn cmd_gateway(ctx: &Ctx, command: GatewayCommand) -> Result<i32> {
    match command {
        GatewayCommand::Start {
            transport,
            bind,
            allow_remote,
            token,
            profile,
        } => {
            let store = load_store(ctx)?;
            validate::validate(&store.registry)?;
            crate::gateway::run_foreground(
                ctx,
                &store,
                &transport,
                &bind,
                allow_remote,
                token.as_deref(),
                profile.as_deref(),
            )
            .await
        }
        GatewayCommand::Status => {
            let state_file = ctx.paths.gateway_state_file();
            if !state_file.exists() {
                if ctx.json {
                    ctx.emit_json(&serde_json::json!({"status": "stopped"}));
                } else {
                    ctx.out(&format!("Gateway: {}", dim("stopped")));
                }
                return Ok(ExitCode::Success as i32);
            }
            let text = std::fs::read_to_string(&state_file)
                .map_err(|e| CerberusError::Other(format!("gateway state: {e}")))?;
            let state: serde_json::Value = serde_json::from_str(&text)
                .map_err(|e| CerberusError::Other(format!("gateway state: {e}")))?;
            let pid = state.get("pid").and_then(|p| p.as_u64());
            let running = pid.is_some(); // full liveness check is OS-specific
            if ctx.json {
                ctx.emit_json(&state);
            } else {
                ctx.out(&format!(
                    "Gateway: {}",
                    if running {
                        green("running")
                    } else {
                        dim("unknown")
                    }
                ));
                if let Some(pid) = pid {
                    ctx.status("PID", &pid.to_string());
                }
                if let Some(t) = state.get("transport").and_then(|t| t.as_str()) {
                    ctx.status("Transport", t);
                }
                if let Some(b) = state.get("bind").and_then(|b| b.as_str()) {
                    ctx.status("Bind", b);
                }
            }
            Ok(ExitCode::Success as i32)
        }
        GatewayCommand::Doctor => {
            let store = load_store(ctx)?;
            let mut issues = Vec::new();
            if let Err(e) = validate::validate(&store.registry) {
                issues.push(format!("registry invalid: {e}"));
            }
            let mut upstreams = Vec::new();
            for (name, def) in &store.registry.servers {
                if !def.enabled {
                    continue;
                }
                let reachable = match &def.transport {
                    Transport::Stdio { command, .. } => crate::process::executable_exists(command),
                    Transport::Http { url, .. } => url::Url::parse(url).is_ok(),
                };
                upstreams.push(serde_json::json!({
                    "name": name,
                    "transport": def.transport.kind(),
                    "reachable": reachable,
                    "mode": format!("{:?}", def.effective_mode(&store.registry.settings)).to_lowercase(),
                }));
                if !reachable {
                    issues.push(format!("server `{name}` looks unreachable"));
                }
            }
            if ctx.json {
                ctx.emit_json(&serde_json::json!({"issues": issues, "upstreams": upstreams}));
            } else {
                ctx.out(&bold("Gateway doctor"));
                for u in &upstreams {
                    ctx.out(&format!(
                        "  {} {} {}",
                        check_mark(u["reachable"].as_bool().unwrap_or(false)),
                        u["name"],
                        dim(u["transport"].as_str().unwrap_or(""))
                    ));
                }
                if issues.is_empty() {
                    ctx.out(&format!("  {}", green("no issues found")));
                } else {
                    for issue in &issues {
                        ctx.out(&format!("  {}", yellow(issue)));
                    }
                }
            }
            Ok(if issues.is_empty() {
                ExitCode::Success as i32
            } else {
                ExitCode::Failure as i32
            })
        }
        GatewayCommand::Validate { profile } => {
            let store = load_store(ctx)?;
            validate::validate(&store.registry)?;
            let servers = crate::sync::selected_servers(&store, profile.as_deref())?;
            if ctx.json {
                ctx.emit_json(&serde_json::json!({
                    "valid": true,
                    "upstreams": servers.keys().collect::<Vec<_>>(),
                }));
            } else {
                ctx.out(&format!(
                    "{} gateway configuration is valid ({} upstream{})",
                    green("✓"),
                    servers.len(),
                    if servers.len() == 1 { "" } else { "s" }
                ));
            }
            Ok(ExitCode::Success as i32)
        }
        GatewayCommand::Stop => {
            let state_file = ctx.paths.gateway_state_file();
            if !state_file.exists() {
                ctx.out(&dim("gateway is not running"));
                return Ok(ExitCode::Success as i32);
            }
            let text = std::fs::read_to_string(&state_file)
                .map_err(|e| CerberusError::Other(e.to_string()))?;
            let state: serde_json::Value =
                serde_json::from_str(&text).map_err(|e| CerberusError::Other(e.to_string()))?;
            let pid = state.get("pid").and_then(|p| p.as_u64()).unwrap_or(0) as i32;
            if pid > 0 {
                #[cfg(unix)]
                {
                    let killed = libc_kill_like(pid, 15) == 0;
                    if killed {
                        ctx.out(&format!("{} sent SIGTERM to pid {pid}", green("✓")));
                    } else {
                        ctx.out(&yellow("process not found; cleaning stale state"));
                    }
                }
                #[cfg(not(unix))]
                {
                    ctx.out(&yellow(
                        "stop is only implemented on unix; terminate the process manually",
                    ));
                }
            }
            let _ = std::fs::remove_file(&state_file);
            Ok(ExitCode::Success as i32)
        }
    }
}

#[cfg(unix)]
fn libc_kill_like(pid: i32, sig: i32) -> i32 {
    // Minimal kill(2) wrapper without a libc dependency in Cargo.toml:
    // use `kill` from the `libc` crate if available, else shell out.
    // We shell out only with validated numeric arguments.
    let status = std::process::Command::new("kill")
        .arg(format!("-{sig}"))
        .arg(pid.to_string())
        .status();
    match status {
        Ok(s) if s.success() => 0,
        _ => -1,
    }
}

// ---------------------------------------------------------------------------
// diagnostics
// ---------------------------------------------------------------------------

async fn cmd_doctor(ctx: &Ctx, client: Option<String>, probe: bool) -> Result<i32> {
    let report = crate::diagnostics::doctor(
        &ctx.paths,
        &crate::diagnostics::DoctorOptions {
            client,
            probe_servers: probe,
        },
        &crate::secrets::SecretStore::default(),
    )
    .await?;

    if ctx.json {
        ctx.emit_json(&report);
    } else {
        ctx.out(&bold("CerberusMCP Doctor"));
        ctx.out("");
        ctx.out(&bold("Configuration"));
        ctx.status(
            "Registry",
            &format!(
                "{} {}",
                check_mark(report.config_valid),
                report.config_path.display()
            ),
        );
        ctx.status(
            "Schema",
            &report
                .schema_version
                .map(|v| format!("v{v}"))
                .unwrap_or_else(|| dim("n/a").to_string()),
        );
        ctx.status("Servers", &report.server_count.to_string());
        ctx.out("");
        ctx.out(&bold("AI Clients"));
        for c in &report.clients {
            ctx.status(&c.display_name, &c.status);
        }
        ctx.out("");
        if !report.diagnostics.is_empty() {
            ctx.out(&bold("Diagnostics"));
            for d in &report.diagnostics {
                let mark = match d.severity {
                    crate::diagnostics::Severity::Critical => red("✗"),
                    crate::diagnostics::Severity::Warning => yellow("!"),
                    crate::diagnostics::Severity::Info => dim("i"),
                };
                ctx.out(&format!("  {} [{}] {}", mark, d.area, d.message));
            }
            ctx.out("");
        }
        let summary = format!(
            "Summary: {} warning{}, {} critical error{}",
            report.warnings,
            if report.warnings == 1 { "" } else { "s" },
            report.critical,
            if report.critical == 1 { "" } else { "s" }
        );
        if report.critical > 0 {
            ctx.out(&red(&summary));
        } else if report.warnings > 0 {
            ctx.out(&yellow(&summary));
        } else {
            ctx.out(&green(&summary));
        }
    }

    Ok(if report.critical > 0 {
        ExitCode::ConfigInvalid as i32
    } else {
        ExitCode::Success as i32
    })
}

async fn cmd_health(ctx: &Ctx) -> Result<i32> {
    let report = crate::diagnostics::health(&ctx.paths)?;
    if ctx.json {
        ctx.emit_json(&report);
    } else {
        ctx.out(&bold("Health"));
        ctx.status(
            "Registry",
            if report.config_valid {
                "valid"
            } else {
                "invalid"
            },
        );
        for s in &report.servers {
            let mark = match s.executable_found {
                Some(true) => green("ok"),
                Some(false) => red("executable missing"),
                None => dim("remote"),
            };
            ctx.out(&format!(
                "  {}  {}  {}  {}",
                if s.enabled { green("●") } else { dim("○") },
                bold(&s.name),
                dim(&s.transport),
                mark
            ));
        }
    }
    Ok(ExitCode::Success as i32)
}

async fn cmd_logs(ctx: &Ctx, lines: usize) -> Result<i32> {
    let logs = crate::diagnostics::recent_logs(&ctx.paths, lines)?;
    if ctx.json {
        ctx.emit_json(&logs);
    } else if logs.is_empty() {
        ctx.out(&dim(
            "no logs (gateway and --debug runs write to the logs directory)",
        ));
    } else {
        for line in logs {
            ctx.out(&line);
        }
    }
    Ok(ExitCode::Success as i32)
}

fn cmd_version(ctx: &Ctx) -> Result<i32> {
    #[derive(Serialize)]
    struct VersionInfo {
        cerberus: &'static str,
        mcp_sdk: String,
        mcp_spec: &'static str,
        rustc: &'static str,
    }
    let info = VersionInfo {
        cerberus: env!("CARGO_PKG_VERSION"),
        mcp_sdk: rmcp::model::ProtocolVersion::LATEST.to_string(),
        // rmcp 3.5.1 implements the 2026-07-28 revision of the MCP spec.
        mcp_spec: "2026-07-28",
        rustc: "1.88+",
    };
    if ctx.json {
        ctx.emit_json(&info);
    } else {
        ctx.out(&format!("cerberus-mcp {}", info.cerberus));
        ctx.status("MCP SDK", &format!("rmcp {}", info.mcp_sdk));
        ctx.status("MCP spec", info.mcp_spec);
        ctx.status("MSRV", info.rustc);
    }
    Ok(ExitCode::Success as i32)
}

fn cmd_completions(ctx: &Ctx, shell: &str) -> Result<i32> {
    use clap::CommandFactory;
    let mut cmd = <crate::cli::Cli as CommandFactory>::command();
    let shell = match shell {
        "bash" => clap_complete::Shell::Bash,
        "zsh" => clap_complete::Shell::Zsh,
        "fish" => clap_complete::Shell::Fish,
        "elvish" => clap_complete::Shell::Elvish,
        "powershell" | "pwsh" => clap_complete::Shell::PowerShell,
        other => {
            return Err(CerberusError::Validation(format!(
                "unsupported shell `{other}` (bash, zsh, fish, elvish, powershell)"
            )));
        }
    };
    let mut out = std::io::stdout();
    clap_complete::generate(shell, &mut cmd, "cerberus", &mut out);
    let _ = ctx;
    Ok(ExitCode::Success as i32)
}

// Silence unused import warnings in some feature combinations.
#[allow(unused_imports)]
use atomic_write as _atomic_write_unused;
