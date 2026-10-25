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

### Structured routing metadata

Every entry in `tools/list` also carries the mapping under `_meta`, so
clients never have to parse the public name:

```json
{
  "name": "remote-filesystem__read_file",
  "description": "[remote-filesystem] Read a file...",
  "_meta": {
    "cerberus.dev/route": {
      "server": "remote-filesystem",
      "tool": "read_file"
    }
  }
}
```

`_meta["cerberus.dev/route"]` is a wire contract with the Cerberus SDKs
(`cerberus-sdk`): `server` and `tool` are authoritative, and the name is only
ever a fallback. The nested reverse-DNS key keeps gateway metadata from
colliding with other `_meta` producers (trace context, host tooling).

## Lifecycle

Upstreams connect eagerly during startup (bounded parallelism) and lazily
again on first use after a failure. Disabled servers (`enabled = false`)
are never connected — same as every client adapter. Each connection is a
long-lived `RunningService` held in gateway state; discovery refreshes the
routing table. Shutdown cancels every upstream and removes the HTTP state
file.

## Hot reload (`--watch`)

`gateway start --watch` (default: off) polls the registry file's stat
(mtime + size) every 500 ms and reconciles changes while serving:

- **Touch-safe:** identical content (an mtime-only touch) changes nothing.
- **Valid documents only:** a file that fails to parse or validate is
  rejected with a warning — the running configuration stays in effect
  until a valid write arrives.
- **Per-server reconciliation:** only `transport`/`pre_launch` changes
  cancel and reconnect an upstream (its pre-launch hooks re-run on that
  reconnect). Policy, tags, mode, description, and similar edits are
  swapped in place while the live connection stays up. Servers entering or
  leaving the profile are connected or disconnected.
- **Always live:** the `[commands.*]` table, the secret redaction list,
  and — without an explicit `--profile` — the active profile selection
  follow every applied document.

A rejected reload never touches live state; a tool call in flight against
a server that gets reconnected fails with the normal upstream error.

## Pre-launch hooks

A stdio server may declare `pre_launch` steps (documented in
[`docs/registry.md`](registry.md)). The gateway executes them — and only the
gateway:

- **When:** immediately before the upstream's process is spawned, in
  registration order — both during startup's eager connect and on every
  reconnect. Steps must be idempotent.
- **Environment:** the same sanitized launch environment the server process
  would get (safe baseline + the server's resolved bindings), with the
  `[commands.*]` bindings and then the step's own bindings on top. `cwd`
  falls back from step to command; `~` expands to home.
- **Timeouts:** `timeout_ms` from the step, else the command, else 30 s. On
  expiry the child is killed and the step fails.
- **Failures:** a failed step aborts that server's connect and surfaces as
  the usual `upstream_unavailable` error with the step location and the
  child's stderr quoted (`pre-launch servers.<name>.pre_launch[i]: …`). A
  step with `continue_on_error = true` logs a warning instead and the
  remaining steps run.
- **Never run elsewhere:** doctor probes, client adapters, and direct-mode
  launches ignore `pre_launch`; `cerberus doctor` reports an Info line for
  hooked servers saying exactly that.

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

## Structured error contract

Every failing `tools/call` returns a tool-level error with **both** a human
message and machine-readable data, so client libraries branch on a stable
enum instead of parsing prose.

```jsonc
// content[0].text  — what MCP clients render
"upstream `filesystem` unavailable: connection refused"

// structuredContent — what SDKs parse
{
  "code": "upstream_unavailable",
  "message": "upstream `filesystem` unavailable: connection refused",
  "retryable": true,
  "exit_code": 1,
  "server": "filesystem",   // present when known
  "tool": "read_file"       // present when known
}
```

| `code` | Meaning | Retryable | CLI exit |
| --- | --- | --- | --- |
| `unknown_tool` | public name isn't `{server}__{tool}` | no | 5 |
| `server_not_found` | server not routed / not in profile | no | 5 |
| `policy_denied` | allow/deny list rejected the call | no | 6 |
| `confirmation_required` | server needs interactive confirmation | no | 6 |
| `upstream_unavailable` | unreachable, not connected, or failed to start | **yes** | 1 |
| `shutting_down` | gateway is draining | **yes** | 1 |
| `concurrency_saturated` | semaphore waited out its timeout | **yes** | 7 |
| `timeout` | upstream exceeded `call_timeout` | no | 7 |
| `upstream_error` | upstream answered with an error | no | 1 |

Notes for SDK authors:

- `retryable: true` means *the identical call may be worth repeating*;
  `false` means retrying will reproduce the failure. `policy_denied` and
  `confirmation_required` are never retried.
- `exit_code` mirrors what the CLI reports for the same condition
  (`cerberus` process exit codes), so the wire and process contracts agree.
- Field names are camelCase on the wire (`structuredContent`, `isError`);
  the `code` values are snake_case and stable.
- `call_tool` failures are tool-level errors (`isError: true` inside a
  `Complete` response), not JSON-RPC protocol errors — the request was valid
  and routed, execution is what failed.

## Limits and safety

- A semaphore caps concurrent upstream calls (`concurrency`).
- A per-call timeout (`call_timeout`) bounds hung upstreams.
- Startup timeout applies while connecting.
- Loopback-only binding by default; `--allow-remote` requires a token, which
  callers must send as a bearer token.

## Securing a remote gateway

The HTTP transport is plaintext by design; treat the network path as
untrusted and terminate TLS in front of the gateway (nginx, Caddy, a service
mesh, or an SSH tunnel). The security model:

- **Bind guard.** `--bind` defaults to `127.0.0.1:8713`. Only literal IPs in
  `127.0.0.0/8`, `::1`, and the exact name `localhost` count as loopback;
  anything else — including hostname lookalikes like `127.0.0.1.nip.io` —
  requires `--allow-remote` and fails otherwise.
- **Bearer token.** Non-loopback binds also require a token: `--token` or the
  `CERBERUS_GATEWAY_TOKEN` environment variable (prefer the env var — CLI
  arguments are visible in process listings). An empty token counts as no
  token. Comparison is constant-time, and401 responses carry
  `WWW-Authenticate: Bearer`.
- **Host-header allowlist.** For loopback binds, rmcp's default DNS-rebinding
  protection stays active (only `localhost`/`127.0.0.1`/`::1` Host headers
  are accepted). `--allow-remote` disables it — remote clients send their
  public Host header, and the token is the access control.
- **State file.** The HTTP state file records pid, transport, and the bound
  address; it never contains the token.
- Upstream errors are redacted with the same secret list the CLI uses before
  they reach logs or responses.

## State file

In HTTP mode the gateway writes a small state snapshot (transport, port, tool
count) under the data directory for `gateway status` to read; it is removed
on clean shutdown.
