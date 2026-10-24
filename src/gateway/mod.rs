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
    ServerCapabilities, ServerConfig, Tool,
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
            "CerberusMCP gateway. {SEPARATOR_NOTE}. Policies are enforced before any \
             request reaches an upstream server."
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
            tools.push(Tool::new(
                route.public_name.clone(),
                description,
                upstream_input_schema(upstream, &route.tool),
            ));
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
            return Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "unknown tool `{public_name}` ({SEPARATOR_NOTE})"
            ))])
            .into());
        };
        let (server, tool) = (server.to_string(), tool.to_string());

        // Policy check before any upstream work.
        {
            let policies = self.state.policies.read().await;
            let Some(p) = policies.get(&server) else {
                return Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                    "server `{server}` is not available through this gateway"
                ))])
                .into());
            };
            if let policy::Decision::Deny { reason } = policy::evaluate(p, &tool) {
                tracing::warn!(%correlation, %server, %tool, %reason, "policy denied");
                return Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                    "policy denied: {reason}"
                ))])
                .into());
            }
        }

        // Confirmation gate for high-impact servers.
        {
            let confirmations = self.state.confirmations.read().await;
            if confirmations.get(&server).copied().unwrap_or(false) {
                return Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                    "server `{server}` requires confirmation for tool calls; this gateway \
                     build does not implement an interactive confirmation channel — call \
                     the tool directly or relax `require_confirmation` in the registry"
                ))])
                .into());
            }
        }

        // Connect (lazily reconnect) and forward.
        if let Err(e) = self.ensure_connected(&server).await {
            let msg = self.redact(&e.to_string());
            tracing::error!(%correlation, %server, error = %msg, "upstream unavailable");
            return Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "upstream `{server}` unavailable: {msg}"
            ))])
            .into());
        }

        let _permit = tokio::select! {
            permit = self.state.semaphore.clone().acquire_owned() => match permit {
                Ok(p) => p,
                Err(_) => {
                    return Ok(CallToolResult::error(vec![ContentBlock::text(
                        "gateway is shutting down".to_string()
                    )]).into());
                }
            },
            _ = tokio::time::sleep(self.state.call_timeout) => {
                return Ok(CallToolResult::error(vec![ContentBlock::text(
                    "gateway concurrency limit saturated; retry shortly".to_string()
                )]).into());
            }
        };

        let upstreams = self.state.upstreams.lock().await;
        let Some(upstream) = upstreams.get(&server) else {
            return Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "server `{server}` is not connected"
            ))])
            .into());
        };
        let Some(service) = upstream.service.as_ref() else {
            return Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "server `{server}` is not connected"
            ))])
            .into());
        };

        let params = CallToolRequestParams::new(tool.clone())
            .with_arguments(request.arguments.clone().unwrap_or_default());

        let timeout = self.state.call_timeout;
        let result = tokio::time::timeout(timeout, service.call_tool(params)).await;

        let elapsed = start.elapsed();
        match result {
            Err(_) => {
                tracing::warn!(%correlation, %server, %tool, elapsed_ms = elapsed.as_millis() as u64, "timeout");
                Ok(
                    CallToolResult::error(vec![ContentBlock::text(self.redact(&format!(
                        "tool `{tool}` on server `{server}` timed out after {} ms",
                        timeout.as_millis()
                    )))])
                    .into(),
                )
            }
            Ok(Err(e)) => {
                let msg = self.redact(&e.to_string());
                tracing::warn!(%correlation, %server, %tool, elapsed_ms = elapsed.as_millis() as u64, error = %msg, "upstream error");
                Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                    "upstream error from `{server}`: {msg}"
                ))])
                .into())
            }
            Ok(Ok(result)) => {
                tracing::info!(%correlation, %server, %tool, elapsed_ms = elapsed.as_millis() as u64, "ok");
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
        ctx.out(&bold("CerberusMCP Gateway"));
        ctx.status("Transport", transport);
        if transport == "http" {
            ctx.status("Bind", bind);
        }
        ctx.out("");
        for u in upstreams.values() {
            let mark = if u.service.is_some() {
                green("connected")
            } else {
                red("failed")
            };
            ctx.out(&format!(
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
        ctx.out("");
        ctx.out(&dim(SEPARATOR_NOTE));
        ctx.out(&dim("press ctrl-c to stop"));
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

    #[test]
    fn constant_time_eq_works() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }
}
