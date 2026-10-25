//! Canonical registry schema.
//!
//! The registry is a versioned, human-readable document that describes MCP
//! servers, profiles, policies, reusable commands, and defaults. The default
//! format is TOML (`conf.cerberus`); legacy YAML documents (`cerberus.yaml`)
//! are still read through the migration chain. It is deliberately
//! independent of any particular AI client.

use std::collections::BTreeMap;
use std::fmt;

use rkyv::{Archive, Deserialize as RkyvDeserialize, Serialize as RkyvSerialize};
use serde::{Deserialize, Serialize};

/// Current schema version understood by this build.
pub const SCHEMA_VERSION: u32 = 2;

/// Top-level registry document.
#[derive(
    Debug, Clone, PartialEq, Serialize, Deserialize, Archive, RkyvSerialize, RkyvDeserialize,
)]
#[serde(deny_unknown_fields)]
pub struct Registry {
    pub schema_version: u32,
    #[serde(default, skip_serializing_if = "Settings::is_default")]
    pub settings: Settings,
    /// Named reusable commands, runnable with `cerberus run <name>` and
    /// referenceable from server `pre_launch` hooks.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub commands: BTreeMap<String, CommandDef>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub servers: BTreeMap<String, ServerDef>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub profiles: BTreeMap<String, Profile>,
    /// Name of the profile used when no explicit profile is requested.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_profile: Option<String>,
}

impl Default for Registry {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            settings: Settings::default(),
            commands: BTreeMap::new(),
            servers: BTreeMap::new(),
            profiles: BTreeMap::new(),
            active_profile: None,
        }
    }
}

/// A named reusable command (`cerberus run <name>`).
///
/// Commands are plain exec vectors — never an implicit shell. Runners that
/// need shell semantics spell them out (`["sh", "-c", "..."]`).
#[derive(
    Debug, Clone, PartialEq, Serialize, Deserialize, Archive, RkyvSerialize, RkyvDeserialize,
)]
#[serde(deny_unknown_fields)]
pub struct CommandDef {
    /// Executable + arguments, executed directly (no shell).
    pub run: Vec<String>,
    /// Working directory; `~` is expanded to the user's home.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// Extra environment entries, resolved like transport env values.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, EnvValue>,
    /// Wall-clock timeout in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    /// Human-readable description shown by `cerberus run`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// One step executed before a stdio upstream is spawned.
///
/// Either references a top-level command (`use`) or carries an inline argv
/// (`run`) — exactly one of the two must be set.
#[derive(
    Debug, Clone, PartialEq, Serialize, Deserialize, Archive, RkyvSerialize, RkyvDeserialize,
)]
#[serde(deny_unknown_fields)]
pub struct HookStep {
    /// Name of a `[commands.<name>]` entry to run.
    #[serde(rename = "use", default, skip_serializing_if = "Option::is_none")]
    pub use_command: Option<String>,
    /// Inline executable + arguments (alternative to `use`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<Vec<String>>,
    /// Working directory; falls back to the server transport's cwd.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// Extra environment entries, merged over the server launch env.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, EnvValue>,
    /// Per-step timeout in milliseconds (default: 30_000).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    /// Log-and-continue instead of aborting the upstream connect.
    #[serde(default, skip_serializing_if = "is_false")]
    pub continue_on_error: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(
    Debug, Clone, PartialEq, Serialize, Deserialize, Archive, RkyvSerialize, RkyvDeserialize,
)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    /// Default execution mode for servers that do not override it.
    #[serde(default)]
    pub default_mode: ExecutionMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_level: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            default_mode: ExecutionMode::Direct,
            log_level: None,
        }
    }
}

impl Settings {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    Serialize,
    Deserialize,
    Archive,
    RkyvSerialize,
    RkyvDeserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    /// The client connects to the upstream server directly.
    #[default]
    Direct,
    /// The client connects to the Cerberus gateway, which proxies the upstream.
    Gateway,
}

