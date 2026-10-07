-- ============================================================================
-- ONRECORD STUDIO - INITIAL SCHEMA MIGRATION
-- Target Database: DuckDB (persistent single-process analytical store)
-- Description: Core schema mapping for municipal portals, requests, 
--              and deep offline document extraction layers.
-- ============================================================================

-- DuckDB does not support SQLite's PRAGMA or cascading foreign-key actions. Referential
-- integrity is enforced by DuckDB constraints and repository-level delete/update boundaries.
-- `verify_logical_relations` audits every intentionally unconstrained relation at startup and
-- refuses a corrupt core with per-relation orphan counts rather than silently repairing evidence.

CREATE SEQUENCE IF NOT EXISTS source_ingestion_failure_seq;
CREATE SEQUENCE IF NOT EXISTS ingestion_frontier_seq;
CREATE SEQUENCE IF NOT EXISTS document_acquisition_seq;
CREATE SEQUENCE IF NOT EXISTS document_source_check_seq;
CREATE SEQUENCE IF NOT EXISTS document_integrity_flag_seq;
CREATE SEQUENCE IF NOT EXISTS state_transition_log_seq;
CREATE SEQUENCE IF NOT EXISTS provenance_assertion_seq;
CREATE SEQUENCE IF NOT EXISTS auxiliary_event_seq;

-- ----------------------------------------------------------------------------
-- 1. PORTAL MANIFESTS & AGENCIES
-- Track target municipal subdomains and their internal agency breakdown structures.
-- ----------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS portals (
    subdomain TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    base_url TEXT NOT NULL,
    origin_url TEXT NOT NULL DEFAULT '',
    requests_endpoint TEXT NOT NULL DEFAULT '',
    requests_pointer TEXT NOT NULL DEFAULT '/requests',
    metadata_pointer TEXT,
    page_parameter TEXT NOT NULL DEFAULT 'page',
    page_size_parameter TEXT NOT NULL DEFAULT 'per_page',
    verified_at TEXT,
    health_status TEXT NOT NULL DEFAULT 'verified',
    last_error TEXT,
    last_synced_at TEXT,
    source_kind TEXT NOT NULL DEFAULT 'NEXT_REQUEST',
    lifecycle_state TEXT NOT NULL DEFAULT 'ACTIVE'
        CHECK (lifecycle_state IN ('ACTIVE', 'PAUSED', 'REMOVED')),
    created_at TEXT DEFAULT strftime(now(), '%Y-%m-%dT%H:%M:%S.%fZ')
);

CREATE TABLE IF NOT EXISTS portal_sync_runs (
    run_id TEXT PRIMARY KEY,
    portal_subdomain TEXT NOT NULL,
    started_at TEXT NOT NULL,
    completed_at TEXT,
    status TEXT NOT NULL,
    pages_processed BIGINT NOT NULL DEFAULT 0,
    crawl_pages_started BIGINT NOT NULL DEFAULT 0 CHECK (crawl_pages_started >= 0),
    resources_started BIGINT NOT NULL DEFAULT 0 CHECK (resources_started >= 0),
    requests_processed BIGINT NOT NULL DEFAULT 0,
    documents_processed BIGINT NOT NULL DEFAULT 0,
    failures BIGINT NOT NULL DEFAULT 0,
    FOREIGN KEY (portal_subdomain) REFERENCES portals(subdomain)
);

CREATE TABLE IF NOT EXISTS source_connector_recipes (
    source_key TEXT PRIMARY KEY,
    root_url TEXT NOT NULL UNIQUE,
    -- The URL the operator submitted, normalized but not resolved. It differs from `root_url`
    -- exactly when the submission redirected, and it is the only key `start_data_source` can
    -- offer, because that reader runs before the fetch that would reveal the final URL.
    -- Nullable with no DEFAULT: a row written before this column recorded no submission, and
    -- NULL is the only way to say that. Deliberately not UNIQUE -- one pasted string may
    -- legitimately have resolved to different canonical URLs over time, and the reader takes the
    -- most recently updated match.
    submitted_url TEXT,
    recipe_version BIGINT NOT NULL DEFAULT 2,
    publisher_pattern TEXT,
    reuse_approved BIGINT NOT NULL DEFAULT 0 CHECK (reuse_approved IN (0, 1)),
    crawl_policy_json TEXT NOT NULL,
    guidance_json TEXT NOT NULL,
    -- Nullable with no DEFAULT on purpose: 1 and 50 are legal window values, so a DEFAULT of the
    -- built-in window could not be told apart from an operator who chose it. NULL says the
    -- operator never set one, which is what resolves to the built-in window at start and resume.
    metadata_page_window_json TEXT,
    updated_at TEXT NOT NULL DEFAULT strftime(now(), '%Y-%m-%dT%H:%M:%S.%fZ'),
    FOREIGN KEY (source_key) REFERENCES portals(subdomain)
);

CREATE TABLE IF NOT EXISTS source_ingestion_failures (
    failure_id BIGINT PRIMARY KEY DEFAULT nextval('source_ingestion_failure_seq'),
    run_id TEXT NOT NULL,
    source_key TEXT NOT NULL,
    target_id TEXT NOT NULL,
    error_type TEXT NOT NULL,
    error_message TEXT NOT NULL,
    retryable BIGINT NOT NULL DEFAULT 0 CHECK (retryable IN (0, 1)),
    repairable BIGINT NOT NULL DEFAULT 0 CHECK (repairable IN (0, 1)),
    occurrences BIGINT NOT NULL DEFAULT 1,
    first_occurred_at TEXT NOT NULL DEFAULT strftime(now(), '%Y-%m-%dT%H:%M:%S.%fZ'),
    last_occurred_at TEXT NOT NULL DEFAULT strftime(now(), '%Y-%m-%dT%H:%M:%S.%fZ'),
    UNIQUE (run_id, target_id, error_type),
    FOREIGN KEY (run_id) REFERENCES portal_sync_runs(run_id),
    FOREIGN KEY (source_key) REFERENCES portals(subdomain)
);

