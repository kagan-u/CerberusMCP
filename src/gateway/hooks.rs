//! Pre-launch hook execution (`server.pre_launch`).
//!
//! Steps run only inside the gateway's connect path
//! ([`crate::gateway::GatewayState::connect_upstream`]), immediately before
//! a stdio upstream's process is spawned. Doctor probes, client adapters,
//! and direct-mode launches never execute them — `cerberus doctor` reports
//! that as an Info diagnostic.
//!
//! Policy mirrors `cerberus run` and the server launcher: explicit exec
//! vectors only (no implicit shell), the same sanitized launch environment
//! the server process would get (safe baseline + the server's resolved
//! bindings) with the step's own bindings on top, `~`-expanded cwd, and a
//! per-step timeout (default 30 s). Hooks re-run on every connect attempt,
//! including lazy reconnects after a failure, so they must be idempotent.

use std::collections::BTreeMap;
use std::time::Duration;

use crate::error::{CerberusError, Result};
use crate::process::{EnvPolicy, build_env};
use crate::registry::schema::{CommandDef, EnvValue, HookStep, ServerDef, Transport};
use crate::secrets::SecretStore;

/// Per-step timeout when neither the step nor its command sets one.
const DEFAULT_STEP_TIMEOUT_MS: u64 = 30_000;
/// Cap on captured output echoed into errors and logs (bytes, per stream).
const OUTPUT_CAP: usize = 4096;

/// Execute `def.pre_launch` in order, just before the stdio process would
/// spawn. Returns `Err(CerberusError::Upstream { .. })` on a failed step —
/// the same envelope every other connect failure uses — unless the step
/// sets `continue_on_error`.
pub(crate) async fn run_pre_launch(
    server: &str,
    def: &ServerDef,
    commands: &BTreeMap<String, CommandDef>,
    secrets: &SecretStore,
) -> Result<()> {
    if def.pre_launch.is_empty() {
        return Ok(());
    }
    for (i, step) in def.pre_launch.iter().enumerate() {
        let location = format!("servers.{server}.pre_launch[{i}]");
        if let Err(e) = run_step(server, &location, step, def, commands, secrets).await {
            if step.continue_on_error {
                tracing::warn!(
                    server = %server,
                    step = %location,
                    error = %e,
                    "pre-launch step failed; continuing (continue_on_error)"
                );
            } else {
                return Err(CerberusError::Upstream {
                    server: server.to_string(),
                    detail: format!("pre-launch {location}: {e}"),
                });
            }
        }
    }
    Ok(())
}

/// A hook step with its `use` reference resolved to concrete fields;
/// step-level fields win over the command's defaults.
struct Resolved {
    argv: Vec<String>,
    cwd: Option<String>,
    env: BTreeMap<String, EnvValue>,
    timeout_ms: u64,
}

fn resolve(
    step: &HookStep,
    commands: &BTreeMap<String, CommandDef>,
) -> std::result::Result<Resolved, String> {
    let mut resolved = match (&step.run, &step.use_command) {
        (Some(run), None) => Resolved {
            argv: run.clone(),
            cwd: None,
            env: BTreeMap::new(),
            timeout_ms: step.timeout_ms.unwrap_or(DEFAULT_STEP_TIMEOUT_MS),
        },
        (None, Some(name)) => {
            let command = commands.get(name).ok_or_else(|| {
                format!("references unknown command `{name}` (no [commands.{name}])")
            })?;
            Resolved {
                argv: command.run.clone(),
                cwd: command.cwd.clone(),
                env: command.env.clone(),
                timeout_ms: step
                    .timeout_ms
                    .or(command.timeout_ms)
                    .unwrap_or(DEFAULT_STEP_TIMEOUT_MS),
            }
        }
        (Some(_), Some(_)) => return Err("both `use` and `run` are set".into()),
        (None, None) => return Err("neither `use` nor `run` is set".into()),
    };
    // Step-level fields override the command's defaults.
    if let Some(cwd) = &step.cwd {
        resolved.cwd = Some(cwd.clone());
    }
    for (k, v) in &step.env {
        resolved.env.insert(k.clone(), v.clone());
    }
    Ok(resolved)
}

