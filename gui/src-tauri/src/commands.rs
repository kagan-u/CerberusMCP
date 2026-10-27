//! Tauri commands: registry editing, diagnostics, and in-process gateway
//! control. All registry mutations re-load the store from disk, apply the
//! change, and save (which validates) under a shared lock.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::State;

use cerberus_mcp::paths::CerberusPaths;
use cerberus_mcp::process;
use cerberus_mcp::registry::schema::{CommandDef, Profile, Registry, ServerDef, Settings};
use cerberus_mcp::registry::{RegistryStore, validate};
use cerberus_mcp::secrets::SecretStore;

use crate::AppCtx;

fn e<E: std::fmt::Display>(err: E) -> String {
    err.to_string()
}

// ---------------------------------------------------------------------------
// app / registry
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct AppInfo {
    pub cerberus_version: String,
    pub gui_version: String,
    pub config_dir: String,
    pub registry_path: String,
    pub data_dir: String,
    pub registry_exists: bool,
}

#[tauri::command]
pub fn app_info(state: State<'_, AppCtx>) -> Result<AppInfo, String> {
    let paths = &state.paths;
    Ok(AppInfo {
        cerberus_version: cerberus_mcp::VERSION.to_string(),
        gui_version: env!("CARGO_PKG_VERSION").to_string(),
        config_dir: paths.config_dir.display().to_string(),
        registry_path: paths.registry_file.display().to_string(),
        data_dir: paths.data_dir.display().to_string(),
        registry_exists: paths.registry_file.exists(),
    })
}

#[derive(Serialize)]
pub struct RegistryView {
    pub registry: Registry,
    pub path: String,
    pub exists: bool,
}

#[tauri::command]
pub async fn get_registry(state: State<'_, AppCtx>) -> Result<RegistryView, String> {
    let paths = state.paths.clone();
    let exists = paths.registry_file.exists();
    let registry = tokio::task::spawn_blocking(move || {
        if exists {
            RegistryStore::load_with_migration(&paths).map(|s| s.registry)
        } else {
            Ok(Registry::default())
        }
    })
    .await
    .map_err(e)?
    .map_err(e)?;
    Ok(RegistryView {
        registry,
        path: state.paths.registry_file.display().to_string(),
        exists,
    })
}

/// Create an empty schema-v2 registry when none exists (`cerberus init`
/// equivalent without the client templates).
#[tauri::command]
pub async fn init_registry(state: State<'_, AppCtx>) -> Result<RegistryView, String> {
    let paths = state.paths.clone();
    tokio::task::spawn_blocking(move || {
        paths.ensure_dirs()?;
        if !paths.registry_file.exists() {
            RegistryStore::load_or_default(&paths)?.save()?;
        }
        Ok::<(), cerberus_mcp::CerberusError>(())
    })
    .await
    .map_err(e)?
    .map_err(e)?;
    get_registry(state).await
}

#[derive(Serialize)]
pub struct ValidateReport {
    pub ok: bool,
    pub error: Option<String>,
}

#[tauri::command]
pub async fn validate_registry(state: State<'_, AppCtx>) -> Result<ValidateReport, String> {
    let paths = state.paths.clone();
    tokio::task::spawn_blocking(move || -> Result<ValidateReport, String> {
        let store = RegistryStore::load_with_migration(&paths).map_err(|e| e.to_string())?;
        match validate::validate(&store.registry) {
            Ok(()) => Ok(ValidateReport {
                ok: true,
                error: None,
            }),
            Err(err) => Ok(ValidateReport {
                ok: false,
                error: Some(err.to_string()),
            }),
        }
    })
    .await
    .map_err(|err| err.to_string())?
    .map_err(|err| err.to_string())
}

#[tauri::command]
pub async fn run_doctor(
    state: State<'_, AppCtx>,
    probe_servers: bool,
) -> Result<cerberus_mcp::diagnostics::DoctorReport, String> {
    let opts = cerberus_mcp::diagnostics::DoctorOptions {
        client: None,
        probe_servers,
    };
    cerberus_mcp::diagnostics::doctor(&state.paths, &opts, &SecretStore::default())
        .await
        .map_err(e)
}

// ---------------------------------------------------------------------------
// registry mutations (load → mutate → save, serialized)
// ---------------------------------------------------------------------------

