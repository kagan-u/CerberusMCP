# The registry

The registry is the canonical, client-independent list of MCP servers: one
YAML file, `schema_version: 1`, validated on load and on save. It is the only
state CerberusMCP owns — client configs are *derived* from it on `sync`, and
the gateway *reads* it. Nothing writes back into the registry from either.

| Path | Purpose |
| --- | --- |
| `CERBERUS_CONFIG_DIR/cerberus.yaml` | the registry |
| `CERBERUS_CONFIG_FILE` | exact registry file path (overrides the dir) |
| `CERBERUS_DATA_DIR` | backups, sync state, logs, registry search cache |
| `CERBERUS_REGISTRY_URL` | registry endpoint used by `search`/`install` |

The machine-readable contract lives in
[`schemas/registry.schema.json`](../schemas/registry.schema.json); the
annotated example is [`examples/registry.yaml`](../examples/registry.yaml).

## Document shape

```yaml
schema_version: 1

settings:
  default_mode: direct          # direct | gateway

servers:
  <name>:                       # [a-z0-9-], no `__`, ≤ 64 chars
    enabled: true
    transport:                  # exactly one of stdio | http
      type: stdio
      command: npx
      args: ["-y", "@modelcontextprotocol/server-filesystem"]
      env: { KEY: { source: env, name: KEY } }   # env | secret | literal
    mode: gateway               # optional override of settings.default_mode
    tags: [local]
    timeout_ms: 30000
    startup_timeout_ms: 15000
    policy: { deny_tools: ["delete_*"] }
    description: ...
    source: { ... }             # install provenance; see below

profiles:
  <name>: { servers: [a, b] }
active_profile: <name>
```

Validation is strict: unknown fields are rejected (`deny_unknown_fields`)
everywhere, so a typo fails at load time instead of being silently ignored.

## The `source` block

Install provenance. It records where an entry came from so `cerberus install`
can re-create it and `cerberus doctor` can verify it. **The gateway and every
client adapter ignore it** — routing, policy, and sync read only `transport`,
`policy`, `enabled`, `mode`, and `tags`.

```yaml
source:
  kind: npm                     # npm | pypi | git | binary
  package: "@modelcontextprotocol/server-filesystem"
  version: "0.6.2"              # pin or requirement
  runner: npx                   # how it is launched (npx, uvx, pnpm dlx, ...)
  args: ["-y"]                  # runner args before the package name
  homepage: https://github.com/modelcontextprotocol/servers
  url: https://github.com/org/repo   # required for kind: git
  ref: v1.2.3                   # optional git ref
  env:
    - name: API_KEY
      required: true
      secret: true
      description: ...
```

Per-kind requirements, enforced by `config validate`:

| `kind` | Required | Notes |
| --- | --- | --- |
| `npm` | `package` | scoped names (`@scope/name`) allowed |
| `pypi` | `package` | PEP 508-ish: letters, digits, `. _ -` |
| `git` | `url` | `http(s)://`, `ssh://`, `git://`, `file://`, or `git@host:path` |
| `binary` | `package` | executable name; `transport.command` still governs launch |

`env` entries are names only — the launch binding itself lives on
`transport.env` (`env:VAR` pass-through, `secret:NAME`, or a literal).

## `search` and `install`

Both talk to the official MCP Registry:

```text
GET {base}/servers?search=<query>&limit=<n>&cursor=<cursor>
→ {"servers":[{"server":{...}}],"metadata":{"nextCursor":"...","count":n}}
```

### `cerberus search <query> [--limit N] [--offline] [--json]`

Results are cached under the data directory (`registry-cache.json`), so
`--offline` replays the last successful search for that query — with no cache
entry it exits `5` and tells you to run it online once.

Two payload quirks are handled for you:

* **One row per published version.** The same server appears as several rows;
  results are deduplicated keeping the newest version (numeric comparison, so
  `1.2.10` beats `1.2.9`).
* **One package per transport variant.** A server may publish `stdio`, `sse`,
  and `streamable-http` packages for the *same* identifier. Only the stdio one
  can be launched as a child process, so it is preferred.

### `cerberus install <name|--file.json> [flags]`

```bash
cerberus install com.modelcontextprotocol/server-filesystem   # derive local name
cerberus install com.example/srv --name my-srv --tag dev      # explicit name/tags
cerberus install com.example/srv --check                      # launcher must be on PATH
cerberus install com.example/srv --dry-run                    # print the entry, write nothing
cerberus install com.example/srv --disabled                   # register but leave disabled
cerberus install ./saved-record.json --name from-file         # offline: a saved record
```

`<source>` is a registry name when it is not a path to an existing JSON file;
a file may be either a bare `server` object or a `{"server": {...}}` envelope.

Install is deliberately limited to **write + verify**:

| Step | Behavior |
| --- | --- |
| resolve | exact registry name (search + exact match), or read a local JSON file |
| materialize | `stdio` package → hosted endpoint → non-stdio package (warned) |
| name | last path segment of the reverse-DNS name, slugified (`com.x/server-fs` → `server-fs`); `--name` overrides |
| conflict | existing name → exit `4`, same as `server add` |
| `--check` | launcher (`npx`, `uvx`, ...) must resolve on `PATH`; on Windows `PATHEXT` is honored |
| required args | a `named` argument with no value and no default cannot be invented → **warning**, pass it yourself with `server add --arg` or by editing the registry |
| required env | recorded as `transport.env` pass-through bindings; unset ones are warned about |
| write | registry only — client configs are untouched until `cerberus sync --apply` |

Unsupported packaging systems (`nuget`, `mcpb`, `docker`, ...) fail with exit
code `8` rather than being installed with a launcher this build cannot verify.

Exit codes for this flow: `0` ok, `3` invalid config, `4` name conflict,
`5` unknown registry name / no offline cache, `8` unsupported package type.

## Adding a server by hand

`cerberus server add` covers servers that are not in any registry:

```bash
cerberus server add gh \
  --command npx --arg=-y --arg=@modelcontextprotocol/server-github \
  --env GITHUB_TOKEN=env:GITHUB_TOKEN --tag vcs
```

The entry starts with no `source` block; add one if you want `doctor` to
track its environment requirements. Every path — `add`, `install`, direct YAML
edits — ends at the same validation, so a hand-written registry and an
installed one are indistinguishable to the rest of the system.
