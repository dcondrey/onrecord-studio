//! Reads the stored identity and text of every document a report cites. Text is loaded only for
//! public documents, and only within a fixed budget: a document that does not fit is reported as
//! unchecked rather than silently trusted.

use super::{SourceDocument, SourceDocuments};
use crate::error::AppError;
use crate::storage::sqlite::DuckDbRepository;
use duckdb::params;
use std::collections::BTreeSet;

pub const MAXIMUM_DOCUMENT_TEXT_CHARACTERS: i64 = 4_000_000;
pub const MAXIMUM_TOTAL_TEXT_CHARACTERS: usize = 64_000_000;
pub const MAXIMUM_DISTINCT_DOCUMENTS: usize = 2_000;

fn digest_or_none(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

pub async fn load_source_documents(
    repository: &DuckDbRepository,
    keys: BTreeSet<(String, i64)>,
) -> Result<SourceDocuments, AppError> {
    if keys.len() > MAXIMUM_DISTINCT_DOCUMENTS {
        return Err(AppError::DataParsing(format!(
            "The report cites {} documents; the limit is {MAXIMUM_DISTINCT_DOCUMENTS}. Select fewer findings.",
            keys.len()
        )));
    }
    repository
        .query_connection("Load report source documents", move |conn| {
            let mut statement = conn
                .prepare(
                    "SELECT d.file_name, d.visibility, d.raw_file_sha256,
                            d.extracted_text_projection_sha256,
                            CASE WHEN LOWER(TRIM(d.visibility)) = 'public' THEN (
                                SELECT CASE WHEN LENGTH(f.extracted_text) <= ?3
                                            THEN f.extracted_text END
                                FROM documents_fts f
                                WHERE f.document_id = d.id
                                  AND f.portal_subdomain = d.portal_subdomain
                                LIMIT 1
                            ) END
                     FROM documents d
                     WHERE d.id = ?1 AND d.portal_subdomain = ?2",
                )
                .map_err(|error| AppError::Storage(error.to_string()))?;
            let mut loaded = SourceDocuments::new();
            let mut text_budget = MAXIMUM_TOTAL_TEXT_CHARACTERS;
            for (portal_key, document_id) in keys {
                let row = statement.query_row(
                    params![document_id, portal_key, MAXIMUM_DOCUMENT_TEXT_CHARACTERS],
                    |row| {
                        Ok((
                            row.get::<_, Option<String>>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, Option<String>>(2)?,
                            row.get::<_, Option<String>>(3)?,
                            row.get::<_, Option<String>>(4)?,
                        ))
                    },
                );
                let (file_name, visibility, raw, projection, text) = match row {
                    Ok(row) => row,
                    Err(duckdb::Error::QueryReturnedNoRows) => continue,
                    Err(error) => return Err(AppError::Storage(error.to_string())),
                };
                // A missing row stays absent from the map; the builder treats an absent
                // document like a non-public one and publishes nothing from it.
                let text = text.filter(|text| {
                    let characters = text.chars().count();
                    if characters > text_budget {
                        return false;
                    }
                    text_budget -= characters;
                    true
                });
                loaded.insert(
                    (portal_key, document_id),
                    SourceDocument {
                        file_name,
                        visibility,
                        raw_file_sha256: digest_or_none(raw),
                        extracted_text_projection_sha256: digest_or_none(projection),
                        text,
                    },
                );
            }
            Ok(loaded)
        })
        .await
}