CREATE TABLE IF NOT EXISTS ingestion_task_frontier (
    frontier_id BIGINT PRIMARY KEY DEFAULT nextval('ingestion_frontier_seq'),
    run_id TEXT NOT NULL,
    source_key TEXT NOT NULL,
    target_id TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'PENDING' CHECK (status IN ('PENDING', 'LEASED')),
    -- Started executions, not claims. Older builds counted claims in `attempts`, which a
    -- legacy table may still carry; nothing reads it.
    executions BIGINT NOT NULL DEFAULT 0,
    lease_expires_at TEXT,
    created_at TEXT NOT NULL DEFAULT strftime(now(), '%Y-%m-%dT%H:%M:%S.%fZ'),
    updated_at TEXT NOT NULL DEFAULT strftime(now(), '%Y-%m-%dT%H:%M:%S.%fZ'),
    UNIQUE (run_id, target_id),
    FOREIGN KEY (run_id) REFERENCES portal_sync_runs(run_id),
    FOREIGN KEY (source_key) REFERENCES portals(subdomain)
);

CREATE TABLE IF NOT EXISTS agencies (
    id BIGINT,
    portal_subdomain TEXT,
    name TEXT NOT NULL,
    department_name TEXT,
    PRIMARY KEY (id, portal_subdomain),
    FOREIGN KEY (portal_subdomain) REFERENCES portals(subdomain)
);

CREATE TABLE IF NOT EXISTS categories (
    id BIGINT,
    portal_subdomain TEXT,
    name TEXT NOT NULL,
    PRIMARY KEY (id, portal_subdomain),
    FOREIGN KEY (portal_subdomain) REFERENCES portals(subdomain)
);

-- ----------------------------------------------------------------------------
-- 2. PUBLIC RECORDS REQUESTS
-- Main operational envelope storing the lifecycle history of requested cases.
-- ----------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS requests (
    id BIGINT NOT NULL,
    portal_subdomain TEXT NOT NULL,
    rpid TEXT,                                -- Internal alphanumeric agency tracking code
    title TEXT,
    description TEXT,
    status TEXT NOT NULL,                     -- e.g., 'open', 'closed', 'referred'
    current_status_id BIGINT,
    
    -- ISO-8601 Datetime Bounds
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    closed_at TEXT,
    due_date TEXT,
    determination_at TEXT,
    determination_source TEXT,
    neighborhood_block TEXT,
    -- Optional label for the requester's class (e.g., 'journalist', 'organization', 'individual').
    -- Nullable on purpose: NULL means "not recorded" and never means "individual" or any specific
    -- class. Populated only where the ingestion adapter can derive it from the source portal.
    requester_attribution TEXT,

    -- Relations
    agency_id BIGINT,
    category_id BIGINT,
    
    PRIMARY KEY (id, portal_subdomain),
    FOREIGN KEY (portal_subdomain) REFERENCES portals(subdomain),
    FOREIGN KEY (agency_id, portal_subdomain) REFERENCES agencies(id, portal_subdomain),
    FOREIGN KEY (category_id, portal_subdomain) REFERENCES categories(id, portal_subdomain)
);

CREATE TABLE IF NOT EXISTS statutory_extensions (
    request_id BIGINT NOT NULL,
    portal_subdomain TEXT NOT NULL,
    notice_at TEXT NOT NULL,
    expected_determination_at TEXT NOT NULL,
    reason TEXT NOT NULL,
    notice_source TEXT NOT NULL,
    recorded_at TEXT NOT NULL DEFAULT strftime(now(), '%Y-%m-%dT%H:%M:%S.%fZ'),
    PRIMARY KEY (request_id, portal_subdomain),
    -- Relations are validated by repository write paths; DuckDB cannot update
    -- referenced parent rows when foreign-key constraints are present.
);

CREATE TABLE IF NOT EXISTS request_keyword_signals (
    request_id BIGINT NOT NULL,
    portal_subdomain TEXT NOT NULL,
    keyword TEXT NOT NULL,
    match_count BIGINT NOT NULL,
    PRIMARY KEY (request_id, portal_subdomain, keyword),
    CHECK (keyword IN ('sweep', 'encampment', 'eviction')),
    CHECK (match_count > 0)
);

-- ----------------------------------------------------------------------------
-- 3. EXTRACTED DOCUMENTS & ATTACHMENTS
-- Houses structural metadata and operational states for raw pulled assets.
-- ----------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS documents (
    id BIGINT NOT NULL,
    request_id BIGINT NOT NULL,
    portal_subdomain TEXT NOT NULL,
    file_name TEXT,
    file_size BIGINT,                        -- Stored cleanly in bytes
    description TEXT,
    content_type TEXT,                        -- e.g., 'application/pdf', 'application/vnd.ms-excel'
    download_url TEXT,
    visibility TEXT NOT NULL,                 -- e.g., 'public', 'staff_only'
    
    -- ISO-8601 Datetime Bounds
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    
    -- Local Ingestion Pipeline Tracking Markers
    local_path TEXT,                          -- Points to exact asset slot on disk
    raw_file_sha256 TEXT,
    extracted_text_sha256 TEXT,               -- Digest of the raw extracted text, which is not
                                              -- retained; checkable only at extraction time.
    -- Digest of the normalized text actually stored in documents_fts, which is what search
    -- returns and legal briefs quote. NULL means the document predates this column or was
    -- imported, and its quoted text cannot be verified.
    extracted_text_projection_sha256 TEXT,
    -- Nullable with no default. 0.0 Shannon entropy is a measurement -- every byte in the
    -- sample identical -- and not "never computed". A DEFAULT 0.0 would report the strongest
    -- possible uniformity signal for every row that was never assessed. NULL is the only
    -- honest reading for those. The CHECK is retained: it is satisfied by NULL and still
    -- bounds every recorded value.
    shannon_entropy REAL CHECK (shannon_entropy >= 0.0 AND shannon_entropy <= 8.0),
    -- These two keep their defaults on purpose. 0 sampled bytes genuinely means no bytes were
    -- sampled, and 0 density is the correct default claim: nothing was flagged.
    entropy_sample_bytes BIGINT NOT NULL DEFAULT 0 CHECK (entropy_sample_bytes >= 0),
    suspicious_data_density BIGINT NOT NULL DEFAULT 0 CHECK (suspicious_data_density IN (0, 1)),
    was_ocr_processed BIGINT DEFAULT 0,      -- Boolean flag check (0=False, 1=True)
    download_status TEXT DEFAULT 'pending',   -- 'pending', 'completed', 'failed'
    extraction_status TEXT NOT NULL DEFAULT 'PENDING'
        CHECK (extraction_status IN ('PENDING', 'SEARCHABLE', 'SALVAGED', 'EXPANDED', 'DEFERRED', 'DOWNLOADED', 'FAILED')),
    extraction_format TEXT,
    extraction_deferred_reason TEXT,
    
    PRIMARY KEY (id, portal_subdomain),
    -- Relations are validated by repository write paths.
);

