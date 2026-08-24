//! Thread-safe SQLite repository layer managing persistent storage and data indexing.

use std::path::Path;
use std::sync::Arc;
use rusqlite::{params, Connection, Result};
use tokio::sync::Mutex;
use crate::error::AppError;
use crate::types::{PublicRequest, DocumentMetadata};

/// Thread-safe database coordinator wrapper.
#[derive(Clone)]
pub struct SqliteRepository {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteRepository {
    /// Establishes a database connection file node and executes initial runtime pragmas.
    pub fn new<P: AsRef<Path>>(db_path: P) -> Result<Self, AppError> {
        let conn = Connection::open(db_path)
            .map_err(|e| AppError::Internal(format!("Failed to open SQLite database: {}", e)))?;

        // Enable multi-threaded safety checkpoints and performance speed boosts
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA cache_size = -64000; -- ~64MB memory pool cache allocation",
        )
        .map_err(|e| AppError::Internal(format!("Failed to configure SQLite database pragmas: {}", e)))?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Run embedded raw text scripts to establish layout boundaries.
    pub async fn run_migrations(&self, migration_sql: &str) -> Result<(), AppError> {
        let conn = self.conn.lock().await;
        conn.execute_batch(migration_sql)
            .map_err(|e| AppError::Internal(format!("Failed to execute initialization migrations: {}", e)))
    }

    /// Commits a PublicRequest object and its sub-relations inside an isolated ACID transaction block.
    pub async fn insert_or_update_request(&self, subdomain: &str, request: &PublicRequest) -> Result<(), AppError> {
        let mut conn = self.conn.lock().await;
        
        // Open transaction window to guarantee structural database alignment across errors
        let tx = conn.transaction()
            .map_err(|e| AppError::Internal(format!("Failed to initialize database transaction: {}", e)))?;

        // 1. Process lookup configurations for underlying entity subdomains
        if let Some(ref agency) = request.agency {
            tx.execute(
                "INSERT INTO agencies (id, portal_subdomain, name, department_name)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(id, portal_subdomain) DO UPDATE SET
                    name = excluded.name,
                    department_name = excluded.department_name",
                params![agency.id, subdomain, agency.name, agency.department_name],
            ).map_err(|e| AppError::Internal(format!("Agency mapping write block failed: {}", e)))?;
        }

        if let Some(ref category) = request.category {
            tx.execute(
                "INSERT INTO categories (id, portal_subdomain, name)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(id, portal_subdomain) DO UPDATE SET name = excluded.name",
                params![category.id, subdomain, category.name],
            ).map_err(|e| AppError::Internal(format!("Category mapping write block failed: {}", e)))?;
        }

        // 2. Persist primary public record row values
        tx.execute(
            "INSERT INTO requests (
                id, portal_subdomain, rpid, title, description, status, current_status_id,
                created_at, updated_at, closed_at, due_date, agency_id, category_id
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
             ON CONFLICT(id, portal_subdomain) DO UPDATE SET
                title = excluded.title,
                description = excluded.description,
                status = excluded.status,
                updated_at = excluded.updated_at,
                closed_at = excluded.closed_at,
                due_date = excluded.due_date",
            params![
                request.id,
                subdomain,
                request.rpid,
                request.title,
                request.description,
                request.status,
                request.current_status_id,
                request.created_at,
                request.updated_at,
                request.closed_at,
                request.due_date,
                request.agency.as_ref().map(|a| a.id),
                request.category.as_ref().map(|c| c.id)
            ],
        ).map_err(|e| AppError::Internal(format!("Main case envelope transaction write failure: {}", e)))?;

        // 3. Process accompanying data documents arrays
        for doc in &request.documents {
            tx.execute(
                "INSERT INTO documents (
                    id, request_id, portal_subdomain, file_name, file_size, description,
                    content_type, download_url, visibility, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT(id, portal_subdomain) DO UPDATE SET
                    file_name = excluded.file_name,
                    file_size = excluded.file_size,
                    updated_at = excluded.updated_at",
                params![
                    doc.id,
                    request.id,
                    subdomain,
                    doc.file_name,
                    doc.file_size,
                    doc.description,
                    doc.content_type,
                    doc.download_url,
                    doc.visibility,
                    doc.created_at,
                    doc.updated_at
                ],
            ).map_err(|e| AppError::Internal(format!("Document attachment matrix sync error: {}", e)))?;
        }

        // 4. Record any computed operational chronological timeline flags
        for anomaly_type in &request.timeline_anomalies {
            // Check if this case audit envelope entry configuration exists first
            let audit_id = format!("auto-{}-{}", subdomain, request.id);
            
            tx.execute(
                "INSERT OR IGNORE INTO case_audits (audit_id, target_request_id, target_subdomain, verified_at, verification_status)
                 VALUES (?1, ?2, ?3, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), 'DataAnomalyFlagged')",
                params![audit_id, request.id, subdomain],
            ).map_err(|e| AppError::Internal(format!("Auto case-audit envelope mapping failed: {}", e)))?;

            tx.execute(
                "INSERT INTO document_integrity_flags (audit_id, flag_type) VALUES (?1, ?2)",
                params![audit_id, anomaly_type],
            ).map_err(|e| AppError::Internal(format!("Integrity timeline flag append failed: {}", e)))?;
        }

