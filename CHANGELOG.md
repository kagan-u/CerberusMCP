# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

### Fixed

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
