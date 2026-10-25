//! MCP gateway.
//!
//! The gateway exposes a single MCP server (stdio or Streamable HTTP) whose
//! tool surface is assembled from registered upstream MCP servers. It:
//!
//! - connects to each enabled upstream (stdio child process or Streamable HTTP),
//! - discovers upstream tools and publishes them under `{server}__{tool}` names,
//! - enforces per-server allow/deny policies before forwarding,
//! - applies per-server timeouts and a global concurrency limit,
//! - isolates upstream failures and supports lazy reconnection,
//! - redacts secret material from error messages.
//!
//! Only the `tools` capability is advertised: resources and prompts are not
//! forwarded (documented limitation in docs/protocol-compatibility.md).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use rmcp::model::{
    CallToolRequestParams, CallToolResult, ContentBlock, ErrorData as McpError, ListToolsResult,
    MetaObject, ServerCapabilities, ServerConfig, Tool,
};
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::{
    ConfigureCommandExt, StreamableHttpClientTransport, StreamableHttpServerConfig,
    StreamableHttpService, TokioChildProcess,
};
use rmcp::{RoleServer, ServerHandler, ServiceExt, schemars, service::RunningService};
use serde::Serialize;
use tokio::sync::{Mutex, RwLock, Semaphore};

use crate::cli::output::{Ctx, bold, cyan, dim, green, red};
use crate::error::{CerberusError, Result};
use crate::paths::CerberusPaths;
use crate::policy::{self, ServerPolicyView};
use crate::process::{EnvPolicy, build_command, executable_exists};
use crate::registry::RegistryStore;
use crate::registry::schema::{ExecutionMode, ServerDef, Transport};
use crate::routing::{RoutingTable, public_tool_name, split_public_name};
use crate::secrets::{SecretStore, collect_literal_secrets, redact_secrets};

const DEFAULT_CALL_TIMEOUT_MS: u64 = 60_000;
const DEFAULT_STARTUP_TIMEOUT_MS: u64 = 30_000;
const DEFAULT_MAX_CONCURRENCY: usize = 32;
const SEPARATOR_NOTE: &str = "tools are named {server}__{tool}";

/// Reverse-DNS `_meta` key under which the gateway publishes tool routing.
///
/// This is a wire contract with the Cerberus SDKs: they read `server` and
/// `tool` from here instead of splitting the public name themselves.
const ROUTE_META_KEY: &str = "cerberus.dev/route";

/// Routing metadata attached to every entry in `tools/list`.
///
/// A tool's own name may contain the `__` separator, so parsing the public
/// name is only ever a fallback; this structure is authoritative.
fn route_meta(server: &str, tool: &str) -> MetaObject {
    let mut route = rmcp::model::JsonObject::new();
    route.insert("server".into(), server.into());
    route.insert("tool".into(), tool.into());
    let mut meta = rmcp::model::JsonObject::new();
    meta.insert(ROUTE_META_KEY.into(), serde_json::Value::Object(route));
    MetaObject(meta)
}

// ---------------------------------------------------------------------------
// Structured error contract
// ---------------------------------------------------------------------------

/// Machine-readable failure codes emitted by the gateway on `tools/call`.
///
/// These are the SDK-facing half of the gateway contract. Every error result
/// carries this `code` inside `structuredContent` alongside a human-readable
/// message in `content`, so client libraries branch on a stable enum instead
/// of parsing prose. Codes align with [`crate::error::ExitCode`] values that
/// the CLI reports for the same condition, and with `retryable`, which tells
/// an SDK whether reconnecting or waiting can help.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewayErrorCode {
    /// The public tool name could not be split into `server` and `tool`.
    UnknownTool,
    /// No such server is routed by this gateway (or not in the active profile).
    ServerNotFound,
    /// Server policy (allow/deny lists) rejected the call.
    PolicyDenied,
    /// The server requires an interactive confirmation this build can't do.
    ConfirmationRequired,
    /// The upstream server is unreachable, not connected, or failed to start.
    UpstreamUnavailable,
    /// The gateway is shutting down.
    ShuttingDown,
    /// The concurrency semaphore was saturated within the timeout.
    ConcurrencySaturated,
    /// The upstream did not answer within the call timeout.
    Timeout,
    /// The upstream answered, but with an error.
    UpstreamError,
}

impl GatewayErrorCode {
    /// Stable snake_case spelling sent on the wire.
    pub fn as_str(&self) -> &'static str {
        match self {
            GatewayErrorCode::UnknownTool => "unknown_tool",
            GatewayErrorCode::ServerNotFound => "server_not_found",
            GatewayErrorCode::PolicyDenied => "policy_denied",
            GatewayErrorCode::ConfirmationRequired => "confirmation_required",
            GatewayErrorCode::UpstreamUnavailable => "upstream_unavailable",
            GatewayErrorCode::ShuttingDown => "shutting_down",
            GatewayErrorCode::ConcurrencySaturated => "concurrency_saturated",
            GatewayErrorCode::Timeout => "timeout",
            GatewayErrorCode::UpstreamError => "upstream_error",
        }
    }

    /// Whether an SDK may usefully retry the identical call.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            GatewayErrorCode::UpstreamUnavailable
                | GatewayErrorCode::ShuttingDown
                | GatewayErrorCode::ConcurrencySaturated
        )
    }

    /// The CLI exit code the same condition maps to.
    ///
    /// Keeps the wire contract and the process contract from drifting: a
    /// `policy_denied` tool error and `exit 6` from the CLI are the same
    /// condition seen from two sides.
    pub fn exit_code(&self) -> crate::error::ExitCode {
        use crate::error::ExitCode as E;
        use GatewayErrorCode::*;
        match self {
            UnknownTool | ServerNotFound => E::NotFound,
            PolicyDenied | ConfirmationRequired => E::PolicyDenied,
            Timeout | ConcurrencySaturated => E::Timeout,
            UpstreamUnavailable | ShuttingDown | UpstreamError => E::Failure,
        }
    }
}

