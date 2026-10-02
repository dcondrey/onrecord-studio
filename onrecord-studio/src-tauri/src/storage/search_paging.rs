//! Keyset-paged request search with an exact count.
//!
//! `search_requests` answers one capped list; this answers the same filter as a total plus pages of
//! any size. The order is `created_at DESC, portal_subdomain ASC, id ASC`, the same as
//! `search_requests`, and the cursor is the last row's three sort columns, so a page boundary stays
//! put when rows are ingested between page requests: a new row sorts either before the cursor
//! (never shown by later pages, counted by the next `count`) or after it (shown once). Offsets
//! would shift by one for every insert and repeat or skip a row.

use crate::error::AppError;
use crate::storage::fts::core_query_tokens;
use crate::storage::pii::mask_pii;
use crate::storage::sqlite::{request_search_filters, DuckDbRepository};
use crate::types::RequestSearchResult;
use duckdb::params;
use serde::{Deserialize, Serialize};

/// A page is a unit of transfer, not a result limit. The ceiling only bounds the memory and the
/// time one request holds the database connection; every row stays reachable by the cursor.
pub const MAXIMUM_PAGE_SIZE: usize = 1_000;

const CURSOR_VERSION: &str = "v1";
const CURSOR_SEPARATOR: char = '\u{1f}';

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AuditRecordPage {
    pub records: Vec<RequestSearchResult>,
    /// Present exactly when more matching rows follow this page. Opaque: pass it back unchanged.
    pub next_cursor: Option<String>,
    /// Exact number of matches, read in the same transaction-free closure as the first page so it
    /// describes the same corpus state. Only the first page (no cursor) carries it; absence means
    /// "not asked for", never zero.
    pub total: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AuditRecordCount {
    pub total: u64,
}

struct Cursor {
    created_at: String,
    portal_key: String,
    request_id: i64,
}

fn encode_cursor(created_at: &str, portal_key: &str, request_id: i64) -> String {
    format!(
        "{CURSOR_VERSION}{CURSOR_SEPARATOR}{created_at}{CURSOR_SEPARATOR}{portal_key}{CURSOR_SEPARATOR}{request_id}"
    )
}

fn decode_cursor(value: &str) -> Result<Cursor, AppError> {
    let invalid = || AppError::DataParsing("The page cursor is not valid".to_string());
    if value.len() > 1_024 {
        return Err(invalid());
    }
    let mut parts = value.split(CURSOR_SEPARATOR);
    let (Some(version), Some(created_at), Some(portal_key), Some(request_id), None) = (
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
    ) else {
        return Err(invalid());
    };
    if version != CURSOR_VERSION {
        return Err(invalid());
    }
    Ok(Cursor {
        created_at: created_at.to_string(),
        portal_key: portal_key.to_string(),
        request_id: request_id.parse().map_err(|_| invalid())?,
    })
}

pub(crate) struct Filters {
    portal_key: Option<String>,
    agency_id: Option<i64>,
    status: Option<String>,
    policy_deviation: Option<String>,
    fts_query: Option<String>,
    pub(crate) sql: String,
}

impl Filters {
    /// The five positional parameters `sql` binds, in order.
    pub(crate) fn params(&self) -> [&dyn duckdb::ToSql; 5] {
        [
            &self.portal_key,
            &self.agency_id,
            &self.status,
            &self.policy_deviation,
            &self.fts_query,
        ]
    }

    /// True when the predicate reads the token projection, which is incomplete while indexing.
    pub(crate) fn reads_tokens(&self) -> bool {
        self.fts_query.is_some()
    }
}

fn trimmed(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(crate) fn build_filters(
    search_term: &str,
    portal_key: Option<&str>,
    agency_id: Option<u64>,
    status: Option<&str>,
    policy_deviation: Option<&str>,
) -> Result<Filters, AppError> {
    let tokens = core_query_tokens(search_term)
        .map_err(|message| AppError::DataParsing(message.to_string()))?;
    Ok(Filters {
        portal_key: trimmed(portal_key),
        agency_id: agency_id.map(i64::try_from).transpose().map_err(|_| {
            AppError::DataParsing("Agency identifier exceeds the supported range".to_string())
        })?,
        status: trimmed(status),
        policy_deviation: trimmed(policy_deviation),
        fts_query: (!tokens.is_empty()).then(|| search_term.trim().to_string()),
        sql: request_search_filters(&tokens),
    })
}

pub(crate) fn count_matches(conn: &duckdb::Connection, filters: &Filters) -> Result<u64, AppError> {
    let total: i64 = conn
        .query_row(
            &format!("SELECT COUNT(*) FROM requests WHERE {}", filters.sql),
            params![
                filters.portal_key,
                filters.agency_id,
                filters.status,
                filters.policy_deviation,
                filters.fts_query
            ],
            |row| row.get(0),
        )
        .map_err(|error| AppError::Storage(format!("Failed to count request matches: {error}")))?;
    u64::try_from(total).map_err(|_| AppError::Storage("Match count was negative".to_string()))
}

impl DuckDbRepository {
    /// Exact number of requests matching the filter. Same predicate text as the pages.
    pub async fn count_requests(
        &self,
        search_term: &str,
        portal_key: Option<&str>,
        agency_id: Option<u64>,
        status: Option<&str>,
        policy_deviation: Option<&str>,
    ) -> Result<u64, AppError> {
        let filters = build_filters(search_term, portal_key, agency_id, status, policy_deviation)?;
        self.query_connection("Request match count", move |conn| {
            count_matches(conn, &filters)
        })
        .await
    }

    /// One page of matches after `cursor` (the start when `None`). A first page also carries the
    /// exact total.
    #[allow(clippy::too_many_arguments)]
    pub async fn search_requests_page(
        &self,
        search_term: &str,
        portal_key: Option<&str>,
        agency_id: Option<u64>,
        status: Option<&str>,
        policy_deviation: Option<&str>,
        cursor: Option<&str>,
        page_size: usize,
    ) -> Result<AuditRecordPage, AppError> {
        if !(1..=MAXIMUM_PAGE_SIZE).contains(&page_size) {
            return Err(AppError::DataParsing(format!(
                "A results page must hold between 1 and {MAXIMUM_PAGE_SIZE} rows"
            )));
        }
        let filters = build_filters(search_term, portal_key, agency_id, status, policy_deviation)?;
        let cursor = cursor.map(decode_cursor).transpose()?;
        // One row past the page says whether another page exists, so the cursor is absent exactly
        // at the end rather than present-and-empty.
        let fetch = i64::try_from(page_size + 1)
            .map_err(|_| AppError::DataParsing("Page size exceeds the supported range".into()))?;
        self.query_connection("Request search page", move |conn| {
            let total = if cursor.is_none() {
                Some(count_matches(conn, &filters)?)
            } else {
                None
            };
            let sql = format!(
                "WITH page AS (
                    SELECT requests.id, requests.portal_subdomain, requests.created_at
                    FROM requests
                    WHERE {filters}
                      AND (?6 IS NULL
                           OR requests.created_at < ?6
                           OR (requests.created_at = ?6
                               AND (requests.portal_subdomain > ?7
                                    OR (requests.portal_subdomain = ?7 AND requests.id > ?8))))
                    ORDER BY requests.created_at DESC, requests.portal_subdomain, requests.id
                    LIMIT ?9
                 )
                 SELECT
                    requests.id,
                    requests.portal_subdomain,
                    requests.agency_id,
                    COALESCE(agencies.name, 'Unmapped Department'),
                    COALESCE(requests.title, ''),
                    COALESCE(requests.description, ''),
                    requests.status,
                    requests.created_at,
                    requests.closed_at,
                    (SELECT COUNT(*) FROM documents
                     WHERE documents.request_id = requests.id
                       AND documents.portal_subdomain = requests.portal_subdomain
                       AND LOWER(TRIM(documents.visibility)) = 'public'),
                    COALESCE((
                        SELECT GROUP_CONCAT(distinct_flags.flag_type, chr(31))
                        FROM (
                            SELECT DISTINCT document_integrity_flags.flag_type
                            FROM case_audits
                            INNER JOIN document_integrity_flags
                              ON document_integrity_flags.audit_id = case_audits.audit_id
                            WHERE case_audits.target_request_id = requests.id
                              AND case_audits.target_subdomain = requests.portal_subdomain
                            ORDER BY document_integrity_flags.flag_type
                        ) AS distinct_flags
                    ), '')
                 FROM page
                 INNER JOIN requests
                    ON requests.id = page.id AND requests.portal_subdomain = page.portal_subdomain
                 LEFT JOIN agencies
                   ON agencies.id = requests.agency_id
                  AND agencies.portal_subdomain = requests.portal_subdomain
                 ORDER BY requests.created_at DESC, requests.portal_subdomain, requests.id",
                filters = filters.sql
            );
            let mut statement = conn.prepare(&sql).map_err(|error| {
                AppError::Storage(format!("Failed to prepare the results page: {error}"))
            })?;
            let rows = statement
                .query_map(
                    params![
                        filters.portal_key,
                        filters.agency_id,
                        filters.status,
                        filters.policy_deviation,
                        filters.fts_query,
                        cursor.as_ref().map(|cursor| cursor.created_at.clone()),
                        cursor.as_ref().map(|cursor| cursor.portal_key.clone()),
                        cursor.as_ref().map(|cursor| cursor.request_id),
                        fetch
                    ],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, Option<i64>>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?,
                            row.get::<_, String>(5)?,
                            row.get::<_, String>(6)?,
                            row.get::<_, String>(7)?,
                            row.get::<_, Option<String>>(8)?,
                            row.get::<_, i64>(9)?,
                            row.get::<_, String>(10)?,
                        ))
                    },
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to execute the results page: {error}"))
                })?;
            let mut records = Vec::new();
            let mut last_key = None;
            let mut has_more = false;
            for row in rows {
                let (
                    id,
                    portal,
                    agency,
                    agency_name,
                    title,
                    description,
                    status,
                    created,
                    closed,
                    documents,
                    flags,
                ) = row.map_err(|error| {
                    AppError::Storage(format!("Failed to decode a results row: {error}"))
                })?;
                if records.len() == page_size {
                    has_more = true;
                    break;
                }
                last_key = Some(encode_cursor(&created, &portal, id));
                let description = mask_pii(&description)?.masked_text;
                records.push(RequestSearchResult {
                    request_id: u64::try_from(id)
                        .map_err(|_| AppError::Storage("Request id was negative".to_string()))?,
                    portal_key: portal,
                    agency_id: agency.and_then(|value| u64::try_from(value).ok()),
                    agency_name: mask_pii(&agency_name)?.masked_text,
                    title: mask_pii(&title)?.masked_text,
                    description_preview: description.chars().take(320).collect(),
                    status,
                    created_at: created,
                    closed_at: closed,
                    document_count: usize::try_from(documents).map_err(|_| {
                        AppError::Storage("Document count was negative".to_string())
                    })?,
                    integrity_flags: flags
                        .split('\u{1f}')
                        .filter(|value| !value.is_empty())
                        .map(str::to_string)
                        .collect(),
                });
            }
            Ok(AuditRecordPage {
                records,
                next_cursor: if has_more { last_key } else { None },
                total,
            })
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::time::Instant;

    const MIGRATION: &str = include_str!("../../migrations/0001_initial_schema.sql");

    type Key = (String, String, u64);

    fn key(record: &RequestSearchResult) -> Key {
        (
            record.created_at.clone(),
            record.portal_key.clone(),
            record.request_id,
        )
    }

    /// `rows` requests over two portals. Ids repeat across portals and `created_at` is shared by
    /// many rows, so every level of the sort key is a tiebreak somewhere. Every 7th request owns a
    /// document carrying the token `quasar`, every 10th says `zebra` in its own text.
    async fn seeded(rows: usize) -> (tempfile::TempDir, DuckDbRepository) {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = DuckDbRepository::new(dir.path().join("core.duckdb")).expect("core opens");
        repo.run_migrations(MIGRATION).await.expect("migrations");
        repo.query_connection("seed", move |conn| {
            conn.execute_batch(&format!(
                "INSERT INTO portals (subdomain, display_name, base_url, origin_url) VALUES
                    ('la', 'Los Angeles', 'https://la.example', 'https://la.example'),
                    ('sd', 'San Diego', 'https://sd.example', 'https://sd.example');
                 INSERT INTO requests (id, portal_subdomain, title, description, status,
                                       created_at, updated_at)
                 SELECT i // 2, CASE WHEN i % 2 = 0 THEN 'la' ELSE 'sd' END,
                        'Request ' || i, CASE WHEN i % 10 = 0 THEN 'zebra crossing' ELSE 'plain' END,
                        'open',
                        strftime(TIMESTAMP '2024-01-01' + INTERVAL (i % 400) MINUTE,
                                 '%Y-%m-%dT%H:%M:%SZ'),
                        '2024-01-01T00:00:00Z'
                 FROM range(2, {end}) t(i);
                 INSERT INTO documents (id, request_id, portal_subdomain, file_name, visibility,
                                        created_at, updated_at)
                 SELECT i, i // 2, CASE WHEN i % 2 = 0 THEN 'la' ELSE 'sd' END, 'f.pdf', 'public',
                        '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z'
                 FROM range(2, {end}) t(i) WHERE i % 7 = 0;
                 INSERT INTO document_search_tokens (document_id, portal_subdomain, token,
                                                     term_frequency)
                 SELECT i, CASE WHEN i % 2 = 0 THEN 'la' ELSE 'sd' END, 'quasar', 1
                 FROM range(2, {end}) t(i) WHERE i % 7 = 0;",
                end = rows + 2
            ))
            .map_err(|error| AppError::Storage(error.to_string()))
        })
        .await
        .expect("seed");
        (dir, repo)
    }

    async fn all_pages(repo: &DuckDbRepository, term: &str, page_size: usize) -> (Vec<Key>, u64) {
        let mut keys = Vec::new();
        let mut cursor: Option<String> = None;
        let mut total = None;
        loop {
            let page = repo
                .search_requests_page(term, None, None, None, None, cursor.as_deref(), page_size)
                .await
                .expect("page");
            assert!(page.records.len() <= page_size);
            if cursor.is_none() {
                total = page.total;
            } else {
                assert_eq!(page.total, None, "only the first page reads the total");
            }
            keys.extend(page.records.iter().map(key));
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => return (keys, total.expect("first page carries the total")),
            }
        }
    }

    #[tokio::test]
    async fn pages_cover_the_exact_total_once_in_order_for_every_query_shape() {
        let (_dir, repo) = seeded(2_503).await;
        for term in ["", "zebra", "quasar", "zebra quasar", "nomatchtoken"] {
            let (keys, total) = all_pages(&repo, term, 97).await;
            let counted = repo
                .count_requests(term, None, None, None, None)
                .await
                .unwrap();
            assert_eq!(
                total, counted,
                "first-page total and count agree for {term:?}"
            );
            assert_eq!(
                keys.len() as u64,
                total,
                "pages sum to the total for {term:?}"
            );
            assert_eq!(
                keys.iter().collect::<HashSet<_>>().len(),
                keys.len(),
                "no row repeats for {term:?}"
            );
            // The capped search is the reference for the same filter and order.
            let reference = repo
                .search_requests(term, None, None, None, None, 5_000)
                .await
                .unwrap();
            assert_eq!(
                keys,
                reference.iter().map(key).collect::<Vec<_>>(),
                "same rows in the same order as search_requests for {term:?}"
            );
        }
    }

    #[tokio::test]
    async fn a_page_boundary_survives_rows_ingested_between_pages() {
        let (_dir, repo) = seeded(1_000).await;
        let before = all_pages(&repo, "", 1_000).await.0;
        let first = repo
            .search_requests_page("", None, None, None, None, None, 300)
            .await
            .unwrap();
        let mut seen: Vec<Key> = first.records.iter().map(key).collect();
        // One row sorts before the cursor (newest), one inside the unread remainder (oldest).
        repo.query_connection("ingest", |conn| {
            conn.execute_batch(
                "INSERT INTO requests (id, portal_subdomain, status, created_at, updated_at) VALUES
                    (900001, 'sd', 'open', '2030-01-01T00:00:00Z', '2030-01-01T00:00:00Z'),
                    (900002, 'sd', 'open', '2000-01-01T00:00:00Z', '2000-01-01T00:00:00Z');",
            )
            .map_err(|error| AppError::Storage(error.to_string()))
        })
        .await
        .unwrap();
        let mut cursor = first.next_cursor;
        while let Some(next) = cursor {
            let page = repo
                .search_requests_page("", None, None, None, None, Some(&next), 300)
                .await
                .unwrap();
            seen.extend(page.records.iter().map(key));
            cursor = page.next_cursor;
        }
        let unique: HashSet<_> = seen.iter().collect();
        assert_eq!(unique.len(), seen.len(), "no repeats");
        assert!(
            before.iter().all(|key| unique.contains(key)),
            "no original row skipped"
        );
        assert!(
            unique.iter().any(|key| key.2 == 900_002),
            "a later row is read once"
        );
        assert!(
            !unique.iter().any(|key| key.2 == 900_001),
            "a row that sorts before the cursor is not shown retroactively"
        );
    }

    #[tokio::test]
    async fn a_malformed_cursor_and_an_out_of_range_page_size_are_rejected() {
        let (_dir, repo) = seeded(10).await;
        let bad = repo
            .search_requests_page("", None, None, None, None, Some("v1\u{1f}x"), 5)
            .await;
        assert!(bad.is_err());
        for size in [0, MAXIMUM_PAGE_SIZE + 1] {
            let result = repo
                .search_requests_page("", None, None, None, None, None, size)
                .await;
            assert!(result.is_err(), "page size {size}");
        }
    }

    /// Measurement, not a gate: `cargo test --lib scale_proof -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore = "seeds 50,000 requests; run explicitly to measure"]
    async fn scale_proof_fifty_thousand_requests() {
        const ROWS: usize = 50_000;
        let seeding = Instant::now();
        let (_dir, repo) = seeded(ROWS).await;
        eprintln!("seed {ROWS} requests: {:?}", seeding.elapsed());
        for term in ["", "zebra", "quasar"] {
            let started = Instant::now();
            let total = repo
                .count_requests(term, None, None, None, None)
                .await
                .unwrap();
            let count_time = started.elapsed();
            let started = Instant::now();
            let first = repo
                .search_requests_page(term, None, None, None, None, None, 200)
                .await
                .unwrap();
            let first_time = started.elapsed();
            let started = Instant::now();
            let second = repo
                .search_requests_page(
                    term,
                    None,
                    None,
                    None,
                    None,
                    first.next_cursor.as_deref(),
                    200,
                )
                .await
                .unwrap();
            let second_time = started.elapsed();
            let walking = Instant::now();
            let (keys, walked_total) = all_pages(&repo, term, 200).await;
            eprintln!(
                "term {term:?}: total {total} count {count_time:?} first page {first_time:?} \
                 second page {second_time:?} full walk {:?}",
                walking.elapsed()
            );
            assert_eq!(first.total, Some(total));
            assert_eq!(second.records.len(), 200);
            assert_eq!(walked_total, total);
            assert_eq!(keys.len() as u64, total);
            assert_eq!(keys.iter().collect::<HashSet<_>>().len(), keys.len());
            assert!(
                count_time.as_secs() < 5 && first_time.as_secs() < 5,
                "count and first page stay interactive"
            );
        }
        repo.query_connection("more documents", |conn| {
            conn.execute_batch(
                "INSERT INTO documents (id, request_id, portal_subdomain, file_name, visibility,
                                        created_at, updated_at, extracted_text_sha256)
                 SELECT 1000000 + i, i // 2, CASE WHEN i % 2 = 0 THEN 'la' ELSE 'sd' END, 'g.pdf',
                        'public', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z',
                        sha256(CAST(i AS VARCHAR))
                 FROM range(2, 50_002) t(i);",
            )
            .map_err(|error| AppError::Storage(error.to_string()))
        })
        .await
        .unwrap();
        let started = Instant::now();
        let (digest, documents) = repo.corpus_state_identity().await.unwrap();
        eprintln!(
            "corpus_state_identity over {documents} documents: {:?} ({})",
            started.elapsed(),
            &digest[..8]
        );
    }
}
