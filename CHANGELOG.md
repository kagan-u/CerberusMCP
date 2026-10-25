# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.0] - 2026-10-25

### Added

- Registry schema **v2** (`schema_version = 2`) with two additive features:
  - Top-level `[commands.*]`: named, reusable executables run via the new
    `cerberus run <name> [-- <extra args...>]` (no name → lists commands;
    `--json` reports `{command, argv, exit_code, duration_ms, stdout, stderr}`).
    The child inherits the process environment with the command's registry
    bindings overlaid; its exit code passes through unchanged and
    `timeout_ms` maps to exit code `7`.
  - Per-server `pre_launch`: ordered hook steps (`use = "…"` referencing a
    command, or an inline `run` vector) executed before a `stdio` server's
    process is spawned. Exactly one of `use`/`run`; per-step `cwd`, `env`,
    `timeout_ms`, `continue_on_error`. Validated at registry load
    (`stdio`-only, command references must exist).
- The registry is now written as **TOML** at `conf.cerberus`; `config show`
  redacts `secret_ref`/`literal` env values in TOML output, `backup restore`
  accepts either format (detected by content), and `cerberus init` refuses to
  overwrite a legacy `cerberus.yaml` without `--force`.

- Gateway **pre-launch hooks**: stdio servers' `pre_launch` steps run in
  `connect_upstream` — the single choke point before a spawn, on both the
  eager startup connect and every reconnect. Steps resolve `use` references
  against `[commands.*]` (step fields override command defaults), inherit
  the sanitized server launch environment with their bindings on top, are
  killed at `timeout_ms` (step → command → 30 s default), and a failure
  aborts that upstream's connect as a normal `upstream_unavailable` error
  quoting the child's stderr (`continue_on_error` downgrades to a warning).
  Doctor probes and client adapters never execute hooks — `cerberus doctor`
  reports an Info diagnostic for hooked servers.
- Compiled registry cache: a load parses `conf.cerberus` once and compiles
  an rkyv snapshot under `<data>/cache/registry-*.cbin`; later loads verify
  the container header, the source stat, and FNV-1a hashes of both source
  and payload, then serve the snapshot without parsing TOML. The cache is
  derived-only (safe to delete, rebuilt on demand), only ever written for
  validated registries, and falls back silently to parsing on any mismatch.
  `CERBERUS_NO_CACHE=1` disables reads and writes;
  `cerberus config compile [--json]` builds and inspects it explicitly;
  `config path` reports the cache directory.
- `gateway start --watch` (default: off): stat-only registry hot reload.
  A background poll (500 ms) re-parses and re-validates `conf.cerberus`
  whenever its mtime/size change; a touch that leaves the content
  identical changes nothing, and a document that fails to parse or
  validate is rejected with a warning while the running configuration
  stays in effect. Reconciliation is per server: only `transport`/
  `pre_launch` edits cancel and reconnect the upstream (re-running its
  pre-launch hooks), everything else — policy, tags, mode, and similar —
  is swapped in place with the connection left up, and profile membership
  changes connect/disconnect. The `[commands.*]` table and the secret
  redaction list always follow the applied document.

### Changed

- The default registry path is `<config dir>/conf.cerberus`. An existing
  `cerberus.yaml` (schema v0/v1) is migrated in place on first access: the
  file is re-saved as `conf.cerberus` at schema v2 and the original moved to
  `<data>/backups/migrations/`.

## [0.2.0] - 2026-10-25

### Added

- `cerberus search <query>` against the official MCP Registry
  (`registry.modelcontextprotocol.io`): deduplicated by newest published
  version, `--limit`, `--json`, and `--offline` replay from a local cache
  (`registry-cache.json`).
- `cerberus install <name|file>`: resolves a registry record (or a saved JSON
  record), prefers a launchable `stdio` package over a hosted endpoint, derives
  a local server name, and writes only the registry — `sync` stays a separate
  explicit step. `--check` verifies the launcher resolves on `PATH` (honoring
  `PATHEXT` on Windows), `--dry-run` prints without writing, `--name`,
  `--tags`, and `--disabled` control the entry. Unsupported packaging systems
  fail with exit code `8`.
- `source` block on server entries: install provenance (`kind`, `package`,
  `version`, `runner`, `homepage`, `url`/`ref`, documented `env`
  requirements), validated per kind and reported by `cerberus doctor`.
  The gateway and client adapters never read it. Documented in
  `docs/registry.md`.
