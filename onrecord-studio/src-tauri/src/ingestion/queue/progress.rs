//! Ingestion progress events and the sequenced envelope they are emitted in.

use super::*;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(
    tag = "type",
    content = "payload",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum IngestionProgress {
    PortalVerified {
        portal: PortalRegistration,
    },
    SyncStarted {
        portal_key: String,
        sync_run_id: String,
    },
    PageStarted {
        portal_key: String,
        page: usize,
    },
    PageCompleted {
        portal_key: String,
        page: usize,
        records_found: usize,
    },
    DocumentStarted {
        portal_key: String,
        document_id: u64,
        file_name: String,
    },
    DocumentExtracted {
        portal_key: String,
        document_id: u64,
        character_count: usize,
        was_ocr_processed: bool,
    },
    DocumentExpanded {
        portal_key: String,
        document_id: u64,
        format: String,
        entry_count: usize,
    },
    DocumentDeferred {
        portal_key: String,
        document_id: u64,
        format: String,
        reason: String,
    },
    DocumentSalvaged {
        portal_key: String,
        document_id: u64,
        format: String,
        provenance_marker: String,
        source_sha256: String,
        recovered_run_count: usize,
        decoder_summary: String,
        primary_error: String,
    },
    DocumentCompleted {
        portal_key: String,
        document_id: u64,
        bytes_saved: u64,
    },
    SyncStopped {
        portal_key: String,
        sync_run_id: String,
    },
    TaskFailed {
        portal_key: String,
        target_id: String,
        error_msg: String,
    },
    TaskRetryScheduled {
        portal_key: String,
        target_id: String,
        attempt: u64,
        delay_seconds: u64,
        error_msg: String,
    },
    SourceNeedsGuidance {
        portal_key: String,
        source_url: String,
        reason: String,
    },
    /// This run will not contact a host again. Emitted once per host, because the alternative is
    /// the failure stream the operator actually saw: the same refusal repeated per URL, with
    /// nothing saying the run had stopped being welcome. `requests_spent` bounds what the run
    /// managed to read from the host before it stopped.
    HostStopped {
        portal_key: String,
        host: String,
        requests_spent: usize,
        reason: String,
    },
    /// A pre-DuckDB core database was converted during startup. Emitted once, before the window
    /// is shown, so the investigator learns where the retained original was kept.
    LegacyCoreDatabaseMigrated {
        backup_path: String,
        copied_tables: u64,
        copied_rows: u64,
        tables_absent_from_legacy: Vec<String>,
        tables_dropped_from_legacy: Vec<String>,
    },
    CorpusReconciliationStarted {},
    CorpusReconciliationCompleted {
        indexed_documents: u64,
        skipped_documents: u64,
        failed_documents: u64,
        first_error: Option<String>,
    },
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestionTelemetryEnvelope {
    pub protocol_version: u16,
    pub sequence: String,
    pub emitted_at: String,
    #[serde(flatten)]
    pub event: IngestionProgress,
}

// NOTE(telemetry): This sequence is process-local deduplication metadata, not a durable event ID;
// consumers must expect it to restart with the native process.
static INGESTION_TELEMETRY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub fn ingestion_telemetry_envelope(event: IngestionProgress) -> IngestionTelemetryEnvelope {
    IngestionTelemetryEnvelope {
        protocol_version: 1,
        sequence: INGESTION_TELEMETRY_SEQUENCE
            .fetch_add(1, Ordering::AcqRel)
            .saturating_add(1)
            .to_string(),
        emitted_at: Utc::now().to_rfc3339(),
        event,
    }
}
