# Client adapters

Each adapter implements `ClientAdapter`: `detect`, `inspect`, `plan_sync`,
`apply_sync`. Adapters share classification (`classify_entry`), removal
planning (`plan_removals`), and atomic JSON/TOML writers from
`adapters/mod.rs`.

## Format quirks that drive the design

| Client | Representation | Env refs | Disabled flag | Merge care |
| --- | --- | --- | --- | --- |
| OpenCode v1 | `mcp.<name>` object | none (inline) | `enabled: false` | preserve unrelated `mcp` keys |
| OpenCode v2 | `mcp.servers.<name>` | none | `disabled: true` | preserve oauth blobs |
| Claude Code | `mcpServers.<name>`, `command` string + `args[]` | `${VAR}` in values | flagless → omit entry | top-level keys beside `mcpServers` |
| Codex | `[mcp_servers.<name>]` TOML | none → resolve at sync | `enabled = false` | preserve every other TOML table |
| Gemini CLI | `mcpServers.<name>`, `httpUrl` for HTTP | `${VAR}` | flagless → omit entry | preserve `theme`, `selectedAuthType`, … |
| Cline | `mcpServers.<name>`, `type: stdio\|streamableHttp` | none → resolve + warn | `disabled: true` | **must** preserve `autoApprove`, oauth/metadata |

Key consequences:

- **Flagless clients omit disabled servers.** Writing `enabled: false` is
  impossible for Claude Code/Gemini, so sync removes the entry instead. On
  OpenCode/Codex/Cline the entry stays with an explicit disabled flag.
- **Env substitution differs per client.** Claude Code and Gemini expand
  `${VAR}` natively, so we write references. Codex and Cline have no
  substitution; the adapter resolves `env`/`secret_ref` values at sync time
  and inlines them, warning that plaintext lands in the config file.
- **Cline timeouts are seconds**, not milliseconds.
- **OpenCode schema generation is detected** from the existing file
  (`mcp.servers` ⇒ v2, else v1) and can be forced with
  `CERBERUS_OPENCODE_SCHEMA`.

## Detection

`detect` reports whether a client's config (or its directory, for Cline's
editor globalStorage) exists. It never creates files. `doctor` aggregates
detections and prints limitations for unsupported clients (Aider) instead of
failing.

## Testing

Unit tests live in each adapter module and use `CerberusPaths::with_roots` +
client-specific env overrides (`OPENCODE_CONFIG`, `CODEX_HOME`,
`CLINE_MCP_SETTINGS_PATH`, `HOME`) so they never touch real configs. Tests
that mutate process-global env vars serialize on
`adapters::test_lock::env_lock()`.