-- ----------------------------------------------------------------------------
-- 4. CASE AUDITS & TIMELINE LOGGING
-- Captures auditor notes, verification states, and historical data transitions.
-- ----------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS case_audits (
    audit_id TEXT PRIMARY KEY,                -- Client-side generated UUID string
    target_request_id BIGINT NOT NULL,
    target_subdomain TEXT NOT NULL,
    auditor_notes TEXT,
    verified_at TEXT NOT NULL,                -- ISO-8601 timestamp logging action
    verification_status TEXT NOT NULL,        -- 'Unaudited', 'Compliant', 'PolicyViolationDetected', etc.
    
    -- Relations are validated by repository write paths.
);

CREATE TABLE IF NOT EXISTS document_integrity_flags (
    id BIGINT PRIMARY KEY DEFAULT nextval('document_integrity_flag_seq'),
    audit_id TEXT NOT NULL,
    flag_type TEXT NOT NULL,                  -- e.g., 'REDACTION_HEAVY', 'CORRUPT_METADATA', 'MISSING_SIGNATURE'
    -- Relations are validated by repository write paths.
);

CREATE TABLE IF NOT EXISTS state_transition_logs (
    id BIGINT PRIMARY KEY DEFAULT nextval('state_transition_log_seq'),
    request_id BIGINT NOT NULL,
    portal_subdomain TEXT NOT NULL,
    transition_timestamp TEXT NOT NULL,
    previous_state TEXT NOT NULL,
    next_state TEXT NOT NULL,
    time_spent_seconds BIGINT NOT NULL,      -- Automatically calculated operational time metrics
    FOREIGN KEY (request_id, portal_subdomain) REFERENCES requests(id, portal_subdomain)
);

-- ----------------------------------------------------------------------------
-- 5. RECORDED PROVENANCE METADATA (C2PA CONTEXT)
-- Stores manifest and assertion metadata as recorded locally, plus the log of verifications this
-- application performed against it. The manifest tables hold claims, not checked results: nothing
-- in this build validates a C2PA signature or evaluates a trust chain, so is_cryptographically_
-- signed records what a manifest asserts. Only provenance_verification_log rows describe work
-- actually done, and their provenance_confidence column says how much that work is worth.
-- ----------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS provenance_manifests (
    document_id BIGINT NOT NULL,
    portal_subdomain TEXT NOT NULL,
    file_hash_sha256 TEXT NOT NULL,
    is_cryptographically_signed BIGINT DEFAULT 0,
    signature_issuer TEXT,
    signing_certificate_serial_number TEXT,
    signing_timestamp TEXT,
    PRIMARY KEY (document_id, portal_subdomain),
    FOREIGN KEY (document_id, portal_subdomain) REFERENCES documents(id, portal_subdomain)
);

CREATE TABLE IF NOT EXISTS provenance_assertions (
    id BIGINT PRIMARY KEY DEFAULT nextval('provenance_assertion_seq'),
    document_id BIGINT NOT NULL,
    portal_subdomain TEXT NOT NULL,
    assertion_type TEXT NOT NULL,             -- e.g., 'c2pa.actions', 'c2pa.metadata'
    actor_identifier TEXT NOT NULL,
    is_tamper_evident BIGINT DEFAULT 1,
    FOREIGN KEY (document_id, portal_subdomain) REFERENCES provenance_manifests(document_id, portal_subdomain)
);

CREATE TABLE IF NOT EXISTS provenance_verification_log (
    verification_id TEXT PRIMARY KEY,
    document_id BIGINT NOT NULL,
    portal_subdomain TEXT NOT NULL,
    verified_at TEXT NOT NULL,
    expected_sha256 TEXT,
    computed_sha256 TEXT NOT NULL,
    hash_matches BIGINT NOT NULL,
    manifest_present BIGINT NOT NULL,
    is_cryptographically_signed BIGINT NOT NULL,
    invalid_assertion_count BIGINT NOT NULL,
    -- The remaining fields of the verification result. A log row has to reconstruct the check it
    -- describes; anything omitted here is a verification an investigator cannot reproduce.
    -- Unconstrained for the reason given below: DuckDB cannot add a constrained column, so the
    -- upgraded shape is the only shape available and the fresh one is kept identical to it.
    evidence_file_present BOOLEAN,
    assertion_count BIGINT,
    signature_issuer TEXT,
    signing_certificate_serial_number TEXT,
    signing_timestamp TEXT,
    text_projection_sha256 TEXT,
    -- Retained for compatibility. True only at TRUSTED_CHAIN_OF_CUSTODY, which no code path in
    -- this build can reach, so it is always 0 here; read provenance_confidence instead.
    chain_of_custody_confirmed BIGINT NOT NULL,
    verification_errors_json TEXT NOT NULL,
    -- Classification of what was actually checked. Rows written before these columns existed keep
    -- the sentinel default: their confidence cannot be reconstructed after the fact.
    -- Deliberately unconstrained apart from the default. DuckDB cannot ADD COLUMN with a
    -- constraint, so an upgraded database can only ever hold this shape; declaring NOT NULL here
    -- would make a fresh database differ from an upgraded one. The repository always supplies all
    -- three values on insert.
    recorded_digest_source TEXT DEFAULT 'UNCLASSIFIED_LEGACY_RECORD',
    signature_verification TEXT DEFAULT 'UNCLASSIFIED_LEGACY_RECORD',
    provenance_confidence TEXT DEFAULT 'UNCLASSIFIED_LEGACY_RECORD',
    FOREIGN KEY (document_id, portal_subdomain) REFERENCES documents(id, portal_subdomain)
);

