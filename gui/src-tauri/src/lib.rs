//! CerberusMCP desktop app (Tauri 2 + React).
//!
//! The GUI embeds the `cerberus-mcp` library directly: the registry editor
//! drives the same load/validate/save chain as the CLI, and the gateway runs
//! in-process over HTTP with its lifecycle exposed to the frontend.

mod commands;
mod logs;

use std::sync::Arc;

use tauri::Manager;

use cerberus_mcp::paths::CerberusPaths;

/// Shared app state (Tauri managed state).
pub struct AppCtx {
    pub paths: CerberusPaths,
    pub logs: Arc<logs::LogStore>,
    pub gateway: Arc<commands::ManagedGateway>,
    /// Serializes load → mutate → save on the registry.
    pub registry_lock: tokio::sync::Mutex<()>,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Bridge tracing into the Logs page before anything logs.
    let log_store = Arc::new(logs::LogStore::default());
    logs::init(log_store.clone());

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(move |app| {
            let paths = CerberusPaths::resolve_with_override(None)
                .map_err(|err| Box::<dyn std::error::Error>::from(err.to_string()))?;
            paths
                .ensure_dirs()
                .map_err(|err| Box::<dyn std::error::Error>::from(err.to_string()))?;
            log_store.set_app(app.handle().clone());
            app.manage(AppCtx {
                paths,
                logs: log_store.clone(),
                gateway: Arc::new(commands::ManagedGateway::default()),
                registry_lock: tokio::sync::Mutex::new(()),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::get_registry,
            commands::init_registry,
            commands::validate_registry,
            commands::run_doctor,
            commands::upsert_server,
            commands::delete_server,
            commands::toggle_server,
            commands::upsert_profile,
            commands::delete_profile,
            commands::set_active_profile,
            commands::set_settings,
            commands::set_commands,
            commands::get_logs,
            commands::clear_logs,
            commands::gateway_start,
            commands::gateway_stop,
            commands::gateway_status,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
