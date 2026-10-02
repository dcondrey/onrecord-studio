//! Request and passage search over the token projection, and the corpus backfill reader.

use super::*;

impl DuckDbRepository {
    /// Loads at most one document descriptor using a stable keyset cursor. The corresponding
    /// full text is loaded only when the derived corpus index is stale or missing.
    pub async fn next_corpus_source_document(
        &self,
        after: Option<(String, u64)>,
    ) -> Result<Option<CorpusSourceDocument>, AppError> {
        let (after_portal, after_document_id) = match after {
            Some((portal_key, document_id)) => (Some(portal_key), document_id),
            None => (None, 0),
        };
        let after_document_id = i64::try_from(after_document_id).map_err(|_| {
            AppError::DataParsing("Corpus backfill cursor exceeds SQLite capacity".to_string())
        })?;
        self.query_connection("Corpus backfill source query", move |connection| {
            let mut statement = connection
                .prepare(
                    "SELECT
                        documents.portal_subdomain,
                        documents.request_id,
                        documents.id,
                        documents.raw_file_sha256,
                        documents.extracted_text_sha256
                     FROM documents
                     WHERE documents.download_status = 'completed'
                       AND LENGTH(documents.raw_file_sha256) = 64
                       AND LENGTH(documents.extracted_text_sha256) = 64
                       AND EXISTS (
                           SELECT 1 FROM documents_fts
                           WHERE documents_fts.document_id = documents.id
                             AND documents_fts.portal_subdomain = documents.portal_subdomain
                             AND documents_fts.extracted_text <> ''
                       )
                       AND (?1 IS NULL
                            OR documents.portal_subdomain > ?1
                            OR (documents.portal_subdomain = ?1 AND documents.id > ?2))
                     ORDER BY documents.portal_subdomain, documents.id
                     LIMIT 1",
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to prepare corpus backfill source: {error}"))
                })?;
            let mut rows = statement
                .query(params![after_portal, after_document_id])
                .map_err(|error| {
                    AppError::Storage(format!("Failed to query corpus backfill source: {error}"))
                })?;
            let Some(row) = rows.next().map_err(|error| {
                AppError::Storage(format!("Failed to read corpus backfill source: {error}"))
            })?
            else {
                return Ok(None);
            };
            let request_id = row.get::<_, i64>(1).map_err(|error| {
                AppError::Storage(format!("Invalid corpus request identifier: {error}"))
            })?;
            let document_id = row.get::<_, i64>(2).map_err(|error| {
                AppError::Storage(format!("Invalid corpus document identifier: {error}"))
            })?;
            Ok(Some(CorpusSourceDocument {
                portal_key: row.get(0).map_err(|error| {
                    AppError::Storage(format!("Invalid corpus portal key: {error}"))
                })?,
                request_id: u64::try_from(request_id).map_err(|_| {
                    AppError::Storage("Corpus request identifier was negative".to_string())
                })?,
                document_id: u64::try_from(document_id).map_err(|_| {
                    AppError::Storage("Corpus document identifier was negative".to_string())
                })?,
                raw_file_sha256: row.get(3).map_err(|error| {
                    AppError::Storage(format!("Invalid corpus raw digest: {error}"))
                })?,
                extracted_text_sha256: row.get(4).map_err(|error| {
                    AppError::Storage(format!("Invalid corpus text digest: {error}"))
                })?,
            }))
        })
        .await
    }

    pub async fn load_corpus_source_text(
        &self,
        portal_key: &str,
        document_id: u64,
    ) -> Result<String, AppError> {
        let portal_key = portal_key.to_string();
        self.query_connection("Corpus source text query", move |connection| {
            connection
                .query_row(
                    "SELECT extracted_text
                     FROM documents_fts
                     WHERE document_id = ?1 AND portal_subdomain = ?2
                     ORDER BY document_id DESC
                     LIMIT 1",
                    params![document_id, portal_key],
                    |row| row.get::<_, String>(0),
                )
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to load corpus source text for {portal_key}/{document_id}: {error}"
                    ))
                })
        })
        .await
    }

    /// Find documents in the corpus whose extracted text mentions terms overlapping with a
    /// redaction's context, excluding the redacted document itself. A document is identified by
    /// its portal and id together, so another portal's document with the same id stays a
    /// candidate. A copy of the redacted document (same extracted-text digest) is excluded and
    /// identical copies elsewhere count once, since neither is independent corroboration. The
    /// excerpt is taken around the first term's match, so values mined from it come from the
    /// matching passage rather than the document's opening. Hits are ranked by matched-term count
    /// before the limit applies. Returns per-hit
    /// `(portal_key, document_id, excerpt, matched_term_count, total_term_count)`.
    ///
    /// The excerpt is a bounded prefix so the caller isn't handed a whole document and can render
    /// the match trail without a second round-trip. Case-insensitive substring matching per term;
    /// deliberately lightweight — this feeds the redaction-inference engine's lexical fallback
    /// when the fastembed model isn't initialized.
    pub async fn find_redaction_context_matches(
        &self,
        exclude_portal_key: &str,
        exclude_document_id: i64,
        terms: Vec<String>,
        limit: usize,
    ) -> Result<Vec<(String, i64, String, u32, u32)>, AppError> {
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        let term_count = u32::try_from(terms.len()).unwrap_or(u32::MAX);
        let limit_i64 = i64::try_from(limit.max(1)).unwrap_or(i64::MAX);
        let exclude_portal_key = exclude_portal_key.to_string();
        self.query_connection("Find redaction context matches", move |connection| {
            // Build a WHERE clause requiring the extracted text to LIKE-match at least the first
            // term (a strong filter), and rank hits by how many additional terms match. Only the
            // first term is a hard filter to keep the SQL bounded; the caller passes terms in
            // order of specificity, so the first is usually the highest-signal.
            let mut sql = String::from(
                "SELECT documents.portal_subdomain, documents.id, \
                        SUBSTR(documents_fts.extracted_text, \
                               GREATEST(1, STRPOS(LOWER(documents_fts.extracted_text), ?4) - 200), \
                               400) AS excerpt",
            );
            let mut term_hits = Vec::with_capacity(terms.len());
            for (i, _) in terms.iter().enumerate() {
                use std::fmt::Write;
                let hit = format!(
                    "CASE WHEN LOWER(documents_fts.extracted_text) LIKE '%' || ?{} || '%' THEN 1 ELSE 0 END",
                    i + 4
                );
                let _ = write!(sql, ", {hit}");
                term_hits.push(hit);
            }
            sql.push_str(
                " FROM documents \
                 INNER JOIN documents_fts \
                    ON documents_fts.document_id = documents.id \
                   AND documents_fts.portal_subdomain = documents.portal_subdomain \
                 WHERE NOT (documents.id = ?1 AND documents.portal_subdomain = ?3) \
                   AND LOWER(TRIM(documents.visibility)) = 'public' \
                   AND LENGTH(documents_fts.extracted_text) > 0 \
                   AND LOWER(documents_fts.extracted_text) LIKE '%' || ?4 || '%' \
                   AND NOT COALESCE(documents.extracted_text_sha256 = ( \
                       SELECT source.extracted_text_sha256 FROM documents AS source \
                       WHERE source.id = ?1 AND source.portal_subdomain = ?3), FALSE) \
                 QUALIFY ROW_NUMBER() OVER ( \
                     PARTITION BY COALESCE(documents.extracted_text_sha256, \
                                           documents.portal_subdomain || ':' || documents.id) \
                     ORDER BY documents.portal_subdomain, documents.id) = 1 ",
            );
            sql.push_str(&format!(
                "ORDER BY {} DESC, documents.portal_subdomain, documents.id LIMIT ?2",
                term_hits.join(" + ")
            ));
            let mut statement = connection.prepare(&sql).map_err(|error| {
                AppError::Storage(format!(
                    "Failed to prepare redaction-context-match query: {error}"
                ))
            })?;
            // Bind exclude id, limit, exclude portal, then each term.
            let mut params_vec: Vec<duckdb::types::Value> = Vec::with_capacity(3 + terms.len());
            params_vec.push(exclude_document_id.into());
            params_vec.push(limit_i64.into());
            params_vec.push(exclude_portal_key.into());
            for term in &terms {
                params_vec.push(term.clone().into());
            }
            let param_refs: Vec<&dyn duckdb::ToSql> =
                params_vec.iter().map(|v| v as &dyn duckdb::ToSql).collect();
            let column_count = 3 + terms.len();
            let rows = statement
                .query_map(param_refs.as_slice(), |row| {
                    let portal_key: String = row.get(0)?;
                    let document_id: i64 = row.get(1)?;
                    let excerpt: String = row.get(2)?;
                    let mut matched: u32 = 0;
                    for i in 3..column_count {
                        let hit: i64 = row.get(i)?;
                        if hit != 0 {
                            matched += 1;
                        }
                    }
                    Ok((portal_key, document_id, excerpt, matched))
                })
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to execute redaction-context-match query: {error}"
                    ))
                })?;
            let mut out: Vec<(String, i64, String, u32, u32)> = Vec::new();
            for row in rows {
                let (portal_key, document_id, excerpt, matched) = row.map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to decode redaction-context-match row: {error}"
                    ))
                })?;
                out.push((portal_key, document_id, excerpt, matched, term_count));
            }
            // Prefer hits with more matched terms.
            out.sort_by_key(|(_, _, _, matched, _)| std::cmp::Reverse(*matched));
            Ok(out)
        })
        .await
    }

    /// Documents still missing from the search token projection. Zero means a search covers the
    /// whole corpus; anything else means it would not, which is a different fact and is reported
    /// as one.
    pub fn pending_search_index_documents(&self) -> u64 {
        self.search_index_pending.load(Ordering::Relaxed)
    }

    /// Whether the background drain stopped short. Arrears that are stalled and arrears that are
    /// still draining are different facts, and a reader is told which one it is.
    pub fn search_index_stalled(&self) -> bool {
        self.search_index_stalled.load(Ordering::Relaxed)
    }

    /// Builds the token projection for documents created before the persistent index existed.
    /// Each batch is atomic and restartable, and the writer lock is released between batches so
    /// ingestion and analytics keep running while the arrears drain.
    pub async fn backfill_search_token_index(&self) -> Result<u64, AppError> {
        let mut completed = 0_u64;
        loop {
            let processed = match self
                .query_connection("Backfill search token batch", |conn| {
                    backfill_search_token_batch(conn)
                })
                .await
            {
                Ok(processed) => processed,
                Err(error) => {
                    self.search_index_stalled.store(true, Ordering::Relaxed);
                    return Err(error);
                }
            };
            if processed == 0 {
                self.search_index_pending.store(0, Ordering::Relaxed);
                return Ok(completed);
            }
            completed = completed.saturating_add(processed);
            self.search_index_pending
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |pending| {
                    Some(pending.saturating_sub(processed))
                })
                .ok();
            // A bare yield returns to this task immediately when nothing else is ready, which
            // keeps the writer lock hot and leaves interactive queries queued behind the drain.
            // A short sleep hands the lock to the frontend's readers between batches.
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    }

    pub async fn search_requests(
        &self,
        search_term: &str,
        portal_key: Option<&str>,
        agency_id: Option<u64>,
        status: Option<&str>,
        policy_deviation: Option<&str>,
        limit: usize,
    ) -> Result<Vec<RequestSearchResult>, AppError> {
        // The flag separator below is `chr(31)`, not `char(31)`. DuckDB parses `char` as a type
        // keyword, so `char(31)` reads as a type modifier and the whole statement fails to parse
        // at prepare time. SQLite accepts `char(31)`, which is where it came from, and the
        // evidence search returned an error banner instead of results for every query.
        if !(1..=5_000).contains(&limit) {
            return Err(AppError::DataParsing(
                "Request search limit must be between 1 and 5000".to_string(),
            ));
        }
        let search_tokens = core_query_tokens(search_term)
            .map_err(|message| AppError::DataParsing(message.to_string()))?;
        let fts_query = (!search_tokens.is_empty()).then_some(search_term.trim().to_string());
        let filters = request_search_filters(&search_tokens);
        let agency_id = agency_id.map(i64::try_from).transpose().map_err(|_| {
            AppError::DataParsing("Agency identifier exceeds SQLite capacity".to_string())
        })?;
        let limit = i64::try_from(limit).map_err(|_| {
            AppError::DataParsing("Search limit exceeds SQLite capacity".to_string())
        })?;
        let portal_key = portal_key
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let status = status
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let policy_deviation = policy_deviation
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        self.query_connection("Request search", move |conn| {
            let query = format!(
                "SELECT
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
                 FROM requests
                 LEFT JOIN agencies
                   ON agencies.id = requests.agency_id
                  AND agencies.portal_subdomain = requests.portal_subdomain
                 WHERE {filters}
                 ORDER BY requests.created_at DESC, requests.portal_subdomain, requests.id
                 LIMIT ?6"
            );
            let mut statement = conn.prepare(&query).map_err(|error| {
                AppError::Storage(format!("Failed to prepare request search: {error}"))
            })?;
            let rows = statement
                .query_map(
                    params![
                        portal_key,
                        agency_id,
                        status,
                        policy_deviation,
                        fts_query,
                        limit
                    ],
                    map_request_search_row,
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to execute request search: {error}"))
                })?;
            let mut results = Vec::new();
            for row in rows {
                let row = row.map_err(|error| {
                    AppError::Storage(format!("Failed to decode request search result: {error}"))
                })?;
                let title = crate::storage::pii::mask_pii(&row.title)?.masked_text;
                let description = crate::storage::pii::mask_pii(&row.description)?.masked_text;
                let agency_name = crate::storage::pii::mask_pii(&row.agency_name)?.masked_text;
                results.push(RequestSearchResult {
                    request_id: row.request_id,
                    portal_key: row.portal_key,
                    agency_id: row.agency_id,
                    agency_name,
                    title,
                    description_preview: description.chars().take(320).collect(),
                    status: row.status,
                    created_at: row.created_at,
                    closed_at: row.closed_at,
                    document_count: usize::try_from(row.document_count).map_err(|_| {
                        AppError::Storage("Document count was negative".to_string())
                    })?,
                    integrity_flags: row
                        .integrity_flags
                        .split('\u{1f}')
                        .filter(|value| !value.is_empty())
                        .map(str::to_string)
                        .collect(),
                });
            }
            Ok(results)
        })
        .await
    }
}

