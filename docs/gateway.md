# Gateway

The gateway is an MCP server (built on `rmcp`, spec 2026-07-28) that fronts
any number of upstream MCP servers behind a single endpoint.

```
client ──▶ cerberus gateway ──┬──▶ filesystem (stdio)
                              ├──▶ github (streamable HTTP)
                              └──▶ memory (stdio)
```

## Modes

- **stdio** (`cerberus gateway start` with stdio configured): for clients
  that spawn the gateway as a subprocess. The client's stdio is the transport.
- **HTTP** (`cerberus gateway start` with the HTTP transport configured):
  streamable HTTP bound to `127.0.0.1` by default. Remote binding requires
  `--allow-remote` **and** a bearer token.

Related commands: `gateway status`, `gateway doctor`, `gateway validate`,
`gateway stop`.

## Routing

Upstream tools are published as `{server}__{tool}`. `routing::RoutingTable`
detects ambiguous public names (possible only if a server name itself
contains the separator, which registration forbids) and refuses to register
colliding routes. `split_public_name` splits on the first `__` only, so
upstream tools containing `__` keep working.

## Lifecycle

Upstreams connect lazily on first use and reconnect on failure. Each
connection is a long-lived `RunningService` held in gateway state; discovery
refreshes the routing table. Shutdown cancels every upstream and removes the
HTTP state file.

## Policy enforcement

Every call is checked against the server's `ServerPolicyView` before it
reaches the upstream:

1. `deny_tools` wins first (trailing-`*` wildcards supported).
2. A non-empty `allow_tools` acts as an allowlist.
3. `require_confirmation` forces a confirmation handshake; unconfirmed calls
   return a policy error.

Denials surface as MCP tool errors with a clear message — the call never
leaves the gateway. Exit path: policy errors map to exit code 6 in the CLI
context.

## Limits and safety

- A semaphore caps concurrent upstream calls (`concurrency`).
- A per-call timeout (`call_timeout`) bounds hung upstreams.
- Startup timeout applies while connecting.
- Loopback-only binding by default; `--allow-remote` requires a token, which
  callers must send as a bearer token.
- Upstream errors are redacted with the same secret list the CLI uses before
  they reach logs or responses.

## State file

In HTTP mode the gateway writes a small state snapshot (transport, port, tool
count) under the data directory for `gateway status` to read; it is removed
on clean shutdown.
