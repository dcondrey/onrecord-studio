//! Registration system formatting IPC pathways for frontend consumption.

pub mod corpus;
pub mod export;
pub mod ingest;
pub mod investigation;
pub mod query;
pub mod runtime;
pub(crate) mod validation;

pub use corpus::{get_corpus_index_status, query_corpus_patterns};
pub use export::export_legal_brief;
pub use ingest::{
    ingest_local_files, ingest_pasted_content, list_portal_runtimes, list_saved_portals,
    pause_source_sync, preview_data_source, preview_source_purge, purge_document, purge_source,
    remove_source, resume_source_sync, start_data_source, start_portal_sync,
};
pub use investigation::{
    audit_document_residual_evidence, carve_document_version_history,
    evaluate_request_legal_compliance, get_agency_performance_profiles,
    get_cross_agency_correlations, get_document_text_chunk, get_legal_authority_version,
    get_request_statutory_extension, import_auxiliary_events, import_legal_authority,
    list_document_provenance_verifications, list_legal_authorities, list_legal_authority_versions,
    list_request_documents, list_request_legal_findings, mask_sensitive_text,
    register_statutory_extension, review_legal_finding, verify_document_provenance,
    verify_document_redaction_integrity,
};
pub use query::query_audit_records;
pub use runtime::get_ocr_capability;
