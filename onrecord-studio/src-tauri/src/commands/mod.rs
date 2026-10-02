//! Registration system formatting IPC pathways for frontend consumption.

pub mod account;
pub mod analysis_workbench;
pub mod annotations;
pub mod collaboration;
pub mod corpus;
pub mod export;
pub mod ingest;
pub mod investigation;
pub mod maintenance;
pub mod original_file;
pub mod paging;
pub mod query;
pub mod reconstruction;
pub mod registry;
pub mod research;
pub mod resolution;
pub mod runtime;
pub mod semantic;
pub mod snapshot;
pub mod standing_search;
pub(crate) mod validation;

pub use analysis_workbench::{
    export_analytical_sql_csv, get_analytical_schema, import_analytical_dataset, run_analytical_sql,
};
pub use corpus::{
    find_entity_paths, get_corpus_entity_neighborhood, get_corpus_index_status,
    get_document_entity_truncations, list_entity_brokers, list_top_corpus_entities,
    query_corpus_patterns,
};
pub use export::{
    case_export_chain, export_case_evidence_package, export_legal_brief,
    get_signing_key_revocations, mark_signing_key_compromised, rotate_export_signing_key,
    verify_evidence_package,
};
pub use ingest::{
    ingest_local_files, ingest_pasted_content, list_portal_runtimes, list_saved_portals,
    pause_source_sync, preview_data_source, preview_source_purge, purge_document, purge_source,
    remove_source, resume_source_sync, start_data_source,
};
pub use investigation::{
    audit_document_residual_evidence, carve_document_version_history,
    evaluate_request_legal_compliance, get_agency_performance_profiles,
    get_cross_agency_correlations, get_document_text_chunk, get_legal_authority_version,
    get_request_statutory_extension, import_auxiliary_events, import_legal_authority,
    list_document_provenance_verifications, list_legal_authorities, list_legal_authority_versions,
    list_request_documents, list_request_legal_findings, register_statutory_extension,
    review_legal_finding, verify_document_provenance, verify_document_redaction_integrity,
};
pub use query::query_audit_records;
pub use runtime::get_ocr_capability;
pub use semantic::{
    cancel_embedding_index, get_embedding_index_status, get_embedding_model_status, hybrid_search,
    initialize_embedding_model, rebuild_embedding_index,
};

pub mod review;

pub mod financial;

pub mod facilities;

pub mod casework;

pub mod monitoring;
pub mod watchlists;

pub mod meetings;

pub mod claim_checks;
pub mod findings;
pub mod intent;
pub mod lead_dossier;
pub mod redaction_inference;
pub mod story_desk;

pub mod artifacts;

pub mod profile;

pub mod report;

pub mod tracker;

pub mod verification;

pub mod topics;

pub mod starter_sets;

pub mod bundle;
