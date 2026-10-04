//! Persistence for spans: anchoring finding citations when they are recorded, backfilling the
//! citations recorded before spans existed, and resolving a finding's citations on read.

use super::*;
use crate::analysis::finding::FindingCitation;
use crate::spans::{
    resolve_span, CitationAnchorState, FindingCitationAnchor, SpanSelectors, UnanchoredReason,
    SPAN_CONTEXT_CHARACTERS,
};
use std::collections::HashMap;

pub(crate) const FINDING_CITATION_ORIGIN: &str = "FINDING_CITATION";

/// `analysis_rule_versions` key recording that citations stored before spans existed were
/// backfilled, so the backfill runs once rather than re-reading every finding at each launch.
const BACKFILL_RULE_KEY: &str = "finding_citation_spans";
const BACKFILL_RULE_VERSION: i64 = 1;

fn storage(error: impl std::fmt::Display) -> AppError {
    AppError::Storage(error.to_string())
}

fn finding_citation_span_id(finding_id: &str, ordinal: usize) -> String {
    sha256_hex(format!("{FINDING_CITATION_ORIGIN}\u{1f}{finding_id}\u{1f}{ordinal}").as_bytes())
        [..32]
        .to_string()
}

/// Whether the projection digest the document records now is stamped on the span.
#[derive(Clone, Copy)]
enum Projection {
    /// The citation was computed against the text the document holds now.
    Current,
    /// The citation predates spans, so which projection it was computed against is unknown.
    Unknown,
}

/// The one write-time guard every citation span passes. The exact text is the projection's own
/// characters over the cited range, never the detector's display quote (which is often a
/// truncated excerpt). When a quote was recorded, the range must contain it, so offsets computed
/// against some other text produce no span instead of an anchor on the wrong words. Character
/// offsets match DuckDB's `substr`/`length`, which count code points as Rust's `chars` does.
fn anchor_citation(
    connection: &Connection,
    finding_id: &str,
    ordinal: usize,
    citation: &FindingCitation,
    projection: Projection,
    anchored_at: &str,
) -> Result<bool, AppError> {
    let (Some(start), Some(end)) = (citation.character_start, citation.character_end) else {
        return Ok(false);
    };
    if matches!(projection, Projection::Unknown) && citation.quote.is_none() {
        return Ok(false);
    }
    let context = SPAN_CONTEXT_CHARACTERS as i64;
    let inserted = connection
        .execute(
            "INSERT INTO spans (
                span_id, origin_kind, origin_id, origin_ordinal, portal_subdomain, document_id,
                start_offset, end_offset, exact_text, prefix_text, suffix_text,
                text_projection_sha256, anchored_at
             )
             SELECT ?1, ?2, ?3, ?4, d.portal_subdomain, d.id, ?5, ?6,
                substr(t.extracted_text, ?5 + 1, ?6 - ?5),
                substr(t.extracted_text, greatest(?5 - ?7, 0) + 1, ?5 - greatest(?5 - ?7, 0)),
                substr(t.extracted_text, ?6 + 1, ?7),
                CASE WHEN ?8 AND length(trim(d.extracted_text_projection_sha256)) = 64
                     THEN lower(trim(d.extracted_text_projection_sha256)) END,
                ?9
             FROM documents d
             JOIN documents_fts t
               ON t.document_id = d.id AND t.portal_subdomain = d.portal_subdomain
             WHERE d.id = ?10 AND d.portal_subdomain = ?11
               AND ?5 >= 0 AND ?6 > ?5 AND ?6 <= length(t.extracted_text)
               AND (?12 IS NULL
                    OR (?12 <> '' AND contains(substr(t.extracted_text, ?5 + 1, ?6 - ?5), ?12)))",
            params![
                finding_citation_span_id(finding_id, ordinal),
                FINDING_CITATION_ORIGIN,
                finding_id,
                ordinal as i64,
                start,
                end,
                context,
                matches!(projection, Projection::Current),
                anchored_at,
                citation.document_id,
                citation.portal_key,
                citation.quote,
            ],
        )
        .map_err(|error| AppError::Storage(format!("Failed to anchor a citation span: {error}")))?;
    Ok(inserted > 0)
}

