//! Process execution helpers shared by health checks and the gateway.
//!
//! Security: commands are launched with an explicit executable and argument
//! vector — never through a shell. The environment is constructed explicitly
//! rather than inherited wholesale.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;

use crate::error::{CerberusError, Result};
use crate::registry::schema::{EnvValue, ServerDef, Transport};
use crate::secrets::SecretStore;

/// Environment policy for spawned servers.
#[derive(Debug, Clone, Default)]
pub struct EnvPolicy {
    /// Inherit the parent environment. Default false: only explicitly
    /// referenced variables plus a small safe baseline are provided.
    pub inherit: bool,
}

const SAFE_BASELINE: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "TMPDIR",
    "TEMP",
    "TMP",
    "LANG",
    "LC_ALL",
    "SYSTEMROOT",
    "WINDIR",
    "COMSPEC",
];

/// Build the child environment for an MCP server.
pub fn build_env(
    extra: &BTreeMap<String, EnvValue>,
    secrets: &SecretStore,
    policy: &EnvPolicy,
) -> Result<BTreeMap<String, String>> {
    let mut env = BTreeMap::new();
    if policy.inherit {
        for (k, v) in std::env::vars() {
            env.insert(k, v);
        }
    } else {
        for key in SAFE_BASELINE {
            if let Ok(v) = std::env::var(key) {
                env.insert((*key).to_string(), v);
            }
        }
    }
    for (k, v) in extra {
        env.insert(k.clone(), secrets.resolve_env_value(v)?);
    }
    Ok(env)
}

/// Construct (but do not spawn) a stdio server command.
pub fn build_command(
    def: &ServerDef,
    secrets: &SecretStore,
    policy: &EnvPolicy,
) -> Result<Command> {
    let Transport::Stdio {
        command,
        args,
        env,
        cwd,
    } = &def.transport
    else {
        return Err(CerberusError::Transport(
            "build_command called for a non-stdio transport".into(),
        ));
    };

    let mut cmd = Command::new(command);
    cmd.args(args);
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::inherit());

    if let Some(cwd) = cwd {
        cmd.current_dir(crate::paths::expand_tilde(cwd));
    }

    if !policy.inherit {
        // Clear inherited environment, then set only what we resolved.
        cmd.env_clear();
        cmd.env_remove("PATH"); // restored below from build_env baseline
    }
    let resolved = build_env(env, secrets, policy)?;
    for (k, v) in resolved {
        cmd.env(k, v);
    }
    Ok(cmd)
}

/// Probe that an executable exists (without running it).
///
/// On Windows the bare name is not enough: launchers such as `npx` are
/// `npx.cmd`, so `PATHEXT` extensions are tried in order.
pub fn executable_exists(command: &str) -> bool {
    let path = PathBuf::from(command);
    if path.is_absolute() || command.contains('/') || command.contains('\\') {
        return path.exists();
    }
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    let mut extensions: Vec<std::ffi::OsString> = vec![std::ffi::OsString::new()];
    if cfg!(windows) {
        let pathtext = std::env::var("PATHEXT").unwrap_or_else(|_| ".EXE;.CMD;.BAT;.COM".into());
        extensions.extend(
            pathtext
                .split(';')
                .filter(|ext| !ext.is_empty())
                .map(std::ffi::OsString::from),
        );
    }
    std::env::split_paths(&paths).any(|dir| {
        extensions.iter().any(|ext| {
            let candidate = dir.join(format!("{command}{}", ext.to_string_lossy()));
            candidate.exists()
        })
    })
}

/// Run a stdio server command with a hard timeout and capture whether it
/// stays alive briefly. Used by health checks.
pub async fn probe_stdio_liveness(
    def: &ServerDef,
    secrets: &SecretStore,
    timeout: Duration,
) -> Result<()> {
    let mut cmd = build_command(def, secrets, &EnvPolicy::default())?;
    cmd.stderr(Stdio::null());

    let mut child = cmd.spawn().map_err(|source| CerberusError::ProcessSpawn {
        command: def.transport.command_display(),
        source,
    })?;

    let result = tokio::time::timeout(timeout, child.wait()).await;
    match result {
        Err(_) => {
            // Still running after the probe window: good sign.
            let _ = child.kill().await;
            Ok(())
        }
        Ok(Ok(status)) => Err(CerberusError::Upstream {
            server: String::new(),
            detail: format!("process exited immediately with status {status} during health probe"),
        }),
        Ok(Err(e)) => Err(CerberusError::Upstream {
            server: String::new(),
            detail: format!("wait failed: {e}"),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::schema::ServerPolicy;
    use std::collections::BTreeMap;

    fn stdio(cmd: &str, args: &[&str]) -> ServerDef {
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
        }
    }

    #[test]
    fn build_env_uses_baseline_not_full_inheritance() {
        unsafe {
            std::env::set_var("CERBERUS_TEST_ONLY_SECRET", "should-not-leak");
        };
        let secrets = SecretStore::default();
        let env = build_env(&BTreeMap::new(), &secrets, &EnvPolicy::default()).unwrap();
        assert!(!env.contains_key("CERBERUS_TEST_ONLY_SECRET"));
        assert!(env.contains_key("PATH") || !env.is_empty());
    }

    #[test]
    fn build_env_resolves_refs() {
        unsafe {
            std::env::set_var("CERBERUS_TEST_RESOLVE_ME", "value123");
        };
        let secrets = SecretStore::default();
        let extra = BTreeMap::from([(
            "MY_VAR".to_string(),
            EnvValue::env("CERBERUS_TEST_RESOLVE_ME"),
        )]);
        let env = build_env(&extra, &secrets, &EnvPolicy::default()).unwrap();
        assert_eq!(env["MY_VAR"], "value123");
    }

    #[test]
    fn missing_env_reference_is_actionable_error() {
        let secrets = SecretStore::default();
        let extra = BTreeMap::from([(
            "MY_VAR".to_string(),
            EnvValue::env("CERBERUS_DEFINITELY_MISSING_999"),
        )]);
        let err = build_env(&extra, &secrets, &EnvPolicy::default()).unwrap_err();
        assert!(err.to_string().contains("not set"));
    }

    #[tokio::test]
    async fn probe_detects_immediate_exit() {
        let secrets = SecretStore::default();
        let def = stdio("false", &[]);
        let err = probe_stdio_liveness(&def, &secrets, Duration::from_secs(2))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("exited immediately"));
    }

    #[tokio::test]
    async fn probe_passes_for_sleeping_process() {
        let secrets = SecretStore::default();
        let def = stdio("sleep", &["5"]);
        probe_stdio_liveness(&def, &secrets, Duration::from_millis(400))
            .await
            .unwrap();
    }
}
