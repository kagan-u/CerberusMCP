//! Client for the official MCP Registry, backing `cerberus search` and
//! `cerberus install`.
//!
//! API surface (verified against `registry.modelcontextprotocol.io`, Oct 2026):
//!
//! ```text
//! GET {base}/servers?search=<query>&limit=<n>&cursor=<cursor>
//! → {"servers":[{"server":{...}}],"metadata":{"nextCursor":"...","count":n}}
//! ```
//!
//! Two shapes of the payload matter here and are easy to get wrong:
//!
//! * The same server can appear several times, once per published version,
//!   so results are deduplicated keeping the newest version.
//! * A server can publish several packages for one identifier that differ
//!   only in transport (`stdio`, `sse`, `streamable-http`). Only a stdio
//!   package can be launched as a child process, so it is preferred.
//!
//! Nothing here is read by the gateway or by the client adapters: it exists
//! to turn a registry record into a `ServerDef` plus its `source` provenance.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::{CerberusError, Result};
use crate::registry::schema::{EnvValue, SourceDef, SourceEnv, SourceKind, Transport};

/// Default registry endpoint.
pub const DEFAULT_REGISTRY_BASE: &str = "https://registry.modelcontextprotocol.io/v0";

const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

/// One `server` object as published by the registry.
///
/// Unknown fields are ignored on purpose: the registry schema evolves
/// independently of this build.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteServer {
    /// Reverse-DNS identifier, e.g. `com.modelcontextprotocol/server-filesystem`.
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub repository: Option<RemoteRepository>,
    #[serde(default)]
    pub packages: Vec<RemotePackage>,
    #[serde(default)]
    pub remotes: Vec<RemoteEndpoint>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteRepository {
    pub url: String,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub subfolder: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemotePackage {
    /// Packaging system: `npm`, `pypi`, `nuget`, `mcpb`, `docker`, ...
    pub registry_type: String,
    #[serde(default)]
    pub registry_base_url: Option<String>,
    pub identifier: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub runtime_hint: Option<String>,
    /// Transport the launched process speaks. Absent means stdio.
    #[serde(default)]
    pub transport: Option<RemoteTransport>,
    #[serde(default)]
    pub runtime_arguments: Vec<RemoteArgument>,
    #[serde(default)]
    pub package_arguments: Vec<RemoteArgument>,
    #[serde(default)]
    pub environment_variables: Vec<RemoteEnvVar>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteTransport {
    /// `stdio`, `sse`, `streamable-http`, ...
    #[serde(rename = "type")]
    pub transport_type: String,
    #[serde(default)]
    pub url: Option<String>,
}

/// One command-line argument. Named arguments carry `name` (and optionally
/// `default`); positional arguments carry `value`. Neither is guaranteed to
/// be present, which is why both are optional here.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteArgument {
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    /// `positional` or `named`.
    #[serde(rename = "type", default)]
    pub arg_type: Option<String>,
    #[serde(default)]
    pub default: Option<String>,
    #[serde(default)]
    pub env: Option<String>,
    #[serde(default)]
    pub is_required: Option<bool>,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteEnvVar {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub is_required: bool,
    #[serde(default)]
    pub is_secret: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteEndpoint {
    /// `streamable-http` or `sse`.
    #[serde(rename = "type")]
    pub endpoint_type: String,
    pub url: String,
}

/// Envelope of a search page. Both the payload and the pagination cursor are
/// captured so `search` can page through more results.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SearchPage {
    #[serde(default)]
    pub servers: Vec<SearchHit>,
    #[serde(default)]
    pub metadata: Option<SearchMetadata>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SearchHit {
    pub server: RemoteServer,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchMetadata {
    #[serde(default)]
    pub next_cursor: Option<String>,
    #[serde(default)]
    pub count: Option<usize>,
}

impl SearchPage {
    pub fn into_servers(self) -> Vec<RemoteServer> {
        self.servers.into_iter().map(|hit| hit.server).collect()
    }
}

/// A registry record ready to be written into the local registry.
#[derive(Debug, Clone, PartialEq)]
pub struct PlannedInstall {
    pub transport: Transport,
    pub source: Option<SourceDef>,
    pub description: Option<String>,
    /// Non-fatal observations worth showing the user (unset variables,
    /// arguments that must be supplied manually, ...).
    pub warnings: Vec<String>,
}

/// HTTP client for one registry base URL.
#[derive(Debug, Clone)]
pub struct RegistryClient {
    base: String,
    http: reqwest::Client,
}

impl RegistryClient {
    /// Build a client from an explicit `--registry` value, the
    /// `CERBERUS_REGISTRY_URL` environment variable, or the default.
    pub fn new(explicit: Option<String>) -> Result<Self> {
        let base = explicit
            .or_else(|| std::env::var("CERBERUS_REGISTRY_URL").ok())
            .unwrap_or_else(|| DEFAULT_REGISTRY_BASE.to_string());
        let parsed = url::Url::parse(&base).map_err(|e| {
            CerberusError::Validation(format!("registry base URL `{base}` is invalid: {e}"))
        })?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(CerberusError::Validation(format!(
                "registry base URL `{base}`: scheme must be http or https"
            )));
        }
        let http = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .user_agent(concat!("cerberus-mcp/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| CerberusError::Other(format!("failed to build HTTP client: {e}")))?;
        Ok(Self {
            base: base.trim_end_matches('/').to_string(),
            http,
        })
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    /// Run one search page. `cursor` continues a previous page.
    pub async fn search(
        &self,
        query: &str,
        limit: usize,
        cursor: Option<&str>,
    ) -> Result<SearchPage> {
        let mut url = format!(
            "{}/servers?search={}&limit={}",
            self.base,
            encode(query),
            limit.clamp(1, 100)
        );
        if let Some(cursor) = cursor {
            url.push_str("&cursor=");
            url.push_str(&encode(cursor));
        }

        let response = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| map_reqwest(e, &url))?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let detail: String = body.chars().take(300).collect();
            return Err(CerberusError::RegistryRequest {
                url,
                detail: format!("HTTP {status}: {detail}"),
            });
        }
        let page: SearchPage =
            response
                .json()
                .await
                .map_err(|e| CerberusError::RegistryRequest {
                    url: url.clone(),
                    detail: format!("invalid JSON response: {e}"),
                })?;
        Ok(page)
    }

    /// Fetch one record by exact registry name. The registry exposes no
    /// single-record endpoint, so this searches for the name and requires an
    /// exact match.
    pub async fn fetch_exact(&self, name: &str) -> Result<RemoteServer> {
        let page = self.search(name, 25, None).await?;
        let hits = dedup_latest(page.into_servers());
        if let Some(exact) = hits.iter().find(|s| s.name == name) {
            return Ok(exact.clone());
        }
        if hits.len() == 1 {
            // A single near-match is unambiguous enough to install.
            return Ok(hits.into_iter().next().expect("one hit"));
        }
        let suggestions = hits
            .iter()
            .take(3)
            .map(|s| format!("  {}", s.name))
            .collect::<Vec<_>>()
            .join("\n");
        let detail = if suggestions.is_empty() {
            String::new()
        } else {
            format!("; closest matches:\n{suggestions}")
        };
        Err(CerberusError::NotFound(format!(
            "no server named `{name}` in the registry{detail}"
        )))
    }
}

fn map_reqwest(e: reqwest::Error, url: &str) -> CerberusError {
    if e.is_timeout() {
        CerberusError::Timeout {
            timeout_ms: REQUEST_TIMEOUT.as_millis() as u64,
        }
    } else {
        CerberusError::RegistryRequest {
            url: url.to_string(),
            detail: e.without_url().to_string(),
        }
    }
}

/// Percent-encode a query value (registry names contain `/`).
fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            b'/' => out.push_str("%2F"),
            b' ' => out.push_str("%20"),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// deduplication
// ---------------------------------------------------------------------------

/// Keep one record per name, preferring the newest published version.
///
/// The registry returns one row per published version, so a plain search
/// shows the same server five times without this.
pub fn dedup_latest(records: Vec<RemoteServer>) -> Vec<RemoteServer> {
    let mut order: Vec<String> = Vec::new();
    let mut best: BTreeMap<String, RemoteServer> = BTreeMap::new();
    for record in records {
        match best.get_mut(&record.name) {
            None => {
                order.push(record.name.clone());
                best.insert(record.name.clone(), record);
            }
            Some(existing) => {
                if version_rank(&record.version) > version_rank(&existing.version) {
                    *existing = record;
                }
            }
        }
    }
    order
        .into_iter()
        .filter_map(|name| best.remove(&name))
        .collect()
}

/// Rank a version string so `1.2.10` beats `1.2.9` and `1.10` beats `1.9`.
/// Unparseable values (e.g. `latest`) sort above nothing but below numbers,
/// since the registry already resolves them for us.
fn version_rank(version: &Option<String>) -> Vec<i64> {
    let Some(version) = version else {
        return vec![];
    };
    let core = version.split(['-', '+']).next().unwrap_or(version);
    let mut ranks: Vec<i64> = core
        .split('.')
        .map(|part| part.trim().parse::<i64>().unwrap_or(-1))
        .collect();
    if ranks.iter().all(|&r| r < 0) {
        return vec![];
    }
    while ranks.last() == Some(&0) {
        ranks.pop();
    }
    ranks
}

// ---------------------------------------------------------------------------
// materialization
// ---------------------------------------------------------------------------

/// Turn a registry record into a registry entry.
///
/// Preference order: stdio package → hosted endpoint → non-stdio package
/// (with a warning) → actionable error. Nothing is guessed silently: an
/// unsupported packaging system is reported instead of installed with a
/// launcher this build cannot verify.
pub fn materialize(record: &RemoteServer) -> Result<PlannedInstall> {
    let mut warnings = Vec::new();

    if let Some(package) = stdio_package(&record.packages) {
        return build_local_install(record, package, warnings);
    }

    if let Some(remote) = record.remotes.first() {
        if !matches!(remote.endpoint_type.as_str(), "streamable-http" | "sse") {
            return Err(CerberusError::Unsupported(format!(
                "server `{}`: remote endpoint type `{}` is not supported \
                 (expected streamable-http or sse)",
                record.name, remote.endpoint_type
            )));
        }
        warnings.push(format!(
            "no stdio package published for `{}`; using the hosted endpoint `{}`",
            record.name, remote.url
        ));
        return Ok(PlannedInstall {
            transport: Transport::Http {
                url: remote.url.clone(),
                headers: BTreeMap::new(),
                bearer_token: None,
            },
            source: None,
            description: record.description.clone(),
            warnings,
        });
    }

    if let Some(package) = record.packages.first() {
        if !is_supported_type(&package.registry_type) {
            let found = record
                .packages
                .iter()
                .map(|p| p.registry_type.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(CerberusError::Unsupported(format!(
                "supported package types are npm and pypi; server `{}` publishes: {found}",
                record.name
            )));
        }
        // The package exists but declares a non-stdio transport. Installing
        // it as stdio is the only option with no remote endpoint; say so.
        warnings.push(format!(
            "package `{}` declares transport `{}`; installed as stdio — verify the \
             server speaks stdio on stdout (or add --url for a hosted endpoint)",
            package.identifier,
            package
                .transport
                .as_ref()
                .map(|t| t.transport_type.as_str())
                .unwrap_or("stdio")
        ));
        return build_local_install(record, package, warnings);
    }

    Err(CerberusError::Validation(format!(
        "server `{}` publishes neither a package nor a remote endpoint",
        record.name
    )))
}

/// First package this build can install and that speaks stdio.
fn stdio_package(packages: &[RemotePackage]) -> Option<&RemotePackage> {
    packages.iter().find(|p| {
        is_supported_type(&p.registry_type)
            && p.transport
                .as_ref()
                .map(|t| t.transport_type == "stdio")
                .unwrap_or(true)
    })
}

fn is_supported_type(registry_type: &str) -> bool {
    matches!(registry_type, "npm" | "pypi")
}

fn build_local_install(
    record: &RemoteServer,
    package: &RemotePackage,
    mut warnings: Vec<String>,
) -> Result<PlannedInstall> {
    let kind = match package.registry_type.as_str() {
        "npm" => SourceKind::Npm,
        "pypi" => SourceKind::Pypi,
        other => {
            return Err(CerberusError::Unsupported(format!(
                "package type `{other}` is not supported (expected npm or pypi)"
            )));
        }
    };

    // Launch line: `<runtime> <runtimeArguments> <identifier> <packageArguments>`.
    let command = package
        .runtime_hint
        .clone()
        .filter(|hint| !hint.trim().is_empty())
        .unwrap_or_else(|| match kind {
            SourceKind::Pypi => "uvx".to_string(),
            _ => "npx".to_string(),
        });

    let runtime_args = expand_arguments(
        &package.runtime_arguments,
        &format!("runtime argument for `{}`", package.identifier),
        &mut warnings,
    );
    let package_args = expand_arguments(
        &package.package_arguments,
        &format!("package argument for `{}`", package.identifier),
        &mut warnings,
    );

    let mut args = runtime_args.clone();
    args.push(package.identifier.clone());
    args.extend(package_args);

    // Only *required* variables become launch bindings: optional ones are
    // recorded on `source` so `cerberus doctor` can report them without
    // binding variables the server does not actually need.
    let mut env = BTreeMap::new();
    let mut source_env = Vec::new();
    for variable in &package.environment_variables {
        source_env.push(SourceEnv {
            name: variable.name.clone(),
            required: variable.is_required,
            secret: variable.is_secret,
            description: variable.description.clone(),
        });
        if variable.is_required && !env.contains_key(&variable.name) {
            env.insert(variable.name.clone(), EnvValue::env(variable.name.clone()));
            if std::env::var_os(&variable.name).is_none() {
                warnings.push(format!(
                    "required environment variable `{}` is not set",
                    variable.name
                ));
            }
        }
    }

    let transport = Transport::Stdio {
        command,
        args,
        env,
        cwd: None,
    };

    let source = SourceDef {
        kind,
        package: Some(package.identifier.clone()),
        version: package.version.clone(),
        homepage: record.repository.as_ref().map(|r| r.url.clone()),
        runner: package.runtime_hint.clone(),
        args: runtime_args,
        url: record.repository.as_ref().map(|r| r.url.clone()),
        git_ref: None,
        env: source_env,
    };

    Ok(PlannedInstall {
        transport,
        source: Some(source),
        description: record.description.clone(),
        warnings,
    })
}

/// Turn registry arguments into command-line tokens.
///
/// * `positional` → `value` (or `default`)
/// * `named` → `--name value` when a value or default exists
/// * `env:VAR` → nothing on the command line; the variable supplies it
///
/// Required arguments with no value and no default cannot be invented —
/// they become a warning telling the user what to pass.
fn expand_arguments(
    arguments: &[RemoteArgument],
    context: &str,
    warnings: &mut Vec<String>,
) -> Vec<String> {
    let mut out = Vec::new();
    for argument in arguments {
        if let Some(var) = &argument.env {
            warnings.push(format!(
                "{context} is supplied by environment variable `{var}`"
            ));
            continue;
        }
        let value = argument
            .value
            .clone()
            .filter(|v| !v.trim().is_empty())
            .or_else(|| argument.default.clone());
        let named = argument.arg_type.as_deref() == Some("named");
        let label = argument
            .name
            .clone()
            .or_else(|| argument.value.clone())
            .unwrap_or_default();

        if named {
            let Some(flag) = argument.name.as_ref().filter(|n| !n.is_empty()) else {
                continue;
            };
            let flag = if flag.starts_with('-') {
                flag.clone()
            } else {
                format!("--{flag}")
            };
            match value {
                Some(value) => {
                    out.push(flag);
                    out.push(value);
                }
                None if argument.is_required.unwrap_or(false) => warnings.push(format!(
                    "{context} `--{}` is required but has no value; pass it with --arg \
                     when adding the server",
                    argument.name.clone().unwrap_or_default()
                )),
                None => {}
            }
            continue;
        }

        match value {
            Some(value) => out.push(value),
            None if argument.is_required.unwrap_or(false) => warnings.push(format!(
                "{context} `{label}` is required but has no value; pass it with --arg \
                 when adding the server"
            )),
            None => {}
        }
    }
    out
}

/// Derive a local registry name from a reverse-DNS registry name.
///
/// `com.modelcontextprotocol/server-filesystem` → `server-filesystem`.
/// Returns `None` when nothing usable is left, so callers can require
/// an explicit `--name`.
pub fn derive_name(remote_name: &str) -> Option<String> {
    let has_path = remote_name.contains('/');
    let tail = remote_name.rsplit('/').next().unwrap_or(remote_name);
    // `io.github.user.My_Server` has no path segment: keep only the last
    // reverse-DNS label so the local name stays short and meaningful.
    let tail = if !has_path && tail.matches('.').count() >= 2 {
        tail.rsplit('.').next().unwrap_or(tail)
    } else {
        tail
    };

    let mut out = String::with_capacity(tail.len());
    let mut last_dash = false;
    for raw in tail.chars() {
        let c = raw.to_ascii_lowercase();
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            out.push(c);
            last_dash = false;
        } else if !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    while out.starts_with('-') {
        out.remove(0);
    }
    if out.is_empty() {
        return None;
    }
    // Names must stay within the registry's 64-character limit.
    let mut truncated: String = out.chars().take(64).collect();
    while truncated.ends_with('-') {
        truncated.pop();
    }
    if truncated.is_empty() {
        return None;
    }
    Some(truncated)
}

// ---------------------------------------------------------------------------
// offline cache
// ---------------------------------------------------------------------------

/// Last-seen search results, so `cerberus search --offline` works without a
/// network. One file under the data directory; entries are replaced per query.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct SearchCache {
    /// ISO-8601 timestamp of the write.
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub queries: BTreeMap<String, CacheEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheEntry {
    #[serde(default)]
    pub fetched_at: String,
    #[serde(default)]
    pub results: Vec<RemoteServer>,
}

impl SearchCache {
    pub fn path(data_dir: &std::path::Path) -> PathBuf {
        data_dir.join("registry-cache.json")
    }

    pub fn load(path: &std::path::Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(path).map_err(|source| CerberusError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        serde_json::from_str(&text).map_err(|e| {
            CerberusError::Validation(format!("failed to parse {}: {e}", path.display()))
        })
    }

    pub fn save(&mut self, path: &std::path::Path, query: &str, results: Vec<RemoteServer>) {
        self.updated_at = now_iso();
        self.queries.insert(
            query.to_string(),
            CacheEntry {
                fetched_at: self.updated_at.clone(),
                results,
            },
        );
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let Ok(text) = serde_json::to_string_pretty(self) else {
            return;
        };
        let _ = std::fs::write(path, text);
    }

    pub fn get(&self, query: &str) -> Option<&CacheEntry> {
        self.queries.get(query)
    }
}

fn now_iso() -> String {
    let now = time::OffsetDateTime::now_utc();
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shaped after a real record: named package arguments without `value`,
    /// a required secret environment variable, and an optional one.
    const NPM_RECORD: &str = r#"{
      "servers": [
        {
          "server": {
            "$schema": "https://static.modelcontextprotocol.io/schemas/2025-09-29/server.schema.json",
            "name": "com.example/server-filesystem",
            "description": "Read and write files",
            "repository": {"url": "https://github.com/example/fs", "source": "github"},
            "version": "0.6.2",
            "packages": [
              {
                "registryType": "npm",
                "registryBaseUrl": "https://registry.npmjs.org",
                "identifier": "@example/server-filesystem",
                "version": "0.6.2",
                "runtimeHint": "npx",
                "transport": {"type": "stdio"},
                "runtimeArguments": [{"value": "-y", "type": "positional"}],
                "packageArguments": [
                  {"value": "/data", "type": "positional"},
                  {"type": "named", "name": "allowed-directories", "isRequired": true,
                   "format": "string", "description": "Comma-separated directories"},
                  {"type": "named", "name": "port", "isRequired": true, "format": "number",
                   "default": "8089"}
                ],
                "environmentVariables": [
                  {"name": "API_KEY", "isRequired": true, "isSecret": true,
                   "description": "API key"},
                  {"name": "OPTIONAL_THING", "description": "nice to have"}
                ]
              }
            ],
            "remotes": []
          }
        }
      ],
      "metadata": {"count": 1}
    }"#;

    const REMOTE_ONLY: &str = r#"{
      "name": "com.example/hosted",
      "description": "Hosted only",
      "remotes": [{"type": "streamable-http", "url": "https://mcp.example.com/mcp"}],
      "packages": []
    }"#;

    /// Three packages for one identifier: sse, streamable-http, and stdio.
    /// The stdio one is last, so selection cannot be "first npm wins".
    const MULTI_TRANSPORT: &str = r#"{
      "name": "io.github.bytedance/mcp-server-filesystem",
      "description": "Local filesystem",
      "version": "1.0.0",
      "packages": [
        {"registryType": "npm", "identifier": "@agent-infra/mcp-server-filesystem",
         "version": "latest", "runtimeHint": "npx",
         "transport": {"type": "sse", "url": "http://127.0.0.1:{port}/sse"},
         "packageArguments": [{"type": "named", "name": "port", "isRequired": true,
                               "default": "8089"}]},
        {"registryType": "npm", "identifier": "@agent-infra/mcp-server-filesystem",
         "version": "latest", "runtimeHint": "npx",
         "transport": {"type": "streamable-http", "url": "http://127.0.0.1:{port}/mcp"}},
        {"registryType": "npm", "identifier": "@agent-infra/mcp-server-filesystem",
         "version": "latest", "runtimeHint": "npx", "transport": {"type": "stdio"},
         "packageArguments": [{"type": "named", "name": "allowed-directories",
                               "isRequired": true, "format": "string"}]}
      ],
      "remotes": []
    }"#;

    fn npm_record() -> RemoteServer {
        let page: SearchPage = serde_json::from_str(NPM_RECORD).unwrap();
        page.into_servers().remove(0)
    }

    #[test]
    fn parses_search_envelope_and_ignores_unknown_fields() {
        let record = npm_record();
        assert_eq!(record.name, "com.example/server-filesystem");
        assert_eq!(record.packages.len(), 1);
        assert_eq!(
            record.packages[0].runtime_arguments[0].value.as_deref(),
            Some("-y")
        );
        // Named arguments have no `value` field at all and still parse.
        let named = &record.packages[0].package_arguments[1];
        assert_eq!(named.name.as_deref(), Some("allowed-directories"));
        assert!(named.value.is_none());
        assert_eq!(named.is_required, Some(true));
        assert_eq!(
            record.repository.as_ref().map(|r| r.url.as_str()),
            Some("https://github.com/example/fs")
        );
        assert_eq!(
            record.remotes.len(),
            0,
            "unknown fields elsewhere are ignored"
        );
    }

    #[test]
    fn materializes_npm_package_into_stdio_and_source() {
        let planned = materialize(&npm_record()).unwrap();

        let Transport::Stdio {
            command, args, env, ..
        } = &planned.transport
        else {
            panic!("expected stdio transport");
        };
        assert_eq!(command, "npx");
        assert_eq!(
            args,
            &[
                "-y",
                "@example/server-filesystem",
                "/data",
                // named arg with a default becomes `--name value`
                "--port",
                "8089",
                // `--allowed-directories` has no value and no default: it is
                // warned about below rather than invented here.
            ]
        );

        // Only required vars become launch bindings.
        assert_eq!(env.len(), 1);
        assert!(matches!(env.get("API_KEY"), Some(EnvValue::Env { name }) if name == "API_KEY"));

        let source = planned.source.as_ref().expect("npm source");
        assert_eq!(source.kind, SourceKind::Npm);
        assert_eq!(
            source.package.as_deref(),
            Some("@example/server-filesystem")
        );
        assert_eq!(source.version.as_deref(), Some("0.6.2"));
        assert_eq!(source.runner.as_deref(), Some("npx"));
        assert_eq!(source.env.len(), 2, "optional vars are still documented");
        assert!(source.env[0].secret);
        assert!(!source.env[1].required);

        assert!(
            planned
                .warnings
                .iter()
                .any(|w| w.contains("`API_KEY` is not set")),
            "warnings: {:?}",
            planned.warnings
        );
        assert!(
            planned
                .warnings
                .iter()
                .any(|w| w.contains("--allowed-directories") && w.contains("required")),
            "warnings: {:?}",
            planned.warnings
        );
        assert_eq!(planned.description.as_deref(), Some("Read and write files"));
    }

    #[test]
    fn prefers_stdio_package_over_hosted_transport_variants() {
        let record: RemoteServer = serde_json::from_str(MULTI_TRANSPORT).unwrap();
        let planned = materialize(&record).unwrap();
        let Transport::Stdio { args, .. } = &planned.transport else {
            panic!("expected stdio transport, not {:?}", planned.transport);
        };
        // The stdio package (last in the list) is the one installed, so the
        // sse/streamable `port` argument must not leak into the command.
        assert_eq!(
            args,
            &["@agent-infra/mcp-server-filesystem".to_string()],
            "stdio package has only a required named arg, which is warned about"
        );
        assert!(
            planned
                .warnings
                .iter()
                .any(|w| w.contains("allowed-directories"))
        );
    }

    #[test]
    fn falls_back_to_hosted_endpoint_when_no_stdio_package() {
        let record: RemoteServer = serde_json::from_str(REMOTE_ONLY).unwrap();
        let planned = materialize(&record).unwrap();
        assert!(matches!(
            &planned.transport,
            Transport::Http { url, .. } if url == "https://mcp.example.com/mcp"
        ));
        assert!(planned.source.is_none());
        assert!(planned.warnings[0].contains("hosted endpoint"));
    }

    #[test]
    fn rejects_unsupported_package_type_with_exit_code_unsupported() {
        let record = RemoteServer {
            name: "com.example/nuget-only".into(),
            description: None,
            version: None,
            repository: None,
            packages: vec![RemotePackage {
                registry_type: "nuget".into(),
                registry_base_url: None,
                identifier: "Example.Nuget".into(),
                version: None,
                runtime_hint: None,
                transport: None,
                runtime_arguments: vec![],
                package_arguments: vec![],
                environment_variables: vec![],
            }],
            remotes: vec![],
        };
        let err = materialize(&record).unwrap_err().to_string();
        assert!(err.contains("npm and pypi"), "{err}");
        assert_eq!(
            materialize(&record).unwrap_err().exit_code(),
            crate::error::ExitCode::Unsupported
        );
    }

    #[test]
    fn rejects_record_without_packages_or_remotes() {
        let record = RemoteServer {
            name: "com.example/empty".into(),
            description: None,
            version: None,
            repository: None,
            packages: vec![],
            remotes: vec![],
        };
        assert!(materialize(&record).is_err());
    }

    #[test]
    fn dedup_keeps_newest_version_and_input_order() {
        let mk = |version: &str| RemoteServer {
            name: "com.example/dupe".into(),
            description: None,
            version: Some(version.into()),
            repository: None,
            packages: vec![],
            remotes: vec![],
        };
        let other = RemoteServer {
            name: "com.example/other".into(),
            description: None,
            version: Some("0.1.0".into()),
            repository: None,
            packages: vec![],
            remotes: vec![],
        };
        let out = dedup_latest(vec![mk("0.1.2"), mk("0.1.10"), other.clone(), mk("0.1.3")]);
        assert_eq!(
            out.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
            vec!["com.example/dupe", "com.example/other"],
            "first-seen order is preserved"
        );
        assert_eq!(out[0].version.as_deref(), Some("0.1.10"));
    }

    #[test]
    fn version_rank_orders_numerically_not_lexically() {
        assert!(version_rank(&Some("1.2.10".into())) > version_rank(&Some("1.2.9".into())));
        assert!(version_rank(&Some("1.10".into())) > version_rank(&Some("1.9".into())));
        assert!(version_rank(&Some("2.0.0".into())) > version_rank(&Some("1.9.9".into())));
        assert!(version_rank(&Some("1.0.0".into())) > version_rank(&None));
        assert_eq!(version_rank(&Some("latest".into())), Vec::<i64>::new());
    }

    #[test]
    fn derives_local_names_from_reverse_dns() {
        assert_eq!(
            derive_name("com.modelcontextprotocol/server-filesystem").as_deref(),
            Some("server-filesystem")
        );
        assert_eq!(
            derive_name("io.github.user.My_Server").as_deref(),
            Some("my-server")
        );
        assert_eq!(derive_name("com.example/___"), None);
        let long = derive_name(&format!("com.example/{}", "a".repeat(80))).unwrap();
        assert_eq!(long.len(), 64);
        assert!(!long.contains("__"));
    }

    #[test]
    fn cache_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = SearchCache::path(dir.path());

        let mut cache = SearchCache::load(&path).unwrap();
        assert!(cache.get("filesystem").is_none());
        cache.save(&path, "filesystem", vec![npm_record()]);

        let reloaded = SearchCache::load(&path).unwrap();
        let entry = reloaded.get("filesystem").expect("cached entry");
        assert_eq!(entry.results.len(), 1);
        assert_eq!(entry.results[0].name, "com.example/server-filesystem");
        assert!(!reloaded.updated_at.is_empty());
    }

    #[test]
    fn encodes_registry_names_in_query_strings() {
        assert_eq!(
            encode("com.example/server-filesystem"),
            "com.example%2Fserver-filesystem"
        );
        assert_eq!(encode("a b"), "a%20b");
        assert_eq!(encode("ok-._~"), "ok-._~");
    }
}
