//! Core-side reads behind the document origin report: which kind of source holds a document, and
//! which collected documents share its file digest or its extracted-text digest.
//!
//! "Collected" means held under a source that is not in `user_supplied_sources`. Other files the
//! user added are never offered as a match, because the report says what a source served.

use crate::error::AppError;
use crate::integrity::is_sha256_hex;
use crate::storage::DuckDbRepository;
use crate::types::{HeldComparison, HeldDocumentMatch, NotComparedReason, SourceKind};
use duckdb::{params, Connection, OptionalExt, ToSql};

/// Matches listed per comparison. One more is fetched to learn whether the list is complete.
pub const MAXIMUM_LISTED_MATCHES: usize = 25;

/// SHA-256 of the empty string: an extraction that produced no text, which matches every other
/// document that produced none and says nothing about either.
const EMPTY_TEXT_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

const HELD_DOCUMENT_SELECT: &str = "SELECT d.portal_subdomain, p.display_name, d.request_id,
            COALESCE(NULLIF(TRIM(r.rpid), ''), NULLIF(TRIM(r.title), '')),
            d.id, d.file_name
     FROM documents d
     INNER JOIN portals p ON p.subdomain = d.portal_subdomain
     LEFT JOIN requests r
        ON r.id = d.request_id AND r.portal_subdomain = d.portal_subdomain
     WHERE LOWER(TRIM(d.visibility)) = 'public'
       AND d.portal_subdomain NOT IN (SELECT subdomain FROM user_supplied_sources)";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentDigests {
    pub source_kind: SourceKind,
    /// Lowercase hex, present only when a well-formed digest is recorded.
    pub raw_file_sha256: Option<String>,
    /// Present only when text was extracted and was not empty.
    pub extracted_text_sha256: Option<String>,
    pub file_name: Option<String>,
    pub content_type: Option<String>,
}

fn read_match(row: &duckdb::Row<'_>) -> duckdb::Result<HeldDocumentMatch> {
    Ok(HeldDocumentMatch {
        portal_key: row.get(0)?,
        source_name: row.get(1)?,
        request_id: row.get(2)?,
        request_label: row.get(3)?,
        document_id: row.get(4)?,
        file_name: row.get(5)?,
    })
}

fn storage_error(action: &'static str) -> impl Fn(duckdb::Error) -> AppError {
    move |error| AppError::Storage(format!("Failed to {action}: {error}"))
}