/// Where a server comes from: install provenance for `cerberus search`,
/// `cerberus install`, and `cerberus doctor`.
///
/// The gateway and the client adapters never read this block — it only
/// records how the entry could be re-created or verified outside Cerberus.
#[derive(
    Debug, Clone, PartialEq, Serialize, Deserialize, Archive, RkyvSerialize, RkyvDeserialize,
)]
#[serde(deny_unknown_fields)]
pub struct SourceDef {
    pub kind: SourceKind,
    /// Package identifier (`npm`/`pypi`) or executable name (`binary`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    /// Version pin or requirement (e.g. `0.6.2`, `^1.2`, `>=1,<2`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Repository or homepage for humans.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    /// Runtime that launches the package: `npx`, `uvx`, `pnpm dlx`, ...
    /// Defaults to `npx` for `npm` and `uvx` for `pypi` when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner: Option<String>,
    /// Positional arguments given to the runner before the package
    /// (e.g. `-y` for npx).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    /// Source location for `git` sources.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Git ref (branch, tag, or commit) for `git` sources.
    #[serde(rename = "ref", default, skip_serializing_if = "Option::is_none")]
    pub git_ref: Option<String>,
    /// Environment variables the server documents. `cerberus doctor`
    /// reports which of these are missing from the launch environment.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env: Vec<SourceEnv>,
}

/// Packaging system a source comes from.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Archive,
    RkyvSerialize,
    RkyvDeserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Npm,
    Pypi,
    Git,
    Binary,
}

/// One environment variable a server documents needing.
#[derive(
    Debug, Clone, PartialEq, Serialize, Deserialize, Archive, RkyvSerialize, RkyvDeserialize,
)]
#[serde(deny_unknown_fields)]
pub struct SourceEnv {
    pub name: String,
    #[serde(default = "default_true")]
    pub required: bool,
    #[serde(default)]
    pub secret: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl SourceDef {
    /// Runtime that would launch this source when the registry entry does
    /// not pin one. `None` for sources whose launcher is the transport
    /// itself (`binary`) or a clone step (`git`).
    pub fn default_runner(&self) -> Option<&'static str> {
        match self.kind {
            SourceKind::Npm => Some("npx"),
            SourceKind::Pypi => Some("uvx"),
            SourceKind::Git | SourceKind::Binary => None,
        }
    }
}

/// A registered MCP server.
#[derive(
    Debug, Clone, PartialEq, Serialize, Deserialize, Archive, RkyvSerialize, RkyvDeserialize,
)]
#[serde(deny_unknown_fields)]
pub struct ServerDef {
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub transport: Transport,
    /// Execution mode override; falls back to `settings.default_mode`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<ExecutionMode>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Per-request timeout applied to tool calls, in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    /// Timeout applied while starting/connecting to the server, in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub startup_timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "ServerPolicy::is_empty")]
    pub policy: ServerPolicy,
    /// Optional human-readable description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Steps executed before this server's process is spawned (stdio only).
    /// The gateway runs them on every connect attempt; direct-mode client
    /// adapters cannot express them and ignore this block.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pre_launch: Vec<HookStep>,
    /// Install provenance. Ignored by the gateway and by client adapters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceDef>,
}

fn default_true() -> bool {
    true
}

impl ServerDef {
    pub fn effective_mode(&self, defaults: &Settings) -> ExecutionMode {
        self.mode.unwrap_or(defaults.default_mode)
    }
}

/// Transport definition. Typed rather than a free-form map so invalid
/// combinations fail validation instead of surfacing at connect time.
#[derive(
    Debug, Clone, PartialEq, Serialize, Deserialize, Archive, RkyvSerialize, RkyvDeserialize,
)]
#[serde(deny_unknown_fields, tag = "type", rename_all = "snake_case")]
pub enum Transport {
    /// Launch a local process and speak MCP over its stdin/stdout.
    Stdio {
        command: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        args: Vec<String>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        env: BTreeMap<String, EnvValue>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
    },
    /// Connect to a remote MCP server over Streamable HTTP.
    Http {
        url: String,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        headers: BTreeMap<String, HeaderValue>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bearer_token: Option<EnvValue>,
    },
}

impl Transport {
    pub fn kind(&self) -> &'static str {
        match self {
            Transport::Stdio { .. } => "stdio",
            Transport::Http { .. } => "http",
        }
    }

    pub fn command_display(&self) -> String {
        match self {
            Transport::Stdio { command, args, .. } => {
                let mut out = command.clone();
                for a in args {
                    out.push(' ');
                    out.push_str(a);
                }
                out
            }
            Transport::Http { url, .. } => url.clone(),
        }
    }
}