/// The WHERE conditions shared by every request search: `?1` portal, `?2` agency, `?3` status,
/// `?4` policy flag, `?5` the free-text term (NULL when none). Counting and paging reuse this
/// text so a total can never describe a different set than the pages it is shown beside.
pub(crate) fn request_search_filters(search_tokens: &[String]) -> String {
    let token_predicate = if search_tokens.is_empty() {
        "TRUE".to_string()
    } else {
        let values = search_tokens
            .iter()
            .map(|token| format!("'{}'", token.replace('\'', "''")))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "(SELECT COUNT(DISTINCT matched.token)
              FROM (
                SELECT LEFT(LOWER(UNNEST(regexp_split_to_array(
                    concat_ws(' ', requests.title, requests.description, requests.rpid,
                        CAST(requests.id AS VARCHAR)), '[^\\p{{L}}\\p{{N}}]+'))), 128) AS token
                UNION ALL
                SELECT document_search_tokens.token
                FROM documents
                INNER JOIN document_search_tokens
                  ON document_search_tokens.document_id = documents.id
                 AND document_search_tokens.portal_subdomain = documents.portal_subdomain
                WHERE documents.request_id = requests.id
                  AND documents.portal_subdomain = requests.portal_subdomain
              ) matched
              WHERE matched.token IN ({values})) = {}",
            search_tokens.len()
        )
    };
    format!(
        "(?1 IS NULL OR requests.portal_subdomain = ?1)
           AND (?2 IS NULL OR requests.agency_id = ?2)
           AND (?3 IS NULL OR LOWER(requests.status) = LOWER(?3))
           AND (?4 IS NULL OR EXISTS (
                SELECT 1
                FROM case_audits
                INNER JOIN document_integrity_flags
                  ON document_integrity_flags.audit_id = case_audits.audit_id
                WHERE case_audits.target_request_id = requests.id
                  AND case_audits.target_subdomain = requests.portal_subdomain
                  AND document_integrity_flags.flag_type = ?4
           ))
           AND (?5 IS NULL OR {token_predicate})"
    )
}