/// Build a tool-level error carrying both a human message and structured data.
///
/// `content` stays prose so MCP clients render something readable;
/// `structured_content` carries `code`, `message`, `retryable`, `exit_code`
/// and, when known, `server` / `tool` / `reason` for SDK branching.
fn tool_error(
    code: GatewayErrorCode,
    message: impl Into<String>,
    fields: serde_json::Map<String, serde_json::Value>,
) -> rmcp::model::CallToolResponse {
    let message = message.into();
    let mut payload = serde_json::Map::new();
    payload.insert(
        "code".into(),
        serde_json::Value::String(code.as_str().into()),
    );
    payload.insert("message".into(), serde_json::Value::String(message.clone()));
    payload.insert(
        "retryable".into(),
        serde_json::Value::Bool(code.is_retryable()),
    );
    payload.insert(
        "exit_code".into(),
        serde_json::Value::from(code.exit_code() as u8),
    );
    for (k, v) in fields {
        payload.entry(k).or_insert(v);
    }

    let mut result = CallToolResult::error(vec![ContentBlock::text(message)]);
    result.structured_content = Some(serde_json::Value::Object(payload));
    result.into()
}

/// Convenience: `tool_error` with only server/tool context.
fn tool_error_for(
    code: GatewayErrorCode,
    message: impl Into<String>,
    server: Option<&str>,
    tool: Option<&str>,
) -> rmcp::model::CallToolResponse {
    let mut extra = serde_json::Map::new();
    if let Some(s) = server {
        extra.insert("server".into(), s.into());
    }
    if let Some(t) = tool {
        extra.insert("tool".into(), t.into());
    }
    tool_error(code, message, extra)
}

// ---------------------------------------------------------------------------
// Upstream management
// ---------------------------------------------------------------------------

type UpstreamService = RunningService<rmcp::RoleClient, ()>;