- `CERBERUS_REGISTRY_URL` overrides the registry endpoint.
- `server inspect` now includes the `source` block in `--json` and in its
  text output.
- Gateway `tools/list` carries structured routing metadata: every tool
  entry has `_meta["cerberus.dev/route"]` = `{server, tool}`, so client
  SDKs never split the `{server}__{tool}` public name (contract documented
  in `docs/gateway.md`).

### Changed

- Upgrade `rmcp` 3.5.1 → 3.7.0 (protocol 2026-07-28 model, `resultType` and
  cache-hint machinery).
- Loopback detection for `gateway start --bind` is now fail-closed: only
  literal IPs in 127.0.0.0/8 or ::1, and the exact name `localhost`, count
  as loopback. Hostnames (including lookalikes such as `127.0.0.1.nip.io`)
  require `--allow-remote` + a token.
- With `--allow-remote`, the streamable-HTTP Host-header allowlist is
  disabled (remote clients legitimately send their public Host); the bearer
  token is the access control. Loopback binds keep rmcp's DNS-rebinding
  allowlist.
- The HTTP state file now records the actually bound address, so
  `--bind 127.0.0.1:0` reports the real port.

### Fixed

- HTTP gateway bearer auth was never enforced: the auth middleware was
  applied before the MCP fallback service was attached, and axum's `layer`
  only wraps routes added at that point — every request reached the gateway
  unauthenticated. The fallback is now installed first so the token check
  covers all traffic. 401 responses carry `WWW-Authenticate: Bearer`, and an
  empty `--token`/`CERBERUS_GATEWAY_TOKEN` is treated as absent (it no
  longer satisfies the non-loopback token requirement).
- `gateway start --transport stdio` no longer prints the startup banner on
  stdout: prose on the MCP channel broke strict clients (the official Python
  SDK rejects non-JSON-RPC lines). For stdio the banner and status lines go
  to stderr; stdout stays JSON-RPC-only.
- The upstream HTTP client (used by `gateway start` with an `http(s)`
  upstream entry) sent `Authorization: Bearer Bearer <token>` — the token
  was double-prefixed because rmcp's `auth_header` field takes the raw
  token and adds the scheme itself. It now passes the raw value.
- `tools/call` results always carry `resultType: "complete"` for peers on
  protocol 2026-07-28 (mcp-py 2.3 validates against that surface even when
  the upstream hop negotiated an older version); rmcp strips the field again
  for legacy peers, so older clients see the unchanged shape.
- Windows: `CerberusPaths::resolve` now honors `CERBERUS_CONFIG_DIR` /
  `CERBERUS_DATA_DIR` before consulting `ProjectDirs`, so sandboxed runs work
  even when no OS profile folder can be resolved.
- Windows: `home_dir()` prefers `USERPROFILE`/`HOME` over `BaseDirs`, which
  resolves the profile through `SHGetKnownFolderPath` and ignores those
  variables — sandboxing was silently ineffective.

## [0.1.0] - 2026-10-10

### Added

- Canonical MCP server registry (YAML, schema v1) with validation,
  optimistic-concurrency checks, and forward migrations.
- Client adapters and sync/diff support for OpenCode (v1 + v2 schemas),
  Claude Code, Codex, Gemini CLI, and Cline. Aider is detected and reported
  as unsupported (no native MCP support).
- Ownership tracking per client (`state/managed.json`) with content hashes;
  conflicts require `--force` and never silently overwrite user edits.
- Atomic config writes (temp file + rename, symlink refusal) with pre-write
  backups and restore.
- Secret handling: env refs, named secret references, and literals; sanitized
  baseline environment by default; secrets redacted from logs and errors.
- Profiles: named subsets of servers with one active profile.
- `doctor`, `health`, and `logs` diagnostics commands with `--json` output.
- Gateway: aggregates multiple upstream MCP servers behind one MCP endpoint
  (stdio or streamable HTTP on loopback), with tool routing
  (`{server}__{tool}`), allow/deny policy enforcement, concurrency limits,
  call timeouts, and secret redaction.
- Structured error contract on the gateway: every `tools/call` failure
  carries `structuredContent` with a stable `code`, `message`, `retryable`,
  `exit_code`, and `server`/`tool`/`reason` context. Codes are enum-checked
  and aligned with CLI exit codes (see `docs/gateway.md`), giving SDKs a
  machine-readable alternative to parsing human error text.
- Shell completions for bash, zsh, fish, elvish, and PowerShell.
