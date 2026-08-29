pub mod analysis;
mod blocking;
pub mod commands;
pub mod error;
mod file_name;
pub mod ingestion;
mod integrity;
pub mod licensing;
mod normalization;
pub mod parsers;
pub mod provenance;
pub mod setup;
pub mod storage;
pub mod types;

use tauri::menu::{Menu, MenuBuilder, MenuItem, SubmenuBuilder};
use tauri::{generate_handler, AppHandle, Emitter, Manager, Runtime};

const NATIVE_COMMAND_EVENT: &str = "onrecord-native-command";

fn licensed_invoke_handler(invoke: tauri::ipc::Invoke<tauri::Wry>) -> bool {
    let command = invoke.message.command();
    if !licensing::command_is_unlicensed(command) {
        let state = invoke.message.state();
        let Some(license) = state.try_get::<licensing::LicenseGate>() else {
            invoke.resolver.reject(crate::error::AppError::License(
                "The native paid-license gate was not initialized.".to_string(),
            ));
            return true;
        };
        if let Err(error) = license.require_authorized() {
            invoke.resolver.reject(error);
            return true;
        }
    }
    let native_handler: fn(tauri::ipc::Invoke<tauri::Wry>) -> bool = generate_handler![
        commands::runtime::get_license_status,
        commands::runtime::refresh_license,
        commands::ingest::start_portal_sync,
        commands::ingest::pause_source_sync,
        commands::ingest::resume_source_sync,
        commands::ingest::remove_source,
        commands::ingest::preview_source_purge,
        commands::ingest::purge_source,
        commands::ingest::purge_document,
        commands::ingest::list_portal_runtimes,
        commands::ingest::list_saved_portals,
        commands::ingest::preview_data_source,
        commands::ingest::start_data_source,
        commands::ingest::ingest_local_files,
        commands::ingest::ingest_pasted_content,
        commands::corpus::query_corpus_patterns,
        commands::corpus::get_corpus_index_status,
        commands::query::query_audit_records,
        commands::export::export_legal_brief,
        commands::investigation::get_agency_performance_profiles,
        commands::investigation::get_cross_agency_correlations,
        commands::investigation::import_auxiliary_events,
        commands::investigation::register_statutory_extension,
        commands::investigation::import_legal_authority,
        commands::investigation::list_legal_authorities,
        commands::investigation::list_legal_authority_versions,
        commands::investigation::get_legal_authority_version,
        commands::investigation::evaluate_request_legal_compliance,
        commands::investigation::list_request_legal_findings,
        commands::investigation::review_legal_finding,
        commands::investigation::mask_sensitive_text,
        commands::investigation::verify_document_provenance,
        commands::investigation::list_document_provenance_verifications,
        commands::investigation::list_request_documents,
        commands::investigation::get_document_text_chunk,
        commands::investigation::verify_document_redaction_integrity,
        commands::investigation::carve_document_version_history,
        commands::investigation::audit_document_residual_evidence,
        commands::runtime::get_ocr_capability
    ];
    native_handler(invoke)
}

fn application_menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let application = SubmenuBuilder::new(app, "OnRecord Studio")
        .about(None)
        .separator()
        .quit()
        .build()?;
    let edit = SubmenuBuilder::new(app, "Edit")
        .undo()
        .redo()
        .separator()
        .cut()
        .copy()
        .paste()
        .select_all()
        .build()?;
    let overview = MenuItem::with_id(
        app,
        "workspace-overview",
        "Overview",
        true,
        Some("CmdOrCtrl+1"),
    )?;
    let patterns = MenuItem::with_id(
        app,
        "workspace-patterns",
        "Patterns",
        true,
        Some("CmdOrCtrl+2"),
    )?;
    let evidence = MenuItem::with_id(
        app,
        "workspace-evidence",
        "Evidence",
        true,
        Some("CmdOrCtrl+3"),
    )?;
    let activity = MenuItem::with_id(
        app,
        "workspace-activity",
        "Activity",
        true,
        Some("CmdOrCtrl+4"),
    )?;
    let add_source = MenuItem::with_id(
        app,
        "add-source",
        "Add Data Source…",
        true,
        Some("CmdOrCtrl+N"),
    )?;
    let command_palette = MenuItem::with_id(
        app,
        "command-palette",
        "Command Palette…",
        true,
        Some("CmdOrCtrl+K"),
    )?;
    let workspace = SubmenuBuilder::new(app, "Workspace")
        .items(&[&overview, &patterns, &evidence, &activity])
        .separator()
        .item(&add_source)
        .item(&command_palette)
        .build()?;
    let window = SubmenuBuilder::new(app, "Window")
        .minimize()
        .maximize()
        .separator()
        .close_window()
        .build()?;
    MenuBuilder::new(app)
        .items(&[&application, &edit, &workspace, &window])
        .build()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .menu(application_menu)
        .on_menu_event(|app, event| {
            let command = event.id().0.as_str();
            if command.starts_with("workspace-")
                || matches!(command, "add-source" | "command-palette")
            {
                let _ = app.emit(NATIVE_COMMAND_EVENT, command);
            }
        })
        .invoke_handler(licensed_invoke_handler)
        .setup(|app| {
            app.manage(licensing::LicenseGate::initialize());
            setup::configure_onrecord_studio_runtime(app)?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("OnRecord Studio terminated after an unrecoverable runtime error");
}