async fn with_store<T, F>(state: &State<'_, AppCtx>, f: F) -> Result<T, String>
where
    F: FnOnce(&mut RegistryStore) -> cerberus_mcp::Result<T> + Send + 'static,
    T: Send + 'static,
{
    let _guard = state.registry_lock.lock().await;
    let paths = state.paths.clone();
    tokio::task::spawn_blocking(move || -> cerberus_mcp::Result<T> {
        let mut store = RegistryStore::load_with_migration(&paths)?;
        let out = f(&mut store)?;
        store.save()?;
        tracing::info!("registry saved ({})", paths.registry_file.display());
        Ok(out)
    })
    .await
    .map_err(e)?
    .map_err(e)
}

#[tauri::command]
pub async fn upsert_server(
    state: State<'_, AppCtx>,
    name: String,
    server: ServerDef,
) -> Result<RegistryView, String> {
    with_store(&state, move |store| {
        validate::validate_server_name(&name)?;
        store.registry.servers.insert(name, server);
        Ok(())
    })
    .await?;
    get_registry(state).await
}

#[tauri::command]
pub async fn delete_server(state: State<'_, AppCtx>, name: String) -> Result<RegistryView, String> {
    with_store(&state, move |store| {
        store.remove_server(&name)?;
        Ok(())
    })
    .await?;
    get_registry(state).await
}

#[tauri::command]
pub async fn toggle_server(
    state: State<'_, AppCtx>,
    name: String,
    enabled: bool,
) -> Result<RegistryView, String> {
    with_store(&state, move |store| {
        store.set_enabled(&name, enabled).map(|_| ())
    })
    .await?;
    get_registry(state).await
}

#[tauri::command]
pub async fn upsert_profile(
    state: State<'_, AppCtx>,
    name: String,
    profile: Profile,
) -> Result<RegistryView, String> {
    with_store(&state, move |store| {
        store.upsert_profile(&name, profile).map(|_| ())
    })
    .await?;
    get_registry(state).await
}

#[tauri::command]
pub async fn delete_profile(
    state: State<'_, AppCtx>,
    name: String,
) -> Result<RegistryView, String> {
    with_store(&state, move |store| store.remove_profile(&name).map(|_| ())).await?;
    get_registry(state).await
}

#[tauri::command]
pub async fn set_active_profile(
    state: State<'_, AppCtx>,
    name: Option<String>,
) -> Result<RegistryView, String> {
    with_store(&state, move |store| {
        store.set_active_profile(name).map(|_| ())
    })
    .await?;
    get_registry(state).await
}

#[tauri::command]
pub async fn set_settings(
    state: State<'_, AppCtx>,
    settings: Settings,
) -> Result<RegistryView, String> {
    with_store(&state, move |store| {
        store.registry.settings = settings;
        Ok(())
    })
    .await?;
    get_registry(state).await
}

#[tauri::command]
pub async fn set_commands(
    state: State<'_, AppCtx>,
    commands: BTreeMap<String, CommandDef>,
) -> Result<RegistryView, String> {
    with_store(&state, move |store| {
        store.registry.commands = commands;
        Ok(())
    })
    .await?;
    get_registry(state).await
}

// ---------------------------------------------------------------------------
// logs
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn get_logs(state: State<'_, AppCtx>, limit: usize) -> Vec<String> {
    state.logs.snapshot(limit.clamp(1, 5000))
}

#[tauri::command]
pub fn clear_logs(state: State<'_, AppCtx>) {
    state.logs.clear();
}

// ---------------------------------------------------------------------------
// gateway (in-process, http transport)
// ---------------------------------------------------------------------------

#[derive(Clone, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct StartOptions {
    pub bind: String,
    pub allow_remote: bool,
    pub token: Option<String>,
    pub profile: Option<String>,
    pub watch: bool,
    pub watch_interval: Option<u64>,
}

impl Default for StartOptions {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:8713".into(),
            allow_remote: false,
            token: None,
            profile: None,
            watch: false,
            watch_interval: None,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayStatus {
    /// A gateway process is alive (ours or an external one from the CLI).
    pub running: bool,
    /// The live gateway is owned by this app instance.
    pub managed: bool,
    pub pid: Option<u64>,
    pub transport: Option<String>,
    pub bind: Option<String>,
    pub watch: Option<bool>,
    pub started_unix: Option<u64>,
    /// Exit reason of the last managed run, when nothing is running.
    pub last_event: Option<String>,
}

pub struct ManagedGateway {
    stop: Mutex<Option<tokio_util::sync::CancellationToken>>,
    active: Arc<AtomicBool>,
    last_event: Arc<Mutex<Option<String>>>,
    pub opts: Mutex<Option<StartOptions>>,
}

impl Default for ManagedGateway {
    fn default() -> Self {
        Self {
            stop: Mutex::new(None),
            active: Arc::new(AtomicBool::new(false)),
            last_event: Arc::new(Mutex::new(None)),
            opts: Mutex::new(None),
        }
    }
}

impl ManagedGateway {
    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::SeqCst)
    }

    fn read_state_file(paths: &CerberusPaths) -> Option<serde_json::Value> {
        let text = std::fs::read_to_string(paths.gateway_state_file()).ok()?;
        serde_json::from_str(&text).ok()
    }

    async fn wait_until_up(&self, paths: &CerberusPaths, timeout_ms: u64) -> Result<(), String> {
        let steps = timeout_ms / 100;
        for _ in 0..steps {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            if let Some(state) = Self::read_state_file(paths) {
                let bound = state
                    .get("bind")
                    .and_then(|b| b.as_str())
                    .is_some_and(|b| !b.is_empty() && !b.ends_with(":0"));
                if bound {
                    return Ok(());
                }
            }
            if !self.is_active() {
                return Err(self
                    .last_event
                    .lock()
                    .expect("last_event lock")
                    .clone()
                    .unwrap_or_else(|| "gateway exited immediately".into()));
            }
        }
        Err("gateway did not report a bound address in time".into())
    }
}

