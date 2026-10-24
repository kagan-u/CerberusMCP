# Architecture

```
src/
├── cli/          clap definitions, output helpers (JSON vs human)
├── commands.rs   command implementations (the only layer that talks to Ctx)
├── registry/     schema v1 types, validation, migrations, atomic store
├── adapters/     per-client translation: plan → classify → apply
├── sync/         orchestrates adapters across clients + profiles
├── policy/       tool allow/deny evaluation (wildcard matching)
├── secrets/      env / secret_ref / literal resolution + redaction
├── process/      sanitized environment construction, launch, probes
├── routing/      {server}__{tool} public names, collision detection
├── gateway/      MCP server (rmcp) that fronts upstreams with policy
├── backup/       content-addressed backups + restore
├── diagnostics/  doctor / health / logs aggregation
└── paths.rs      OS-appropriate dirs, env overrides
```

## Sync pipeline

Sync is plan/apply, never "write and hope":

1. **Plan** (`ClientAdapter::plan_sync`): read the client's current config,
   build the desired entry for each selected server (translating transports,
   env references, flags), merge fields we must preserve (e.g. Cline
   `autoApprove`), then classify each name:
   `Add` / `Update` / `Unchanged` / `Remove` / `Conflict`.
2. **Classify** compares three hashes: the current entry, the entry we last
   wrote (recorded in `state/managed.json`), and the desired entry. A current
   entry that differs from what we last wrote is a hand edit → `Conflict`
   unless `--force`.
3. **Apply** re-resolves secrets, writes the file atomically (temp file +
   rename in the same directory, refusing to follow symlinks), records new
   hashes, and stores a backup of the previous content.

Dry-run (the default for `sync`) stops after step 1 and prints the plan.

## Registry store

`RegistryStore` keeps the parsed registry plus a content hash of the file it
was loaded from. Saves re-hash and fail with a "concurrent modification"
error if the file changed underneath us, then write atomically. Migrations
(`registry/migrate.rs`) upgrade legacy layouts (e.g. string-form servers) to
schema v1 on load; a backup is taken when a migration rewrites the file.

## Environment policy

`process::build_env` starts from a small allowlist (`PATH`, `HOME`, `TMPDIR`,
locale basics) and adds only what a server declares. This keeps
credentials from the parent shell out of child processes by default.
`EnvValue::Env` defers resolution to launch time; `secret_ref` goes through
the secret store; `literal` is for non-sensitive knobs.

## Error/exit contract

`CerberusError` carries a machine-checkable `exit_code()`. `main` maps it to
the process exit code, prints the error (redacted) to stderr, and never
writes diagnostics to stdout — stdout is reserved for command output and
`--json` payloads, so pipelines stay clean.