        tx.commit()
            .map_err(|e| AppError::Internal(format!("Failed to commit request transaction block: {}", e)))
    }

    /// High-speed local FTS5 vector search query execution block.
    pub async fn full_text_search(&self, query_string: &str) -> Result<Vec<u64>, AppError> {
        let conn = self.conn.lock().await;
        
        let mut stmt = conn.prepare(
            "SELECT DISTINCT document_id FROM documents_fts WHERE documents_fts MATCH ?1 ORDER BY rank",
        ).map_err(|e| AppError::Internal(format!("FTS statement initialization breakdown: {}", e)))?;

        let rows = stmt.query_map(params![query_string], |row| row.get::<_, u64>(0))
            .map_err(|e| AppError::Internal(format!("Search execution query error: {}", e)))?;

        let mut results = Vec::new();
        for id_res in rows {
            if let Ok(id) = id_res {
                results.push(id);
            }
        }
        
        Ok(results)
    }

    /// Natively flushes extracted PDF character layers into both the main document record and the FTS5 search index.
    pub async fn finalize_document_extraction(
        &self, 
        doc_id: u64, 
        subdomain: &str, 
        text_content: &str, 
        sha256_hash: &str,
        was_ocr: bool
    ) -> Result<(), AppError> {
        let mut conn = self.conn.lock().await;
        let tx = conn.transaction()
            .map_err(|e| AppError::Internal(format!("Failed to open extraction update transaction: {}", e)))?;

        // 1. Mark ingestion pipeline states as completed on the document relation
        tx.execute(
            "UPDATE documents SET 
                download_status = 'completed',
                extracted_text_sha256 = ?1,
                was_ocr_processed = ?2
             WHERE id = ?3 AND portal_subdomain = ?4",
            params![sha256_hash, if was_ocr { 1 } else { 0 }, doc_id, subdomain],
        ).map_err(|e| AppError::Internal(format!("Failed to update document metadata record: {}", e)))?;

        // 2. Refresh virtual index layers with the newly processed text content
        tx.execute(
            "INSERT INTO documents_fts (document_id, portal_subdomain, file_name, extracted_text)
             SELECT id, portal_subdomain, file_name, ?1 FROM documents WHERE id = ?2 AND portal_subdomain = ?3
             ON CONFLICT(document_id) DO UPDATE SET extracted_text = excluded.extracted_text",
            params![text_content, doc_id, subdomain],
        ).map_err(|e| AppError::Internal(format!("Full-Text Search synchronization failure: {}", e)))?;

        tx.commit()
            .map_err(|e| AppError::Internal(format!("Failed to commit extraction transaction transaction block: {}", e)))
    }

    /// Submits manual case verification details and updates investigator flags.
    pub async fn record_manual_case_audit(
        &self,
        audit_id: &str,
        request_id: u64,
        subdomain: &str,
        notes: Option<&str>,
        status: &str,
        integrity_flags: &[String]
    ) -> Result<(), AppError> {
        let mut conn = self.conn.lock().await;
        let tx = conn.transaction()
            .map_err(|e| AppError::Internal(format!("Failed to open case audit window: {}", e)))?;

        tx.execute(
            "INSERT INTO case_audits (audit_id, target_request_id, target_subdomain, auditor_notes, verified_at, verification_status)
             VALUES (?1, ?2, ?3, ?4, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), ?5)
             ON CONFLICT(audit_id) DO UPDATE SET
                auditor_notes = excluded.auditor_notes,
                verified_at = excluded.verified_at,
                verification_status = excluded.verification_status",
            params![audit_id, request_id, subdomain, notes, status],
        ).map_err(|e| AppError::Internal(format!("Audit envelope update statement failure: {}", e)))?;

        // Clear outdated validation metrics before updating them with newer definitions
        tx.execute("DELETE FROM document_integrity_flags WHERE audit_id = ?1", params![audit_id])
            .map_err(|e| AppError::Internal(format!("Failed to clear stale integrity parameters: {}", e)))?;

        for flag in integrity_flags {
            tx.execute(
                "INSERT INTO document_integrity_flags (audit_id, flag_type) VALUES (?1, ?2)",
                params![audit_id, flag],
            ).map_err(|e| AppError::Internal(format!("Failed to map case integrity parameters: {}", e)))?;
        }

        tx.commit()
            .map_err(|e| AppError::Internal(format!("Failed to write investigator case audit parameters: {}", e)))
    }

}
