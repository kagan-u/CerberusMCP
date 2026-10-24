<p align="center">
  <img src="images/logo.png" alt="CerberusMCP logo" width="180">
</p>

# CerberusMCP

A single source of truth for your MCP servers. CerberusMCP keeps a canonical
registry of MCP server definitions and synchronizes them into the config
files that AI coding CLIs actually read — OpenCode, Claude Code, Codex,
Gemini CLI, and Cline — without clobbering edits you made by hand. An
optional gateway fronts every server behind one MCP endpoint with centralized
policy enforcement.

```
cerberus registry  ──sync──▶  OpenCode   (opencode.json, v1 or v2 schema)
                  ──sync──▶  Claude Code (~/.claude.json)
                  ──sync──▶  Codex      (~/.codex/config.toml)
                  ──sync──▶  Gemini CLI (~/.gemini/settings.json)
                  ──sync──▶  Cline      (cline_mcp_settings.json)
                  ──gateway─▶ any MCP client (stdio or streamable HTTP)
```

## Why

Every AI CLI has its own MCP config format, its own env-var conventions, and
its own idea of what "disabled" means. Copy-pasting a server into five files
drifts immediately. CerberusMCP inverts that: declare each server once,
declare who gets it (profiles), and let `cerberus sync` do the translation.
Files it manages are tracked by content hash, so a hand edit is detected and
reported as a conflict — never silently overwritten.

## Install

```bash
cargo install --path .        # from a checkout
cerberus init                 # create ~/.config/cerberus-mcp (paths per OS)
```

## Quick start

```bash
# Register a local stdio server
cerberus server add filesystem \
  --command npx --arg=-y --arg=@modelcontextprotocol/server-filesystem --arg=/tmp

# Register a remote server; the token is read from the environment at launch
cerberus server add github \
  --transport http \
  --url https://api.githubcopilot.com/mcp/ \
  --env GITHUB_TOKEN=env:GITHUB_TOKEN

# See what sync would do (dry-run is the default)
cerberus sync --client opencode
# Write it
cerberus sync --client opencode --apply

# Or sync everything that's installed
cerberus sync --apply

# What changed since last time?
cerberus diff --client codex
cerberus doctor
```

Every machine-readable command accepts `--json`.

## Concepts

### Registry

One YAML file (`schema_version: 1`), validated on load and on save. See
[`schemas/registry.schema.json`](schemas/registry.schema.json) and the
annotated [`examples/registry.yaml`](examples/registry.yaml).

### Profiles

A profile is a named subset of servers. `--profile minimal` syncs only that
subset; the active profile (or no profile) controls the default.

### Ownership and conflicts

On each sync, CerberusMCP records a hash of every entry it wrote, per client.
On the next sync:

| Situation | Result |
| --- | --- |
| Entry matches our last write | update in place |
| Entry changed since we wrote it | **conflict** — reported, not applied; `--force` takes ownership |
| Entry we never wrote occupies the name | **conflict** — `--force` to take ownership |
| Managed entry deleted from the client config | **conflict** — not re-added without `--force` |

Writes are atomic (temp file + rename, symlinks refused) and a backup of the
previous file is stored under the data directory before every overwrite.
`cerberus backup list` / `cerberus backup restore <id>` manage them.

### Environment values

Three kinds, everywhere:

```yaml
env:
  GITHUB_TOKEN: { source: env, name: GITHUB_TOKEN }        # read at launch
  API_KEY:      { source: secret_ref, name: my-api-key }   # via secret store
  REGION:       { source: literal, value: "us-east-1" }    # plain value
```

On the CLI: `--env KEY=env:VAR`, `--env KEY=secret:NAME`, `--env KEY=literal:value`.

Servers inherit a sanitized baseline environment only (plus `PATH`); pass
`--env` or use `secret_ref` for anything a server needs. Client formats that
cannot represent env references (Cline) get values resolved and inlined at
sync time, with an explicit warning. Secrets are redacted from logs, errors,
and `--debug` output.

