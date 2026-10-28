//! C ABI for the desktop GUI (`gui/`).
//!
//! The GUI links this crate as a static library and talks to it through
//! `extern "C"` functions. All functions that can fail return `NULL` and set
//! [`cerberus_last_error`]; payload-bearing functions return a heap JSON
//! string freed by [`cerberus_string_free`].
//!
//! Long-running operations (doctor, gateway start/stop) run as *jobs* on an
//! internal tokio runtime: `*_start` returns a job id, the caller polls
//! [`cerberus_job_poll`] until `state` is `done` or `error`, then
//! [`cerberus_job_free`]s it.

use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, Once, OnceLock};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::runtime::Runtime;
use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::fmt::MakeWriter;

use crate::cli::output::Ctx;
use crate::paths::CerberusPaths;
use crate::process;
use crate::registry::schema::Registry;
use crate::registry::{RegistryStore, validate};
use crate::secrets::SecretStore;

// ---------------------------------------------------------------------------
// plumbing
// ---------------------------------------------------------------------------

static LAST_ERROR: Mutex<Option<CString>> = Mutex::new(None);
static LOGS: Mutex<Vec<String>> = Mutex::new(Vec::new());
static LOG_SEQ: AtomicU64 = AtomicU64::new(0);
static LOG_TRUNCATED: AtomicU64 = AtomicU64::new(0);
static LOG_INIT: Once = Once::new();

const LOG_CAP: usize = 2000;

fn set_error(msg: impl Into<String>) {
    let msg = msg.into();
    tracing::error!("gui: {msg}");
    if let Ok(mut slot) = LAST_ERROR.lock() {
        *slot = CString::new(msg).ok();
    }
}

fn to_c_string(value: &str) -> *mut c_char {
    match CString::new(value) {
        Ok(s) => s.into_raw(),
        Err(_) => {
            set_error("value contains interior NUL bytes");
            std::ptr::null_mut()
        }
    }
}

fn json_ok<T: Serialize>(value: &T) -> *mut c_char {
    match serde_json::to_string(value) {
        Ok(text) => to_c_string(&text),
        Err(err) => {
            set_error(format!("serialize: {err}"));
            std::ptr::null_mut()
        }
    }
}

fn fail(err: impl std::fmt::Display) -> *mut c_char {
    set_error(err.to_string());
    std::ptr::null_mut()
}

/// Run `f` as an FFI call: panics become `NULL` + last-error instead of
/// unwinding across the C ABI boundary.
fn guard<F: FnOnce() -> *mut c_char>(f: F) -> *mut c_char {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(ptr) => ptr,
        Err(_) => {
            set_error("internal panic in cerberus-ffi");
            std::ptr::null_mut()
        }
    }
}

fn paths() -> Result<CerberusPaths, String> {
    CerberusPaths::resolve().map_err(|e| e.to_string())
}

/// Load the on-disk registry, migrating legacy files, or fall back to a
/// default store when no registry exists yet.
fn load_store(paths: &CerberusPaths) -> Result<RegistryStore, String> {
    if paths.registry_file.exists() {
        RegistryStore::load_with_migration(paths).map_err(|e| e.to_string())
    } else {
        RegistryStore::load_or_default(paths).map_err(|e| e.to_string())
    }
}

// ---------------------------------------------------------------------------
// logging bridge
// ---------------------------------------------------------------------------

struct LogLineWriter(String);

impl std::io::Write for LogLineWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.push_str(&String::from_utf8_lossy(buf));
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Drop for LogLineWriter {
    fn drop(&mut self) {
        let line = self.0.trim_end().to_string();
        if line.is_empty() {
            return;
        }
        if let Ok(mut logs) = LOGS.lock() {
            logs.push(line);
            if logs.len() > LOG_CAP {
                let cut = logs.len() - LOG_CAP;
                logs.drain(..cut);
                LOG_TRUNCATED.fetch_add(cut as u64, Ordering::SeqCst);
            }
            LOG_SEQ.fetch_add(1, Ordering::SeqCst);
        }
    }
}

struct LogMaker;

impl<'a> MakeWriter<'a> for LogMaker {
    type Writer = LogLineWriter;
    fn make_writer(&'a self) -> Self::Writer {
        LogLineWriter(String::new())
    }
}

