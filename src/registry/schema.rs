//! Canonical registry schema.
//!
//! The registry is a versioned, human-readable YAML document that describes
//! MCP servers, profiles, policies, and defaults. It is deliberately
//! independent of any particular AI client.

use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Current schema version understood by this build.
pub const SCHEMA_VERSION: u32 = 1;

/// Top-level registry document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registry {
    pub schema_version: u32,
    #[serde(default, skip_serializing_if = "Settings::is_default")]
    pub settings: Settings,
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
            servers: BTreeMap::new(),
            profiles: BTreeMap::new(),
            active_profile: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    /// The client connects to the upstream server directly.
    #[default]
    Direct,
    /// The client connects to the Cerberus gateway, which proxies the upstream.
    Gateway,
}

/// A registered MCP server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
        cwd: Option<PathBuf>,
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
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
        let yaml = r#"
schema_version: 1
settings:
  default_mode: direct
  log_level: info
servers:
  github:
    enabled: true
    transport:
      type: stdio
      command: npx
      args: ["-y", "@modelcontextprotocol/server-github"]
      env:
        GITHUB_PERSONAL_ACCESS_TOKEN:
          source: secret_ref
          name: github_token
    tags: [development]
    policy:
      allow_tools: []
      deny_tools: []
profiles:
  development:
    servers: [github]
active_profile: development
"#;
        let reg: Registry = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(reg.schema_version, SCHEMA_VERSION);
        assert!(reg.servers.contains_key("github"));
        assert_eq!(reg.active_profile.as_deref(), Some("development"));
        let rendered = serde_yaml::to_string(&reg).unwrap();
        let back: Registry = serde_yaml::from_str(&rendered).unwrap();
        assert_eq!(reg, back);
    }

    #[test]
    fn rejects_unknown_fields() {
        let yaml = "schema_version: 1\nunknown_top: true\n";
        assert!(serde_yaml::from_str::<Registry>(yaml).is_err());
    }

    #[test]
    fn http_transport_parses() {
        let yaml = r#"
schema_version: 1
servers:
  remote:
    transport:
      type: http
      url: https://example.com/mcp
      headers:
        Authorization: Bearer xyz
"#;
        let reg: Registry = serde_yaml::from_str(yaml).unwrap();
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
        let yaml = r#"
schema_version: 1
servers:
  a: { transport: { type: stdio, command: echo } }
  b:
    enabled: false
    transport: { type: stdio, command: echo }
profiles:
  dev:
    servers: [a, b]
"#;
        let reg: Registry = serde_yaml::from_str(yaml).unwrap();
        let resolved = reg.resolve_profile_servers(Some("dev")).unwrap();
        assert_eq!(resolved, vec!["a".to_string()]);
    }
}
