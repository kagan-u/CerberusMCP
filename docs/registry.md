# The registry

The registry is the canonical, client-independent list of MCP servers: one
TOML file, `conf.cerberus`, `schema_version = 2`, validated on load and on
save. It is the only state CerberusMCP owns — client configs are *derived*
from it on `sync`, and the gateway *reads* it. Nothing writes back into the
registry from either.

| Path | Purpose |
| --- | --- |
| `CERBERUS_CONFIG_DIR/conf.cerberus` | the registry (TOML) |
| `CERBERUS_CONFIG_FILE` | exact registry file path (overrides the dir) |
| `CERBERUS_DATA_DIR` | backups, sync state, logs, registry search cache, compiled registry cache |
| `CERBERUS_REGISTRY_URL` | registry endpoint used by `search`/`install` |

**Legacy installs:** an existing `cerberus.yaml` (`schema_version: 0/1`) is
migrated automatically on first access — the content is parsed and upgraded,
written to `conf.cerberus`, and the original file is moved into
`<data>/backups/migrations/` alongside a pre-migration copy. An explicit
`CERBERUS_CONFIG_FILE`/`--config` path is never auto-migrated; reading it
works for both formats (detection is content-based), and the format of new
writes follows the file extension (`.yaml`/`.yml` → YAML, anything else →
TOML).

The machine-readable contract lives in
[`schemas/registry.schema.json`](../schemas/registry.schema.json); the
annotated example is [`examples/conf.cerberus`](../examples/conf.cerberus).

## Document shape

```toml
schema_version = 2
active_profile = "dev"           # optional default profile

[settings]
default_mode = "direct"          # direct | gateway

[servers.<name>]                 # [a-z0-9-], no `__`, ≤ 64 chars
enabled = true
tags = ["local"]
timeout_ms = 30000
startup_timeout_ms = 15000
description = "..."
pre_launch = [{ use = "bootstrap" }]   # see "Commands and pre-launch hooks"

[servers.<name>.transport]       # exactly one of stdio | http
type = "stdio"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem"]
env = { KEY = { source = "env", name = "KEY" } }   # env | secret | literal

[servers.<name>.policy]
deny_tools = ["delete_*"]

profiles = {}                    # see below: [profiles.<name>]
```

Profiles are tables of server lists:

```toml
[profiles.dev]
servers = ["a", "b"]
description = "everything for local development"
```

Validation is strict: unknown fields are rejected (`deny_unknown_fields`)
everywhere, so a typo fails at load time instead of being silently ignored.

## Compiled cache

Parsing the TOML document happens once; the result is also compiled into a
**derived binary snapshot** (rkyv) at
`<data>/cache/registry-<source-path-hash>.cbin` so subsequent loads skip text
parsing entirely.

- **Derived only.** The cache is never an input of record — delete it at any
  time and the next load rebuilds it. `cerberus config compile` builds it
  explicitly and prints what was written (`--json` reports sizes, FNV-1a
  fingerprints, the schema version, and the cache revision); `cerberus config
  path` reports the cache directory.
- **Never masks errors.** Only registries that pass validation are compiled,
  so a cache hit cannot make an invalid configuration look valid.
- **Invalidation is layered** — every guard must pass before the snapshot is
  used: the container header (magic `CBIN`, schema version, cache revision,
  build version), the source stat (mtime + length), an FNV-1a hash of the
  source content, an FNV-1a hash of the payload, and finally rkyv/bytecheck
  validation of the archived graph. Any failure — including a truncated or
  corrupt cache file — falls back silently to parsing the document.
- **`CERBERUS_NO_CACHE=1`** disables both reads and writes. The explicit
  `config compile` subcommand still builds, since producing the file is the
  point of that command.

## Commands and pre-launch hooks

`[commands.<name>]` defines reusable commands — plain exec vectors, never an
implicit shell (a runner that needs shell semantics spells it out:
`run = ["sh", "-c", "..."]`):

```toml
[commands.bootstrap]
run = ["./scripts/env.sh", "--profile", "dev"]
description = "prepare the local environment"
cwd = "~/src/my-project"
timeout_ms = 10000
env = { REGION = { source = "env", name = "AWS_REGION" } }
```

Run one with `cerberus run <name> [-- <extra args...>]`.

A stdio server can declare steps that must succeed before it is spawned:

```toml
[servers.mcp]
pre_launch = [
  { use = "bootstrap" },                          # reference a [commands.*] entry
  { run = ["docker", "compose", "up", "-d"], timeout_ms = 60000 },
  { run = ["true"], continue_on_error = true },   # failure logs, connect continues
]

[servers.mcp.transport]
type = "stdio"
command = "npx"
```

Rules (enforced by `config validate`):

* each step sets exactly one of `use` or `run`;
* `use` must name an existing `[commands.<name>]` entry;
* `pre_launch` is only valid on `stdio` transports — the gateway spawns
  those processes itself; direct-mode client adapters cannot express hooks
  and ignore the block (the gateway is where hooks run);
* per-step `timeout_ms` (default 30 s), `cwd`, `env`, and
  `continue_on_error` behave like their command counterparts.

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