async fn run_step(
    server: &str,
    location: &str,
    step: &HookStep,
    def: &ServerDef,
    commands: &BTreeMap<String, CommandDef>,
    secrets: &SecretStore,
) -> std::result::Result<(), String> {
    let resolved = resolve(step, commands)?;
    let (first, rest) = resolved.argv.split_first().ok_or("run vector is empty")?;
    if first.is_empty() {
        return Err("executable must not be empty".into());
    }

    let mut cmd = tokio::process::Command::new(first);
    cmd.args(rest);
    // Hooks are batch steps: they never read stdin, and both streams are
    // captured so failures can quote them.
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    // Base = the sanitized launch environment the server itself would get
    // (safe baseline + the server's resolved bindings), then the command's
    // bindings, then the step's — matching the schema's documented order.
    if let Transport::Stdio {
        env: server_env, ..
    } = &def.transport
    {
        let base = build_env(server_env, secrets, &EnvPolicy::default())
            .map_err(|e| format!("launch environment: {e}"))?;
        cmd.envs(base);
    }
    for (k, v) in &resolved.env {
        let value = secrets
            .resolve_env_value(v)
            .map_err(|e| format!("env `{k}`: {e}"))?;
        cmd.env(k, value);
    }
    if let Some(cwd) = &resolved.cwd {
        cmd.current_dir(crate::paths::expand_tilde(std::path::Path::new(cwd)));
    }

    let label = resolved.argv.join(" ");
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("failed to spawn `{first}`: {e}"))?;

    // Drain both pipes while waiting: a chatty child must not deadlock on
    // a full pipe buffer.
    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();
    let stdout_task = tokio::spawn(drain(stdout_pipe));
    let stderr_task = tokio::spawn(drain(stderr_pipe));

    let status = match tokio::time::timeout(
        Duration::from_millis(resolved.timeout_ms),
        child.wait(),
    )
    .await
    {
        Ok(Ok(status)) => status,
        Ok(Err(e)) => return Err(format!("waiting for `{label}`: {e}")),
        Err(_) => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            return Err(format!("timed out after {} ms", resolved.timeout_ms));
        }
    };
    let stdout = String::from_utf8_lossy(&stdout_task.await.unwrap_or_default()).into_owned();
    let stderr = String::from_utf8_lossy(&stderr_task.await.unwrap_or_default()).into_owned();

    if !status.success() {
        let code = status
            .code()
            .map(|c| c.to_string())
            .unwrap_or_else(|| "signal".into());
        let detail = if stderr.trim().is_empty() {
            tail(&stdout, OUTPUT_CAP)
        } else {
            tail(&stderr, OUTPUT_CAP)
        };
        let detail = detail.trim();
        return Err(if detail.is_empty() {
            format!("exited with code {code}")
        } else {
            format!("exited with code {code}: {detail}")
        });
    }
    tracing::debug!(
        server = %server,
        step = %location,
        cmd = %label,
        stdout = %tail(&stdout, 512),
        "pre-launch step succeeded"
    );
    Ok(())
}

async fn drain(pipe: Option<impl tokio::io::AsyncRead + Unpin + Send + 'static>) -> Vec<u8> {
    use tokio::io::AsyncReadExt;
    let mut buf = Vec::new();
    if let Some(mut pipe) = pipe {
        let _ = pipe.read_to_end(&mut buf).await;
    }
    buf
}