#[tauri::command]
pub async fn gateway_start(
    state: State<'_, AppCtx>,
    opts: StartOptions,
) -> Result<GatewayStatus, String> {
    let gw = state.gateway.clone();
    if gw.is_active() {
        return Err("the gateway is already running (managed by this app)".into());
    }
    // An external instance (CLI `gateway start`) owns the state file.
    if let Some(pid) = ManagedGateway::read_state_file(&state.paths)
        .and_then(|v| v.get("pid").and_then(|p| p.as_u64()))
    {
        if process::process_alive(pid as i32) {
            return Err(format!(
                "another gateway is running (pid {pid}); stop it first (`cerberus gateway stop`)"
            ));
        }
        let _ = std::fs::remove_file(state.paths.gateway_state_file());
    }

    let paths = state.paths.clone();
    let store = tokio::task::spawn_blocking(move || {
        let store = RegistryStore::load_with_migration(&paths)?;
        validate::validate(&store.registry)?;
        Ok::<RegistryStore, cerberus_mcp::CerberusError>(store)
    })
    .await
    .map_err(e)?
    .map_err(e)?;

    let ctx = cerberus_mcp::cli::output::Ctx {
        json: true,
        quiet: true,
        paths: state.paths.clone(),
    };
    let stop = tokio_util::sync::CancellationToken::new();
    *gw.stop.lock().expect("gateway stop lock") = Some(stop.clone());
    gw.active.store(true, Ordering::SeqCst);

    let active = gw.active.clone();
    let last_event = gw.last_event.clone();
    let start_opts = opts.clone();
    let bind = opts.bind.clone();
    tracing::info!("gateway start requested ({bind})");
    tokio::spawn(async move {
        let result = cerberus_mcp::gateway::run_foreground(
            &ctx,
            &store,
            "http",
            &bind,
            opts.allow_remote,
            opts.token.as_deref(),
            opts.profile.as_deref(),
            opts.watch,
            opts.watch_interval,
            stop,
        )
        .await;
        active.store(false, Ordering::SeqCst);
        let message = match &result {
            Ok(code) => format!("gateway exited (code {code})"),
            Err(err) => format!("gateway error: {err}"),
        };
        *last_event.lock().expect("last_event lock") = Some(message);
        result
    });

    *gw.opts.lock().expect("gateway opts lock") = Some(start_opts);
    gw.wait_until_up(&state.paths, 30_000).await?;
    gateway_status(state).await
}

#[tauri::command]
pub async fn gateway_stop(state: State<'_, AppCtx>) -> Result<GatewayStatus, String> {
    let gw = state.gateway.clone();
    let stop = {
        let mut guard = gw.stop.lock().expect("gateway stop lock");
        guard.take()
    };
    if let Some(stop) = stop {
        tracing::info!("gateway stop requested");
        stop.cancel();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while gw.is_active() && std::time::Instant::now() < deadline {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    } else if let Some(pid) = ManagedGateway::read_state_file(&state.paths)
        .and_then(|v| v.get("pid").and_then(|p| p.as_u64()))
    {
        // External (CLI-managed) gateway: terminate it and drop its state.
        if !process::terminate(pid as i32) {
            return Err(format!("could not signal gateway pid {pid}"));
        }
        let _ = std::fs::remove_file(state.paths.gateway_state_file());
    }
    gateway_status(state).await
}

#[tauri::command]
pub async fn gateway_status(state: State<'_, AppCtx>) -> Result<GatewayStatus, String> {
    let file = ManagedGateway::read_state_file(&state.paths);
    let pid = file
        .as_ref()
        .and_then(|v| v.get("pid").and_then(|p| p.as_u64()));
    let managed = state.gateway.is_active();
    let running = managed || pid.is_some_and(|p| process::process_alive(p as i32));
    let last_event = state
        .gateway
        .last_event
        .lock()
        .expect("last_event lock")
        .clone();
    Ok(GatewayStatus {
        running,
        managed,
        pid,
        transport: file
            .as_ref()
            .and_then(|v| v.get("transport"))
            .and_then(|t| t.as_str())
            .map(str::to_string),
        bind: file
            .as_ref()
            .and_then(|v| v.get("bind"))
            .and_then(|b| b.as_str())
            .map(str::to_string),
        watch: file
            .as_ref()
            .and_then(|v| v.get("watch"))
            .and_then(|w| w.as_bool()),
        started_unix: file
            .as_ref()
            .and_then(|v| v.get("started_unix"))
            .and_then(|s| s.as_u64()),
        last_event,
    })
}
