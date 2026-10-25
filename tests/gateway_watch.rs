//! End-to-end: `gateway start --watch` reconciles registry edits live.
//!
//! Three reload paths are exercised against one running gateway:
//!
//! 1. an mtime-only touch (identical content) is a no-op — no reload log;
//! 2. a document that parses but fails validation is rejected with a
//!    warning and a syntax-broken write is rejected the same way, while
//!    the gateway keeps serving;
//! 3. a valid content change applies (`registry reloaded` in the log).

use std::fs;
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command as ProcCommand, Stdio};
use std::time::{Duration, Instant};

struct Sandbox {
    root: tempfile::TempDir,
}

impl Sandbox {
    fn config(&self) -> PathBuf {
        self.root.path().join("config")
    }

    fn data(&self) -> PathBuf {
        self.root.path().join("data")
    }

    fn log(&self) -> PathBuf {
        self.root.path().join("gateway.log")
    }
}

fn argv_toml(argv: &[String]) -> String {
    let items: Vec<String> = argv.iter().map(|a| format!("{a:?}")).collect();
    items.join(", ")
}

/// Argv that exits immediately instead of speaking MCP (connect fails fast).
fn silent_server_argv() -> Vec<String> {
    if cfg!(windows) {
        vec!["cmd".into(), "/C".into(), "exit 0".into()]
    } else {
        vec!["sh".into(), "-c".into(), "exit 0".into()]
    }
}

fn shell_toml(argv: &[String]) -> String {
    let shell = format!("{:?}", argv[0]);
    let rest: Vec<String> = argv[1..].to_vec();
    format!(
        "type = \"stdio\"\ncommand = {shell}\nargs = [{}]",
        argv_toml(&rest)
    )
}

fn registry_v1() -> String {
    format!(
        r#"schema_version = 2

[servers.main]
description = "first"

[servers.main.transport]
{transport}
"#,
        transport = shell_toml(&silent_server_argv()),
    )
}

/// Parses fine, fails `config validate`: `use` references a command that
/// does not exist. Exercises the validation gate of the reload path.
fn registry_validate_broken() -> String {
    format!(
        r#"schema_version = 2

[servers.main]
description = "first"
pre_launch = [{{use = "does_not_exist"}}]

[servers.main.transport]
{transport}
"#,
        transport = shell_toml(&silent_server_argv()),
    )
}

/// Valid v2 with changed content: a new description plus a `[commands.*]`
/// entry (the command table always follows the document, even when no
/// server's spawn inputs changed).
fn registry_v2() -> String {
    format!(
        r#"schema_version = 2

[commands.ping]
run = [{run}]

[servers.main]
description = "second"

[servers.main.transport]
{transport}
"#,
        run = argv_toml(&silent_server_argv()),
        transport = shell_toml(&silent_server_argv()),
    )
}

struct WatchedGateway {
    child: Child,
    bind: String,
    log: PathBuf,
    _data: PathBuf,
}

impl Drop for WatchedGateway {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl WatchedGateway {
    fn log_text(&self) -> String {
        fs::read_to_string(&self.log).unwrap_or_default()
    }

    fn is_serving(&self) -> bool {
        TcpStream::connect(&self.bind).is_ok()
    }
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind probe")
        .local_addr()
        .expect("local addr")
        .port()
}

fn state_file_is_bound(state_file: &Path) -> bool {
    fs::read_to_string(state_file)
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v.get("bind").and_then(|b| b.as_str()).map(str::to_string))
        .is_some_and(|b| !b.is_empty() && !b.ends_with(":0"))
}

fn spawn_watch(sb: &Sandbox) -> WatchedGateway {
    let bind = format!("127.0.0.1:{}", free_port());
    let log_file = fs::File::create(sb.log()).expect("create log file");
    let child = ProcCommand::new(env!("CARGO_BIN_EXE_cerberus"))
        .args([
            "gateway",
            "start",
            "--transport",
            "http",
            "--bind",
            &bind,
            "--watch",
        ])
        .env("CERBERUS_CONFIG_DIR", sb.config())
        .env("CERBERUS_DATA_DIR", sb.data())
        .env("HOME", sb.root.path().join("home"))
        .env("USERPROFILE", sb.root.path().join("home"))
        .env("RUST_LOG", "info")
        .stdout(Stdio::null())
        .stderr(Stdio::from(log_file))
        .spawn()
        .expect("spawn gateway");

    let state_file = sb.data().join("state").join("gateway.json");
    let deadline = Instant::now() + Duration::from_secs(20);
    while !state_file_is_bound(&state_file) {
        assert!(
            Instant::now() < deadline,
            "gateway state file never reported a bound address: {state_file:?}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    WatchedGateway {
        child,
        bind,
        log: sb.log(),
        _data: sb.data(),
    }
}

/// Poll the log until `needle` occurs at least `min` times.
fn wait_for_log_count(gw: &WatchedGateway, needle: &str, min: usize, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    loop {
        let count = gw.log_text().matches(needle).count();
        if count >= min {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "log never contained {min}x {needle:?}; got {count} in {:?}",
            gw.log_text()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn watch_applies_valid_edits_rejects_broken_ones_and_ignores_touches() {
    let sb = Sandbox {
        root: tempfile::tempdir().expect("tempdir"),
    };
    fs::create_dir_all(sb.config()).expect("config dir");
    fs::create_dir_all(sb.data()).expect("data dir");
    let registry = sb.config().join("conf.cerberus");
    fs::write(&registry, registry_v1()).expect("write registry");

    let gw = spawn_watch(&sb);

    // 1. mtime-only touch: same bytes, new stat — must not reload.
    let v1 = fs::read_to_string(&registry).expect("read registry");
    fs::write(&registry, &v1).expect("touch registry");
    std::thread::sleep(Duration::from_millis(1300));
    let log = gw.log_text();
    assert!(
        !log.contains("registry reloaded"),
        "identical content must not reload: {log}"
    );
    assert!(
        !log.contains("registry reload rejected"),
        "identical content must not hit the parse/validate gate: {log}"
    );

    // 2. parses but fails validation — rejected, still serving.
    fs::write(&registry, registry_validate_broken()).expect("write invalid registry");
    wait_for_log_count(&gw, "registry reload rejected", 1, Duration::from_secs(5));
    assert!(gw.is_serving(), "gateway must survive a rejected reload");

    // 3. syntax-broken write — same gate, no shutdown.
    fs::write(&registry, format!("{v1}\n[[[")).expect("write broken registry");
    wait_for_log_count(&gw, "registry reload rejected", 2, Duration::from_secs(5));
    assert!(gw.is_serving(), "gateway must survive a parse failure");

    // 4. valid content change — applied.
    fs::write(&registry, registry_v2()).expect("write v2 registry");
    wait_for_log_count(&gw, "registry reloaded", 1, Duration::from_secs(5));
    assert!(gw.is_serving(), "gateway must keep serving after reload");
}
