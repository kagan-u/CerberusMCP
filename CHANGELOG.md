# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