fn init_logging() {
    LOG_INIT.call_once(|| {
        let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
        let _ = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_ansi(false)
            .with_writer(LogMaker)
            .try_init();
    });
}

// ---------------------------------------------------------------------------
// jobs (long-running operations)
// ---------------------------------------------------------------------------

type JobCell = Arc<Mutex<Option<Result<Value, String>>>>;

static JOBS: LazyLock<Mutex<HashMap<u64, JobCell>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
static NEXT_JOB: AtomicU64 = AtomicU64::new(1);
static RUNTIME: OnceLock<Runtime> = OnceLock::new();

fn runtime() -> &'static Runtime {
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_name("cerberus-ffi")
            .build()
            .expect("tokio runtime")
    })
}

fn spawn_job<F>(work: F) -> u64
where
    F: FnOnce() -> Result<Value, String> + Send + 'static,
{
    let id = NEXT_JOB.fetch_add(1, Ordering::SeqCst);
    let cell: JobCell = Arc::new(Mutex::new(None));
    if let Ok(mut jobs) = JOBS.lock() {
        jobs.insert(id, cell.clone());
    } else {
        set_error("job table poisoned");
        return 0;
    }
    runtime().spawn_blocking(move || {
        let outcome = match catch_unwind(AssertUnwindSafe(work)) {
            Ok(result) => result,
            Err(_) => Err("internal panic in cerberus-ffi job".into()),
        };
        if let Ok(mut slot) = cell.lock() {
            *slot = Some(outcome);
        }
    });
    id
}

#[derive(Serialize)]
struct JobPoll {
    state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

// ---------------------------------------------------------------------------
// gateway (in-process, http transport)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct StartOptions {
    pub bind: String,
    pub allow_remote: bool,
    pub token: Option<String>,
    pub profile: Option<String>,
    pub watch: bool,
    pub watch_interval: Option<u64>,
}

impl StartOptions {
    fn normalized(mut self) -> Self {
        if self.bind.is_empty() {
            self.bind = "127.0.0.1:8713".into();
        }
        self
    }
}

struct ManagedGateway {
    stop: Mutex<Option<tokio_util::sync::CancellationToken>>,
    active: AtomicBool,
    last_event: Mutex<Option<String>>,
    opts: Mutex<Option<StartOptions>>,
}

static GATEWAY: OnceLock<ManagedGateway> = OnceLock::new();

fn gateway() -> &'static ManagedGateway {
    GATEWAY.get_or_init(|| ManagedGateway {
        stop: Mutex::new(None),
        active: AtomicBool::new(false),
        last_event: Mutex::new(None),
        opts: Mutex::new(None),
    })
}

fn read_state_file(paths: &CerberusPaths) -> Option<Value> {
    let text = std::fs::read_to_string(paths.gateway_state_file()).ok()?;
    serde_json::from_str(&text).ok()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GatewayStatus {
    running: bool,
    managed: bool,
    pid: Option<u64>,
    transport: Option<String>,
    bind: Option<String>,
    watch: Option<bool>,
    started_unix: Option<u64>,
    last_event: Option<String>,
    /// Options of the last managed start (present while managed).
    opts: Option<StartOptions>,
}

fn gateway_status(paths: &CerberusPaths) -> GatewayStatus {
    let file = read_state_file(paths);
    let pid = file
        .as_ref()
        .and_then(|v| v.get("pid"))
        .and_then(|p| p.as_u64());
    let gw = gateway();
    let managed = gw.active.load(Ordering::SeqCst);
    let running = managed || pid.is_some_and(|p| process::process_alive(p as i32));
    GatewayStatus {
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
        last_event: gw.last_event.lock().ok().and_then(|g| g.clone()),
        opts: gw.opts.lock().ok().and_then(|g| g.clone()),
    }
}

async fn wait_until_up(
    paths: &CerberusPaths,
    gw: &'static ManagedGateway,
    timeout_ms: u64,
) -> Result<(), String> {
    let steps = timeout_ms / 100;
    for _ in 0..steps {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        if let Some(state) = read_state_file(paths) {
            let bound = state
                .get("bind")
                .and_then(|b| b.as_str())
                .is_some_and(|b| !b.is_empty() && !b.ends_with(":0"));
            if bound {
                return Ok(());
            }
        }
        if !gw.active.load(Ordering::SeqCst) {
            return Err(gw
                .last_event
                .lock()
                .ok()
                .and_then(|g| g.clone())
                .unwrap_or_else(|| "gateway exited immediately".into()));
        }
    }
    Err("gateway did not report a bound address in time".into())
}

// ---------------------------------------------------------------------------
// exports
// ---------------------------------------------------------------------------

/// Free a string returned by any `cerberus_*` function.
///
/// # Safety
/// `ptr` must come from this library and not be freed twice.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cerberus_string_free(ptr: *mut c_char) {
    if !ptr.is_null() {
        unsafe { drop(CString::from_raw(ptr)) };
    }
}