-- Provenance verification rows are append-only by repository API contract. DuckDB does not
-- implement SQLite triggers, so mutation paths reject updates/deletes in Rust.

CREATE TABLE IF NOT EXISTS auxiliary_data_sources (
    source_id TEXT PRIMARY KEY,
    portal_subdomain TEXT,
    source_kind TEXT NOT NULL,
    display_name TEXT NOT NULL,
    source_uri TEXT,
    source_sha256 TEXT NOT NULL,
    imported_at TEXT NOT NULL,
    record_count BIGINT NOT NULL,
    FOREIGN KEY (portal_subdomain) REFERENCES portals(subdomain)
);

CREATE TABLE IF NOT EXISTS auxiliary_events (
    source_id TEXT NOT NULL,
    external_id TEXT NOT NULL,
    occurred_at TEXT NOT NULL,
    neighborhood_block TEXT NOT NULL,
    event_type TEXT NOT NULL,
    description TEXT NOT NULL,
    latitude REAL,
    longitude REAL,
    source_payload_json TEXT NOT NULL,
    imported_at TEXT NOT NULL DEFAULT strftime(now(), '%Y-%m-%dT%H:%M:%S.%fZ'),
    PRIMARY KEY (source_id, external_id),
    FOREIGN KEY (source_id) REFERENCES auxiliary_data_sources(source_id),
    CHECK (latitude IS NULL OR (latitude >= -90.0 AND latitude <= 90.0)),
    CHECK (longitude IS NULL OR (longitude >= -180.0 AND longitude <= 180.0))
);

CREATE TABLE IF NOT EXISTS cross_correlation_alerts (
    request_id BIGINT NOT NULL,
    portal_subdomain TEXT NOT NULL,
    neighborhood_block TEXT NOT NULL,
    complaint_count BIGINT NOT NULL,
    correlation_window_hours BIGINT NOT NULL,
    distinct_source_count BIGINT NOT NULL,
    keyword_signal_count BIGINT NOT NULL,
    correlation_density REAL NOT NULL,
    first_correlated_event_at TEXT NOT NULL,
    last_correlated_event_at TEXT NOT NULL,
    evaluated_at TEXT NOT NULL,
    PRIMARY KEY (request_id, portal_subdomain),
    FOREIGN KEY (request_id, portal_subdomain) REFERENCES requests(id, portal_subdomain)
);

CREATE TABLE IF NOT EXISTS report_exports (
    export_id TEXT PRIMARY KEY,
    request_id BIGINT NOT NULL,
    portal_subdomain TEXT NOT NULL,
    output_path TEXT NOT NULL,
    report_sha256 TEXT NOT NULL,
    package_path TEXT,
    package_sha256 TEXT,
    signature_algorithm TEXT,
    signer_key_id TEXT,
    generated_at TEXT NOT NULL,
    violation_markers_json TEXT NOT NULL,
    FOREIGN KEY (request_id, portal_subdomain) REFERENCES requests(id, portal_subdomain)
);

CREATE TABLE IF NOT EXISTS legal_statutes (
    statute_id TEXT PRIMARY KEY,
    jurisdiction TEXT NOT NULL,
    statute_citation TEXT NOT NULL,
    title TEXT NOT NULL,
    authority_kind TEXT NOT NULL,
    full_text TEXT NOT NULL,
    mandated_deadline_days BIGINT,
    canonical_source_uri TEXT,
    current_version_id TEXT NOT NULL,
    content_sha256 TEXT NOT NULL,
    effective_from TEXT,
    effective_to TEXT,
    imported_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    CHECK (mandated_deadline_days IS NULL OR mandated_deadline_days >= 0),
    CHECK (length(content_sha256) = 64)
);

CREATE TABLE IF NOT EXISTS legal_statute_versions (
    version_id TEXT PRIMARY KEY,
    statute_id TEXT NOT NULL,
    full_text TEXT NOT NULL,
    content_sha256 TEXT NOT NULL,
    effective_from TEXT,
    effective_to TEXT,
    source_uri TEXT,
    imported_at TEXT NOT NULL,
    UNIQUE (statute_id, content_sha256),
    FOREIGN KEY (statute_id) REFERENCES legal_statutes(statute_id),
    CHECK (length(content_sha256) = 64)
);

CREATE TABLE IF NOT EXISTS legal_provisions (
    provision_id TEXT PRIMARY KEY,
    version_id TEXT NOT NULL,
    statute_id TEXT NOT NULL,
    ordinal BIGINT NOT NULL,
    heading_level BIGINT NOT NULL,
    heading_path TEXT NOT NULL,
    title TEXT NOT NULL,
    provision_text TEXT NOT NULL,
    token_count BIGINT NOT NULL,
    content_sha256 TEXT NOT NULL,
    UNIQUE (version_id, ordinal),
    FOREIGN KEY (version_id) REFERENCES legal_statute_versions(version_id),
    FOREIGN KEY (statute_id) REFERENCES legal_statutes(statute_id),
    CHECK (ordinal >= 0),
    CHECK (heading_level BETWEEN 0 AND 6),
    CHECK (token_count >= 0),
    CHECK (length(content_sha256) = 64)
);

CREATE TABLE IF NOT EXISTS legal_provisions_fts (
    provision_id TEXT NOT NULL,
    version_id TEXT NOT NULL,
    statute_id TEXT NOT NULL,
    title TEXT NOT NULL,
    heading_path TEXT NOT NULL,
    provision_text TEXT NOT NULL,
    PRIMARY KEY (provision_id, version_id)
);

