pub mod analysis;
mod blocking;
pub mod commands;
pub mod error;
mod file_name;
pub mod ingestion;
mod integrity;
mod normalization;
pub mod parsers;
pub mod provenance;
pub mod setup;
pub mod storage;
pub mod types;

use tauri::generate_handler;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(generate_handler![
            commands::ingest::pause_source_sync,
            commands::ingest::resume_source_sync,
            commands::ingest::remove_source,
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
            commands::investigation::import_legal_authority,
            commands::investigation::list_legal_authorities,
            commands::investigation::evaluate_request_legal_compliance,
            commands::investigation::list_request_legal_findings,
            commands::investigation::review_legal_finding,
            commands::investigation::verify_document_provenance,
            commands::investigation::list_document_provenance_verifications,
            commands::investigation::list_request_documents,
            commands::investigation::get_document_text_chunk,
            commands::investigation::verify_document_redaction_integrity,
            commands::investigation::carve_document_version_history
        ])
        .setup(|app| {
            setup::configure_onrecord_studio_runtime(app)?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("OnRecord Studio terminated after an unrecoverable runtime error");
}
