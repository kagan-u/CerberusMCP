//! End-to-end: the gateway executes `pre_launch` hooks on connect.
//!
//! The upstream here is a stdio server that exits immediately, so the MCP
//! handshake fails — that is fine: hooks run *before* the spawn, and a
//! failed upstream must not brick gateway startup. What is asserted is
//! that both hook shapes (a `use = "…"` reference into `[commands.*]` and
//! an inline `run` vector) executed and left their marker files behind.

use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command as ProcCommand, Stdio};
use std::time::Duration;

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

    fn marker(&self, name: &str) -> PathBuf {
        self.root.path().join(name)
    }
}

/// Render an argv as a TOML inline array. Rust's `Debug` quoting matches
/// TOML basic-string escaping for the characters paths and shell snippets
/// use (`\` and `"`), so raw paths go in unescaped.
fn argv_toml(argv: &[String]) -> String {
    let items: Vec<String> = argv.iter().map(|a| format!("{a:?}")).collect();
    items.join(", ")
}

/// A portable "append `token` to file" argv. The name is relative and the
/// step/command cwd carries the directory: cmd.exe does not honour Rust's
/// `\"` escapes, so quoted paths break redirects there.
fn append_argv(file: &Path, token: &str) -> Vec<String> {
    let name = file.file_name().unwrap().to_string_lossy().into_owned();
    if cfg!(windows) {
        vec!["cmd".into(), "/C".into(), format!("echo {token}>>{name}")]
    } else {
        vec!["sh".into(), "-c".into(), format!("echo {token} >> {name}")]
    }
}

/// Argv that exits immediately instead of speaking MCP (the connect fails
/// after the hooks already ran).
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

fn write_registry(root: &Path, extra_server_toml: &str, prepare: &Path) {
    fs::create_dir_all(root.join("config")).ok();
    fs::create_dir_all(root.join("data")).ok();
    let cwd = prepare
        .parent()
        .expect("marker has a parent")
        .display()
        .to_string();
    let registry = format!(
        r#"schema_version = 2

[commands.prepare]
run = [{prepare}]
cwd = {cwd:?}

{extra}
"#,
        prepare = argv_toml(&append_argv(prepare, "P")),
        cwd = cwd,
        extra = extra_server_toml,
    );
    fs::write(root.join("config/conf.cerberus"), registry).expect("write registry");
}

/// Registry text for one stdio server whose `pre_launch` is `pre_launch`.
fn hooked_server_toml(pre_launch: &str) -> String {
    format!(
        "[servers.hooked]\n\
         description = \"exits immediately; hooks run first\"\n\
         pre_launch = [{pre_launch}]\n\n\
         [servers.hooked.transport]\n\
         {transport}",
        transport = shell_toml(&silent_server_argv()),
    )
}

struct GatewayChild {
    child: Child,
    _data: PathBuf,
}

impl Drop for GatewayChild {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind probe")
        .local_addr()
        .expect("local addr")
        .port()
}

/// Spawn the gateway on loopback and wait for the state file, which is
/// written only after `build_state` (and therefore the hooks) completed.
fn spawn_gateway(sb: &Sandbox) -> GatewayChild {
    let bind = format!("127.0.0.1:{}", free_port());
    let child = ProcCommand::new(env!("CARGO_BIN_EXE_cerberus"))
        .args(["gateway", "start", "--transport", "http", "--bind", &bind])
        .env("CERBERUS_CONFIG_DIR", sb.config())
        .env("CERBERUS_DATA_DIR", sb.data())
        .env("HOME", sb.root.path().join("home"))
        .env("USERPROFILE", sb.root.path().join("home"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn gateway");

    let state_file = sb.data().join("state").join("gateway.json");
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while !state_file_is_bound(&state_file) {
        assert!(
            std::time::Instant::now() < deadline,
            "gateway state file never reported a bound address: {state_file:?}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    GatewayChild {
        child,
        _data: sb.data(),
    }
}

fn state_file_is_bound(state_file: &Path) -> bool {
    fs::read_to_string(state_file)
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v.get("bind").and_then(|b| b.as_str()).map(str::to_string))
        .is_some_and(|b| !b.is_empty() && !b.ends_with(":0"))
}

#[test]
fn gateway_runs_use_and_inline_pre_launch_steps() {
    let root = tempfile::tempdir().expect("tempdir");
    let marker_cmd = root.path().join("marker-command.txt");
    let marker_inline = root.path().join("marker-inline.txt");

    // Step 1: `use = "prepare"` resolves `[commands.prepare]`.
    // Step 2: an inline `run` vector.
    let inline = format!(
        "{{ run = [{}], cwd = {:?} }}",
        argv_toml(&append_argv(&marker_inline, "I")),
        marker_inline.parent().expect("marker has a parent")
    );
    let server = hooked_server_toml(&format!("{{ use = \"prepare\" }}, {inline}"));
    write_registry(root.path(), &server, &marker_cmd);

    let sb = Sandbox { root };
    let _gw = spawn_gateway(&sb);

    assert!(
        sb.marker("marker-command.txt").is_file(),
        "`use = \"prepare\"` step must have run before the connect"
    );
    assert!(
        sb.marker("marker-inline.txt").is_file(),
        "inline `run` step must have run before the connect"
    );
}

#[test]
fn gateway_starts_even_when_a_pre_launch_step_fails() {
    let fail_step = if cfg!(windows) {
        "{ run = [\"cmd\", \"/C\", \"exit /b 9\"] }".to_string()
    } else {
        "{ run = [\"sh\", \"-c\", \"exit 9\"] }".to_string()
    };
    let root = tempfile::tempdir().expect("tempdir");
    let server = hooked_server_toml(&fail_step);
    write_registry(
        root.path(),
        &server,
        &root.path().join("marker-command.txt"),
    );

    let sb = Sandbox { root };
    // The eager connect fails on the hook, but the failure is isolated per
    // upstream: the gateway still binds and writes its state file.
    let _gw = spawn_gateway(&sb);
}