pub struct Upstream {
    server: String,
    def: ServerDef,
    service: Option<UpstreamService>,
    /// Tools discovered on this upstream, as (public_name, original, description).
    tools: Vec<DiscoveredTool>,
    last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiscoveredTool {
    pub public_name: String,
    pub original_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

pub struct GatewayState {
    pub paths: CerberusPaths,
    pub secrets: SecretStore,
    pub upstreams: Mutex<HashMap<String, Upstream>>,
    pub routing: RwLock<RoutingTable>,
    pub policies: RwLock<HashMap<String, ServerPolicyView>>,
    pub confirmations: RwLock<HashMap<String, bool>>,
    pub call_timeout: Duration,
    pub startup_timeout: Duration,
    pub semaphore: Arc<Semaphore>,
    pub redact: Vec<String>,
}

impl GatewayState {
    async fn connect_upstream(
        server: &str,
        def: &ServerDef,
        secrets: &SecretStore,
    ) -> Result<UpstreamService> {
        match &def.transport {
            Transport::Stdio { .. } => {
                if !executable_exists(match &def.transport {
                    Transport::Stdio { command, .. } => command.as_str(),
                    _ => unreachable!(),
                }) {
                    return Err(CerberusError::Upstream {
                        server: server.to_string(),
                        detail: format!(
                            "executable `{}` not found on PATH",
                            match &def.transport {
                                Transport::Stdio { command, .. } => command.as_str(),
                                _ => unreachable!(),
                            }
                        ),
                    });
                }
                let cmd = build_command(def, secrets, &EnvPolicy::default())?;
                let transport = TokioChildProcess::new(cmd.configure(|_| {})).map_err(|e| {
                    CerberusError::Upstream {
                        server: server.to_string(),
                        detail: format!("spawn failed: {e}"),
                    }
                })?;
                let service =
                    ().serve(transport)
                        .await
                        .map_err(|e| CerberusError::Upstream {
                            server: server.to_string(),
                            detail: format!("MCP initialize failed: {e}"),
                        })?;
                Ok(service)
            }
            Transport::Http {
                url,
                headers,
                bearer_token,
            } => {
                let mut config = StreamableHttpClientTransportConfig::with_uri(url.clone());
                for (k, v) in headers {
                    let value = match v {
                        crate::registry::schema::HeaderValue::Resolved(ev) => {
                            secrets.resolve_env_value(ev)?
                        }
                        crate::registry::schema::HeaderValue::Literal(s) => s.clone(),
                    };
                    if let (Ok(name), Ok(val)) = (
                        k.parse::<http::HeaderName>(),
                        value.parse::<http::HeaderValue>(),
                    ) {
                        config.custom_headers.insert(name, val);
                    }
                }
                if let Some(token) = bearer_token {
                    let value = secrets.resolve_env_value(token)?;
                    config.auth_header = Some(format!("Bearer {value}"));
                }
                let transport = StreamableHttpClientTransport::from_config(config);
                let service =
                    ().serve(transport)
                        .await
                        .map_err(|e| CerberusError::Upstream {
                            server: server.to_string(),
                            detail: format!("MCP initialize failed: {e}"),
                        })?;
                Ok(service)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Gateway handler (downstream server)
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct GatewayHandler {
    state: Arc<GatewayState>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct GatewayCallParams {
    /// Public tool name (`{server}__{tool}`). The actual arguments are
    /// forwarded verbatim from the request.
    #[allow(dead_code)]
    pub _placeholder: Option<bool>,
}

impl GatewayHandler {
    pub fn new(state: Arc<GatewayState>) -> Self {
        Self { state }
    }

    async fn ensure_connected(&self, server: &str) -> Result<()> {
        let mut upstreams = self.state.upstreams.lock().await;
        let needs_connect = match upstreams.get(server) {
            Some(u) => u.service.is_none(),
            None => {
                return Err(CerberusError::ServerNotFound {
                    name: server.to_string(),
                });
            }
        };
        if !needs_connect {
            return Ok(());
        }
        let def = upstreams
            .get(server)
            .map(|u| u.def.clone())
            .ok_or_else(|| CerberusError::ServerNotFound {
                name: server.to_string(),
            })?;
        match GatewayState::connect_upstream(server, &def, &self.state.secrets).await {
            Ok(service) => {
                let upstream = upstreams.get_mut(server).expect("checked above");
                // Re-discover tools.
                let tools = match service.list_all_tools().await {
                    Ok(tools) => tools
                        .into_iter()
                        .map(|t| DiscoveredTool {
                            public_name: public_tool_name(server, t.name.as_ref()),
                            original_name: t.name.to_string(),
                            description: t.description.as_ref().map(|d| d.to_string()),
                        })
                        .collect::<Vec<_>>(),
                    Err(e) => {
                        upstream.last_error = Some(format!("tool discovery failed: {e}"));
                        Vec::new()
                    }
                };
                let mut table = self.state.routing.write().await;
                for t in &tools {
                    let _ = table.insert(server, &t.original_name);
                }
                upstream.service = Some(service);
                upstream.tools = tools;
                upstream.last_error = None;
                Ok(())
            }
            Err(e) => {
                if let Some(u) = upstreams.get_mut(server) {
                    u.last_error = Some(e.to_string());
                }
                Err(e)
            }
        }
    }

    async fn refresh_routes(&self) {
        let upstreams = self.state.upstreams.lock().await;
        let policies = self.state.policies.read().await;
        let mut table = RoutingTable::new();
        for (name, upstream) in upstreams.iter() {
            if upstream.service.is_none() {
                continue;
            }
            let allowed_tools: Vec<String> = upstream
                .tools
                .iter()
                .filter(|t| {
                    policies
                        .get(name)
                        .map(|p| policy::evaluate(p, &t.original_name).is_allow())
                        .unwrap_or(false)
                })
                .map(|t| t.original_name.clone())
                .collect();
            for tool in allowed_tools {
                let _ = table.insert(name, &tool);
            }
        }
        *self.state.routing.write().await = table;
    }

    fn redact(&self, text: &str) -> String {
        redact_secrets(text, &self.state.redact)
    }
}

impl ServerHandler for GatewayHandler {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_tool_list_changed()
                .build(),
        )
        .with_instructions(format!(
            "CerberusMCP gateway. {SEPARATOR_NOTE}. Each tool also carries \
             `_meta.{ROUTE_META_KEY}` with {{server, tool}} for clients that prefer \
             structured routing. Policies are enforced before any request reaches an \
             upstream server."
        ))
    }

    async fn list_tools(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: rmcp::service::RequestContext<RoleServer>,
    ) -> std::result::Result<ListToolsResult, McpError> {
        self.refresh_routes().await;
        let upstreams = self.state.upstreams.lock().await;
        let table = self.state.routing.read().await;

        let mut tools = Vec::new();
        for route in table.all() {
            let Some(upstream) = upstreams.get(&route.server) else {
                continue;
            };
            let Some(discovered) = upstream
                .tools
                .iter()
                .find(|t| t.original_name == route.tool)
            else {
                continue;
            };
            let mut description = discovered
                .description
                .clone()
                .unwrap_or_else(|| format!("Tool `{}` from server `{}`", route.tool, route.server));
            description = format!("[{}] {}", route.server, description);
            let mut tool = Tool::new(
                route.public_name.clone(),
                description,
                upstream_input_schema(upstream, &route.tool),
            );
            tool.meta = Some(route_meta(&route.server, &route.tool));
            tools.push(tool);
        }
        Ok(ListToolsResult::with_all_items(tools))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: rmcp::service::RequestContext<RoleServer>,
    ) -> std::result::Result<rmcp::model::CallToolResponse, McpError> {
        let public_name = request.name.to_string();
        let start = std::time::Instant::now();
        let correlation = uuid::Uuid::new_v4().to_string();

        let Some((server, tool)) = split_public_name(&public_name) else {
            let mut extra = serde_json::Map::new();
            extra.insert("tool".into(), public_name.as_str().into());
            return Ok(tool_error(
                GatewayErrorCode::UnknownTool,
                format!("unknown tool `{public_name}` ({SEPARATOR_NOTE})"),
                extra,
            ));
        };
        let (server, tool) = (server.to_string(), tool.to_string());

        // Policy check before any upstream work.
        {
            let policies = self.state.policies.read().await;
            let Some(p) = policies.get(&server) else {
                return Ok(tool_error_for(
                    GatewayErrorCode::ServerNotFound,
                    format!("server `{server}` is not available through this gateway"),
                    Some(&server),
                    Some(&tool),
                ));
            };
            if let policy::Decision::Deny { reason } = policy::evaluate(p, &tool) {
                tracing::warn!(%correlation, %server, %tool, %reason, "policy denied");
                let mut extra = serde_json::Map::new();
                extra.insert("server".into(), server.as_str().into());
                extra.insert("tool".into(), tool.as_str().into());
                extra.insert("reason".into(), reason.as_str().into());
                return Ok(tool_error(
                    GatewayErrorCode::PolicyDenied,
                    format!("policy denied: {reason}"),
                    extra,
                ));
            }
        }

        // Confirmation gate for high-impact servers.
        {
            let confirmations = self.state.confirmations.read().await;
            if confirmations.get(&server).copied().unwrap_or(false) {
                return Ok(tool_error_for(
                    GatewayErrorCode::ConfirmationRequired,
                    format!(
                        "server `{server}` requires confirmation for tool calls; this gateway \
                         build does not implement an interactive confirmation channel — call \
                         the tool directly or relax `require_confirmation` in the registry"
                    ),
                    Some(&server),
                    Some(&tool),
                ));
            }
        }

        // Connect (lazily reconnect) and forward.
        if let Err(e) = self.ensure_connected(&server).await {
            let msg = self.redact(&e.to_string());
            tracing::error!(%correlation, %server, error = %msg, "upstream unavailable");
            let mut extra = serde_json::Map::new();
            extra.insert("server".into(), server.as_str().into());
            extra.insert("detail".into(), msg.as_str().into());
            return Ok(tool_error(
                GatewayErrorCode::UpstreamUnavailable,
                format!("upstream `{server}` unavailable: {msg}"),
                extra,
            ));
        }

        let _permit = tokio::select! {
            permit = self.state.semaphore.clone().acquire_owned() => match permit {
                Ok(p) => p,
                Err(_) => {
                    return Ok(tool_error(
                        GatewayErrorCode::ShuttingDown,
                        "gateway is shutting down",
                        serde_json::Map::new(),
                    ));
                }
            },
            _ = tokio::time::sleep(self.state.call_timeout) => {
                return Ok(tool_error(
                    GatewayErrorCode::ConcurrencySaturated,
                    "gateway concurrency limit saturated; retry shortly",
                    serde_json::Map::new(),
                ));
            }
        };

        let upstreams = self.state.upstreams.lock().await;
        let upstream = upstreams.get(&server);
        let service = upstream.and_then(|u| u.service.as_ref());
        let Some(service) = service else {
            return Ok(tool_error_for(
                GatewayErrorCode::UpstreamUnavailable,
                format!("server `{server}` is not connected"),
                Some(&server),
                Some(&tool),
            ));
        };

        let params = CallToolRequestParams::new(tool.clone())
            .with_arguments(request.arguments.clone().unwrap_or_default());

        let timeout = self.state.call_timeout;
        let result = tokio::time::timeout(timeout, service.call_tool(params)).await;

        let elapsed = start.elapsed();
        match result {
            Err(_) => {
                tracing::warn!(%correlation, %server, %tool, elapsed_ms = elapsed.as_millis() as u64, "timeout");
                let mut extra = serde_json::Map::new();
                extra.insert("server".into(), server.as_str().into());
                extra.insert("tool".into(), tool.as_str().into());
                extra.insert(
                    "timeout_ms".into(),
                    serde_json::Value::from(timeout.as_millis() as u64),
                );
                Ok(tool_error(
                    GatewayErrorCode::Timeout,
                    self.redact(&format!(
                        "tool `{tool}` on server `{server}` timed out after {} ms",
                        timeout.as_millis()
                    )),
                    extra,
                ))
            }
            Ok(Err(e)) => {
                let msg = self.redact(&e.to_string());
                tracing::warn!(%correlation, %server, %tool, elapsed_ms = elapsed.as_millis() as u64, error = %msg, "upstream error");
                let mut extra = serde_json::Map::new();
                extra.insert("server".into(), server.as_str().into());
                extra.insert("tool".into(), tool.as_str().into());
                extra.insert("detail".into(), msg.as_str().into());
                Ok(tool_error(
                    GatewayErrorCode::UpstreamError,
                    format!("upstream error from `{server}`: {msg}"),
                    extra,
                ))
            }
            Ok(Ok(mut result)) => {
                tracing::info!(%correlation, %server, %tool, elapsed_ms = elapsed.as_millis() as u64, "ok");
                // The upstream session may be on a legacy protocol version,
                // where rmcp strips `resultType` from results. Clients on
                // 2026-07-28 (e.g. mcp-py 2.3's default "auto" mode) require
                // the field, and rmcp re-strips it for legacy peers after this
                // handler returns, so assert it unconditionally here.
                if result.result_type.is_none() {
                    result.result_type = Some(rmcp::model::ResultType::COMPLETE);
                }
                Ok(result.into())
            }
        }
    }
}

fn upstream_input_schema(upstream: &Upstream, tool: &str) -> Arc<rmcp::model::JsonObject> {
    // Best effort: reuse the schema we captured at discovery time. The
    // discovery path stores only names/descriptions; fall back to an empty
    // object schema (valid per MCP: inputSchema is required but may be {}).
    let _ = (upstream, tool);
    static EMPTY: std::sync::OnceLock<Arc<rmcp::model::JsonObject>> = std::sync::OnceLock::new();
    EMPTY
        .get_or_init(|| {
            let mut map = serde_json::Map::new();
            map.insert("type".to_string(), serde_json::json!("object"));
            Arc::new(map)
        })
        .clone()
}

// ---------------------------------------------------------------------------
// Runtime
// ---------------------------------------------------------------------------

/// Connect all upstreams from the registry and build initial routing.
pub async fn build_state(
    paths: &CerberusPaths,
    store: &RegistryStore,
    secrets: SecretStore,
    profile: Option<&str>,
) -> Result<Arc<GatewayState>> {
    let selected = crate::sync::selected_servers(store, profile)?;
    let redact = collect_literal_secrets(&store.registry);

    let state = Arc::new(GatewayState {
        paths: paths.clone(),
        secrets,
        upstreams: Mutex::new(HashMap::new()),
        routing: RwLock::new(RoutingTable::new()),
        policies: RwLock::new(HashMap::new()),
        confirmations: RwLock::new(HashMap::new()),
        call_timeout: Duration::from_millis(DEFAULT_CALL_TIMEOUT_MS),
        startup_timeout: Duration::from_millis(DEFAULT_STARTUP_TIMEOUT_MS),
        semaphore: Arc::new(Semaphore::new(DEFAULT_MAX_CONCURRENCY)),
        redact,
    });

    // Register policies and empty upstream slots first so failures are
    // isolated per server.
    {
        let mut upstreams = state.upstreams.lock().await;
        let mut policies = state.policies.write().await;
        let mut confirmations = state.confirmations.write().await;
        for (name, def) in &selected {
            upstreams.insert(
                name.clone(),
                Upstream {
                    server: name.clone(),
                    def: def.clone(),
                    service: None,
                    tools: Vec::new(),
                    last_error: None,
                },
            );
            policies.insert(name.clone(), ServerPolicyView::from(&def.policy));
            confirmations.insert(name.clone(), def.policy.require_confirmation);
        }
    }

    // Connect with bounded parallelism.
    let names: Vec<String> = selected.keys().cloned().collect();
    let connect_sema = Arc::new(Semaphore::new(4));
    let mut handles = Vec::new();
    for name in names {
        let state = state.clone();
        let sema = connect_sema.clone();
        handles.push(tokio::spawn(async move {
            let _permit = sema.acquire().await.ok();
            let def = {
                let upstreams = state.upstreams.lock().await;
                upstreams.get(&name).map(|u| u.def.clone())
            };
            let Some(def) = def else { return };
            match GatewayState::connect_upstream(&name, &def, &state.secrets).await {
                Ok(service) => {
                    let tools = match service.list_all_tools().await {
                        Ok(tools) => tools
                            .into_iter()
                            .map(|t| DiscoveredTool {
                                public_name: public_tool_name(&name, t.name.as_ref()),
                                original_name: t.name.to_string(),
                                description: t.description.as_ref().map(|d| d.to_string()),
                            })
                            .collect(),
                        Err(e) => {
                            tracing::warn!(server = %name, error = %e, "tool discovery failed");
                            Vec::new()
                        }
                    };
                    let mut upstreams = state.upstreams.lock().await;
                    if let Some(u) = upstreams.get_mut(&name) {
                        u.service = Some(service);
                        u.tools = tools;
                        u.last_error = None;
                    }
                }
                Err(e) => {
                    tracing::warn!(server = %name, error = %e, "upstream connect failed");
                    let mut upstreams = state.upstreams.lock().await;
                    if let Some(u) = upstreams.get_mut(&name) {
                        u.last_error = Some(e.to_string());
                    }
                }
            }
        }));
    }
    for h in handles {
        let _ = h.await;
    }

    // Build the initial routing table.
    {
        let upstreams = state.upstreams.lock().await;
        let policies = state.policies.read().await;
        let mut table = RoutingTable::new();
        for (name, upstream) in upstreams.iter() {
            for t in &upstream.tools {
                let permitted = policies
                    .get(name)
                    .map(|p| policy::evaluate(p, &t.original_name).is_allow())
                    .unwrap_or(false);
                if permitted {
                    if let Err(e) = table.insert(name, &t.original_name) {
                        tracing::error!(error = %e, "routing collision");
                        return Err(CerberusError::Gateway(e));
                    }
                }
            }
        }
        *state.routing.write().await = table;
    }

    Ok(state)
}

pub struct GatewayStatusSnapshot {
    pub transport: String,
    pub bind: Option<String>,
    pub pid: u32,
    pub upstreams: Vec<UpstreamStatus>,
    pub tools: usize,
}

#[derive(Serialize)]
pub struct UpstreamStatus {
    pub name: String,
    pub transport: String,
    pub connected: bool,
    pub tools: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

pub async fn status_snapshot(
    state: &GatewayState,
    transport: &str,
    bind: Option<&str>,
) -> GatewayStatusSnapshot {
    let upstreams = state.upstreams.lock().await;
    let routing = state.routing.read().await;
    GatewayStatusSnapshot {
        transport: transport.to_string(),
        bind: bind.map(str::to_string),
        pid: std::process::id(),
        upstreams: upstreams
            .values()
            .map(|u| UpstreamStatus {
                name: u.server.clone(),
                transport: u.def.transport.kind().to_string(),
                connected: u.service.is_some(),
                tools: u.tools.len(),
                last_error: u
                    .last_error
                    .clone()
                    .map(|e| redact_secrets(&e, &state.redact)),
            })
            .collect(),
        tools: routing.len(),
    }
}

fn write_state_file(paths: &CerberusPaths, snapshot: &GatewayStatusSnapshot) -> Result<()> {
    let text = serde_json::json!({
        "pid": snapshot.pid,
        "transport": snapshot.transport,
        "bind": snapshot.bind,
        "started_unix": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    });
    crate::registry::atomic_write(
        &paths.gateway_state_file(),
        serde_json::to_string_pretty(&text)
            .map_err(|e| CerberusError::Other(e.to_string()))?
            .as_bytes(),
    )
}

/// Run the gateway in the foreground until ctrl-c.
pub async fn run_foreground(
    ctx: &Ctx,
    store: &RegistryStore,
    transport: &str,
    bind: &str,
    allow_remote: bool,
    token: Option<&str>,
    profile: Option<&str>,
) -> Result<i32> {
    // Network exposure guard.
    let is_loopback =
        bind.starts_with("127.0.0.1") || bind.starts_with("localhost") || bind.starts_with("[::1]");
    if !is_loopback {
        if !allow_remote {
            return Err(CerberusError::Gateway(format!(
                "refusing to bind {bind}: non-loopback binds require --allow-remote"
            )));
        }
        if token.is_none() {
            return Err(CerberusError::Gateway(
                "non-loopback binds require an auth token (--token or CERBERUS_GATEWAY_TOKEN)"
                    .into(),
            ));
        }
        ctx.warn(&format!(
            "gateway will be reachable from the network at {bind}; ensure a firewall or TLS \
             terminator is in front of it"
        ));
    }

    let state = build_state(&ctx.paths, store, SecretStore::default(), profile).await?;

    {
        let upstreams = state.upstreams.lock().await;
        // `stdio` reserves stdout for MCP JSON-RPC: the banner goes to stderr
        // there, so strict clients (e.g. the Python SDK) never see prose.
        let stdio_mode = transport == "stdio";
        let line = |text: &str| {
            if ctx.json || ctx.quiet {
                return;
            }
            if stdio_mode {
                eprintln!("{text}");
            } else {
                println!("{text}");
            }
        };
        let status = |label: &str, value: &str| {
            if ctx.json || ctx.quiet {
                return;
            }
            let width: usize = 22;
            let dots = width.saturating_sub(label.chars().count());
            line(&format!("  {label} {} {}", ".".repeat(dots), value));
        };
        line(&bold("CerberusMCP Gateway"));
        status("Transport", transport);
        if transport == "http" {
            status("Bind", bind);
        }
        line("");
        for u in upstreams.values() {
            let mark = if u.service.is_some() {
                green("connected")
            } else {
                red("failed")
            };
            line(&format!(
                "  {} {} {} tools{}",
                mark,
                bold(&u.server),
                cyan(&u.tools.len().to_string()),
                u.last_error
                    .as_ref()
                    .map(|e| format!("  {}", dim(&redact_secrets(e, &state.redact))))
                    .unwrap_or_default()
            ));
        }
        line("");
        line(&dim(SEPARATOR_NOTE));
        line(&dim("press ctrl-c to stop"));
    }

    match transport {
        "stdio" => run_stdio(state).await,
        "http" => run_http(state, bind, token).await,
        other => Err(CerberusError::Validation(format!(
            "unknown gateway transport `{other}` (expected stdio or http)"
        ))),
    }
}

async fn run_stdio(state: Arc<GatewayState>) -> Result<i32> {
    let paths = state.paths.clone();
    let handler = GatewayHandler::new(state.clone());
    let ct = tokio_util::sync::CancellationToken::new();
    let ct_watch = ct.clone();

    let service = handler
        .serve_with_ct(rmcp::transport::stdio(), ct.clone())
        .await
        .map_err(|e| CerberusError::Gateway(format!("stdio serve failed: {e}")))?;

    let snapshot = status_snapshot(&state, "stdio", None).await;
    write_state_file(&paths, &snapshot)?;
    tracing::info!(tools = snapshot.tools, "gateway running (stdio)");

    let waiting = service.waiting();
    tokio::pin!(waiting);
    let mut ended = false;
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("shutdown requested");
            ct_watch.cancel();
        }
        result = &mut waiting => {
            ended = true;
            if let Err(e) = result {
                tracing::warn!(error = %e, "service ended with error");
            }
        }
    }
    if !ended {
        let _ = waiting.await;
    }

    // Release upstreams cleanly.
    let mut upstreams = state.upstreams.lock().await;
    for u in upstreams.values_mut() {
        if let Some(s) = u.service.take() {
            let _ = s.cancel().await;
        }
    }
    let _ = std::fs::remove_file(paths.gateway_state_file());
    Ok(EXIT_OK)
}

const EXIT_OK: i32 = 0;

async fn run_http(state: Arc<GatewayState>, bind: &str, token: Option<&str>) -> Result<i32> {
    let paths = state.paths.clone();
    let state_factory_state = state.clone();
    let service = StreamableHttpService::new(
        move || Ok(GatewayHandler::new(state_factory_state.clone())),
        std::sync::Arc::new(
            rmcp::transport::streamable_http_server::session::local::LocalSessionManager::default(),
        ),
        StreamableHttpServerConfig::default(),
    );

    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .map_err(|e| CerberusError::Gateway(format!("bind {bind}: {e}")))?;

    let auth_token = token.map(str::to_string);
    let mut app = axum::Router::new();
    if let Some(required) = auth_token.clone() {
        app = app.layer(axum::middleware::from_fn(
            move |req: axum::extract::Request, next: axum::middleware::Next| {
                let required = required.clone();
                async move {
                    let ok = req
                        .headers()
                        .get(axum::http::header::AUTHORIZATION)
                        .and_then(|v| v.to_str().ok())
                        .and_then(|v| v.strip_prefix("Bearer "))
                        .map(|v| constant_time_eq(v.as_bytes(), required.as_bytes()))
                        .unwrap_or(false);
                    if ok {
                        Ok(next.run(req).await)
                    } else {
                        Err(axum::http::StatusCode::UNAUTHORIZED)
                    }
                }
            },
        ));
    }
    app = app.fallback_service(service);

    let snapshot = status_snapshot(&state, "http", Some(bind)).await;
    write_state_file(&paths, &snapshot)?;

    let shutdown_state = state.clone();
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = tokio::signal::ctrl_c().await;
            tracing::info!("shutdown requested");
            let mut upstreams = shutdown_state.upstreams.lock().await;
            for u in upstreams.values_mut() {
                if let Some(s) = u.service.take() {
                    let _ = s.cancel().await;
                }
            }
            let _ = std::fs::remove_file(shutdown_state.paths.gateway_state_file());
        })
        .await
        .map_err(|e| CerberusError::Gateway(format!("http serve: {e}")))?;
    Ok(EXIT_OK)
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Validate that a gateway can start: registry valid, upstreams reachable.
pub fn validate_config(store: &RegistryStore, profile: Option<&str>) -> Result<Vec<String>> {
    crate::registry::validate::validate(&store.registry)?;
    let selected = crate::sync::selected_servers(store, profile)?;
    let mut warnings = Vec::new();
    for (name, def) in &selected {
        if !def.enabled {
            continue;
        }
        match &def.transport {
            Transport::Stdio { command, .. } => {
                if !executable_exists(command) {
                    warnings.push(format!("server `{name}`: executable `{command}` not found"));
                }
            }
            Transport::Http { url, .. } => {
                url::Url::parse(url).map_err(|e| {
                    CerberusError::Validation(format!("server `{name}`: invalid url: {e}"))
                })?;
            }
        }
        if def.effective_mode(&store.registry.settings) == ExecutionMode::Direct {
            warnings.push(format!(
                "server `{name}` has mode=direct; it will still be exposed by the gateway \
                 when the gateway is running (mode affects client wiring, not gateway membership)"
            ));
        }
    }
    Ok(warnings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ExitCode;
    use rmcp::model::CallToolResponse;

    #[test]
    fn constant_time_eq_works() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }

    fn payload_of(response: CallToolResponse) -> serde_json::Value {
        match response {
            CallToolResponse::Complete(result) => {
                assert_eq!(result.is_error, Some(true));
                assert_eq!(result.content.len(), 1);
                result
                    .structured_content
                    .expect("error results must carry structured content")
            }
            other => panic!("expected Complete response, got {other:?}"),
        }
    }

    #[test]
    fn tool_error_payload_is_machine_readable() {
        let mut extra = serde_json::Map::new();
        extra.insert("server".into(), "github".into());
        extra.insert("tool".into(), "search_repos".into());
        extra.insert("reason".into(), "deny pattern matched".into());

        let payload = payload_of(tool_error(
            GatewayErrorCode::PolicyDenied,
            "policy denied: deny pattern matched",
            extra,
        ));

        assert_eq!(payload["code"], "policy_denied");
        assert_eq!(payload["retryable"], false);
        assert_eq!(payload["exit_code"], ExitCode::PolicyDenied as u8);
        assert_eq!(payload["server"], "github");
        assert_eq!(payload["tool"], "search_repos");
        assert_eq!(payload["reason"], "deny pattern matched");
        assert!(
            payload["message"]
                .as_str()
                .unwrap()
                .contains("policy denied"),
            "human message must stay in structured payload too"
        );
    }

    #[test]
    fn tool_error_for_adds_server_and_tool_only_when_known() {
        let payload = payload_of(tool_error_for(
            GatewayErrorCode::Timeout,
            "timed out",
            Some("filesystem"),
            Some("read_file"),
        ));
        assert_eq!(payload["code"], "timeout");
        assert_eq!(payload["server"], "filesystem");
        assert_eq!(payload["tool"], "read_file");

        let payload = payload_of(tool_error(
            GatewayErrorCode::ShuttingDown,
            "gateway is shutting down",
            serde_json::Map::new(),
        ));
        assert_eq!(payload["code"], "shutting_down");
        assert!(payload.get("server").is_none());
        assert!(payload.get("tool").is_none());
    }

    /// The wire spelling of every code is a stability contract with the SDKs.
    #[test]
    fn error_codes_serialize_as_stable_snake_case() {
        let all = [
            (GatewayErrorCode::UnknownTool, "unknown_tool"),
            (GatewayErrorCode::ServerNotFound, "server_not_found"),
            (GatewayErrorCode::PolicyDenied, "policy_denied"),
            (
                GatewayErrorCode::ConfirmationRequired,
                "confirmation_required",
            ),
            (
                GatewayErrorCode::UpstreamUnavailable,
                "upstream_unavailable",
            ),
            (GatewayErrorCode::ShuttingDown, "shutting_down"),
            (
                GatewayErrorCode::ConcurrencySaturated,
                "concurrency_saturated",
            ),
            (GatewayErrorCode::Timeout, "timeout"),
            (GatewayErrorCode::UpstreamError, "upstream_error"),
        ];
        for (code, expected) in all {
            assert_eq!(code.as_str(), expected);
            assert_eq!(
                serde_json::to_value(code).unwrap(),
                serde_json::Value::String(expected.to_string()),
                "serde and as_str() must agree"
            );
            // Round-trip: SDKs must be able to parse what we emit.
            let parsed: GatewayErrorCode =
                serde_json::from_value(serde_json::json!(expected)).unwrap();
            assert_eq!(parsed, code);
        }
    }

    /// Wire codes and CLI exit codes describe the same conditions.
    #[test]
    fn exit_codes_align_with_cli_contract() {
        use GatewayErrorCode as C;
        assert_eq!(C::UnknownTool.exit_code(), ExitCode::NotFound);
        assert_eq!(C::ServerNotFound.exit_code(), ExitCode::NotFound);
        assert_eq!(C::PolicyDenied.exit_code(), ExitCode::PolicyDenied);
        assert_eq!(C::ConfirmationRequired.exit_code(), ExitCode::PolicyDenied);
        assert_eq!(C::Timeout.exit_code(), ExitCode::Timeout);
        assert_eq!(
            C::ConcurrencySaturated.exit_code(),
            ExitCode::Timeout,
            "a saturated gateway is a wait-too-long condition, not a hard failure"
        );
        assert_eq!(C::UpstreamUnavailable.exit_code(), ExitCode::Failure);
        assert_eq!(C::ShuttingDown.exit_code(), ExitCode::Failure);
        assert_eq!(C::UpstreamError.exit_code(), ExitCode::Failure);
    }

    /// Only transient conditions may invite a retry from an SDK.
    #[test]
    fn retryable_is_limited_to_transient_failures() {
        use GatewayErrorCode as C;
        assert!(C::UpstreamUnavailable.is_retryable());
        assert!(C::ShuttingDown.is_retryable());
        assert!(C::ConcurrencySaturated.is_retryable());
        assert!(!C::PolicyDenied.is_retryable());
        assert!(!C::ConfirmationRequired.is_retryable());
        assert!(!C::UnknownTool.is_retryable());
        assert!(!C::ServerNotFound.is_retryable());
        assert!(!C::Timeout.is_retryable());
        assert!(!C::UpstreamError.is_retryable());
    }

    /// The exact JSON an SDK parses off the wire. If rmcp ever renames the
    /// envelope keys, this fails before an SDK silently starts reading nulls.
    #[test]
    fn wire_envelope_uses_camel_case_keys() {
        let response = tool_error_for(
            GatewayErrorCode::UpstreamUnavailable,
            "upstream `filesystem` unavailable",
            Some("filesystem"),
            Some("read_file"),
        );

        let (serialized, text) = match response {
            CallToolResponse::Complete(result) => {
                let text = match &result.content[0] {
                    ContentBlock::Text(t) => t.text.clone(),
                    other => panic!("expected text block, got {other:?}"),
                };
                (serde_json::to_value(result).unwrap(), text)
            }
            other => panic!("expected Complete response, got {other:?}"),
        };

        // Envelope keys are camelCase per the MCP schema.
        assert_eq!(serialized["isError"], true);
        assert!(serialized.get("resultType").is_some());
        assert!(serialized.get("structuredContent").is_some());
        // The old snake_case spellings must not appear.
        assert!(serialized.get("is_error").is_none());
        assert!(serialized.get("structured_content").is_none());

        let sc = &serialized["structuredContent"];
        assert_eq!(sc["code"], "upstream_unavailable");
        assert_eq!(sc["server"], "filesystem");
        assert_eq!(sc["tool"], "read_file");
        assert_eq!(sc["retryable"], true);
        assert_eq!(sc["exit_code"], ExitCode::Failure as u8);
        assert_eq!(sc["message"], "upstream `filesystem` unavailable");

        // Human-readable text still rides in content for MCP clients that
        // only render content blocks.
        assert_eq!(text, "upstream `filesystem` unavailable");
    }

    /// Codes survive a full serialize/deserialize round-trip nested inside
    /// structured content, which is how an SDK actually reads them.
    #[test]
    fn codes_round_trip_from_structured_content() {
        let response = tool_error(
            GatewayErrorCode::PolicyDenied,
            "policy denied: deny_tools matched",
            serde_json::Map::new(),
        );
        let wire = match response {
            CallToolResponse::Complete(result) => serde_json::to_value(result).unwrap(),
            other => panic!("expected Complete, got {other:?}"),
        };
        let code: GatewayErrorCode =
            serde_json::from_value(wire["structuredContent"]["code"].clone()).unwrap();
        assert_eq!(code, GatewayErrorCode::PolicyDenied);
    }

    /// The exact `_meta` object an SDK reads off `tools/list`.
    #[test]
    fn route_meta_serializes_under_stable_reverse_dns_key() {
        let meta = route_meta("remote-filesystem", "read_file");
        let wire = serde_json::to_value(&meta).unwrap();

        let route = wire.get(ROUTE_META_KEY).expect("route key must be present");
        assert_eq!(route["server"], "remote-filesystem");
        assert_eq!(route["tool"], "read_file");
        // No stray sibling keys.
        assert_eq!(wire.as_object().unwrap().len(), 1);
    }

    /// `Tool` must emit the metadata as `_meta` on the wire, with no
    /// serde renaming of the key itself.
    #[test]
    fn tool_listing_carries_route_meta_on_the_wire() {
        let schema = serde_json::json!({"type": "object", "properties": {}});
        let mut tool = Tool::new(
            "remote-filesystem__read_file".to_string(),
            "[remote-filesystem] Read a file".to_string(),
            std::sync::Arc::new(schema.as_object().unwrap().clone()),
        );
        tool.meta = Some(route_meta("remote-filesystem", "read_file"));

        let wire = serde_json::to_value(&tool).unwrap();
        assert_eq!(wire["name"], "remote-filesystem__read_file");
        assert_eq!(wire["_meta"][ROUTE_META_KEY]["server"], "remote-filesystem");
        assert_eq!(wire["_meta"][ROUTE_META_KEY]["tool"], "read_file");
        // Snake_case spellings must not appear.
        assert!(wire.get("route_meta").is_none());
        assert!(wire.get("routeMeta").is_none());
    }

    /// A tool whose own name contains the separator resolves back through the
    /// public name only via first-split, and the metadata keeps them apart.
    #[test]
    fn route_meta_is_authoritative_for_separator_bearing_tool_names() {
        let public = crate::routing::public_tool_name("srv", "do__it");
        assert_eq!(public, "srv__do__it");
        assert_eq!(
            crate::routing::split_public_name(&public),
            Some(("srv", "do__it"))
        );
        let wire = serde_json::to_value(route_meta("srv", "do__it")).unwrap();
        assert_eq!(wire[ROUTE_META_KEY]["tool"], "do__it");
    }
}