/// Replaces one finding's citation spans with anchors over the text its citations were just
/// computed against. Runs inside the transaction that records the finding.
pub(crate) fn anchor_finding_citations(
    connection: &Connection,
    finding_id: &str,
    citations: &[FindingCitation],
    anchored_at: &str,
) -> Result<(), AppError> {
    connection
        .execute(
            "DELETE FROM spans WHERE origin_kind = ?1 AND origin_id = ?2",
            params![FINDING_CITATION_ORIGIN, finding_id],
        )
        .map_err(|error| {
            AppError::Storage(format!(
                "Failed to replace a finding's citation spans: {error}"
            ))
        })?;
    for (ordinal, citation) in citations.iter().enumerate() {
        anchor_citation(
            connection,
            finding_id,
            ordinal,
            citation,
            Projection::Current,
            anchored_at,
        )?;
    }
    Ok(())
}

/// Removes citation spans whose finding no longer exists. Every path that deletes findings calls
/// this in the same transaction.
pub(crate) fn delete_orphaned_finding_spans(connection: &Connection) -> Result<usize, AppError> {
    connection
        .execute(
            "DELETE FROM spans
             WHERE origin_kind = ?1
               AND NOT EXISTS (SELECT 1 FROM findings f WHERE f.id = spans.origin_id)",
            params![FINDING_CITATION_ORIGIN],
        )
        .map_err(|error| {
            AppError::Storage(format!("Failed to remove orphaned citation spans: {error}"))
        })
}

/// Anchors the citations of findings recorded before spans existed. Each gets a span only when
/// its range still holds its recorded quote; the span's projection digest is left unrecorded,
/// because the text the detector read then is not known. A citation with no quote has nothing to
/// check its range by and gets no span. Runs once, recorded under `BACKFILL_RULE_KEY`.
pub(super) fn backfill_finding_citation_spans(connection: &mut Connection) -> Result<(), AppError> {
    let recorded = connection
        .query_row(
            "SELECT version FROM analysis_rule_versions WHERE rule_key = ?1",
            params![BACKFILL_RULE_KEY],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(storage)?;
    if recorded.is_some_and(|version| version >= BACKFILL_RULE_VERSION) {
        return Ok(());
    }
    let transaction = connection.transaction().map_err(storage)?;
    let rows = {
        let mut statement = transaction
            .prepare("SELECT id, citations_json FROM findings ORDER BY id")
            .map_err(storage)?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(storage)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage)?;
        rows
    };
    let anchored_at = chrono::Utc::now().to_rfc3339();
    for (finding_id, citations_json) in rows {
        // A row whose citations do not parse is left without spans rather than refusing to open
        // the database: listing that finding already reports the malformed row.
        let Ok(citations) = serde_json::from_str::<Vec<FindingCitation>>(&citations_json) else {
            continue;
        };
        for (ordinal, citation) in citations.iter().enumerate() {
            let exists: bool = transaction
                .query_row(
                    "SELECT EXISTS (SELECT 1 FROM spans WHERE span_id = ?1)",
                    params![finding_citation_span_id(&finding_id, ordinal)],
                    |row| row.get(0),
                )
                .map_err(storage)?;
            if !exists {
                anchor_citation(
                    &transaction,
                    &finding_id,
                    ordinal,
                    citation,
                    Projection::Unknown,
                    &anchored_at,
                )?;
            }
        }
    }
    transaction
        .execute(
            "INSERT INTO analysis_rule_versions (rule_key, version) VALUES (?1, ?2)
             ON CONFLICT (rule_key) DO UPDATE SET version = excluded.version",
            params![BACKFILL_RULE_KEY, BACKFILL_RULE_VERSION],
        )
        .map_err(storage)?;
    transaction.commit().map_err(storage)
}

struct StoredSpan {
    span_id: String,
    start: usize,
    exact: String,
    prefix: String,
    suffix: String,
    text_projection_sha256: Option<String>,
}

struct CurrentText {
    projection_sha256: Option<String>,
    text: Option<String>,
}

