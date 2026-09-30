pub mod analysis;
mod atomic_write;
mod blocking;
pub mod bundle;
pub mod commands;
pub mod error;
mod file_name;
pub mod ingestion;
mod integrity;
pub mod licensing;
mod normalization;
pub mod parsers;
pub mod profile;
pub mod provenance;
pub mod report;
pub mod research;
pub mod setup;
pub mod storage;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
pub mod tracker;
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
        // Two independent verdicts: the managed gate, then the process-wide entitlement check.
        if let Err(error) = license
            .require_authorized()
            .and_then(|()| licensing::recheck())
        {
            invoke.resolver.reject(error);
            return true;
        }
    }
    let native_handler: fn(tauri::ipc::Invoke<tauri::Wry>) -> bool = generate_handler![
        commands::runtime::get_license_status,
        commands::runtime::refresh_license,
        commands::account::get_account_status,
        commands::account::begin_sign_in,
        commands::account::sign_out,
        commands::account::delete_account,
        commands::account::start_checkout,
        commands::account::activate_this_device,
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
        commands::corpus::list_top_corpus_entities,
        commands::corpus::get_corpus_entity_neighborhood,
        commands::corpus::get_document_entity_truncations,
        commands::corpus::find_entity_paths,
        commands::corpus::list_entity_brokers,
        commands::semantic::get_embedding_model_status,
        commands::semantic::initialize_embedding_model,
        commands::semantic::get_embedding_index_status,
        commands::semantic::rebuild_embedding_index,
        commands::semantic::cancel_embedding_index,
        commands::semantic::hybrid_search,
        commands::analysis_workbench::import_analytical_dataset,
        commands::analysis_workbench::list_analytical_import_candidates,
        commands::analysis_workbench::get_analytical_schema,
        commands::maintenance::get_data_maintenance_status,
        commands::maintenance::set_data_maintenance_policy,
        commands::maintenance::run_data_maintenance,
        commands::reconstruction::reconstruct_document_context,
        commands::financial::get_financial_workspace,
        commands::financial::save_financial_mapping,
        commands::financial::run_financial_investigation,
        commands::financial::evaluate_financial_scenario,
        commands::facilities::get_facility_coverage,
        commands::facilities::save_facility_coverage_mapping,
        commands::casework::get_casework_state,
        commands::casework::create_casework_case,
        commands::casework::save_casework_case,
        commands::casework::export_casework_packet,
        commands::casework::preview_casework_import,
        commands::casework::import_casework_packet,
        commands::casework::compromised_case_imports,
        commands::monitoring::get_monitoring_status,
        commands::monitoring::refresh_monitoring_now,
        commands::monitoring::save_monitoring_coverage,
        commands::monitoring::save_monitoring_observation,
        commands::watchlists::list_watches,
        commands::watchlists::save_watch,
        commands::watchlists::delete_watch,
        commands::watchlists::check_watches,
        commands::watchlists::list_watch_changes,
        commands::meetings::get_meeting_calendar,
        commands::review::get_review_workspace,
        commands::review::save_finding_review,
        commands::review::replay_review_evidence,
        commands::review::save_public_relationship,
        commands::review::get_investigation_evaluations,
        commands::review::save_pilot_assessment,
        commands::research::get_research_status,
        commands::research::set_research_policy,
        commands::research::run_research_now,
        commands::research::set_research_action_state,
        commands::analysis_workbench::run_analytical_sql,
        commands::analysis_workbench::export_analytical_sql_csv,
        commands::query::query_audit_records,
        commands::export::export_legal_brief,
        commands::export::export_case_evidence_package,
        commands::export::case_export_chain,
        commands::export::rotate_export_signing_key,
        commands::export::mark_signing_key_compromised,
        commands::export::get_signing_key_revocations,
        commands::export::verify_evidence_package,
        commands::investigation::get_agency_performance_profiles,
        commands::investigation::get_cross_agency_correlations,
        commands::investigation::import_auxiliary_events,
        commands::investigation::register_statutory_extension,
        commands::investigation::get_request_statutory_extension,
        commands::investigation::import_legal_authority,
        commands::investigation::list_legal_authorities,
        commands::investigation::list_legal_authority_versions,
        commands::investigation::get_legal_authority_version,
        commands::investigation::evaluate_request_legal_compliance,
        commands::investigation::list_request_legal_findings,
        commands::investigation::review_legal_finding,
        commands::investigation::verify_document_provenance,
        commands::investigation::list_document_provenance_verifications,
        commands::investigation::list_request_documents,
        commands::investigation::get_document_text_chunk,
        commands::investigation::verify_document_redaction_integrity,
        commands::investigation::carve_document_version_history,
        commands::investigation::audit_document_residual_evidence,
        commands::runtime::get_ocr_capability,
        commands::findings::refresh_findings,
        commands::findings::list_evidence_gaps,
        commands::claim_checks::list_document_claim_checks,
        commands::claim_checks::export_claim_query_pack,
        commands::story_desk::list_story_leads,
        commands::lead_dossier::build_lead_dossier,
        commands::findings::list_findings,
        commands::findings::count_findings,
        commands::findings::dismiss_finding,
        commands::topics::list_topic_packs,
        commands::starter_sets::list_starter_sets,
        commands::intent::run_intent_query,
        commands::redaction_inference::infer_redaction,
        commands::artifacts::draft_followup_from_finding,
        commands::artifacts::draft_appeal_from_finding,
        commands::artifacts::draft_availability_for_finding,
        commands::artifacts::draft_appeal_from_statutory_deadline,
        commands::report::export_verifiable_report,
        commands::report::export_evidence_packet,
        commands::bundle::preview_shareable_bundle,
        commands::bundle::export_shareable_bundle,
        commands::profile::get_profile,
        commands::profile::save_profile,
        commands::profile::delete_profile,
        commands::profile::list_affiliation_matches,
        commands::profile::confirm_affiliation_match,
        commands::profile::reject_affiliation_match,
        commands::profile::get_affiliation_disclosure,
        commands::verification::request_work_email_code,
        commands::verification::confirm_work_email_code,
        commands::verification::link_affiliation_verified_domain,
        commands::tracker::list_claimed_requests,
        commands::tracker::claim_request,
        commands::tracker::update_claimed_request,
        commands::tracker::remove_claimed_request,
        commands::tracker::add_timeline_event,
        commands::tracker::remove_timeline_event,
        commands::tracker::import_request_emails,
        commands::tracker::confirm_suggested_event,
        commands::tracker::reject_suggested_event,
        commands::tracker::get_request_deadline
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
        Some("CmdOrCtrl+4"),
    )?;
    let analysis = MenuItem::with_id(
        app,
        "workspace-analysis",
        "Analysis",
        true,
        Some("CmdOrCtrl+3"),
    )?;
    let activity = MenuItem::with_id(
        app,
        "workspace-activity",
        "Activity",
        true,
        Some("CmdOrCtrl+5"),
    )?;
    let briefing = MenuItem::with_id(
        app,
        "workspace-briefing",
        "Briefing",
        true,
        Some("CmdOrCtrl+6"),
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
        .items(&[
            &overview, &patterns, &analysis, &evidence, &activity, &briefing,
        ])
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
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init());
    // Registered only in the Developer ID build; the App Store build must never self-update.
    #[cfg(feature = "direct-license")]
    let builder = builder
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init());
    builder
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
