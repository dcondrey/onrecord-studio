-- ============================================================================
-- ONRECORD STUDIO - INITIAL SCHEMA MIGRATION
-- Target Database: SQLite 3 (with FTS5 support enabled)
-- Description: Core schema mapping for municipal portals, requests, 
--              and deep offline document extraction layers.
-- ============================================================================

PRAGMA foreign_keys = ON;

-- ----------------------------------------------------------------------------
-- 1. PORTAL MANIFESTS & AGENCIES
-- Track target municipal subdomains and their internal agency breakdown structures.
-- ----------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS portals (
    subdomain TEXT PRIMARY KEY,               -- e.g., 'sandiego', 'sandiegohousingcommission'
    display_name TEXT NOT NULL,
    base_url TEXT NOT NULL,
    last_synced_at TEXT,                      -- ISO-8601 Timestamp
    created_at TEXT DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE TABLE IF NOT EXISTS agencies (
    id INTEGER,
    portal_subdomain TEXT,
    name TEXT NOT NULL,
    department_name TEXT,
    PRIMARY KEY (id, portal_subdomain),
    FOREIGN KEY (portal_subdomain) REFERENCES portals(subdomain) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS categories (
    id INTEGER,
    portal_subdomain TEXT,
    name TEXT NOT NULL,
    PRIMARY KEY (id, portal_subdomain),
    FOREIGN KEY (portal_subdomain) REFERENCES portals(subdomain) ON DELETE CASCADE
);

-- ----------------------------------------------------------------------------
-- 2. PUBLIC RECORDS REQUESTS
-- Main operational envelope storing the lifecycle history of requested cases.
-- ----------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS requests (
    id INTEGER NOT NULL,
    portal_subdomain TEXT NOT NULL,
    rpid TEXT,                                -- Internal alphanumeric agency tracking code
    title TEXT,
    description TEXT,
    status TEXT NOT NULL,                     -- e.g., 'open', 'closed', 'referred'
    current_status_id INTEGER,
    
    -- ISO-8601 Datetime Bounds
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    closed_at TEXT,
    due_date TEXT,
    
    -- Relations
    agency_id INTEGER,
    category_id INTEGER,
    
    PRIMARY KEY (id, portal_subdomain),
    FOREIGN KEY (portal_subdomain) REFERENCES portals(subdomain) ON DELETE CASCADE,
    FOREIGN KEY (agency_id, portal_subdomain) REFERENCES agencies(id, portal_subdomain),
    FOREIGN KEY (category_id, portal_subdomain) REFERENCES categories(id, portal_subdomain)
);

-- ----------------------------------------------------------------------------
-- 3. EXTRACTED DOCUMENTS & ATTACHMENTS
-- Houses structural metadata and operational states for raw pulled assets.
-- ----------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS documents (
    id INTEGER NOT NULL,
    request_id INTEGER NOT NULL,
    portal_subdomain TEXT NOT NULL,
    file_name TEXT,
    file_size INTEGER,                        -- Stored cleanly in bytes
    description TEXT,
    content_type TEXT,                        -- e.g., 'application/pdf', 'application/vnd.ms-excel'
    download_url TEXT,
    visibility TEXT NOT NULL,                 -- e.g., 'public', 'staff_only'
    
    -- ISO-8601 Datetime Bounds
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    
    -- Local Ingestion Pipeline Tracking Markers
    local_path TEXT,                          -- Points to exact asset slot on disk
    extracted_text_sha256 TEXT,               -- File-content matching signature
    was_ocr_processed INTEGER DEFAULT 0,      -- Boolean flag check (0=False, 1=True)
    download_status TEXT DEFAULT 'pending',   -- 'pending', 'completed', 'failed'
    
    PRIMARY KEY (id, portal_subdomain),
    FOREIGN KEY (request_id, portal_subdomain) REFERENCES requests(id, portal_subdomain) ON DELETE CASCADE
);

-- ----------------------------------------------------------------------------
-- 4. CASE AUDITS & TIMELINE LOGGING
-- Captures auditor notes, verification states, and historical data transitions.
-- ----------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS case_audits (
    audit_id TEXT PRIMARY KEY,                -- Client-side generated UUID string
    target_request_id INTEGER NOT NULL,
    target_subdomain TEXT NOT NULL,
    auditor_notes TEXT,
    verified_at TEXT NOT NULL,                -- ISO-8601 timestamp logging action
    verification_status TEXT NOT NULL,        -- 'Unaudited', 'Compliant', 'PolicyViolationDetected', etc.
    
    FOREIGN KEY (target_request_id, target_subdomain) REFERENCES requests(id, portal_subdomain) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS document_integrity_flags (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    audit_id TEXT NOT NULL,
    flag_type TEXT NOT NULL,                  -- e.g., 'REDACTION_HEAVY', 'CORRUPT_METADATA', 'MISSING_SIGNATURE'
    FOREIGN KEY (audit_id) REFERENCES case_audits(audit_id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS state_transition_logs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    request_id INTEGER NOT NULL,
    portal_subdomain TEXT NOT NULL,
    transition_timestamp TEXT NOT NULL,
    previous_state TEXT NOT NULL,
    next_state TEXT NOT NULL,
    time_spent_seconds INTEGER NOT NULL,      -- Automatically calculated operational time metrics
    FOREIGN KEY (request_id, portal_subdomain) REFERENCES requests(id, portal_subdomain) ON DELETE CASCADE
);

-- ----------------------------------------------------------------------------
-- 5. CRYPTOGRAPHIC PROVENANCE (C2PA CONTEXT)
-- Stores tamper-evident asset confirmation metadata and validation logs.
-- ----------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS provenance_manifests (
    document_id INTEGER NOT NULL,
    portal_subdomain TEXT NOT NULL,
    file_hash_sha256 TEXT NOT NULL,
    is_cryptographically_signed INTEGER DEFAULT 0,
    signature_issuer TEXT,
    signing_timestamp TEXT,
    PRIMARY KEY (document_id, portal_subdomain),
    FOREIGN KEY (document_id, portal_subdomain) REFERENCES documents(id, portal_subdomain) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS provenance_assertions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id INTEGER NOT NULL,
    portal_subdomain TEXT NOT NULL,
    assertion_type TEXT NOT NULL,             -- e.g., 'c2pa.actions', 'c2pa.metadata'
    actor_identifier TEXT NOT NULL,
    is_tamper_evident INTEGER DEFAULT 1,
    FOREIGN KEY (document_id, portal_subdomain) REFERENCES provenance_manifests(document_id, portal_subdomain) ON DELETE CASCADE
);

-- ----------------------------------------------------------------------------
-- 6. FULL-TEXT SEARCH (FTS5) ENGINE
-- Virtual data table mapping for high-speed local keyword searches across text layers.
-- ----------------------------------------------------------------------------
CREATE VIRTUAL TABLE IF NOT EXISTS documents_fts USING fts5(
    document_id,
    portal_subdomain UNINDEXED,               -- Allows structural filtering without indexing overhead
    file_name,
    extracted_text,
    tokenize="porter unicode61"                -- Porter stemmer allows searches like 'sweeping' to match 'sweep'
);

-- ----------------------------------------------------------------------------
-- 7. PERFORMANCE & ANALYTICS INDEXES
-- Accelerates analytical timelines, cross-agency metrics, and outlier checks.
-- ----------------------------------------------------------------------------
CREATE INDEX IF NOT EXISTS idx_requests_agency ON requests(agency_id, portal_subdomain);
CREATE INDEX IF NOT EXISTS idx_requests_created ON requests(created_at);
CREATE INDEX IF NOT EXISTS idx_requests_status ON requests(status);
CREATE INDEX IF NOT EXISTS idx_documents_status ON documents(download_status);
CREATE INDEX IF NOT EXISTS idx_documents_request ON documents(request_id, portal_subdomain);

-- ----------------------------------------------------------------------------
-- 8. DEEP SEARCH SYNCHRONIZATION TRIGGERS
-- Keeps your full-text search table automatically matching the binary document nodes.
-- ----------------------------------------------------------------------------
CREATE TRIGGER IF NOT EXISTS trg_documents_fts_insert AFTER UPDATE OF local_path ON documents
WHEN NEW.local_path IS NOT NULL AND OLD.local_path IS NULL
BEGIN
    -- This slot gets populated by your Rust engine when offline text extraction succeeds
    INSERT INTO documents_fts(document_id, portal_subdomain, file_name, extracted_text)
    VALUES (NEW.id, NEW.portal_subdomain, NEW.file_name, 'Extraction pending backend stream...');
END;

CREATE TRIGGER IF NOT EXISTS trg_documents_delete AFTER DELETE ON documents
BEGIN
    DELETE FROM documents_fts WHERE document_id = OLD.id AND portal_subdomain = OLD.portal_subdomain;
END;