/// Last error message for the most recent failing call. Valid until the next
/// FFI call; never freed. Returns an empty string when there is no error.
#[unsafe(no_mangle)]
pub extern "C" fn cerberus_last_error() -> *const c_char {
    match LAST_ERROR.lock() {
        Ok(slot) => match slot.as_ref() {
            Some(c) => c.as_ptr(),
            None => c"".as_ptr(),
        },
        Err(_) => c"".as_ptr(),
    }
}

fn take_error() {
    if let Ok(mut slot) = LAST_ERROR.lock() {
        *slot = None;
    }
}

/// Initialize the GUI bridge: logging + directory setup.
///
/// Returns JSON `{version, configDir, dataDir, registryPath, registryExists}`.
#[unsafe(no_mangle)]
pub extern "C" fn cerberus_init() -> *mut c_char {
    guard(|| {
        take_error();
        init_logging();
        let paths = match paths() {
            Ok(p) => p,
            Err(err) => return fail(err),
        };
        if let Err(err) = paths.ensure_dirs() {
            return fail(err);
        }
        json_ok(&json!({
            "version": crate::VERSION,
            "configDir": paths.config_dir.display().to_string(),
            "dataDir": paths.data_dir.display().to_string(),
            "registryPath": paths.registry_file.display().to_string(),
            "registryExists": paths.registry_file.exists(),
        }))
    })
}

/// Load the registry as JSON (schema v2; secrets are included verbatim so
/// the editor can round-trip them). Missing file yields the default registry.
#[unsafe(no_mangle)]
pub extern "C" fn cerberus_registry_get() -> *mut c_char {
    guard(|| {
        take_error();
        let paths = match paths() {
            Ok(p) => p,
            Err(err) => return fail(err),
        };
        match load_store(&paths) {
            Ok(store) => json_ok(&store.registry),
            Err(err) => fail(err),
        }
    })
}

/// Replace the registry with the JSON payload and validate + persist it
/// atomically. Returns `{ok:true}`.
///
/// # Safety
/// `payload` must be NULL or a valid NUL-terminated UTF-8 string that outlives
/// the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cerberus_registry_set(payload: *const c_char) -> *mut c_char {
    guard(|| {
        take_error();
        if payload.is_null() {
            return fail("payload is null");
        }
        let text = unsafe { CStr::from_ptr(payload) }.to_string_lossy();
        let registry: Registry = match serde_json::from_str(&text) {
            Ok(r) => r,
            Err(err) => return fail(format!("invalid registry JSON: {err}")),
        };
        let paths = match paths() {
            Ok(p) => p,
            Err(err) => return fail(err),
        };
        let _lock = REGISTRY_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        match (|| -> Result<(), String> {
            paths.ensure_dirs().map_err(|e| e.to_string())?;
            let mut store = load_store(&paths)?;
            store.registry = registry;
            store.save().map_err(|e| e.to_string())
        })() {
            Ok(()) => {
                tracing::info!("registry saved ({})", paths.registry_file.display());
                json_ok(&json!({"ok": true}))
            }
            Err(err) => fail(err),
        }
    })
}

static REGISTRY_LOCK: Mutex<()> = Mutex::new(());

