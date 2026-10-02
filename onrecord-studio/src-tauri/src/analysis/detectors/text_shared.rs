//! Logic shared across detectors: sentence/unit splitting, shingle-Jaccard similarity, and the
//! agency-sweep query used by detectors that pair a document with its request's agency.

use crate::{error::AppError, storage::DuckDbRepository};
use duckdb::params;
use std::collections::{HashMap, HashSet};

/// Byte ranges of sentence-like units: text broken on `\n`, `;`, `!`, `?`, or a `.` that precedes
/// whitespace or end of text, with whitespace-only segments dropped.
///
/// `exclude_abbreviations`: when true, a `.` after a run of three letters or fewer ("Mar.", "St.",
/// "No.") does not end a unit, so an abbreviated month stays with its date
/// (`encampment_notice`'s need). `min_len`: a candidate unit whose trimmed length is below this is
/// dropped (`response_time_claim` filters sub-12-character noise; pass `1` for "non-empty only").
pub(super) fn split_units(
    text: &str,
    exclude_abbreviations: bool,
    min_len: usize,
) -> Vec<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut start = 0;
    for (index, character) in text.char_indices() {
        let ends = match character {
            '\n' | ';' | '!' | '?' => true,
            '.' => {
                let next_is_break = bytes
                    .get(index + 1)
                    .is_none_or(|next| next.is_ascii_whitespace());
                if exclude_abbreviations {
                    let letters = bytes[..index]
                        .iter()
                        .rev()
                        .take_while(|byte| byte.is_ascii_alphabetic())
                        .count();
                    next_is_break && !(1..=3).contains(&letters)
                } else {
                    next_is_break
                }
            }
            _ => false,
        };
        if ends {
            let end = index + character.len_utf8();
            if text[start..end].trim().len() >= min_len {
                spans.push((start, end));
            }
            start = end;
        }
    }
    if text[start..].trim().len() >= min_len {
        spans.push((start, text.len()));
    }
    spans
}

/// Jaccard similarity between two hashed-shingle/token sets: `|A∩B| / |A∪B|`, `0.0` when either
/// is empty. Generic over whatever 64-bit hash the caller's shingling uses — the ratio depends
/// only on set membership, not on how the hashes were produced.
pub(super) fn jaccard(a: &HashSet<u64>, b: &HashSet<u64>) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let intersection = a.intersection(b).count();
    let union = a.len() + b.len() - intersection;
    if union == 0 {
        return 0.0;
    }
    intersection as f64 / union as f64
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct Agency {
    pub id: i64,
    pub name: String,
}

/// `(portal, request id)` to the agency the request was made to.
pub(super) type AgencyIndex = HashMap<(String, i64), Agency>;

/// Every agency a request in the corpus was made to, keyed by `(portal, request id)`. Bounded by
/// `max_requests` so a huge corpus keeps a detector refresh responsive. `operation` names the
/// query for the calling detector's tracing/error context.
pub(super) async fn load_agencies(
    repository: &DuckDbRepository,
    operation: &'static str,
    max_requests: i64,
) -> Result<AgencyIndex, AppError> {
    repository
        .query_connection(operation, move |conn| {
            let mut statement = conn
                .prepare(
                    "SELECT r.portal_subdomain, r.id, a.id, a.name
                     FROM requests r
                     INNER JOIN agencies a
                        ON a.id = r.agency_id AND a.portal_subdomain = r.portal_subdomain
                     ORDER BY r.created_at DESC
                     LIMIT ?1",
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to prepare agency sweep: {error}"))
                })?;
            let rows = statement
                .query_map(params![max_requests], |row| {
                    Ok((
                        (row.get::<_, String>(0)?, row.get::<_, i64>(1)?),
                        Agency {
                            id: row.get(2)?,
                            name: row.get(3)?,
                        },
                    ))
                })
                .map_err(|error| {
                    AppError::Storage(format!("Failed to run agency sweep: {error}"))
                })?;
            rows.collect::<Result<AgencyIndex, _>>()
                .map_err(|error| AppError::Storage(format!("Failed to decode agency row: {error}")))
        })
        .await
}