/// `matching` selects the matches among collected documents; `comparable` selects the collected
/// documents that carry the digest at all, which is what a result of no matches was drawn from.
fn compare(
    conn: &Connection,
    matching: &str,
    matching_parameters: &[&dyn ToSql],
    comparable: &str,
) -> Result<HeldComparison<HeldDocumentMatch>, AppError> {
    let fetched = MAXIMUM_LISTED_MATCHES + 1;
    let mut statement = conn
        .prepare(&format!(
            "{HELD_DOCUMENT_SELECT} AND {matching}
             ORDER BY d.portal_subdomain, d.id LIMIT {fetched}"
        ))
        .map_err(storage_error("prepare the held-document comparison"))?;
    let mut matches = statement
        .query_map(matching_parameters, read_match)
        .map_err(storage_error("run the held-document comparison"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage_error("decode a held document"))?;
    let truncated = matches.len() > MAXIMUM_LISTED_MATCHES;
    matches.truncate(MAXIMUM_LISTED_MATCHES);
    let compared_count = conn
        .query_row(
            &format!(
                "SELECT COUNT(*) FROM documents d
                 WHERE LOWER(TRIM(d.visibility)) = 'public'
                   AND d.portal_subdomain NOT IN (SELECT subdomain FROM user_supplied_sources)
                   AND {comparable}"
            ),
            [],
            |row| row.get::<_, u64>(0),
        )
        .map_err(storage_error("count the comparable held documents"))?;
    Ok(HeldComparison::Compared {
        matches,
        truncated,
        compared_count,
    })
}

impl DuckDbRepository {
    pub async fn document_digests(
        &self,
        document_id: u64,
        portal_key: String,
    ) -> Result<DocumentDigests, AppError> {
        self.query_connection("Document origin lookup", move |conn| {
            conn.query_row(
                "SELECT p.source_kind, d.raw_file_sha256, d.extracted_text_sha256,
                        d.file_name, d.content_type
                 FROM documents d
                 INNER JOIN portals p ON p.subdomain = d.portal_subdomain
                 WHERE d.id = ?1 AND d.portal_subdomain = ?2
                   AND LOWER(TRIM(d.visibility)) = 'public'",
                params![document_id, portal_key],
                |row| {
                    let digest = |value: Option<String>| {
                        value
                            .map(|value| value.trim().to_ascii_lowercase())
                            .filter(|value| is_sha256_hex(value))
                    };
                    Ok(DocumentDigests {
                        source_kind: SourceKind::from_stored(&row.get::<_, String>(0)?),
                        raw_file_sha256: digest(row.get(1)?),
                        extracted_text_sha256: digest(row.get(2)?)
                            .filter(|value| value != EMPTY_TEXT_SHA256),
                        file_name: row.get(3)?,
                        content_type: row.get(4)?,
                    })
                },
            )
            .optional()
            .map_err(storage_error("read the document's origin"))?
            .ok_or_else(|| {
                AppError::Storage(format!(
                    "Public document {portal_key}/{document_id} was not found"
                ))
            })
        })
        .await
    }

    /// Collected documents with the same file digest, and those with the same extracted-text
    /// digest that are not byte-identical.
    pub async fn held_documents_sharing_digests(
        &self,
        digests: DocumentDigests,
    ) -> Result<
        (
            HeldComparison<HeldDocumentMatch>,
            HeldComparison<HeldDocumentMatch>,
        ),
        AppError,
    > {
        self.query_connection("Held-document digest comparison", move |conn| {
            let raw = digests.raw_file_sha256.as_deref();
            let exact_copies = match raw {
                Some(raw) => compare(
                    conn,
                    "LOWER(TRIM(d.raw_file_sha256)) = ?1",
                    &[&raw],
                    "LENGTH(TRIM(d.raw_file_sha256)) = 64",
                )?,
                None => HeldComparison::NotCompared {
                    reason: NotComparedReason::FileDigestNotRecorded,
                },
            };
            let same_text = match digests.extracted_text_sha256.as_deref() {
                // A held document with the same file digest is an exact copy and is listed there.
                Some(text) => compare(
                    conn,
                    "LOWER(TRIM(d.extracted_text_sha256)) = ?1
                     AND LOWER(TRIM(COALESCE(d.raw_file_sha256, ''))) <> ?2",
                    &[&text, &raw.unwrap_or("-")],
                    &format!(
                        "LENGTH(TRIM(d.extracted_text_sha256)) = 64
                         AND LOWER(TRIM(d.extracted_text_sha256)) <> '{EMPTY_TEXT_SHA256}'"
                    ),
                )?,
                None => HeldComparison::NotCompared {
                    reason: NotComparedReason::TextNotExtracted,
                },
            };
            Ok((exact_copies, same_text))
        })
        .await
    }

    /// Names the collected documents among `keys`, in the order given. A key that is not a public
    /// document of a collected source is dropped.
    pub async fn held_documents_by_key(
        &self,
        keys: Vec<(String, i64)>,
    ) -> Result<Vec<HeldDocumentMatch>, AppError> {
        self.query_connection("Held-document lookup", move |conn| {
            let mut statement = conn
                .prepare(&format!(
                    "{HELD_DOCUMENT_SELECT} AND d.portal_subdomain = ?1 AND d.id = ?2"
                ))
                .map_err(storage_error("prepare the held-document lookup"))?;
            let mut found = Vec::new();
            for (portal_key, document_id) in keys {
                if let Some(held) = statement
                    .query_row(params![portal_key, document_id], read_match)
                    .optional()
                    .map_err(storage_error("read a held document"))?
                {
                    found.push(held);
                }
            }
            Ok(found)
        })
        .await
    }
}
