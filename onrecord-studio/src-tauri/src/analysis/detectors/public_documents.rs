//! Shared bounded sweep over public documents that have extracted text and were collected from a
//! source.
//!
//! A file the user added by hand (`user_supplied_sources`) is never evidence of what an agency
//! released, withheld, redacted or said, so no detector may read one as such. Every detector query
//! that selects documents excludes that view or goes through [`load_public_documents`], unless it
//! already requires a fact a hand-added file's synthetic request never has (an agency, a
//! determination date, a statutory extension).

use crate::{error::AppError, storage::DuckDbRepository};
use duckdb::params;

/// Larger extracted texts are exports and spreadsheets. Sweeping them costs minutes per detector
/// (one real corpus held 362 MB of CSV text) and no detector reads a policy or letter that size.
pub(super) const MAX_DOCUMENT_TEXT_BYTES: i64 = 2 * 1024 * 1024;

pub(super) struct PublicDocument {
    pub portal_key: String,
    pub document_id: i64,
    pub request_id: i64,
    pub file_name: Option<String>,
    pub created_at: String,
    pub text: String,
}

/// Newest first, so the cap drops the oldest evidence.
pub(super) async fn load_public_documents(
    repository: &DuckDbRepository,
    limit: i64,
) -> Result<Vec<PublicDocument>, AppError> {
    repository
        .query_connection("Public document text sweep", move |conn| {
            let mut statement = conn
                .prepare(
                    "SELECT documents.portal_subdomain, documents.id, documents.request_id,
                            documents.file_name, documents.created_at, documents_fts.extracted_text
                     FROM documents
                     INNER JOIN documents_fts
                        ON documents_fts.document_id = documents.id
                       AND documents_fts.portal_subdomain = documents.portal_subdomain
                     WHERE LOWER(TRIM(documents.visibility)) = 'public'
                       AND documents.portal_subdomain NOT IN (SELECT subdomain FROM user_supplied_sources)
                       AND COALESCE(documents.extracted_text_sha256, '') <> ''
                       AND LENGTH(documents_fts.extracted_text) > 0
                       AND strlen(documents_fts.extracted_text) <= ?2
                     ORDER BY documents.updated_at DESC
                     LIMIT ?1",
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to prepare document sweep: {error}"))
                })?;
            let rows = statement
                .query_map(params![limit, MAX_DOCUMENT_TEXT_BYTES], |row| {
                    Ok(PublicDocument {
                        portal_key: row.get(0)?,
                        document_id: row.get(1)?,
                        request_id: row.get(2)?,
                        file_name: row.get(3)?,
                        created_at: row.get(4)?,
                        text: row.get(5)?,
                    })
                })
                .map_err(|error| {
                    AppError::Storage(format!("Failed to run document sweep: {error}"))
                })?;
            rows.collect::<Result<Vec<_>, _>>().map_err(|error| {
                AppError::Storage(format!("Failed to decode document row: {error}"))
            })
        })
        .await
}