/// A secret-bearing or plain configuration value.
///
/// Credentials should never be embedded directly; use `env` or `secret_ref`.
#[derive(
    Debug, Clone, PartialEq, Serialize, Deserialize, Archive, RkyvSerialize, RkyvDeserialize,
)]
#[serde(deny_unknown_fields, tag = "source", rename_all = "snake_case")]
pub enum EnvValue {
    /// Read from an environment variable of the given name at launch time.
    Env { name: String },
    /// Resolve through the configured secret provider.
    SecretRef { name: String },
    /// Embed a literal value. Discouraged for credentials; accepted for
    /// non-sensitive knobs and validated only structurally.
    Literal { value: String },
}

impl EnvValue {
    pub fn literal(value: impl Into<String>) -> Self {
        EnvValue::Literal {
            value: value.into(),
        }
    }

    pub fn env(name: impl Into<String>) -> Self {
        EnvValue::Env { name: name.into() }
    }

    /// A display form that never reveals secret material.
    pub fn redacted_display(&self) -> String {
        match self {
            EnvValue::Env { name } => format!("env:{name}"),
            EnvValue::SecretRef { name } => format!("secret:{name}"),
            EnvValue::Literal { .. } => "<literal>".to_string(),
        }
    }
}

/// HTTP header value, resolved from env/secret or literal.
#[derive(
    Debug, Clone, PartialEq, Serialize, Deserialize, Archive, RkyvSerialize, RkyvDeserialize,
)]
#[serde(untagged)]
pub enum HeaderValue {
    Resolved(EnvValue),
    Literal(String),
}

impl HeaderValue {
    pub fn redacted_display(&self) -> String {
        match self {
            HeaderValue::Resolved(v) => v.redacted_display(),
            HeaderValue::Literal(_) => "<literal>".to_string(),
        }
    }
}

/// Per-server access policy evaluated by the gateway.
#[derive(
    Debug,
    Clone,
    Default,
    PartialEq,
    Serialize,
    Deserialize,
    Archive,
    RkyvSerialize,
    RkyvDeserialize,
)]
#[serde(deny_unknown_fields, default)]
pub struct ServerPolicy {
    /// When non-empty, only these tools may be called. Supports a trailing
    /// `*` wildcard (e.g. `read_*`).
    pub allow_tools: Vec<String>,
    /// Tools that may never be called. Evaluated after `allow_tools`.
    pub deny_tools: Vec<String>,
    /// When true, every tool call requires configured confirmation (surfaced
    /// by the gateway as an error unless the caller passes the confirm
    /// handshake). Used for high-impact servers.
    pub require_confirmation: bool,
}

impl ServerPolicy {
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

/// A named set of servers activated together.
#[derive(
    Debug,
    Clone,
    Default,
    PartialEq,
    Serialize,
    Deserialize,
    Archive,
    RkyvSerialize,
    RkyvDeserialize,
)]
#[serde(deny_unknown_fields, default)]
pub struct Profile {
    pub servers: Vec<String>,
    pub description: Option<String>,
}

impl Registry {
    pub fn get_server(&self, name: &str) -> Option<&ServerDef> {
        self.servers.get(name)
    }

    pub fn server_names(&self) -> Vec<&str> {
        self.servers.keys().map(String::as_str).collect()
    }

    /// Resolve which servers should be active for the given profile (or the
    /// active profile when `None`).
    pub fn resolve_profile_servers(
        &self,
        profile: Option<&str>,
    ) -> crate::error::Result<Vec<String>> {
        let name = profile
            .map(str::to_string)
            .or_else(|| self.active_profile.clone());

        let Some(name) = name else {
            // No profile selected: all enabled servers.
            return Ok(self
                .servers
                .iter()
                .filter(|(_, s)| s.enabled)
                .map(|(n, _)| n.clone())
                .collect());
        };

        let profile = self
            .profiles
            .get(&name)
            .ok_or_else(|| crate::error::CerberusError::ProfileNotFound { name: name.clone() })?;

        let mut out = Vec::new();
        for server in &profile.servers {
            let def = self.servers.get(server).ok_or_else(|| {
                crate::error::CerberusError::Validation(format!(
                    "profile `{name}` references unknown server `{server}`"
                ))
            })?;
            if def.enabled {
                out.push(server.clone());
            }
        }
        Ok(out)
    }
}