CREATE TABLE IF NOT EXISTS legal_mandates (
    mandate_id TEXT PRIMARY KEY,
    statute_id TEXT NOT NULL,
    version_id TEXT NOT NULL,
    title TEXT NOT NULL,
    description TEXT NOT NULL,
    applicability_phrases_json TEXT NOT NULL,
    required_evidence_phrases_json TEXT NOT NULL,
    severity TEXT NOT NULL,
    enabled BIGINT NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    FOREIGN KEY (statute_id) REFERENCES legal_statutes(statute_id),
    FOREIGN KEY (version_id) REFERENCES legal_statute_versions(version_id),
    CHECK (severity IN ('ADVISORY', 'MODERATE', 'HIGH', 'CRITICAL')),
    CHECK (enabled IN (0, 1))
);

CREATE TABLE IF NOT EXISTS statutory_violations (
    violation_id TEXT PRIMARY KEY,
    portal_subdomain TEXT NOT NULL,
    request_id BIGINT NOT NULL,
    matched_statute_id TEXT NOT NULL,
    matched_version_id TEXT NOT NULL,
    mandate_id TEXT,
    classification TEXT NOT NULL,
    calculated_variance_metric TEXT,
    variance_days BIGINT,
    confidence_basis_points BIGINT NOT NULL,
    evidence_summary TEXT NOT NULL,
    supporting_document_ids_json TEXT NOT NULL,
    missing_evidence_json TEXT NOT NULL,
    classifier_version TEXT NOT NULL,
    review_status TEXT NOT NULL DEFAULT 'OPEN',
    reviewed_at TEXT,
    reviewer_notes TEXT,
    first_detected_at TEXT NOT NULL,
    evaluated_at TEXT NOT NULL,
    FOREIGN KEY (request_id, portal_subdomain) REFERENCES requests(id, portal_subdomain),
    FOREIGN KEY (matched_statute_id) REFERENCES legal_statutes(statute_id),
    FOREIGN KEY (matched_version_id) REFERENCES legal_statute_versions(version_id),
    FOREIGN KEY (mandate_id) REFERENCES legal_mandates(mandate_id),
    CHECK (confidence_basis_points BETWEEN 0 AND 10000),
    CHECK (review_status IN ('OPEN', 'CONFIRMED', 'DISMISSED', 'RESOLVED_BY_NEW_EVIDENCE'))
);

INSERT OR IGNORE INTO legal_statutes (
    statute_id, jurisdiction, statute_citation, title, authority_kind, full_text,
    mandated_deadline_days, canonical_source_uri, current_version_id, content_sha256,
    effective_from, effective_to, imported_at, updated_at
) VALUES (
    'CPRA_7922.535', 'State of California', 'California Government Code § 7922.535',
    'California Public Records Act determination timing', 'STATE_STATUTE',
    'Determination timing authority for California public records requests; consult the official current text at the recorded source URL.',
    10, 'https://leginfo.legislature.ca.gov/faces/codes_displaySection.xhtml?lawCode=GOV&sectionNum=7922.535.',
    'builtin-cpra-7922-535-2026', '4b81987a37b0bf933f36220dc1ab9d8b33bb734f92a6ebdb4bb1d085db8c7371',
    '2026-01-01', NULL, strftime(now(), '%Y-%m-%dT%H:%M:%S.%fZ'), strftime(now(), '%Y-%m-%dT%H:%M:%S.%fZ')
);

INSERT OR IGNORE INTO legal_statute_versions (
    version_id, statute_id, full_text, content_sha256, effective_from, effective_to, source_uri, imported_at
) SELECT
    current_version_id, statute_id, full_text, content_sha256, effective_from, effective_to,
    canonical_source_uri, imported_at
FROM legal_statutes
WHERE statute_id = 'CPRA_7922.535';

INSERT OR IGNORE INTO legal_provisions (
    provision_id, version_id, statute_id, ordinal, heading_level, heading_path, title,
    provision_text, token_count, content_sha256
)
SELECT
    'builtin-cpra-7922-535-provision', current_version_id, statute_id,
    0, 1, title, title, full_text, 17, content_sha256
FROM legal_statutes
WHERE statute_id = 'CPRA_7922.535';

INSERT INTO legal_provisions_fts (
    provision_id, version_id, statute_id, title, heading_path, provision_text
)
SELECT
    'builtin-cpra-7922-535-provision', current_version_id, statute_id,
    title, title, full_text
FROM legal_statutes
WHERE statute_id = 'CPRA_7922.535'
  AND NOT EXISTS (
    SELECT 1 FROM legal_provisions_fts
    WHERE provision_id = 'builtin-cpra-7922-535-provision'
);

-- ----------------------------------------------------------------------------
-- 6. SEARCH TEXT PROJECTION
-- Plain DuckDB table maintained transactionally by the repository. This is not an FTS5 virtual
-- table; the versioned companion token projection provides exact-token lookup without claiming
-- engine-provided stemming or relevance ranking.
-- ----------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS documents_fts (
    document_id BIGINT NOT NULL,
    portal_subdomain TEXT NOT NULL,
    file_name TEXT,
    extracted_text TEXT NOT NULL,
    PRIMARY KEY (document_id, portal_subdomain)
);

CREATE TABLE IF NOT EXISTS document_search_tokens (
    document_id BIGINT NOT NULL,
    portal_subdomain TEXT NOT NULL,
    token TEXT NOT NULL,
    term_frequency BIGINT NOT NULL CHECK (term_frequency > 0),
    PRIMARY KEY (document_id, portal_subdomain, token)
);

CREATE TABLE IF NOT EXISTS document_search_index_state (
    document_id BIGINT NOT NULL,
    portal_subdomain TEXT NOT NULL,
    index_version BIGINT NOT NULL CHECK (index_version > 0),
    token_count BIGINT NOT NULL CHECK (token_count >= 0),
    -- Characters actually projected. A document longer than the cap is indexed to its prefix, and
    -- this records how far that reached, so a miss beyond it is a known limit rather than a claim
    -- that the term is absent from the corpus.
    indexed_characters BIGINT CHECK (indexed_characters IS NULL OR indexed_characters >= 0),
    PRIMARY KEY (document_id, portal_subdomain)
);