/// Validate the registry on disk. Returns `{ok, error?}`.
#[unsafe(no_mangle)]
pub extern "C" fn cerberus_registry_validate() -> *mut c_char {
    guard(|| {
        take_error();
        let paths = match paths() {
            Ok(p) => p,
            Err(err) => return fail(err),
        };
        if !paths.registry_file.exists() {
            return json_ok(&json!({"ok": true, "error": null}));
        }
        let store = match load_store(&paths) {
            Ok(s) => s,
            Err(err) => return fail(err),
        };
        match validate::validate(&store.registry) {
            Ok(()) => json_ok(&json!({"ok": true, "error": null})),
            Err(err) => json_ok(&json!({"ok": false, "error": err.to_string()})),
        }
    })
}

/// Start a doctor job. `probe_servers` != 0 spawns server probes.
/// Returns a job id (0 on failure; see [`cerberus_last_error`]).
#[unsafe(no_mangle)]
pub extern "C" fn cerberus_doctor_start(probe_servers: i32) -> u64 {
    take_error();
    let probe = probe_servers != 0;
    match paths() {
        Ok(paths) => spawn_job(move || {
            let rt = runtime();
            let report = rt.block_on(async {
                crate::diagnostics::doctor(
                    &paths,
                    &crate::diagnostics::DoctorOptions {
                        client: None,
                        probe_servers: probe,
                    },
                    &SecretStore::default(),
                )
                .await
            });
            report
                .map(|r| serde_json::to_value(&r).map_err(|e| e.to_string()))
                .map_err(|e| e.to_string())?
        }),
        Err(err) => {
            set_error(err);
            0
        }
    }
}

/// Start the embedded gateway (HTTP transport) as a job.
/// `opts_json`: `{bind, allowRemote, token, profile, watch, watchInterval}`.
/// Returns a job id whose value is the gateway status once it is bound.
///
/// # Safety
/// `opts_json` must be NULL or a valid NUL-terminated UTF-8 string that
/// outlives the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cerberus_gateway_start(opts_json: *const c_char) -> u64 {
    take_error();
    let raw = if opts_json.is_null() {
        "{}".to_string()
    } else {
        unsafe { CStr::from_ptr(opts_json) }
            .to_string_lossy()
            .into_owned()
    };
    let opts: StartOptions =
        match serde_json::from_str::<StartOptions>(&raw).map(|o| o.normalized()) {
            Ok(o) => o,
            Err(err) => {
                set_error(format!("invalid start options: {err}"));
                return 0;
            }
        };
    let paths = match paths() {
        Ok(p) => p,
        Err(err) => {
            set_error(err);
            return 0;
        }
    };
    let gw = gateway();
    if gw.active.load(Ordering::SeqCst) {
        set_error("the gateway is already running (managed by this app)");
        return 0;
    }
    if let Some(pid) = read_state_file(&paths).and_then(|v| v.get("pid").and_then(|p| p.as_u64())) {
        if process::process_alive(pid as i32) {
            set_error(format!(
                "another gateway is running (pid {pid}); stop it first (`cerberus gateway stop`)"
            ));
            return 0;
        }
        let _ = std::fs::remove_file(paths.gateway_state_file());
    }

    spawn_job(move || {
        let store = load_store(&paths)?;
        validate::validate(&store.registry).map_err(|e| e.to_string())?;
        let ctx = Ctx {
            json: true,
            quiet: true,
            paths: paths.clone(),
        };
        let stop = tokio_util::sync::CancellationToken::new();
        *gw.stop.lock().expect("gateway stop lock") = Some(stop.clone());
        gw.active.store(true, Ordering::SeqCst);
        let bind = opts.bind.clone();
        tracing::info!("gateway start requested ({bind})");

        let opts_task = opts.clone();
        let stop_task = stop.clone();
        let active = &gw.active;
        let last_event = &gw.last_event;
        runtime().spawn(async move {
            let result = crate::gateway::run_foreground(
                &ctx,
                &store,
                "http",
                &bind,
                opts_task.allow_remote,
                opts_task.token.as_deref(),
                opts_task.profile.as_deref(),
                opts_task.watch,
                opts_task.watch_interval,
                stop_task,
            )
            .await;
            active.store(false, Ordering::SeqCst);
            let message = match &result {
                Ok(code) => format!("gateway exited (code {code})"),
                Err(err) => format!("gateway error: {err}"),
            };
            if let Ok(mut slot) = last_event.lock() {
                *slot = Some(message);
            }
        });

        *gw.opts.lock().expect("gateway opts lock") = Some(opts);
        let up = runtime().block_on(wait_until_up(&paths, gw, 30_000));
        match up {
            Ok(()) => Ok(serde_json::to_value(gateway_status(&paths)).unwrap_or(Value::Null)),
            Err(err) => {
                let stop = gw.stop.lock().ok().and_then(|mut g| g.take());
                if let Some(stop) = stop {
                    stop.cancel();
                }
                Err(err)
            }
        }
    })
}