/// Last `cap` bytes of `text` as a lossy string (for diagnostics).
fn tail(text: &str, cap: usize) -> String {
    if text.len() <= cap {
        return text.to_string();
    }
    let start = text.len() - cap;
    // Do not split a UTF-8 code point.
    let start = text
        .char_indices()
        .find(|(i, _)| *i >= start)
        .map(|(i, _)| i)
        .unwrap_or(start);
    text[start..].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::schema::ServerDef;

    fn hook_run(args: &[&str]) -> HookStep {
        HookStep {
            use_command: None,
            run: Some(args.iter().map(|s| s.to_string()).collect()),
            cwd: None,
            env: BTreeMap::new(),
            timeout_ms: None,
            continue_on_error: false,
        }
    }

    fn stdio_def(pre_launch: Vec<HookStep>) -> ServerDef {
        ServerDef {
            enabled: true,
            transport: Transport::Stdio {
                command: "cat".into(),
                args: vec![],
                env: BTreeMap::new(),
                cwd: None,
            },
            mode: None,
            tags: vec![],
            timeout_ms: None,
            startup_timeout_ms: None,
            policy: Default::default(),
            description: None,
            source: None,
            pre_launch,
        }
    }

    /// A command appending `token` to `file`, portable across platforms.
    /// Runs in the file's directory with a bare relative name: cmd.exe does
    /// not honour Rust's `\"` escapes, so quoted paths break redirects.
    fn append_step(file: &std::path::Path, token: &str) -> HookStep {
        let name = file.file_name().unwrap().to_string_lossy().into_owned();
        let mut step = if cfg!(windows) {
            hook_run(&["cmd", "/C", &format!("echo {token}>>{name}")])
        } else {
            hook_run(&["sh", "-c", &format!("echo {token} >> {name}")])
        };
        step.cwd = Some(file.parent().unwrap().to_string_lossy().replace('\\', "/"));
        step
    }

    #[tokio::test]
    async fn runs_steps_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("out.txt");
        let def = stdio_def(vec![append_step(&file, "A"), append_step(&file, "B")]);
        run_pre_launch("s", &def, &BTreeMap::new(), &SecretStore::default())
            .await
            .expect("steps succeed");
        let content = fs_read(&file);
        let a = content.find('A').expect("A written");
        let b = content.find('B').expect("B written");
        assert!(a < b, "steps must run in order: {content:?}");
    }

    #[tokio::test]
    async fn unknown_use_reference_fails() {
        let def = stdio_def(vec![HookStep {
            use_command: Some("nope".into()),
            run: None,
            cwd: None,
            env: BTreeMap::new(),
            timeout_ms: None,
            continue_on_error: false,
        }]);
        let err = run_pre_launch("s", &def, &BTreeMap::new(), &SecretStore::default())
            .await
            .expect_err("unknown command");
        let msg = err.to_string();
        assert!(msg.contains("unknown command `nope`"), "{msg}");
        assert!(msg.contains("pre_launch[0]"), "{msg}");
    }

    #[tokio::test]
    async fn non_zero_exit_surfaces_stderr() {
        let fail = if cfg!(windows) {
            hook_run(&["cmd", "/C", "(echo boom 1>&2 & exit /b 3)"])
        } else {
            hook_run(&["sh", "-c", "echo boom >&2; exit 3"])
        };
        let def = stdio_def(vec![fail]);
        let err = run_pre_launch("s", &def, &BTreeMap::new(), &SecretStore::default())
            .await
            .expect_err("must fail");
        let msg = err.to_string();
        assert!(msg.contains("exited with code 3"), "{msg}");
        assert!(msg.contains("boom"), "stderr must be quoted: {msg}");
        assert!(msg.contains("upstream") || matches!(err, CerberusError::Upstream { .. }));
    }

    #[tokio::test]
    async fn continue_on_error_swallows_failure_but_others_still_run() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("out.txt");
        let mut failing = if cfg!(windows) {
            hook_run(&["cmd", "/C", "exit /b 9"])
        } else {
            hook_run(&["sh", "-c", "exit 9"])
        };
        failing.continue_on_error = true;
        let def = stdio_def(vec![failing, append_step(&file, "K")]);
        run_pre_launch("s", &def, &BTreeMap::new(), &SecretStore::default())
            .await
            .expect("continue_on_error must not abort");
        assert!(fs_read(&file).contains('K'), "later steps still run");
    }

    #[tokio::test]
    async fn failing_step_aborts_following_steps() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("out.txt");
        let failing = if cfg!(windows) {
            hook_run(&["cmd", "/C", "exit /b 9"])
        } else {
            hook_run(&["sh", "-c", "exit 9"])
        };
        let def = stdio_def(vec![failing, append_step(&file, "K")]);
        let err = run_pre_launch("s", &def, &BTreeMap::new(), &SecretStore::default())
            .await
            .expect_err("must abort");
        assert!(matches!(err, CerberusError::Upstream { .. }));
        assert!(
            !file.exists(),
            "steps after a failure must not run: {}",
            fs_read(&file)
        );
    }

    #[tokio::test]
    async fn step_timeout_is_enforced() {
        let mut step = if cfg!(windows) {
            hook_run(&["ping", "-n", "6", "127.0.0.1"])
        } else {
            hook_run(&["sh", "-c", "sleep 5"])
        };
        step.timeout_ms = Some(100);
        let def = stdio_def(vec![step]);
        let started = std::time::Instant::now();
        let err = run_pre_launch("s", &def, &BTreeMap::new(), &SecretStore::default())
            .await
            .expect_err("timed out");
        assert!(
            started.elapsed() < Duration::from_secs(4),
            "child must be killed on timeout"
        );
        let msg = err.to_string();
        assert!(msg.contains("timed out after 100 ms"), "{msg}");
    }

    #[tokio::test]
    async fn env_bindings_and_cwd_resolve_from_step() {
        let dir = tempfile::tempdir().unwrap();
        // Succeed only if HOOK_X=42 reaches the child AND cwd is the temp dir.
        let mut step = if cfg!(windows) {
            hook_run(&[
                "cmd",
                "/C",
                "if not \"%HOOK_X%\"==\"42\" exit /b 1 & if not exist \"marker.txt\" exit /b 2",
            ])
        } else {
            hook_run(&["sh", "-c", "[ \"$HOOK_X\" = 42 ] && test -f marker.txt"])
        };
        step.env
            .insert("HOOK_X".into(), EnvValue::Literal { value: "42".into() });
        step.cwd = Some(dir.path().to_string_lossy().replace('\\', "/"));
        std::fs::write(dir.path().join("marker.txt"), "x").unwrap();

        let def = stdio_def(vec![step]);
        run_pre_launch("s", &def, &BTreeMap::new(), &SecretStore::default())
            .await
            .expect("env + cwd honored");
    }

    #[tokio::test]
    async fn use_reference_takes_cwd_and_env_from_command() {
        let dir = tempfile::tempdir().unwrap();
        let mut command = if cfg!(windows) {
            crate::registry::schema::CommandDef {
                run: vec![
                    "cmd".into(),
                    "/C".into(),
                    "if not exist marker.txt exit /b 2".into(),
                ],
                cwd: None,
                env: BTreeMap::new(),
                timeout_ms: None,
                description: None,
            }
        } else {
            crate::registry::schema::CommandDef {
                run: vec!["sh".into(), "-c".into(), "test -f marker.txt".into()],
                cwd: None,
                env: BTreeMap::new(),
                timeout_ms: None,
                description: None,
            }
        };
        command.cwd = Some(dir.path().to_string_lossy().replace('\\', "/"));
        command
            .env
            .insert("FROM_CMD".into(), EnvValue::Literal { value: "1".into() });
        std::fs::write(dir.path().join("marker.txt"), "x").unwrap();

        let mut commands = BTreeMap::new();
        commands.insert("check".to_string(), command);
        let def = stdio_def(vec![HookStep {
            use_command: Some("check".into()),
            run: None,
            cwd: None,
            env: BTreeMap::new(),
            timeout_ms: None,
            continue_on_error: false,
        }]);
        run_pre_launch("s", &def, &commands, &SecretStore::default())
            .await
            .expect("command cwd + env honored via use");
    }

    #[tokio::test]
    async fn empty_pre_launch_is_a_noop() {
        let def = stdio_def(vec![]);
        run_pre_launch("s", &def, &BTreeMap::new(), &SecretStore::default())
            .await
            .expect("no steps, no work");
    }

    fn fs_read(path: &std::path::Path) -> String {
        std::fs::read_to_string(path).unwrap_or_default()
    }
}