### Tool naming

Public tool names are `{server}__{tool}` (server names cannot contain `__`).
Client adapters write each server as its own entry, so clients that don't
flatten tools see natural per-server names; the gateway flattens and routes.

## Client support

| Client | Config | Notes |
| --- | --- | --- |
| OpenCode | `opencode.json` or `$OPENCODE_CONFIG` | v1 (`mcp`) and v2 (`mcp.servers`) schemas, auto-detected; `--env CERBERUS_OPENCODE_SCHEMA=1\|2` to force |
| Claude Code | `~/.claude.json` | `command` + `args`; env values written as `${VAR}` references; disabled servers omitted (no flag exists) |
| Codex | `$CODEX_HOME/config.toml` or `~/.codex/` | TOML; `url` for HTTP; no env substitution — env refs resolved at sync |
| Gemini CLI | `~/.gemini/settings.json` | `httpUrl` for HTTP; env values as `${VAR}`; disabled servers omitted |
| Cline | `$CLINE_MCP_SETTINGS_PATH` or editor globalStorage | `type: stdio\|streamableHttp`; `autoApprove` and oauth blobs preserved; `timeout` in seconds |
| Aider | — | **Unsupported**: Aider has no native MCP support; reported by `doctor` |

## Gateway

```bash
cerberus gateway start                            # stdio (for clients that spawn it)
cerberus gateway start --transport http           # streamable HTTP on 127.0.0.1:8713
cerberus gateway start --transport http \
  --bind 0.0.0.0:8713 --allow-remote --token …    # remote requires a token
cerberus gateway status                           # is it running? which tools are routed?
cerberus gateway doctor                           # diagnose upstreams and configuration
cerberus gateway validate                         # check config without starting
cerberus gateway stop                             # stop a background instance
```

The gateway connects to upstreams on demand, aggregates their tools under
`{server}__{tool}` names, and enforces each server's `policy`
(`allow_tools` / `deny_tools` with trailing-`*` wildcards,
`require_confirmation`). Calls are limited by a concurrency semaphore and a
per-call timeout. It binds loopback only unless `--allow-remote` is combined
with an auth token.

## Commands

```
init                 Create an initial registry and data directories
config               path | validate
server               list | add | inspect | enable | disable | remove
client               list | detect | inspect
sync                 Sync registry into clients (dry-run unless --apply)
diff                 Show drift since last sync
backup               create | list | restore
profile              list | show | use | create | remove
gateway              start | status | doctor | validate | stop
doctor               Diagnose configuration, clients, and servers
health               Registry and server health
logs                 Recent diagnostic logs
version              Version and MCP protocol information
completions          bash | zsh | fish | elvish | powershell
```

Global flags: `--config FILE`, `--json`, `--quiet`, `--verbose`, `--debug`.

Exit codes: `0` success, `1` failure, `2` usage, `3` invalid config, `4`
conflict, `5` not found, `6` policy denied, `7` timeout, `8` unsupported.

## Environment overrides

| Variable | Effect |
| --- | --- |
| `CERBERUS_CONFIG_DIR` | Config directory (registry lives here) |
| `CERBERUS_DATA_DIR` | Data directory (backups, state, logs) |
| `CERBERUS_CONFIG_FILE` | Exact registry file path |
| `OPENCODE_CONFIG` | OpenCode config file (also used by OpenCode itself) |
| `CODEX_HOME` | Codex home (config.toml lives here) |
| `CLINE_MCP_SETTINGS_PATH` | Exact Cline settings path |
| `CERBERUS_OPENCODE_SCHEMA` | `1` or `2` to force OpenCode schema generation |

## Development

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

MSRV 1.88, edition 2024. Built on [rmcp](https://crates.io/crates/rmcp)
(implements MCP spec 2026-07-28). See [`docs/`](docs/) for architecture,
adapter notes, and gateway internals.

## License

MIT — see [LICENSE](LICENSE).