-- ----------------------------------------------------------------------------
-- 7. DURABLE ANALYTICAL DATASETS
-- Typed San Diego datasets live in the core database because they are original analytical inputs
-- and provenance, not disposable search projections.
-- ----------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS analytical_snapshots (
    snapshot_id TEXT PRIMARY KEY,
    dataset_kind TEXT NOT NULL,
    display_name TEXT NOT NULL,
    table_name TEXT NOT NULL,
    source_uri TEXT,
    local_path TEXT NOT NULL,
    source_sha256 TEXT NOT NULL,
    retrieved_at TEXT NOT NULL,
    observation_period_start TEXT,
    observation_period_end TEXT,
    partial_dataset BIGINT NOT NULL DEFAULT 0,
    imported_rows BIGINT NOT NULL DEFAULT 0,
    rejected_rows BIGINT NOT NULL DEFAULT 0,
    malformed_rows BIGINT NOT NULL DEFAULT 0,
    duplicate_key_rows BIGINT NOT NULL DEFAULT 0,
    validation_summary_json TEXT NOT NULL,
    CHECK (length(source_sha256) = 64),
    CHECK (partial_dataset IN (0, 1))
);

CREATE TABLE IF NOT EXISTS analytical_columns (
    snapshot_id TEXT NOT NULL,
    table_name TEXT NOT NULL,
    column_name TEXT NOT NULL,
    original_name TEXT,
    logical_type TEXT NOT NULL,
    nullable BIGINT NOT NULL DEFAULT 1,
    dictionary TEXT,
    ordinal BIGINT NOT NULL,
    PRIMARY KEY (snapshot_id, column_name),
    FOREIGN KEY (snapshot_id) REFERENCES analytical_snapshots(snapshot_id)
);

-- Rows of snapshots imported before per-snapshot column stores. Readers query the
-- `analytical_rows` view, which the application rebuilds over this table and every column store.
CREATE TABLE IF NOT EXISTS analytical_row_ledger (
    snapshot_id TEXT NOT NULL,
    row_number BIGINT NOT NULL,
    row_hash TEXT NOT NULL,
    source_payload_json TEXT NOT NULL,
    normalized_payload_json TEXT NOT NULL,
    malformed BIGINT NOT NULL DEFAULT 0,
    validation_errors_json TEXT NOT NULL,
    PRIMARY KEY (snapshot_id, row_number),
    FOREIGN KEY (snapshot_id) REFERENCES analytical_snapshots(snapshot_id)
);

-- ----------------------------------------------------------------------------
-- 8. PERFORMANCE & ANALYTICS INDEXES
-- Accelerates analytical timelines, cross-agency metrics, and outlier checks.
-- ----------------------------------------------------------------------------
CREATE INDEX IF NOT EXISTS idx_requests_agency ON requests(agency_id, portal_subdomain);
CREATE INDEX IF NOT EXISTS idx_requests_created ON requests(created_at);
CREATE INDEX IF NOT EXISTS idx_requests_status ON requests(status);
CREATE INDEX IF NOT EXISTS idx_documents_status ON documents(download_status);
CREATE INDEX IF NOT EXISTS idx_documents_request ON documents(request_id, portal_subdomain);
CREATE INDEX IF NOT EXISTS idx_document_search_tokens_token ON document_search_tokens(token);
CREATE INDEX IF NOT EXISTS idx_requests_portal_agency_status ON requests(portal_subdomain, agency_id, status);
CREATE INDEX IF NOT EXISTS idx_request_keyword_signals_keyword ON request_keyword_signals(keyword, portal_subdomain);
CREATE INDEX IF NOT EXISTS idx_statutory_extensions_notice ON statutory_extensions(notice_at);
CREATE INDEX IF NOT EXISTS idx_auxiliary_events_neighborhood_time ON auxiliary_events(neighborhood_block, occurred_at);
CREATE INDEX IF NOT EXISTS idx_auxiliary_events_type_time ON auxiliary_events(event_type, occurred_at);
CREATE INDEX IF NOT EXISTS idx_cross_correlation_density ON cross_correlation_alerts(correlation_density DESC);
CREATE INDEX IF NOT EXISTS idx_portal_sync_runs_portal_time ON portal_sync_runs(portal_subdomain, started_at);
CREATE INDEX IF NOT EXISTS idx_legal_statutes_jurisdiction ON legal_statutes(jurisdiction, authority_kind);
CREATE INDEX IF NOT EXISTS idx_legal_versions_statute ON legal_statute_versions(statute_id, imported_at DESC);
CREATE INDEX IF NOT EXISTS idx_legal_provisions_statute ON legal_provisions(statute_id, version_id, ordinal);
CREATE INDEX IF NOT EXISTS idx_legal_mandates_statute ON legal_mandates(statute_id, enabled);
CREATE INDEX IF NOT EXISTS idx_statutory_violations_request ON statutory_violations(request_id, portal_subdomain, review_status);
CREATE INDEX IF NOT EXISTS idx_statutory_violations_statute ON statutory_violations(matched_statute_id, classification, evaluated_at DESC);
CREATE INDEX IF NOT EXISTS idx_analytical_row_ledger_snapshot_hash ON analytical_row_ledger(snapshot_id, row_hash);

