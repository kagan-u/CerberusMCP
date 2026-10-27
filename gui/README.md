# CerberusMCP GUI

Tauri 2 + React + TypeScript desktop app for [CerberusMCP](../). It embeds
the `cerberus-mcp` library directly: the same registry
(`conf.cerberus`), the same gateway, no shelling out.

## Develop

```sh
npm install
npm run tauri dev
```

## Build

```sh
npm run tauri build
```

Environment overrides (`CERBERUS_CONFIG_DIR`, `CERBERUS_DATA_DIR`) are
honored, so the GUI works against the same registry as the `cerberus` CLI.

## Layout

| Path | Purpose |
| --- | --- |
| `src/` | React app: `pages/` (Dashboard, Servers, Registry, Diagnostics, Logs), `components/ServerForm.tsx`, `api.ts` (invoke wrappers), `types.ts` (registry schema types) |
| `src-tauri/src/commands.rs` | Tauri commands: registry CRUD, validate/doctor, gateway start/stop/status |
| `src-tauri/src/logs.rs` | `tracing` → in-memory buffer + `log-line` events |