fn map_request_search_row(row: &Row<'_>) -> duckdb::Result<RequestSearchRow> {
    Ok(RequestSearchRow {
        request_id: row.get(0)?,
        portal_key: row.get(1)?,
        agency_id: row.get(2)?,
        agency_name: row.get(3)?,
        title: row.get(4)?,
        description: row.get(5)?,
        status: row.get(6)?,
        created_at: row.get(7)?,
        closed_at: row.get(8)?,
        document_count: row.get(9)?,
        integrity_flags: row.get(10)?,
    })
}

pub(super) fn synchronize_fts_row(
    tx: &Transaction<'_>,
    document_id: i64,
    subdomain: &str,
    file_name: Option<&str>,
    cleaned_text: &str,
) -> Result<(), AppError> {
    let updated_rows = tx
        .execute(
            "UPDATE documents_fts
             SET file_name = ?1, extracted_text = ?2
             WHERE document_id = ?3 AND portal_subdomain = ?4",
            params![file_name, cleaned_text, document_id, subdomain],
        )
        .map_err(|error| {
            AppError::Internal(format!(
                "Full-text search conflict-update branch failed: {error}"
            ))
        })?;

    if updated_rows == 0 {
        tx.execute(
            "INSERT INTO documents_fts (
                document_id,
                portal_subdomain,
                file_name,
                extracted_text
             ) VALUES (?1, ?2, ?3, ?4)",
            params![document_id, subdomain, file_name, cleaned_text],
        )
        .map_err(|error| {
            AppError::Internal(format!(
                "Full-text search conflict-insert branch failed: {error}"
            ))
        })?;
    }

    let synchronized_row_count = tx
        .query_row(
            "SELECT COUNT(*)
             FROM documents_fts
             WHERE document_id = ?1 AND portal_subdomain = ?2",
            params![document_id, subdomain],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| {
            AppError::Internal(format!(
                "Failed to verify full-text search synchronization: {error}"
            ))
        })?;

    if synchronized_row_count != 1 {
        return Err(AppError::Internal(format!(
            "Full-text search synchronization produced {synchronized_row_count} rows for document {document_id} in portal '{subdomain}'"
        )));
    }

    synchronize_search_tokens(tx, document_id, subdomain, cleaned_text)?;

    Ok(())
}