/// Stop the gateway (managed or external) as a job.
/// Returns a job id whose value is the resulting gateway status.
#[unsafe(no_mangle)]
pub extern "C" fn cerberus_gateway_stop() -> u64 {
    take_error();
    let paths = match paths() {
        Ok(p) => p,
        Err(err) => {
            set_error(err);
            return 0;
        }
    };
    spawn_job(move || {
        let gw = gateway();
        let stop = gw.stop.lock().ok().and_then(|mut g| g.take());
        if let Some(stop) = stop {
            tracing::info!("gateway stop requested");
            stop.cancel();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while gw.active.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        } else if let Some(pid) =
            read_state_file(&paths).and_then(|v| v.get("pid").and_then(|p| p.as_u64()))
        {
            if !process::terminate(pid as i32) {
                return Err(format!("could not signal gateway pid {pid}"));
            }
            let _ = std::fs::remove_file(paths.gateway_state_file());
        }
        Ok(serde_json::to_value(gateway_status(&paths)).unwrap_or(Value::Null))
    })
}

/// Current gateway status (cheap; safe to poll).
#[unsafe(no_mangle)]
pub extern "C" fn cerberus_gateway_status() -> *mut c_char {
    guard(|| {
        take_error();
        let paths = match paths() {
            Ok(p) => p,
            Err(err) => return fail(err),
        };
        json_ok(&gateway_status(&paths))
    })
}

// ---------------------------------------------------------------------------
// logs
// ---------------------------------------------------------------------------

/// Drain log lines produced since index `since`.
///
/// Returns `{lines, next, earliest, dropped}`: `next` is the index to pass to
/// the next call, `earliest` the oldest retained index (callers that fell off
/// the ring get `dropped=true`).
#[unsafe(no_mangle)]
pub extern "C" fn cerberus_logs_take(since: u64) -> *mut c_char {
    guard(|| {
        take_error();
        let logs = LOGS.lock().unwrap_or_else(|e| e.into_inner());
        let total = LOG_SEQ.load(Ordering::SeqCst);
        let earliest = LOG_TRUNCATED.load(Ordering::SeqCst);
        let start = since.max(earliest);
        let lines: Vec<&String> = logs.iter().skip((start - earliest) as usize).collect();
        json_ok(&json!({
            "lines": lines,
            "next": total,
            "earliest": earliest,
            "dropped": start > earliest,
        }))
    })
}

// ---------------------------------------------------------------------------
// jobs: poll / free
// ---------------------------------------------------------------------------

/// Poll a job. Returns `{"state":"pending"|"done"|"error", "value"?, "error"?}`.
#[unsafe(no_mangle)]
pub extern "C" fn cerberus_job_poll(id: u64) -> *mut c_char {
    guard(|| {
        take_error();
        let cell = match JOBS.lock() {
            Ok(jobs) => jobs.get(&id).cloned(),
            Err(_) => None,
        };
        let cell = match cell {
            Some(c) => c,
            None => return fail(format!("unknown job id {id}")),
        };
        let slot = cell.lock().unwrap_or_else(|e| e.into_inner());
        match slot.as_ref() {
            None => json_ok(&JobPoll {
                state: "pending",
                value: None,
                error: None,
            }),
            Some(Ok(value)) => json_ok(&JobPoll {
                state: "done",
                value: Some(value.clone()),
                error: None,
            }),
            Some(Err(err)) => json_ok(&JobPoll {
                state: "error",
                value: None,
                error: Some(err.clone()),
            }),
        }
    })
}

/// Release a finished (or abandoned) job.
///
/// # Safety
/// `id` must come from a `*_start` call in this process.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cerberus_job_free(id: u64) {
    if let Ok(mut jobs) = JOBS.lock() {
        jobs.remove(&id);
    }
}