-- Durable maintenance choices, retries and crash-recoverable reference-retention receipts.
CREATE TABLE IF NOT EXISTS data_maintenance_policy (
    singleton BOOLEAN PRIMARY KEY, policy_json TEXT NOT NULL, effective_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS data_maintenance_jobs (
    job_key TEXT PRIMARY KEY, attempts BIGINT NOT NULL, next_attempt_at TEXT NOT NULL,
    action TEXT NOT NULL, detail TEXT NOT NULL, recorded_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS data_retention_receipts (
    receipt_id TEXT PRIMARY KEY, portal_key TEXT NOT NULL, document_id BIGINT NOT NULL,
    original_path TEXT NOT NULL, pending_path TEXT NOT NULL, source_uri TEXT NOT NULL,
    source_sha256 TEXT NOT NULL, byte_count BIGINT NOT NULL, state TEXT NOT NULL, recorded_at TEXT NOT NULL
);
-- The index on provenance_verification_log is created in `run_migrations`, next to the columns it
-- covers, so the index and the shim that adds those columns stay in one place. DuckDB permits
-- ADD COLUMN on an indexed table (it refuses only DROP COLUMN), so either location would work; the
-- migration test pins that behavior rather than leaving it assumed.

-- ----------------------------------------------------------------------------
-- 9. SEARCH SYNCHRONIZATION
-- Search rows are maintained transactionally by the repository.
-- ----------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS data_maintenance_origins (
    snapshot_id TEXT NOT NULL,
    portal_key TEXT NOT NULL,
    document_id BIGINT NOT NULL,
    PRIMARY KEY (snapshot_id, portal_key, document_id)
);

-- The data pack a snapshot's bytes were imported from. Beside the snapshot, never part of its
-- identity, so the same bytes fetched by the user and shipped in a pack stay one snapshot with
-- both origins. No foreign key: re-importing the same bytes deletes and re-inserts the snapshot.
CREATE TABLE IF NOT EXISTS pack_origins (
    snapshot_id TEXT NOT NULL,
    pack TEXT NOT NULL,
    pack_version TEXT NOT NULL,
    built_at TEXT NOT NULL,
    signer_key_id TEXT NOT NULL,
    file_path TEXT NOT NULL,
    file_sha256 TEXT NOT NULL,
    imported_at TEXT NOT NULL,
    PRIMARY KEY (snapshot_id, pack, pack_version, signer_key_id, file_path)
);

-- ----------------------------------------------------------------------------
-- 10. FINDINGS INBOX
-- Universal shape every autonomous detector emits. One row = one statement the app believes is
-- worth a user's attention. `id` is a deterministic content hash so a re-run over unchanged
-- evidence upserts cleanly. `confidence_basis_points` is nullable *on purpose*: an unassessed
-- finding (the detector had no numeric basis) is not the same as a computed-zero finding, and a
-- DEFAULT 0 would collapse the two.
-- ----------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS findings (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    severity TEXT NOT NULL
        CHECK (severity IN ('INFO', 'NOTABLE', 'SERIOUS', 'CRITICAL')),
    confidence_basis_points INTEGER
        CHECK (confidence_basis_points IS NULL
            OR (confidence_basis_points >= 0 AND confidence_basis_points <= 10000)),
    subject_portal_key TEXT,
    subject_agency_id BIGINT,
    subject_request_id BIGINT,
    subject_document_id BIGINT,
    subject_entity_id TEXT,
    headline TEXT NOT NULL,
    explanation TEXT NOT NULL,
    citations_json TEXT NOT NULL DEFAULT '[]',
    discovered_at TEXT NOT NULL,
    last_seen_at TEXT NOT NULL,
    dismissed_at TEXT,
    source_detector TEXT NOT NULL
);
-- Ranking sort: severity descending, then confidence descending (NULLs last), then freshness.
CREATE INDEX IF NOT EXISTS idx_findings_ranking
    ON findings(dismissed_at, severity, confidence_basis_points DESC, last_seen_at DESC);
-- Per-detector sweeps use this when clearing stale rows.
CREATE INDEX IF NOT EXISTS idx_findings_detector_last_seen
    ON findings(source_detector, last_seen_at);
-- Subject-anchored queries (dossier view later).
CREATE INDEX IF NOT EXISTS idx_findings_subject_document
    ON findings(subject_portal_key, subject_document_id);
CREATE INDEX IF NOT EXISTS idx_findings_subject_agency
    ON findings(subject_portal_key, subject_agency_id);

-- Append-only: one row per network fetch of a document's bytes. `documents` holds only the latest
-- digest, so without this a re-fetch that returned changed content would erase what was received
-- before. `previous_raw_file_sha256` is the digest the document held when this fetch began.
CREATE TABLE IF NOT EXISTS document_acquisitions (
    acquisition_id BIGINT PRIMARY KEY DEFAULT nextval('document_acquisition_seq'),
    portal_subdomain TEXT NOT NULL,
    document_id BIGINT NOT NULL,
    sync_run_id TEXT,
    requested_url TEXT NOT NULL,
    resolved_url TEXT NOT NULL,
    redirect_chain_json TEXT NOT NULL,
    http_status BIGINT NOT NULL,
    content_type TEXT,
    etag TEXT,
    last_modified TEXT,
    raw_file_sha256 TEXT NOT NULL CHECK (length(raw_file_sha256) = 64),
    file_size BIGINT NOT NULL CHECK (file_size >= 0),
    previous_raw_file_sha256 TEXT,
    retrieved_at TEXT NOT NULL,
    -- The response's Date header. NULL for fetches recorded before it was kept: no default,
    -- because any default would read as a server time that was never observed.
    response_date TEXT
);
CREATE INDEX IF NOT EXISTS idx_document_acquisitions_document
    ON document_acquisitions(portal_subdomain, document_id);

-- One row per re-fetch of a collected document. Each outcome states only what was observed:
-- SAME_BYTES and DIFFERENT_BYTES compare the received digest with the one collected (a different
-- digest is not a claim of alteration: a soft 404 page or a redesign changes it too); GONE means
-- the source answered 404 or 410; UNREACHABLE means this check got no answer about the record
-- (network, server error, refusal) and says nothing about it. A document with no row was not
-- checked. A new table, so its constraints hold on an upgraded database as well.
CREATE TABLE IF NOT EXISTS document_source_checks (
    check_id BIGINT PRIMARY KEY DEFAULT nextval('document_source_check_seq'),
    run_id TEXT NOT NULL,
    portal_subdomain TEXT NOT NULL,
    document_id BIGINT NOT NULL,
    requested_url TEXT NOT NULL,
    checked_at TEXT NOT NULL,
    outcome TEXT NOT NULL
        CHECK (outcome IN ('SAME_BYTES', 'DIFFERENT_BYTES', 'GONE', 'UNREACHABLE')),
    http_status BIGINT CHECK (http_status IS NULL OR http_status BETWEEN 100 AND 599),
    collected_sha256 TEXT NOT NULL CHECK (length(collected_sha256) = 64),
    observed_sha256 TEXT CHECK (observed_sha256 IS NULL OR length(observed_sha256) = 64),
    detail TEXT,
    CHECK ((outcome IN ('SAME_BYTES', 'DIFFERENT_BYTES')) = (observed_sha256 IS NOT NULL)),
    CHECK ((outcome = 'SAME_BYTES') = (observed_sha256 IS NOT DISTINCT FROM collected_sha256)),
    CHECK (outcome <> 'GONE' OR (http_status IS NOT NULL AND http_status IN (404, 410)))
);
CREATE INDEX IF NOT EXISTS idx_document_source_checks_document
    ON document_source_checks(portal_subdomain, document_id);

-- One row per release event a portal's request timeline records ("Document(s) Released", or
-- "... to Requester" when only the requester was shown it), oldest first by `event_ordinal`.
-- `released_at` is the byline's wall-clock time at the offset it is recorded at; it is when the
-- portal shows an upload, not the agency's determination, and nothing derives `determination_at`
-- from it. `listed_document_count` is the number of file names the event lists, NULL when it lists
-- none (not a count of zero). The relation to `requests` is logical (DuckDB refuses updates of a
-- parent row a physical key references) and is checked by `verify_logical_relations`. A new
-- table, so its constraints hold on an upgraded database as well.
CREATE TABLE IF NOT EXISTS request_release_events (
    portal_subdomain TEXT NOT NULL,
    request_id BIGINT NOT NULL,
    event_ordinal BIGINT NOT NULL CHECK (event_ordinal >= 0),
    timeline_event_id BIGINT,
    event_name TEXT NOT NULL
        CHECK (event_name IN ('Document(s) Released', 'Document(s) Released to Requester')),
    released_at TEXT NOT NULL,
    listed_document_count BIGINT
        CHECK (listed_document_count IS NULL OR listed_document_count > 0),
    listed_file_names TEXT,
    source_byline TEXT NOT NULL,
    CHECK ((listed_document_count IS NULL) = (listed_file_names IS NULL)),
    PRIMARY KEY (portal_subdomain, request_id, event_ordinal)
);

-- The rule version each stored derivation was computed under. A rule change bumps the version in
-- code, and startup recomputes what the old rule stored instead of leaving its findings standing.
CREATE TABLE IF NOT EXISTS analysis_rule_versions (
    rule_key TEXT PRIMARY KEY,
    version BIGINT NOT NULL
);

-- ----------------------------------------------------------------------------
-- REDACTION SWEEP
-- What the automatic comparison of redaction marks against other sources has covered. Derived
-- state: every row can be rebuilt from `documents_fts` and the imported tables, and none of it is
-- evidence. Runs are numbered; a source whose `scanned_run` is below the newest span's
-- `added_run` has not been compared with that span yet. Offsets are characters of the stored text.
-- ----------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS redaction_sweep_documents (
    portal_key TEXT NOT NULL,
    document_id BIGINT NOT NULL,
    -- The text digest and byte length the row was computed from; a different value means the
    -- stored text changed and the document is taken in again.
    text_version TEXT NOT NULL,
    scanned_run BIGINT NOT NULL,
    PRIMARY KEY (portal_key, document_id)
);
CREATE TABLE IF NOT EXISTS redaction_sweep_spans (
    portal_key TEXT NOT NULL,
    document_id BIGINT NOT NULL,
    passage_start BIGINT NOT NULL,
    passage_end BIGINT NOT NULL,
    left_anchor TEXT NOT NULL,
    right_anchor TEXT NOT NULL,
    -- The anchors cut to the mark's own line, which is all a table row can hold. NULL when the
    -- line has fewer than three words on either side of the mark.
    row_left_anchor TEXT,
    row_right_anchor TEXT,
    CHECK ((row_left_anchor IS NULL) = (row_right_anchor IS NULL)),
    quote TEXT NOT NULL,
    person_like BOOLEAN NOT NULL,
    -- Two different values were found between these anchors, so the wording is reused prose.
    reused_wording BOOLEAN NOT NULL,
    added_run BIGINT NOT NULL,
    PRIMARY KEY (portal_key, document_id, passage_start)
);
CREATE TABLE IF NOT EXISTS redaction_sweep_tables (
    snapshot_id TEXT PRIMARY KEY,
    source_sha256 TEXT NOT NULL,
    -- NULL until one pass over every row has finished.
    scanned_run BIGINT,
    -- The run a pass in progress began in and the last row it read; both NULL between passes.
    pass_run BIGINT,
    resume_after_row BIGINT,
    CHECK ((pass_run IS NULL) = (resume_after_row IS NULL))
);
CREATE TABLE IF NOT EXISTS redaction_sweep_matches (
    portal_key TEXT NOT NULL,
    document_id BIGINT NOT NULL,
    passage_start BIGINT NOT NULL,
    source_kind TEXT NOT NULL CHECK (source_kind IN ('DOCUMENT', 'TABLE')),
    -- A document's portal and id, or a table's snapshot and row number.
    source_scope TEXT NOT NULL,
    source_item BIGINT NOT NULL,
    -- Digest of the value between the anchors. The value itself is not copied here.
    value_sha256 TEXT NOT NULL,
    name_shaped BOOLEAN NOT NULL,
    source_start BIGINT,
    source_end BIGINT,
    column_name TEXT,
    CHECK ((source_kind = 'DOCUMENT') = (source_start IS NOT NULL AND source_end IS NOT NULL)),
    CHECK ((source_kind = 'TABLE') = (column_name IS NOT NULL)),
    PRIMARY KEY (portal_key, document_id, passage_start, source_kind, source_scope, source_item, value_sha256)
);