impl DuckDbRepository {
    /// Where each of a finding's citations stands against its document's current text. `None`
    /// when no finding has this id. Nothing is written: a drifted or stale citation keeps its
    /// stored anchor.
    pub async fn resolve_finding_citations(
        &self,
        finding_id: String,
    ) -> Result<Option<Vec<FindingCitationAnchor>>, AppError> {
        let Some(finding) = self.get_finding_by_id(finding_id).await? else {
            return Ok(None);
        };
        self.query_connection("Resolve finding citations", move |conn| {
            let mut spans: HashMap<i64, StoredSpan> = HashMap::new();
            {
                let mut statement = conn
                    .prepare(
                        "SELECT origin_ordinal, span_id, start_offset, exact_text, prefix_text,
                                suffix_text, text_projection_sha256
                         FROM spans WHERE origin_kind = ?1 AND origin_id = ?2",
                    )
                    .map_err(storage)?;
                let rows = statement
                    .query_map(params![FINDING_CITATION_ORIGIN, finding.id], |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            StoredSpan {
                                span_id: row.get(1)?,
                                start: usize::try_from(row.get::<_, i64>(2)?).unwrap_or(usize::MAX),
                                exact: row.get(3)?,
                                prefix: row.get(4)?,
                                suffix: row.get(5)?,
                                text_projection_sha256: row.get(6)?,
                            },
                        ))
                    })
                    .map_err(storage)?;
                for row in rows {
                    let (ordinal, span) = row.map_err(storage)?;
                    spans.insert(ordinal, span);
                }
            }
            let mut texts: HashMap<(String, i64), CurrentText> = HashMap::new();
            let mut anchors = Vec::with_capacity(finding.citations.len());
            for (index, citation) in finding.citations.iter().enumerate() {
                let anchor = match spans.get(&(index as i64)) {
                    _ if citation.character_start.is_none() || citation.character_end.is_none() => {
                        CitationAnchorState::Unanchored {
                            reason: UnanchoredReason::NoRange,
                        }
                    }
                    None => CitationAnchorState::Unanchored {
                        reason: UnanchoredReason::NotAnchored,
                    },
                    Some(span) => {
                        let key = (citation.portal_key.clone(), citation.document_id);
                        if !texts.contains_key(&key) {
                            let current = current_text(conn, &key.0, key.1)?;
                            texts.insert(key.clone(), current);
                        }
                        let current = &texts[&key];
                        CitationAnchorState::Anchored {
                            span_id: span.span_id.clone(),
                            resolution: resolve_span(
                                &SpanSelectors {
                                    start: span.start,
                                    exact: &span.exact,
                                    prefix: &span.prefix,
                                    suffix: &span.suffix,
                                    text_projection_sha256: span.text_projection_sha256.as_deref(),
                                },
                                current.projection_sha256.as_deref(),
                                current.text.as_deref(),
                            ),
                        }
                    }
                };
                anchors.push(FindingCitationAnchor {
                    citation_index: index,
                    document_id: citation.document_id,
                    character_start: citation.character_start,
                    character_end: citation.character_end,
                    anchor,
                });
            }
            Ok(Some(anchors))
        })
        .await
    }
}

fn current_text(
    connection: &Connection,
    portal_key: &str,
    document_id: i64,
) -> Result<CurrentText, AppError> {
    let row = connection
        .query_row(
            "SELECT d.extracted_text_projection_sha256, t.extracted_text
             FROM documents d
             LEFT JOIN documents_fts t
               ON t.document_id = d.id AND t.portal_subdomain = d.portal_subdomain
             WHERE d.id = ?1 AND d.portal_subdomain = ?2",
            params![document_id, portal_key],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                ))
            },
        )
        .optional()
        .map_err(storage)?;
    Ok(match row {
        Some((digest, text)) => CurrentText {
            projection_sha256: digest
                .map(|value| value.trim().to_ascii_lowercase())
                .filter(|value| value.len() == 64),
            text,
        },
        None => CurrentText {
            projection_sha256: None,
            text: None,
        },
    })
}
