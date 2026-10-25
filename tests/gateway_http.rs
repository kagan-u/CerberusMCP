//! Live HTTP-transport tests for the gateway's security contract:
//! bearer-token auth, loopback guard, and Host-header allowlisting.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command as ProcCommand, Stdio};
use std::time::Duration;

struct Sandbox {
    root: tempfile::TempDir,
}

impl Sandbox {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(root.path().join("config")).ok();
        fs::create_dir_all(root.path().join("data")).ok();
        fs::write(
            root.path().join("config/cerberus.yaml"),
            "schema_version: 1\n",
        )
        .expect("write registry");
        Self { root }
    }

    fn config(&self) -> PathBuf {
        self.root.path().join("config")
    }

    fn data(&self) -> PathBuf {
        self.root.path().join("data")
    }

    fn cmd(&self) -> Command {
        let mut cmd = Command::cargo_bin("cerberus").expect("binary");
        cmd.env("CERBERUS_CONFIG_DIR", self.config())
            .env("CERBERUS_DATA_DIR", self.data())
            .env("HOME", self.root.path().join("home"))
            .env("USERPROFILE", self.root.path().join("home"));
        cmd
    }
}

/// A live `cerberus gateway start --transport http` child.
struct GatewayChild {
    child: Child,
    /// Actual bound endpoint (host:port), from the state file.
    endpoint: String,
    data: PathBuf,
}

impl GatewayChild {
    fn url(&self) -> String {
        format!("http://{}/mcp", self.endpoint)
    }
}

impl Drop for GatewayChild {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(self.data.parent().expect("root"));
    }
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind probe")
        .local_addr()
        .expect("local addr")
        .port()
}

/// Spawn the gateway on loopback with a token and wait for its state file to
/// report the bound address.
fn spawn_gateway(sb: &Sandbox, port: u16, extra: &[&str]) -> GatewayChild {
    let bind = format!("127.0.0.1:{port}");
    let child = ProcCommand::new(env!("CARGO_BIN_EXE_cerberus"))
        .args(["gateway", "start", "--transport", "http", "--bind", &bind])
        .args(extra)
        .env("CERBERUS_CONFIG_DIR", sb.config())
        .env("CERBERUS_DATA_DIR", sb.data())
        .env("HOME", sb.root.path().join("home"))
        .env("USERPROFILE", sb.root.path().join("home"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn gateway");

    let state_file = sb.data().join("state").join("gateway.json");
    let endpoint = wait_for_state(&state_file, Duration::from_secs(15))
        .unwrap_or_else(|| panic!("gateway state file never appeared at {state_file:?}"));
    // State file exists before axum::serve starts accepting; give it a beat.
    std::thread::sleep(Duration::from_millis(100));
    GatewayChild {
        child,
        endpoint,
        data: sb.data(),
    }
}

/// Poll the state file until it names a non-empty bind address; return it.
fn wait_for_state(state_file: &Path, timeout: Duration) -> Option<String> {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        let bind = fs::read_to_string(state_file)
            .ok()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .and_then(|v| v.get("bind").and_then(|b| b.as_str()).map(str::to_string))
            .filter(|b| !b.is_empty() && !b.ends_with(":0"));
        if let Some(bind) = bind {
            return Some(bind);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    None
}

async fn probe(url: &str, token: Option<&str>) -> reqwest::Result<reqwest::StatusCode> {
    let client = reqwest::Client::new();
    // The state file lands just before axum starts accepting; retry through
    // the brief connection-refused window.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let mut req = client.post(url).json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": {"name": "e2e", "version": "0"}
            }
        }));
        if let Some(t) = token {
            req = req.bearer_auth(t);
        }
        match req.send().await {
            Ok(resp) => return Ok(resp.status()),
            Err(e) if e.is_connect() && std::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Err(e) => return Err(e),
        }
    }
}

#[test]
fn non_loopback_bind_requires_allow_remote() {
    let sb = Sandbox::new();
    sb.cmd()
        .args([
            "gateway",
            "start",
            "--transport",
            "http",
            "--bind",
            "0.0.0.0:0",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--allow-remote"));
}

#[test]
fn non_loopback_bind_requires_token_even_with_allow_remote() {
    let sb = Sandbox::new();
    sb.cmd()
        .args([
            "gateway",
            "start",
            "--transport",
            "http",
            "--bind",
            "0.0.0.0:0",
            "--allow-remote",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("auth token"));
}

#[test]
fn loopback_lookalike_hostname_is_not_loopback() {
    let sb = Sandbox::new();
    sb.cmd()
        .args([
            "gateway",
            "start",
            "--transport",
            "http",
            "--bind",
            "127.0.0.1.nip.io:0",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--allow-remote"));
}

#[tokio::test]
async fn http_enforces_bearer_token() {
    let sb = Sandbox::new();
    let gw = spawn_gateway(&sb, free_port(), &["--token", "sekrit"]);
    let url = gw.url();

    // No token: rejected before any MCP processing.
    let status = probe(&url, None).await.expect("request");
    assert_eq!(status, reqwest::StatusCode::UNAUTHORIZED);

    // Wrong token: also rejected.
    let status = probe(&url, Some("wrong")).await.expect("request");
    assert_eq!(status, reqwest::StatusCode::UNAUTHORIZED);

    // Right token: the request passes auth (4xx from MCP parsing is fine;
    // anything but 401 proves the gate opened).
    let status = probe(&url, Some("sekrit")).await.expect("request");
    assert_ne!(status, reqwest::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn token_env_var_is_honored_and_not_leaked() {
    let sb = Sandbox::new();
    let bind = format!("127.0.0.1:{}", free_port());
    let mut child = ProcCommand::new(env!("CARGO_BIN_EXE_cerberus"))
        .args(["gateway", "start", "--transport", "http", "--bind", &bind])
        .env("CERBERUS_CONFIG_DIR", sb.config())
        .env("CERBERUS_DATA_DIR", sb.data())
        .env("CERBERUS_GATEWAY_TOKEN", "env-secret")
        .env("HOME", sb.root.path().join("home"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn");

    let state_file = sb.data().join("state").join("gateway.json");
    let endpoint =
        wait_for_state(&state_file, Duration::from_secs(15)).expect("gateway state file");
    let url = format!("http://{endpoint}/mcp");

    // The env-var token gates the endpoint exactly like --token.
    let status = probe(&url, None).await.expect("request");
    assert_eq!(status, reqwest::StatusCode::UNAUTHORIZED);
    let status = probe(&url, Some("env-secret")).await.expect("request");
    assert_ne!(status, reqwest::StatusCode::UNAUTHORIZED);

    // The token never lands in the state file (or anywhere on disk).
    let state = fs::read_to_string(&state_file).expect("state read");
    assert!(
        !state.contains("env-secret") && !state.contains("token"),
        "state file must not mention the token: {state}"
    );

    let _ = child.kill();
    let _ = child.wait();
}

#[tokio::test]
async fn empty_token_is_treated_as_absent() {
    let sb = Sandbox::new();
    // An empty --token must not silently disable auth on a non-loopback bind.
    sb.cmd()
        .args([
            "gateway",
            "start",
            "--transport",
            "http",
            "--bind",
            "0.0.0.0:0",
            "--allow-remote",
            "--token",
            "",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("auth token"));
}
