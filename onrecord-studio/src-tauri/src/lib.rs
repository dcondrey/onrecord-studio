//! Primary structural library framework linking backend subcomponents.

pub mod error;
pub mod types;
pub mod storage;
pub mod ingestion;
pub mod parsers;
pub mod analysis;
pub mod commands;
pub mod setup;

use tauri::generate_handler;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(generate_handler![
            commands::start_subdomain_sync,
            commands::query_audit_records,
            commands::export_case_dataset
        ])
        .setup(|app| {
            setup::configure_onrecord_studio_runtime(app)?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Fatal Error: Unexpected breakdown during application initialization.");
}
