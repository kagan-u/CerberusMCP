//! End-to-end tests that drive the compiled `cerberus` binary.
//!
//! Every test sandboxes the registry, data dir, and client config paths via
//! environment overrides so nothing outside the temp directory is touched.

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

struct Sandbox {
    root: tempfile::TempDir,
}

impl Sandbox {
    fn new() -> Self {
        Self {
            root: tempfile::tempdir().expect("tempdir"),
        }
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.root.path().join(rel)
    }

    fn cmd(&self) -> Command {
        let mut cmd = Command::cargo_bin("cerberus").expect("binary");
        cmd.env("CERBERUS_CONFIG_DIR", self.path("config"))
            .env("CERBERUS_DATA_DIR", self.path("data"))
            // Fully sandbox HOME so client adapters cannot see real configs.
            // Windows BaseDirs resolves USERPROFILE, not HOME.
            .env("HOME", self.path("home"))
            .env("USERPROFILE", self.path("home"))
            .env("OPENCODE_CONFIG", self.path("home/opencode.json"))
            .env("CODEX_HOME", self.path("home/.codex"))
            .env("CLINE_MCP_SETTINGS_PATH", self.path("home/cline.json"))
            // Redirect (not remove) the Windows well-known folders: removing
            // them makes `BaseDirs`/`ProjectDirs` resolution fail outright.
            .env("APPDATA", self.path("home/AppData/Roaming"))
            .env("LOCALAPPDATA", self.path("home/AppData/Local"))
            .env_remove("XDG_CONFIG_HOME");
        fs::create_dir_all(self.path("home")).ok();
        fs::create_dir_all(self.path("home/.codex")).ok();
        cmd
    }

    fn init(&self) {
        self.cmd().arg("init").assert().success();
        assert!(self.path("config/conf.cerberus").exists());
    }

    fn add_stdio(&self, name: &str) {
        self.cmd()
            .args([
                "server",
                "add",
                name,
                "--command",
                "npx",
                "--arg=-y",
                "--arg=@example/server",
            ])
            .assert()
            .success();
    }
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).expect("read")).expect("parse json")
}

#[test]
fn init_creates_registry_and_refuses_overwrite() {
    let sb = Sandbox::new();
    sb.cmd()
        .arg("init")
        .assert()
        .success()
        .stdout(predicate::str::contains("initialized"));
    assert!(sb.path("config/conf.cerberus").exists());
    assert!(sb.path("data").exists());

    sb.cmd()
        .arg("init")
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("already exists"));

    sb.cmd().args(["init", "--force"]).assert().success();
}