const SEARCH_TOKEN_INDEX_VERSION: i64 = 1;

const SEARCH_TOKEN_BACKFILL_BATCH_SIZE: i64 = 8;

/// The longest text projected into the token index. The corpus holds bulk data exports -- a
/// 128 MB CSV of service requests, for one -- whose tokens are overwhelmingly identifiers and
/// coordinates: millions of rows of index for no readable match. Beyond this point a document is
/// indexed to its prefix and `indexed_characters` records how far that reached.
pub(super) const SEARCH_TOKEN_MAX_INDEXED_CHARACTERS: usize = 4_000_000;

/// Token rows written per statement. Bounded so a single document with millions of distinct tokens
/// cannot build one statement large enough to matter.
const SEARCH_TOKEN_INSERT_CHUNK_SIZE: usize = 512;

pub(super) fn synchronize_search_tokens(
    tx: &Transaction<'_>,
    document_id: i64,
    portal_subdomain: &str,
    cleaned_text: &str,
) -> Result<(), AppError> {
    tx.execute(
        "DELETE FROM document_search_tokens
         WHERE document_id = ?1 AND portal_subdomain = ?2",
        params![document_id, portal_subdomain],
    )
    .map_err(|error| AppError::Storage(format!("Failed to clear stale search tokens: {error}")))?;

    // One statement per distinct token is the difference between seconds and hours here: the corpus
    // holds documents with over a hundred megabytes of extracted text, and DuckDB pays full
    // statement overhead on every single-row insert. Rows are written in bounded multi-value
    // batches instead, which keeps one document's projection inside one transaction either way.
    let indexed_text = match cleaned_text
        .char_indices()
        .nth(SEARCH_TOKEN_MAX_INDEXED_CHARACTERS)
    {
        Some((boundary, _)) => &cleaned_text[..boundary],
        None => cleaned_text,
    };
    let indexed_characters = i64::try_from(indexed_text.chars().count()).map_err(|_| {
        AppError::Storage("Indexed character count exceeds DuckDB capacity".to_string())
    })?;
    let frequencies = token_frequencies(indexed_text);
    let entries: Vec<(&String, i64)> = frequencies
        .iter()
        .map(|(token, frequency)| {
            i64::try_from(*frequency)
                .map(|frequency| (token, frequency))
                .map_err(|_| {
                    AppError::Storage("Search term frequency exceeds DuckDB capacity".to_string())
                })
        })
        .collect::<Result<Vec<_>, AppError>>()?;
    for chunk in entries.chunks(SEARCH_TOKEN_INSERT_CHUNK_SIZE) {
        let placeholders = std::iter::repeat_n("(?, ?, ?, ?)", chunk.len())
            .collect::<Vec<_>>()
            .join(", ");
        let mut values: Vec<duckdb::types::Value> = Vec::with_capacity(chunk.len() * 4);
        for (token, frequency) in chunk {
            values.push(duckdb::types::Value::BigInt(document_id));
            values.push(duckdb::types::Value::Text(portal_subdomain.to_string()));
            values.push(duckdb::types::Value::Text((*token).clone()));
            values.push(duckdb::types::Value::BigInt(*frequency));
        }
        tx.execute(
            &format!(
                "INSERT INTO document_search_tokens (
                    document_id, portal_subdomain, token, term_frequency
                 ) VALUES {placeholders}"
            ),
            params_from_iter(values.iter()),
        )
        .map_err(|error| {
            AppError::Storage(format!("Failed to persist a search token batch: {error}"))
        })?;
    }

    let token_count = i64::try_from(frequencies.len())
        .map_err(|_| AppError::Storage("Search token count exceeds DuckDB capacity".to_string()))?;
    tx.execute(
        "INSERT INTO document_search_index_state (
            document_id, portal_subdomain, index_version, token_count, indexed_characters
         ) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(document_id, portal_subdomain) DO UPDATE SET
            index_version = excluded.index_version,
            token_count = excluded.token_count,
            indexed_characters = excluded.indexed_characters",
        params![
            document_id,
            portal_subdomain,
            SEARCH_TOKEN_INDEX_VERSION,
            token_count,
            indexed_characters
        ],
    )
    .map_err(|error| AppError::Storage(format!("Failed to persist search index state: {error}")))?;
    Ok(())
}