impl fmt::Display for EnvValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.redacted_display())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_full_document() {
        let toml = r#"
schema_version = 2
active_profile = "development"

[settings]
default_mode = "direct"
log_level = "info"

[servers.github]
tags = ["development"]

[servers.github.transport]
type = "stdio"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-github"]

[servers.github.transport.env.GITHUB_PERSONAL_ACCESS_TOKEN]
source = "secret_ref"
name = "github_token"

[profiles.development]
servers = ["github"]
"#;
        let reg: Registry = toml::from_str(toml).unwrap();
        assert_eq!(reg.schema_version, SCHEMA_VERSION);
        assert!(reg.servers.contains_key("github"));
        assert_eq!(reg.active_profile.as_deref(), Some("development"));
        let rendered = toml::to_string(&reg).unwrap();
        let back: Registry = toml::from_str(&rendered).unwrap();
        assert_eq!(reg, back);
    }

    #[test]
    fn rejects_unknown_fields() {
        let toml = "schema_version = 2\nunknown_top = true\n";
        assert!(toml::from_str::<Registry>(toml).is_err());
    }

    #[test]
    fn http_transport_parses() {
        let toml = r#"
schema_version = 2

[servers.remote.transport]
type = "http"
url = "https://example.com/mcp"

[servers.remote.transport.headers]
Authorization = "Bearer xyz"
"#;
        let reg: Registry = toml::from_str(toml).unwrap();
        match &reg.servers["remote"].transport {
            Transport::Http { url, headers, .. } => {
                assert_eq!(url, "https://example.com/mcp");
                assert!(headers.contains_key("Authorization"));
            }
            other => panic!("expected http transport, got {other:?}"),
        }
    }

    #[test]
    fn profile_resolution_filters_disabled() {
        let toml = r#"
schema_version = 2

[servers.a.transport]
type = "stdio"
command = "echo"

[servers.b]
enabled = false

[servers.b.transport]
type = "stdio"
command = "echo"

[profiles.dev]
servers = ["a", "b"]
"#;
        let reg: Registry = toml::from_str(toml).unwrap();
        let resolved = reg.resolve_profile_servers(Some("dev")).unwrap();
        assert_eq!(resolved, vec!["a".to_string()]);
    }

    #[test]
    fn commands_and_pre_launch_roundtrip() {
        let toml = r#"
schema_version = 2

[commands.setup-env]
run = ["./scripts/env.sh", "--profile", "dev"]
description = "Prepare local env"
timeout_ms = 10000

[commands.setup-env.env]
REGION = { source = "env", name = "AWS_REGION" }

[servers.echo]
pre_launch = [{ use = "setup-env" }, { run = ["docker", "compose", "up", "-d"], continue_on_error = true }]

[servers.echo.transport]
type = "stdio"
command = "npx"
"#;
        let reg: Registry = toml::from_str(toml).unwrap();
        let cmd = &reg.commands["setup-env"];
        assert_eq!(cmd.run, vec!["./scripts/env.sh", "--profile", "dev"]);
        assert_eq!(cmd.timeout_ms, Some(10_000));
        let hooks = &reg.servers["echo"].pre_launch;
        assert_eq!(hooks.len(), 2);
        assert_eq!(hooks[0].use_command.as_deref(), Some("setup-env"));
        assert!(hooks[0].run.is_none());
        assert!(hooks[1].continue_on_error);
        let rendered = toml::to_string(&reg).unwrap();
        let back: Registry = toml::from_str(&rendered).unwrap();
        assert_eq!(reg, back);
    }

    #[test]
    fn empty_pre_launch_is_skipped_when_serializing() {
        let mut reg = Registry::default();
        reg.servers.insert(
            "a".into(),
            ServerDef {
                enabled: true,
                transport: Transport::Stdio {
                    command: "echo".into(),
                    args: vec![],
                    env: BTreeMap::new(),
                    cwd: None,
                },
                mode: None,
                tags: vec![],
                timeout_ms: None,
                startup_timeout_ms: None,
                policy: ServerPolicy::default(),
                description: None,
                pre_launch: vec![],
                source: None,
            },
        );
        let rendered = toml::to_string(&reg).unwrap();
        assert!(!rendered.contains("pre_launch"));
    }
}