#[test]
fn init_json_reports_paths_and_schema() {
    let sb = Sandbox::new();
    let out = sb
        .cmd()
        .args(["init", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["schema_version"], 2);
    assert!(v["config"].as_str().unwrap().ends_with("conf.cerberus"));
}

#[test]
fn legacy_yaml_registry_migrates_to_conf_cerberus() {
    let sb = Sandbox::new();
    // Pre-v2 install: only the legacy file exists, no init was ever run.
    fs::create_dir_all(sb.path("config")).unwrap();
    fs::write(
        sb.path("config/cerberus.yaml"),
        r#"schema_version: 1
servers:
  legacy:
    transport:
      type: stdio
      command: echo
      args: ["hi"]
"#,
    )
    .unwrap();

    // Any read command converts the document: TOML appears, the legacy
    // file is moved into the backups dir, and the data survives.
    sb.cmd()
        .args(["server", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("legacy"));

    assert!(
        sb.path("config/conf.cerberus").exists(),
        "conf.cerberus written by the migration"
    );
    assert!(
        !sb.path("config/cerberus.yaml").exists(),
        "legacy file moved aside"
    );
    let backups: Vec<_> = fs::read_dir(sb.path("data/backups/migrations"))
        .map(|d| d.filter_map(|e| e.ok()).collect())
        .unwrap_or_default();
    assert!(
        backups
            .iter()
            .any(|e| { e.file_name().to_string_lossy().ends_with(".yaml.bak") }),
        "pre-migration backup kept: {backups:?}"
    );

    let migrated = fs::read_to_string(sb.path("config/conf.cerberus")).unwrap();
    assert!(migrated.contains("schema_version = 2"), "{migrated}");
    assert!(migrated.contains("legacy"), "{migrated}");

    // A second run is a no-op conversion (conf.cerberus now exists).
    sb.cmd()
        .args(["config", "validate"])
        .assert()
        .success()
        .stdout(predicate::str::contains("valid"));
}

#[test]
fn config_show_redacts_literal_env_values_in_toml() {
    let sb = Sandbox::new();
    sb.init();
    sb.cmd()
        .args([
            "server",
            "add",
            "svc",
            "--command",
            "npx",
            "--env",
            "TOKEN=literal:super-secret-value",
        ])
        .assert()
        .success();

    // The file itself keeps the value; only the rendered view redacts it.
    let raw = fs::read_to_string(sb.path("config/conf.cerberus")).unwrap();
    assert!(raw.contains("super-secret-value"), "{raw}");
    assert!(raw.contains("schema_version = 2"), "{raw}");

    let out = sb
        .cmd()
        .args(["config", "show"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let shown = String::from_utf8_lossy(&out);
    assert!(shown.contains("<redacted>"), "{shown}");
    assert!(
        !shown.contains("super-secret-value"),
        "literal leaked in config show:\n{shown}"
    );
}

#[test]
fn add_list_inspect_enable_disable_remove() {
    let sb = Sandbox::new();
    sb.init();
    sb.add_stdio("filesystem");

    sb.cmd()
        .args(["server", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("filesystem"));

    let out = sb
        .cmd()
        .args(["server", "inspect", "filesystem", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["name"], "filesystem");
    assert_eq!(v["enabled"], true);
    assert_eq!(v["transport"], "stdio");

    sb.cmd()
        .args(["server", "disable", "filesystem"])
        .assert()
        .success();
    let out = sb
        .cmd()
        .args(["server", "inspect", "filesystem", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["enabled"], false);

    sb.cmd()
        .args(["server", "enable", "filesystem"])
        .assert()
        .success();

    sb.cmd()
        .args(["server", "remove", "filesystem", "--yes"])
        .assert()
        .success();
    sb.cmd()
        .args(["server", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("filesystem").not());
}

#[test]
fn rejects_invalid_server_names() {
    let sb = Sandbox::new();
    sb.init();
    sb.cmd()
        .args(["server", "add", "bad__name", "--command", "x"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("__"));
    sb.cmd()
        .args(["server", "add", "Bad_Name", "--command", "x"])
        .assert()
        .failure();
}

#[test]
fn duplicate_server_add_conflicts() {
    let sb = Sandbox::new();
    sb.init();
    sb.add_stdio("dup");
    sb.cmd()
        .args(["server", "add", "dup", "--command", "x"])
        .assert()
        .failure()
        .code(4)
        .stderr(predicate::str::contains("exists"));
}

#[test]
fn sync_opencode_dry_run_then_apply_is_idempotent() {
    let sb = Sandbox::new();
    sb.init();
    sb.add_stdio("filesystem");

    // Dry-run must not create the config file.
    sb.cmd()
        .args(["sync", "--client", "opencode"])
        .assert()
        .success()
        .stdout(predicate::str::contains("+ add").and(predicate::str::contains("dry-run")));
    assert!(!sb.path("home/opencode.json").exists());

    // Apply writes a v1-shaped entry.
    sb.cmd()
        .args(["sync", "--client", "opencode", "--apply"])
        .assert()
        .success();
    let v = read_json(&sb.path("home/opencode.json"));
    let entry = &v["mcp"]["filesystem"];
    assert_eq!(entry["type"], "local");
    assert_eq!(entry["enabled"], true);
    assert_eq!(entry["command"][0], "npx");
    assert_eq!(entry["command"][2], "@example/server");

    // Second apply reports in-sync and changes nothing.
    let before = fs::read_to_string(sb.path("home/opencode.json")).unwrap();
    sb.cmd()
        .args(["sync", "--client", "opencode", "--apply"])
        .assert()
        .success()
        .stdout(predicate::str::contains("unchanged").or(predicate::str::contains("in sync")));
    let after = fs::read_to_string(sb.path("home/opencode.json")).unwrap();
    assert_eq!(before, after);
}

#[test]
fn sync_conflict_with_unmanaged_entry_requires_force() {
    let sb = Sandbox::new();
    sb.init();
    sb.add_stdio("filesystem");

    // Occupy the name with a user-owned entry.
    fs::write(
        sb.path("home/opencode.json"),
        r#"{"mcp":{"filesystem":{"type":"local","command":["custom"],"enabled":true}}}"#,
    )
    .unwrap();

    sb.cmd()
        .args(["sync", "--client", "opencode", "--apply"])
        .assert()
        .failure()
        .code(4)
        .stdout(predicate::str::contains("unmanaged"))
        .stdout(predicate::str::contains("filesystem"));

    // --force takes ownership.
    sb.cmd()
        .args(["sync", "--client", "opencode", "--apply", "--force"])
        .assert()
        .success();
    let v = read_json(&sb.path("home/opencode.json"));
    assert_eq!(v["mcp"]["filesystem"]["command"][0], "npx");
}

#[test]
fn sync_writes_backup_before_overwriting() {
    let sb = Sandbox::new();
    sb.init();
    sb.add_stdio("filesystem");
    sb.cmd()
        .args(["sync", "--client", "opencode", "--apply"])
        .assert()
        .success();
    // Occupy + take ownership with --force creates a pre-sync backup.
    fs::write(
        sb.path("home/opencode.json"),
        r#"{"mcp":{"filesystem":{"type":"local","command":["custom"],"enabled":true}}}"#,
    )
    .unwrap();
    sb.cmd()
        .args(["sync", "--client", "opencode", "--apply", "--force"])
        .assert()
        .success();

    let out = sb
        .cmd()
        .args(["backup", "list", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: Value = serde_json::from_slice(&out).unwrap();
    let arr = v.as_array().expect("backup list is an array");
    assert!(!arr.is_empty(), "expected at least one backup");
    let contains_custom = arr.iter().any(|b| {
        let stored = b["stored"].as_str().unwrap_or_default();
        Path::new(stored).exists()
            && fs::read_to_string(stored)
                .unwrap_or_default()
                .contains("custom")
    });
    assert!(
        contains_custom,
        "backup should contain the pre-sync content"
    );
}

#[test]
fn sync_codex_writes_toml_with_url_for_http() {
    let sb = Sandbox::new();
    sb.init();
    sb.cmd()
        .args([
            "server",
            "add",
            "remote",
            "--transport",
            "http",
            "--url",
            "https://example.com/mcp",
        ])
        .assert()
        .success();

    sb.cmd()
        .args(["sync", "--client", "codex", "--apply"])
        .assert()
        .success();

    let toml = fs::read_to_string(sb.path("home/.codex/config.toml")).unwrap();
    assert!(toml.contains("[mcp_servers.remote]"), "toml was: {toml}");
    assert!(toml.contains("url = \"https://example.com/mcp\""));
}

#[test]
fn sync_claude_code_preserves_command_and_args() {
    let sb = Sandbox::new();
    sb.init();
    sb.add_stdio("filesystem");

    sb.cmd()
        .args(["sync", "--client", "claude-code", "--apply"])
        .assert()
        .success();

    let v = read_json(&sb.path("home/.claude.json"));
    let entry = &v["mcpServers"]["filesystem"];
    // Claude Code uses command (string) + args (array).
    assert_eq!(entry["command"], "npx");
    assert_eq!(entry["args"][0], "-y");
    assert_eq!(entry["args"][1], "@example/server");
    // No `enabled` flag — Claude Code has none; disabled servers are omitted.
    assert!(entry.get("enabled").is_none());
}

#[test]
fn disabled_server_is_omitted_for_flagless_clients() {
    let sb = Sandbox::new();
    sb.init();
    sb.add_stdio("filesystem");
    sb.cmd()
        .args(["server", "disable", "filesystem"])
        .assert()
        .success();

    // Pre-existing Claude Code config: disabled servers must not appear.
    fs::create_dir_all(sb.path("home")).unwrap();
    fs::write(sb.path("home/.claude.json"), r#"{"mcpServers":{}}"#).unwrap();
    sb.cmd()
        .args(["sync", "--client", "claude-code", "--apply"])
        .assert()
        .success();
    let v = read_json(&sb.path("home/.claude.json"));
    assert!(v["mcpServers"].get("filesystem").is_none());

    // OpenCode has an enabled/disabled flag — entry present but disabled.
    sb.cmd()
        .args(["sync", "--client", "opencode", "--apply"])
        .assert()
        .success();
    let v = read_json(&sb.path("home/opencode.json"));
    assert_eq!(v["mcp"]["filesystem"]["enabled"], false);
}

#[test]
fn profile_limits_which_servers_sync() {
    let sb = Sandbox::new();
    sb.init();
    sb.add_stdio("alpha");
    sb.add_stdio("beta");
    sb.cmd()
        .args(["profile", "create", "just-alpha", "--server", "alpha"])
        .assert()
        .success();
    sb.cmd()
        .args(["profile", "use", "just-alpha"])
        .assert()
        .success();

    sb.cmd()
        .args(["sync", "--client", "opencode", "--apply"])
        .assert()
        .success();
    let v = read_json(&sb.path("home/opencode.json"));
    assert!(v["mcp"].get("alpha").is_some());
    assert!(v["mcp"].get("beta").is_none());
}

#[test]
fn sync_without_registry_fails_actionably() {
    let sb = Sandbox::new();
    sb.cmd()
        .args(["sync", "--client", "opencode"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("init").or(predicate::str::contains("registry")));
}

#[test]
fn doctor_json_reports_registry_and_clients() {
    let sb = Sandbox::new();
    sb.init();
    sb.add_stdio("filesystem");
    sb.cmd()
        .args(["sync", "--client", "opencode", "--apply"])
        .assert()
        .success();

    let out = sb
        .cmd()
        .args(["doctor", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["config_valid"], true);
    assert_eq!(v["server_count"], 1);
    let clients = v["clients"].as_array().unwrap();
    let opencode = clients
        .iter()
        .find(|c| c["client"] == "opencode")
        .expect("opencode entry");
    assert_eq!(opencode["detected"], true);
    assert_eq!(opencode["servers"], 1);
    // Aider must be reported as unsupported, never silently skipped.
    let aider = clients.iter().find(|c| c["client"] == "aider").unwrap();
    assert_eq!(aider["supported"], false);
}

#[test]
fn diff_reports_drift_after_manual_edit() {
    let sb = Sandbox::new();
    sb.init();
    sb.add_stdio("filesystem");
    sb.cmd()
        .args(["sync", "--client", "opencode", "--apply"])
        .assert()
        .success();
    sb.cmd()
        .args(["diff", "--client", "opencode"])
        .assert()
        .success()
        .stdout(predicate::str::contains("in sync"));

    // Manually edit the managed entry → diff shows drift.
    let path = sb.path("home/opencode.json");
    let mut v = read_json(&path);
    v["mcp"]["filesystem"]["command"][0] = Value::String("python".into());
    fs::write(&path, serde_json::to_vec_pretty(&v).unwrap()).unwrap();

    sb.cmd()
        .args(["diff", "--client", "opencode"])
        .assert()
        .success()
        .stdout(predicate::str::contains("~").or(predicate::str::contains("!")));
}

#[test]
fn version_prints_semver_and_protocol() {
    let sb = Sandbox::new();
    sb.cmd()
        .args(["version", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("mcp_spec"));
}

#[test]
fn completions_generate_for_bash_and_zsh() {
    let sb = Sandbox::new();
    for shell in ["bash", "zsh", "fish"] {
        sb.cmd()
            .args(["completions", shell])
            .assert()
            .success()
            .stdout(predicate::str::contains("cerberus"));
    }
}

// ---------------------------------------------------------------------------
// registry search / install
// ---------------------------------------------------------------------------

/// Minimal stand-in for the official MCP Registry: one route serving a fixed
/// body on loopback, shut down when the guard drops. Keeps these tests
/// deterministic without touching the network.
struct RegistryFixture {
    url: String,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl RegistryFixture {
    fn spawn(body: &'static str) -> Self {
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<u16>();
        let handle = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio runtime");
            runtime.block_on(async move {
                let app = axum::Router::new()
                    .route("/servers", axum::routing::get(move || async move { body }));
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                    .await
                    .expect("bind loopback");
                let port = listener.local_addr().expect("local addr").port();
                ready_tx.send(port).expect("report port");
                let _ = axum::serve(listener, app)
                    .with_graceful_shutdown(async {
                        let _ = shutdown_rx.await;
                    })
                    .await;
            });
        });
        let port = ready_rx.recv().expect("fixture port");
        Self {
            url: format!("http://127.0.0.1:{port}"),
            shutdown: Some(shutdown_tx),
            handle: Some(handle),
        }
    }
}

impl Drop for RegistryFixture {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Two published versions of one server plus an unrelated server: search must
/// collapse them to two results and keep `0.2.0`.
const SEARCH_FIXTURE: &str = r#"{
  "servers": [
    {"server": {"name": "com.example/server-filesystem", "version": "0.1.0",
                "description": "old", "packages": [
                  {"registryType": "npm", "identifier": "@example/fs", "version": "0.1.0",
                   "runtimeHint": "npx", "transport": {"type": "stdio"}}]}},
    {"server": {"name": "com.example/server-filesystem", "version": "0.2.0",
                "description": "Read and write files",
                "repository": {"url": "https://github.com/example/fs"},
                "packages": [
                  {"registryType": "npm", "identifier": "@example/fs", "version": "0.2.0",
                   "runtimeHint": "npx", "transport": {"type": "stdio"},
                   "runtimeArguments": [{"value": "-y", "type": "positional"}],
                   "packageArguments": [{"value": "/data", "type": "positional"}],
                   "environmentVariables": [
                     {"name": "API_KEY", "isRequired": true, "isSecret": true}]}]}},
    {"server": {"name": "com.example/other-server", "version": "1.0.0",
                "description": "something else", "packages": [
                  {"registryType": "npm", "identifier": "@example/other", "version": "1.0.0",
                   "runtimeHint": "npx", "transport": {"type": "stdio"}}]}}
  ],
  "metadata": {"count": 3}
}"#;

const UNSUPPORTED_FIXTURE: &str = r#"{
  "servers": [
    {"server": {"name": "com.example/windows-only", "version": "1.0.0",
                "packages": [
                  {"registryType": "nuget", "identifier": "Example.Nuget",
                   "version": "1.0.0", "transport": {"type": "stdio"}}]}}
  ]
}"#;

fn record_file(sb: &Sandbox, name: &str, body: &str) -> PathBuf {
    let path = sb.path(name);
    fs::write(&path, body).expect("write record");
    path
}

const LOCAL_RECORD: &str = r#"{
  "name": "com.example/server-filesystem",
  "description": "Local filesystem",
  "version": "0.6.2",
  "repository": {"url": "https://github.com/example/fs"},
  "packages": [
    {"registryType": "npm", "identifier": "@example/server-filesystem", "version": "0.6.2",
     "runtimeHint": "npx", "transport": {"type": "stdio"},
     "runtimeArguments": [{"value": "-y", "type": "positional"}],
     "packageArguments": [{"type": "named", "name": "allowed-directories",
                           "isRequired": true, "format": "string"}],
     "environmentVariables": [
       {"name": "API_KEY", "isRequired": true, "isSecret": true},
       {"name": "OPTIONAL", "description": "optional knob"}]}
  ],
  "remotes": []
}"#;

#[test]
fn search_deduplicates_versions_and_caches_for_offline() {
    let sb = Sandbox::new();
    sb.init();
    let fixture = RegistryFixture::spawn(SEARCH_FIXTURE);

    let out = sb
        .cmd()
        .env("CERBERUS_REGISTRY_URL", &fixture.url)
        .args(["--json", "search", "filesystem"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["count"], 2, "duplicate version rows collapse: {v}");
    assert_eq!(v["results"][0]["name"], "com.example/server-filesystem");
    assert_eq!(
        v["results"][0]["version"], "0.2.0",
        "newest published version wins: {v}"
    );
    assert_eq!(v["results"][0]["kind"], "npm");
    assert_eq!(v["results"][1]["name"], "com.example/other-server");

    // The result was cached: replay works without the registry URL.
    sb.cmd()
        .args(["search", "filesystem", "--offline"])
        .assert()
        .success()
        .stdout(predicate::str::contains("com.example/other-server"));

    // No cache entry for an unknown query → not found, with a way out.
    sb.cmd()
        .args(["search", "never-searched", "--offline"])
        .assert()
        .failure()
        .code(5)
        .stderr(predicate::str::contains("--offline"));
}

#[test]
fn install_from_registry_writes_source_and_validates() {
    let sb = Sandbox::new();
    sb.init();
    let fixture = RegistryFixture::spawn(SEARCH_FIXTURE);

    sb.cmd()
        .env("CERBERUS_REGISTRY_URL", &fixture.url)
        .args([
            "install",
            "com.example/server-filesystem",
            "--check",
            "--tag",
            "files",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("installed"));

    let registry = fs::read_to_string(sb.path("config/conf.cerberus")).expect("registry");
    assert!(registry.contains("server-filesystem"), "{registry}");
    assert!(registry.contains(r#"kind = "npm""#), "{registry}");
    assert!(registry.contains("@example/fs"), "{registry}");
    assert!(
        registry.contains(r#"name = "API_KEY""#),
        "required env is recorded for doctor: {registry}"
    );
    assert!(!registry.contains("never-searched"));

    sb.cmd()
        .args(["config", "validate"])
        .assert()
        .success()
        .stdout(predicate::str::contains("valid"));

    // Re-installing the same name is a conflict, not a silent overwrite.
    sb.cmd()
        .env("CERBERUS_REGISTRY_URL", &fixture.url)
        .args(["install", "com.example/server-filesystem"])
        .assert()
        .failure()
        .code(4);
}

#[test]
fn install_from_local_record_needs_no_network() {
    let sb = Sandbox::new();
    sb.init();
    record_file(&sb, "record.json", LOCAL_RECORD);

    // No CERBERUS_REGISTRY_URL: the file path short-circuits the registry.
    sb.cmd()
        .args([
            "install",
            sb.path("record.json").to_str().unwrap(),
            "--json",
        ])
        .assert()
        .success();

    let out = sb
        .cmd()
        .args(["server", "inspect", "server-filesystem", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["source"]["kind"], "npm");
    assert_eq!(v["source"]["package"], "@example/server-filesystem");
    assert_eq!(v["source"]["runner"], "npx");
    // The named argument has no value: it must be warned about, never invented.
    let installed = fs::read_to_string(sb.path("config/conf.cerberus")).unwrap();
    assert!(!installed.contains("allowed-directories"), "{installed}");
}

#[test]
fn install_dry_run_reports_without_writing() {
    let sb = Sandbox::new();
    sb.init();
    record_file(&sb, "record.json", LOCAL_RECORD);

    sb.cmd()
        .args([
            "install",
            sb.path("record.json").to_str().unwrap(),
            "--dry-run",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("would install as"));

    let registry = fs::read_to_string(sb.path("config/conf.cerberus")).unwrap();
    assert!(
        !registry.contains("server-filesystem"),
        "nothing written:\n{registry}"
    );

    sb.cmd()
        .args(["server", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("no servers").or(predicate::str::is_empty()));
}

#[test]
fn install_rejects_unsupported_package_type() {
    let sb = Sandbox::new();
    sb.init();
    let fixture = RegistryFixture::spawn(UNSUPPORTED_FIXTURE);

    sb.cmd()
        .env("CERBERUS_REGISTRY_URL", &fixture.url)
        .args(["install", "com.example/windows-only"])
        .assert()
        .failure()
        .code(8)
        .stderr(predicate::str::contains("npm and pypi"));
}

#[test]
fn doctor_reports_unset_source_requirements() {
    let sb = Sandbox::new();
    sb.init();
    // Hand-written entry: `source` documents a required variable that the
    // transport does not bind at all.
    fs::write(
        sb.path("config/conf.cerberus"),
        r#"schema_version = 2

[servers.svc.transport]
type = "stdio"
command = "npx"

[servers.svc.source]
kind = "npm"
package = "example-pkg"

[[servers.svc.source.env]]
name = "CERBERUS_TEST_REQUIRED_VAR"
required = true
"#,
    )
    .unwrap();

    let out = sb
        .cmd()
        .args(["--json", "doctor"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let report: Value = serde_json::from_slice(&out).unwrap();
    let messages: Vec<&str> = report["diagnostics"]
        .as_array()
        .expect("diagnostics array")
        .iter()
        .filter_map(|d| d["message"].as_str())
        .collect();
    assert!(
        messages
            .iter()
            .any(|m| m.contains("CERBERUS_TEST_REQUIRED_VAR")),
        "source requirement must be reported: {messages:?}"
    );

    // Set the variable → the diagnostic disappears.
    let out = sb
        .cmd()
        .env("CERBERUS_TEST_REQUIRED_VAR", "set")
        .args(["--json", "doctor"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let report: Value = serde_json::from_slice(&out).unwrap();
    let messages: Vec<&str> = report["diagnostics"]
        .as_array()
        .expect("diagnostics array")
        .iter()
        .filter_map(|d| d["message"].as_str())
        .collect();
    assert!(
        !messages
            .iter()
            .any(|m| m.contains("CERBERUS_TEST_REQUIRED_VAR")),
        "variable is set now: {messages:?}"
    );
}

// ---------------------------------------------------------------------------
// run
// ---------------------------------------------------------------------------

fn append_toml(sb: &Sandbox, block: &str) {
    let path = sb.path("config/conf.cerberus");
    let mut content = fs::read_to_string(&path).expect("read registry");
    content.push('\n');
    content.push_str(block);
    fs::write(&path, content).expect("write registry");
}

fn toml_argv(argv: &[&str]) -> String {
    let items: Vec<String> = argv.iter().map(|a| format!("{a:?}")).collect();
    format!("[{}]", items.join(", "))
}

#[test]
fn run_lists_and_executes_registry_commands() {
    let sb = Sandbox::new();
    sb.init();
    fs::create_dir_all(sb.path("workdir")).unwrap();

    // Shell differs per platform; everything else is portable.
    let (echo, exit7, echo_env, pwd, slow) = if cfg!(windows) {
        (
            vec!["cmd", "/C", "echo hello-from-run"],
            vec!["cmd", "/C", "exit 7"],
            vec!["cmd", "/C", "echo %RUN_VAR%"],
            vec!["cmd", "/C", "cd"],
            vec!["ping", "-n", "6", "127.0.0.1"],
        )
    } else {
        (
            vec!["sh", "-c", "echo hello-from-run"],
            vec!["sh", "-c", "exit 7"],
            vec!["sh", "-c", "echo $RUN_VAR"],
            vec!["sh", "-c", "pwd"],
            vec!["sh", "-c", "sleep 5"],
        )
    };
    let cwd = sb.path("workdir").to_string_lossy().replace('\\', "/");
    let block = format!(
        r#"
[commands.hello]
run = {}
description = "print a greeting"

[commands.boom]
run = {}

[commands.envcheck]
run = {}
env = {{ RUN_VAR = {{ source = "literal", value = "42" }} }}

[commands.cwddir]
run = {}
cwd = "{}"

[commands.slow]
run = {}
timeout_ms = 500
"#,
        toml_argv(&echo),
        toml_argv(&exit7),
        toml_argv(&echo_env),
        toml_argv(&pwd),
        cwd,
        toml_argv(&slow),
    );
    append_toml(&sb, &block);

    // No name → list available commands.
    sb.cmd()
        .arg("run")
        .assert()
        .success()
        .stdout(predicate::str::contains("hello"));
    let out = sb
        .cmd()
        .args(["run", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let list: Value = serde_json::from_slice(&out).unwrap();
    assert!(
        list.as_array()
            .expect("command array")
            .iter()
            .any(|c| c["name"] == "boom")
    );

    // Execute: stdout streams through.
    sb.cmd()
        .args(["run", "hello"])
        .assert()
        .success()
        .stdout(predicate::str::contains("hello-from-run"));

    // Registry env bindings overlay the inherited environment.
    sb.cmd()
        .args(["run", "envcheck"])
        .assert()
        .success()
        .stdout(predicate::str::contains("42"));

    // cwd is honored.
    sb.cmd()
        .args(["run", "cwddir"])
        .assert()
        .success()
        .stdout(predicate::str::contains("workdir"));

    // Child exit code passes through untouched.
    sb.cmd().args(["run", "boom"]).assert().code(7);

    // Unknown command → NotFound (exit 5).
    sb.cmd()
        .args(["run", "nope"])
        .assert()
        .code(5)
        .stderr(predicate::str::contains("not defined"));

    // --json reports the structured result alongside the passthrough code.
    let out = sb
        .cmd()
        .args(["run", "boom", "--json"])
        .assert()
        .code(7)
        .get_output()
        .stdout
        .clone();
    let result: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(result["command"], "boom");
    assert_eq!(result["exit_code"], 7);
    assert!(result["duration_ms"].as_u64().is_some());
    assert!(result["argv"].as_array().is_some());
}

#[test]
fn run_enforces_command_timeout() {
    let sb = Sandbox::new();
    sb.init();
    let (slow, timeout_ms) = if cfg!(windows) {
        (vec!["ping", "-n", "6", "127.0.0.1"], 500)
    } else {
        (vec!["sh", "-c", "sleep 5"], 500)
    };
    append_toml(
        &sb,
        &format!(
            "[commands.slow]\nrun = {}\ntimeout_ms = {}\n",
            toml_argv(&slow),
            timeout_ms
        ),
    );

    sb.cmd()
        .args(["run", "slow"])
        .assert()
        .code(7)
        .stderr(predicate::str::contains("timed out"));
}

// ---------------------------------------------------------------------------
// compiled cache
// ---------------------------------------------------------------------------

fn find_cbin(root: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = fs::read_dir(dir) else { return };
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|ext| ext == "cbin") {
                out.push(p);
            }
        }
    }
    let mut out = Vec::new();
    walk(root, &mut out);
    out
}

#[test]
fn config_compile_writes_cache_and_loads_from_it() {
    let sb = Sandbox::new();
    sb.init();
    sb.add_stdio("fs");

    let out = sb
        .cmd()
        .args(["config", "compile", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: Value = serde_json::from_slice(&out).unwrap();
    let cache = PathBuf::from(v["cache"].as_str().expect("cache path"));
    assert!(cache.is_file(), "compile must write the cache file");
    assert_eq!(v["schema_version"], 2);
    assert_eq!(v["cache_rev"], 1);
    assert!(v["payload_bytes"].as_u64().unwrap() > 0);
    assert!(v["container_bytes"].as_u64().unwrap() > v["payload_bytes"].as_u64().unwrap());

    // A load that hits the cache must not rewrite it.
    let m1 = fs::metadata(&cache).unwrap().modified().unwrap();
    sb.cmd()
        .args(["config", "show"])
        .assert()
        .success()
        .stdout(predicate::str::contains("fs"));
    let m2 = fs::metadata(&cache).unwrap().modified().unwrap();
    assert_eq!(m1, m2, "cache hit must not rewrite the cache file");

    // `config path` reports the cache directory too.
    let out = sb
        .cmd()
        .args(["config", "path", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let paths: Value = serde_json::from_slice(&out).unwrap();
    assert!(
        paths["cache_dir"].as_str().unwrap().ends_with("cache"),
        "cache_dir: {}",
        paths["cache_dir"]
    );

    // Editing the source invalidates the cache: the next load recompiles
    // (observable as a rewritten cache file) and still serves correct data.
    sb.add_stdio("second");
    let m3 = fs::metadata(&cache).unwrap().modified().unwrap();
    sb.cmd()
        .args(["config", "show"])
        .assert()
        .success()
        .stdout(predicate::str::contains("second"));
    let m4 = fs::metadata(&cache).unwrap().modified().unwrap();
    assert_ne!(m3, m4, "an edited source must force a cache recompile");
}

#[test]
fn corrupt_cache_falls_back_to_parsing_the_source() {
    let sb = Sandbox::new();
    sb.init();
    sb.add_stdio("fs");
    let out = sb
        .cmd()
        .args(["config", "compile", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: Value = serde_json::from_slice(&out).unwrap();
    let cache = PathBuf::from(v["cache"].as_str().unwrap());

    // Flip a payload byte: the payload-hash guard must reject the file and
    // the load must silently fall back to parsing conf.cerberus.
    let mut bytes = fs::read(&cache).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0xff;
    fs::write(&cache, &bytes).unwrap();
    sb.cmd()
        .args(["config", "validate"])
        .assert()
        .success()
        .stdout(predicate::str::contains("valid"));

    // The fallback recompiles, so the very next load is healthy again.
    sb.cmd()
        .args(["config", "show"])
        .assert()
        .success()
        .stdout(predicate::str::contains("fs"));
    let reparsed = fs::read(&cache).unwrap();
    assert_ne!(
        reparsed.last(),
        bytes.last(),
        "fallback must rewrite the cache"
    );
}

#[test]
fn no_cache_env_disables_cache_writes() {
    let sb = Sandbox::new();
    sb.init();
    sb.add_stdio("fs");
    // Drop anything a prior (non-sandboxed) step may have produced.
    let cache_dir = sb.path("data/cache");
    let _ = fs::remove_dir_all(&cache_dir);

    sb.cmd()
        .env("CERBERUS_NO_CACHE", "1")
        .args(["config", "validate"])
        .assert()
        .success();
    assert!(
        find_cbin(&sb.path("data")).is_empty(),
        "CERBERUS_NO_CACHE=1 must not write cache files"
    );

    // The explicit compile subcommand always builds (it is not gated).
    sb.cmd()
        .env("CERBERUS_NO_CACHE", "1")
        .args(["config", "compile"])
        .assert()
        .success();
    assert_eq!(find_cbin(&sb.path("data")).len(), 1);
}