/// Counts the documents whose tokens have not been projected at the current index version.
pub(super) fn count_pending_search_index_documents(conn: &Connection) -> Result<u64, AppError> {
    conn.query_row(
        "SELECT COUNT(*)
         FROM documents_fts
         LEFT JOIN document_search_index_state
           ON document_search_index_state.document_id = documents_fts.document_id
          AND document_search_index_state.portal_subdomain = documents_fts.portal_subdomain
          AND document_search_index_state.index_version = ?1
         WHERE document_search_index_state.document_id IS NULL",
        params![SEARCH_TOKEN_INDEX_VERSION],
        |row| row.get::<_, i64>(0),
    )
    .map(|count| count.max(0) as u64)
    .map_err(|error| AppError::Storage(format!("Failed to count search index arrears: {error}")))
}

/// Projects one bounded batch and returns how many documents it covered. Zero means the projection
/// has caught up.
fn backfill_search_token_batch(conn: &mut Connection) -> Result<u64, AppError> {
    let rows = {
        let mut statement = conn
                .prepare(
                    "SELECT
                        documents_fts.document_id,
                        documents_fts.portal_subdomain,
                        documents_fts.extracted_text
                     FROM documents_fts
                     LEFT JOIN document_search_index_state
                       ON document_search_index_state.document_id = documents_fts.document_id
                      AND document_search_index_state.portal_subdomain = documents_fts.portal_subdomain
                      AND document_search_index_state.index_version = ?1
                     WHERE document_search_index_state.document_id IS NULL
                     ORDER BY documents_fts.portal_subdomain, documents_fts.document_id
                     LIMIT ?2",
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to prepare search index backfill: {error}"))
                })?;
        statement
            .query_map(
                params![SEARCH_TOKEN_INDEX_VERSION, SEARCH_TOKEN_BACKFILL_BATCH_SIZE],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .map_err(|error| {
                AppError::Storage(format!("Failed to query search index backfill: {error}"))
            })?
            .collect::<duckdb::Result<Vec<_>>>()
            .map_err(|error| {
                AppError::Storage(format!("Failed to decode search index backfill: {error}"))
            })?
    };
    if rows.is_empty() {
        return Ok(0);
    }

    // Empty documents receive a state row with a zero token count, so the keyset scan still
    // terminates rather than re-selecting them forever.
    let covered = rows.len() as u64;
    let transaction = conn.transaction().map_err(|error| {
        AppError::Storage(format!("Failed to begin search index backfill: {error}"))
    })?;
    for (document_id, portal_subdomain, extracted_text) in rows {
        synchronize_search_tokens(
            &transaction,
            document_id,
            &portal_subdomain,
            &extracted_text,
        )?;
    }
    transaction.commit().map_err(|error| {
        AppError::Storage(format!("Failed to commit search index backfill: {error}"))
    })?;
    Ok(covered)
}
