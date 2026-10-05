//! Bookkeeping for the automatic comparison of redaction marks against other sources.
//!
//! Four derived tables, all rebuildable from `documents_fts` and the imported tables:
//! `redaction_sweep_documents` lists every document taken in and the run it was last read in as a
//! possible corroborating source, `redaction_sweep_spans` holds the words around each redaction
//! mark, `redaction_sweep_tables` holds how far each imported table has been read, and
//! `redaction_sweep_matches` holds where the same words were found with text between them.
//!
//! Runs are numbered. A source read in run `n` has been compared with every span added in or
//! before run `n`, so a source needs reading again exactly when a span newer than its
//! `scanned_run` exists. That one comparison is what lets an old redacted document meet a new
//! source and a new redacted document meet every old source, without ordering the corpus.
//!
//! A document is identified by portal and id together. Hand-added files (`user_supplied_sources`)
//! are never taken in, on either side.

use super::sqlite::DuckDbRepository;
use crate::error::AppError;
use duckdb::{params, Connection};

fn storage(error: impl std::fmt::Display) -> AppError {
    AppError::Storage(format!("Redaction sweep: {error}"))
}

fn literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// Public documents collected from a source, with stored text no larger than the cap.
fn eligible_documents(max_text_bytes: i64) -> String {
    format!(
        "SELECT d.portal_subdomain AS portal_key, d.id AS document_id,
                concat(COALESCE(d.extracted_text_projection_sha256, d.extracted_text_sha256, ''),
                       ':', strlen(f.extracted_text)) AS text_version
         FROM documents d
         INNER JOIN documents_fts f
            ON f.document_id = d.id AND f.portal_subdomain = d.portal_subdomain
         WHERE LOWER(TRIM(d.visibility)) = 'public'
           AND d.portal_subdomain NOT IN (SELECT subdomain FROM user_supplied_sources)
           AND strlen(f.extracted_text) BETWEEN 1 AND {max_text_bytes}"
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DocumentKey {
    pub portal_key: String,
    pub document_id: i64,
}

#[derive(Debug)]
pub(crate) struct SweepDocument {
    pub key: DocumentKey,
    pub text_version: String,
    pub text: String,
}

/// The words around one redaction mark. Offsets are characters of the document's stored text and
/// cover the left anchor, the mark and the right anchor.
#[derive(Debug, Clone)]
pub(crate) struct SweepSpan {
    pub document: DocumentKey,
    pub passage_start: i64,
    pub passage_end: i64,
    pub left_anchor: String,
    pub right_anchor: String,
    /// The anchors cut to the mark's own line, when it has three words on each side.
    pub row_anchors: Option<(String, String)>,
    pub quote: String,
    pub person_like: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct SweepAnchor {
    pub document: DocumentKey,
    pub passage_start: i64,
    pub left_anchor: String,
    pub right_anchor: String,
    pub row_anchors: Option<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MatchSource {
    /// Characters of the passage in another document's stored text.
    Document {
        key: DocumentKey,
        start: i64,
        end: i64,
    },
    TableCell {
        snapshot_id: String,
        row_number: i64,
        column_name: String,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct SweepMatch {
    pub document: DocumentKey,
    pub passage_start: i64,
    pub source: MatchSource,
    pub value_sha256: String,
    pub name_shaped: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct SweepTable {
    pub snapshot_id: String,
    pub source_sha256: String,
    pub resume_after_row: i64,
    pub last_row: i64,
}

/// One stored match with what a finding needs to cite both sides.
#[derive(Debug)]
pub(crate) struct SweepCandidate {
    pub document: DocumentKey,
    pub passage_start: i64,
    pub passage_end: i64,
    pub quote: String,
    pub person_like: bool,
    pub file_name: Option<String>,
    pub request_id: Option<i64>,
    pub source: MatchSource,
    pub value_sha256: String,
    pub name_shaped: bool,
    pub source_file_name: Option<String>,
    /// Text digest of a source document, or file digest of a source table: two sources with the
    /// same one are copies, not two observations.
    pub source_digest: String,
    pub table: Option<SweepTableCitation>,
}

#[derive(Debug, Clone)]
pub(crate) struct SweepTableCitation {
    pub display_name: String,
    pub source_uri: Option<String>,
    pub source_sha256: String,
    pub retrieved_at: String,
}

/// Same-value matches kept per span. Later ones add nothing a finding states.
const MAX_STORED_MATCHES_PER_VALUE: i64 = 8;

fn text_bytes(text_version: &str) -> u64 {
    text_version
        .rsplit(':')
        .next()
        .and_then(|bytes| bytes.parse().ok())
        .unwrap_or(0)
}

/// Reads the listed documents' text, stopping before the byte budget is passed. The first
/// document is always read, so one document larger than the budget cannot stall the sweep.
fn read_texts(
    conn: &Connection,
    listed: Vec<(DocumentKey, String)>,
    max_bytes: u64,
) -> Result<Vec<SweepDocument>, AppError> {
    let mut statement = conn
        .prepare(
            "SELECT extracted_text FROM documents_fts
             WHERE document_id = ?1 AND portal_subdomain = ?2",
        )
        .map_err(storage)?;
    let mut documents = Vec::new();
    let mut bytes = 0_u64;
    for (key, text_version) in listed {
        bytes += text_bytes(&text_version);
        if !documents.is_empty() && bytes > max_bytes {
            break;
        }
        let text: String = statement
            .query_row(params![key.document_id, key.portal_key], |row| row.get(0))
            .map_err(storage)?;
        documents.push(SweepDocument {
            key,
            text_version,
            text,
        });
    }
    Ok(documents)
}

fn insert_matches(tx: &duckdb::Transaction<'_>, matches: &[SweepMatch]) -> Result<(), AppError> {
    let mut statement = tx
        .prepare(
            "INSERT INTO redaction_sweep_matches (portal_key, document_id, passage_start,
                source_kind, source_scope, source_item, value_sha256, name_shaped,
                source_start, source_end, column_name)
             SELECT ?1::TEXT, ?2::BIGINT, ?3::BIGINT, ?4::TEXT, ?5::TEXT, ?6::BIGINT, ?7::TEXT,
                    ?8::BOOLEAN, ?9::BIGINT, ?10::BIGINT, ?11::TEXT
             WHERE (SELECT COUNT(*) FROM redaction_sweep_matches m
                    WHERE m.portal_key = ?1 AND m.document_id = ?2 AND m.passage_start = ?3
                      AND m.value_sha256 = ?7) < ?12
             ON CONFLICT DO NOTHING",
        )
        .map_err(storage)?;
    for found in matches {
        let (kind, scope, item, start, end, column) = match &found.source {
            MatchSource::Document { key, start, end } => (
                "DOCUMENT",
                key.portal_key.as_str(),
                key.document_id,
                Some(*start),
                Some(*end),
                None,
            ),
            MatchSource::TableCell {
                snapshot_id,
                row_number,
                column_name,
            } => (
                "TABLE",
                snapshot_id.as_str(),
                *row_number,
                None,
                None,
                Some(column_name.as_str()),
            ),
        };
        statement
            .execute(params![
                found.document.portal_key,
                found.document.document_id,
                found.passage_start,
                kind,
                scope,
                item,
                found.value_sha256,
                found.name_shaped,
                start,
                end,
                column,
                MAX_STORED_MATCHES_PER_VALUE,
            ])
            .map_err(storage)?;
    }
    tx.execute_batch(
        "UPDATE redaction_sweep_spans SET reused_wording = TRUE
         WHERE NOT reused_wording
           AND (SELECT COUNT(DISTINCT m.value_sha256) FROM redaction_sweep_matches m
                WHERE m.portal_key = redaction_sweep_spans.portal_key
                  AND m.document_id = redaction_sweep_spans.document_id
                  AND m.passage_start = redaction_sweep_spans.passage_start) > 1",
    )
    .map_err(storage)
}

impl DuckDbRepository {
    /// Drops bookkeeping for documents and tables that are gone, no longer eligible or changed,
    /// and returns this run's number. A changed document loses its row and is taken in again.
    pub(crate) async fn begin_redaction_sweep(&self, max_text_bytes: i64) -> Result<i64, AppError> {
        self.query_connection("Begin redaction sweep", move |conn| {
            let tx = conn.transaction().map_err(storage)?;
            tx.execute_batch(&format!(
                "DELETE FROM redaction_sweep_documents WHERE NOT EXISTS (
                     SELECT 1 FROM ({eligible}) e
                     WHERE e.portal_key = redaction_sweep_documents.portal_key
                       AND e.document_id = redaction_sweep_documents.document_id
                       AND e.text_version = redaction_sweep_documents.text_version);
                 DELETE FROM redaction_sweep_spans WHERE NOT EXISTS (
                     SELECT 1 FROM redaction_sweep_documents l
                     WHERE l.portal_key = redaction_sweep_spans.portal_key
                       AND l.document_id = redaction_sweep_spans.document_id);
                 DELETE FROM redaction_sweep_tables WHERE NOT EXISTS (
                     SELECT 1 FROM analytical_snapshots s
                     WHERE s.snapshot_id = redaction_sweep_tables.snapshot_id
                       AND s.source_sha256 = redaction_sweep_tables.source_sha256);
                 DELETE FROM redaction_sweep_matches WHERE NOT EXISTS (
                         SELECT 1 FROM redaction_sweep_spans s
                         WHERE s.portal_key = redaction_sweep_matches.portal_key
                           AND s.document_id = redaction_sweep_matches.document_id
                           AND s.passage_start = redaction_sweep_matches.passage_start)
                     OR (source_kind = 'DOCUMENT' AND NOT EXISTS (
                         SELECT 1 FROM redaction_sweep_documents l
                         WHERE l.portal_key = redaction_sweep_matches.source_scope
                           AND l.document_id = redaction_sweep_matches.source_item))
                     OR (source_kind = 'TABLE' AND NOT EXISTS (
                         SELECT 1 FROM redaction_sweep_tables t
                         WHERE t.snapshot_id = redaction_sweep_matches.source_scope));",
                eligible = eligible_documents(max_text_bytes)
            ))
            .map_err(storage)?;
            let run: i64 = tx
                .query_row(
                    "SELECT 1 + GREATEST(
                        COALESCE((SELECT MAX(added_run) FROM redaction_sweep_spans), 0),
                        COALESCE((SELECT MAX(scanned_run) FROM redaction_sweep_documents), 0),
                        COALESCE((SELECT MAX(GREATEST(COALESCE(scanned_run, 0), COALESCE(pass_run, 0)))
                                  FROM redaction_sweep_tables), 0))",
                    [],
                    |row| row.get(0),
                )
                .map_err(storage)?;
            tx.commit().map_err(storage)?;
            Ok(run)
        })
        .await
    }

    /// Eligible documents not yet taken in, in portal and id order, within the two budgets.
    pub(crate) async fn load_unswept_documents(
        &self,
        max_text_bytes: i64,
        max_documents: usize,
        max_bytes: u64,
    ) -> Result<Vec<SweepDocument>, AppError> {
        self.query_connection("Load unswept documents", move |conn| {
            let listed = conn
                .prepare(&format!(
                    "SELECT e.portal_key, e.document_id, e.text_version FROM ({}) e
                     WHERE NOT EXISTS (SELECT 1 FROM redaction_sweep_documents l
                         WHERE l.portal_key = e.portal_key AND l.document_id = e.document_id)
                     ORDER BY e.portal_key, e.document_id LIMIT ?1",
                    eligible_documents(max_text_bytes)
                ))
                .map_err(storage)?
                .query_map(params![max_documents as i64], |row| {
                    Ok((
                        DocumentKey {
                            portal_key: row.get(0)?,
                            document_id: row.get(1)?,
                        },
                        row.get::<_, String>(2)?,
                    ))
                })
                .map_err(storage)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(storage)?;
            read_texts(conn, listed, max_bytes)
        })
        .await
    }

    /// Spans still being compared, and the newest run any of them was added in.
    pub(crate) async fn load_sweep_anchors(&self) -> Result<(Vec<SweepAnchor>, i64), AppError> {
        self.query_connection("Load redaction anchors", |conn| {
            let anchors = conn
                .prepare(
                    "SELECT portal_key, document_id, passage_start, left_anchor, right_anchor,
                            row_left_anchor, row_right_anchor
                     FROM redaction_sweep_spans WHERE NOT reused_wording
                     ORDER BY portal_key, document_id, passage_start",
                )
                .map_err(storage)?
                .query_map([], |row| {
                    Ok(SweepAnchor {
                        document: DocumentKey {
                            portal_key: row.get(0)?,
                            document_id: row.get(1)?,
                        },
                        passage_start: row.get(2)?,
                        left_anchor: row.get(3)?,
                        right_anchor: row.get(4)?,
                        row_anchors: row
                            .get::<_, Option<String>>(5)?
                            .zip(row.get::<_, Option<String>>(6)?),
                    })
                })
                .map_err(storage)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(storage)?;
            let newest: i64 = conn
                .query_row(
                    "SELECT COALESCE(MAX(added_run), 0) FROM redaction_sweep_spans
                     WHERE NOT reused_wording",
                    [],
                    |row| row.get(0),
                )
                .map_err(storage)?;
            Ok((anchors, newest))
        })
        .await
    }

    /// Documents last read before `newest_span_run`, longest-waiting first, within the budgets.
    pub(crate) async fn load_documents_to_rescan(
        &self,
        newest_span_run: i64,
        max_documents: usize,
        max_bytes: u64,
    ) -> Result<Vec<SweepDocument>, AppError> {
        self.query_connection("Load documents to rescan", move |conn| {
            let listed = conn
                .prepare(
                    "SELECT portal_key, document_id, text_version FROM redaction_sweep_documents
                     WHERE scanned_run < ?1
                     ORDER BY scanned_run, portal_key, document_id LIMIT ?2",
                )
                .map_err(storage)?
                .query_map(params![newest_span_run, max_documents as i64], |row| {
                    Ok((
                        DocumentKey {
                            portal_key: row.get(0)?,
                            document_id: row.get(1)?,
                        },
                        row.get::<_, String>(2)?,
                    ))
                })
                .map_err(storage)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(storage)?;
            read_texts(conn, listed, max_bytes)
        })
        .await
    }

    /// Records one run's work on documents in a single transaction: the documents taken in with
    /// their spans, the documents read again, and every match found in either.
    pub(crate) async fn record_document_sweep(
        &self,
        run: i64,
        admitted: Vec<(DocumentKey, String)>,
        spans: Vec<SweepSpan>,
        rescanned: Vec<DocumentKey>,
        matches: Vec<SweepMatch>,
    ) -> Result<(), AppError> {
        self.query_connection("Record redaction sweep", move |conn| {
            let tx = conn.transaction().map_err(storage)?;
            for (key, text_version) in &admitted {
                tx.execute(
                    "INSERT INTO redaction_sweep_documents
                        (portal_key, document_id, text_version, scanned_run)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![key.portal_key, key.document_id, text_version, run],
                )
                .map_err(storage)?;
            }
            for span in &spans {
                tx.execute(
                    "INSERT INTO redaction_sweep_spans (portal_key, document_id, passage_start,
                        passage_end, left_anchor, right_anchor, row_left_anchor,
                        row_right_anchor, quote, person_like, reused_wording, added_run)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, FALSE, ?11)",
                    params![
                        span.document.portal_key,
                        span.document.document_id,
                        span.passage_start,
                        span.passage_end,
                        span.left_anchor,
                        span.right_anchor,
                        span.row_anchors.as_ref().map(|(left, _)| left),
                        span.row_anchors.as_ref().map(|(_, right)| right),
                        span.quote,
                        span.person_like,
                        run,
                    ],
                )
                .map_err(storage)?;
            }
            for key in &rescanned {
                tx.execute(
                    "UPDATE redaction_sweep_documents SET scanned_run = ?3
                     WHERE portal_key = ?1 AND document_id = ?2",
                    params![key.portal_key, key.document_id, run],
                )
                .map_err(storage)?;
            }
            insert_matches(&tx, &matches)?;
            tx.commit().map_err(storage)
        })
        .await
    }

    /// Imported tables with rows not yet compared with every span: a pass in progress first, then
    /// tables never read, then the longest-waiting.
    pub(crate) async fn load_tables_to_sweep(
        &self,
        newest_span_run: i64,
        max_tables: usize,
    ) -> Result<Vec<SweepTable>, AppError> {
        self.query_connection("Load tables to sweep", move |conn| {
            let listed = conn
                .prepare(
                    "SELECT s.snapshot_id, s.source_sha256, COALESCE(t.resume_after_row, 0)
                     FROM analytical_snapshots s
                     LEFT JOIN redaction_sweep_tables t
                        ON t.snapshot_id = s.snapshot_id AND t.source_sha256 = s.source_sha256
                     WHERE t.snapshot_id IS NULL OR t.pass_run IS NOT NULL
                        OR t.scanned_run IS NULL OR t.scanned_run < ?1
                     ORDER BY (t.pass_run IS NULL), COALESCE(t.scanned_run, 0), s.snapshot_id
                     LIMIT ?2",
                )
                .map_err(storage)?
                .query_map(params![newest_span_run, max_tables as i64], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                })
                .map_err(storage)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(storage)?;
            listed
                .into_iter()
                .map(|(snapshot_id, source_sha256, resume_after_row)| {
                    // A literal, not a parameter: the view prunes other snapshots' branches only
                    // when the planner can see the value.
                    let last_row: i64 = conn
                        .query_row(
                            &format!(
                                "SELECT COALESCE(MAX(row_number), 0) FROM analytical_rows
                                 WHERE snapshot_id = {}",
                                literal(&snapshot_id)
                            ),
                            [],
                            |row| row.get(0),
                        )
                        .map_err(storage)?;
                    Ok(SweepTable {
                        snapshot_id,
                        source_sha256,
                        resume_after_row,
                        last_row,
                    })
                })
                .collect()
        })
        .await
    }

    /// Rows `after_row + 1 ..= through_row` of one table, each as its original cells in the
    /// `{"headers": [...], "values": [...]}` shape the row view serves.
    pub(crate) async fn load_table_rows(
        &self,
        snapshot_id: String,
        after_row: i64,
        through_row: i64,
    ) -> Result<Vec<(i64, String)>, AppError> {
        self.query_connection("Load table rows for redaction sweep", move |conn| {
            let rows = conn
                .prepare(&format!(
                    "SELECT row_number, source_payload_json FROM analytical_rows
                     WHERE snapshot_id = {} AND row_number > ?1 AND row_number <= ?2",
                    literal(&snapshot_id)
                ))
                .map_err(storage)?
                .query_map(params![after_row, through_row], |row| {
                    Ok((row.get(0)?, row.get(1)?))
                })
                .map_err(storage)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(storage)?;
            Ok(rows)
        })
        .await
    }

    /// Records how far a table has been read, with the matches found on the way. A pass that
    /// reaches the last row is stamped with the run it began in, because spans added while it was
    /// under way were not compared with the rows read before them.
    pub(crate) async fn record_table_sweep(
        &self,
        run: i64,
        table: SweepTable,
        through_row: i64,
        matches: Vec<SweepMatch>,
    ) -> Result<(), AppError> {
        self.query_connection("Record table sweep", move |conn| {
            let tx = conn.transaction().map_err(storage)?;
            tx.execute(
                "INSERT INTO redaction_sweep_tables
                    (snapshot_id, source_sha256, scanned_run, pass_run, resume_after_row)
                 VALUES (?1, ?2, NULL, ?3, ?4)
                 ON CONFLICT (snapshot_id) DO UPDATE SET
                    pass_run = COALESCE(redaction_sweep_tables.pass_run, EXCLUDED.pass_run),
                    resume_after_row = EXCLUDED.resume_after_row",
                params![table.snapshot_id, table.source_sha256, run, through_row],
            )
            .map_err(storage)?;
            insert_matches(&tx, &matches)?;
            if through_row >= table.last_row {
                tx.execute(
                    "UPDATE redaction_sweep_tables
                     SET scanned_run = pass_run, pass_run = NULL, resume_after_row = NULL
                     WHERE snapshot_id = ?1",
                    params![table.snapshot_id],
                )
                .map_err(storage)?;
            }
            tx.commit().map_err(storage)
        })
        .await
    }

    /// Every stored match on a span whose wording is not reused, ordered by span.
    pub(crate) async fn load_sweep_candidates(&self) -> Result<Vec<SweepCandidate>, AppError> {
        self.query_connection("Load redaction sweep candidates", |conn| {
            let candidates = conn
                .prepare(
                    "SELECT s.portal_key, s.document_id, s.passage_start, s.passage_end,
                            s.quote, s.person_like, d.file_name, d.request_id,
                            m.source_kind, m.source_scope, m.source_item, m.value_sha256,
                            m.name_shaped, m.source_start, m.source_end, m.column_name,
                            sd.file_name, sl.text_version,
                            t.display_name, t.source_uri, t.source_sha256, t.retrieved_at
                     FROM redaction_sweep_spans s
                     INNER JOIN documents d
                        ON d.portal_subdomain = s.portal_key AND d.id = s.document_id
                     INNER JOIN redaction_sweep_matches m
                        ON m.portal_key = s.portal_key AND m.document_id = s.document_id
                       AND m.passage_start = s.passage_start
                     LEFT JOIN documents sd
                        ON m.source_kind = 'DOCUMENT' AND sd.portal_subdomain = m.source_scope
                       AND sd.id = m.source_item
                     LEFT JOIN redaction_sweep_documents sl
                        ON m.source_kind = 'DOCUMENT' AND sl.portal_key = m.source_scope
                       AND sl.document_id = m.source_item
                     LEFT JOIN analytical_snapshots t
                        ON m.source_kind = 'TABLE' AND t.snapshot_id = m.source_scope
                     WHERE NOT s.reused_wording
                       AND (sl.portal_key IS NOT NULL OR t.snapshot_id IS NOT NULL)
                     ORDER BY s.portal_key, s.document_id, s.passage_start,
                              m.source_kind, m.source_scope, m.source_item",
                )
                .map_err(storage)?
                .query_map([], |row| {
                    let document = DocumentKey {
                        portal_key: row.get(0)?,
                        document_id: row.get(1)?,
                    };
                    let scope: String = row.get(9)?;
                    let item: i64 = row.get(10)?;
                    let table = match row.get::<_, Option<String>>(18)? {
                        Some(display_name) => Some(SweepTableCitation {
                            display_name,
                            source_uri: row.get(19)?,
                            source_sha256: row.get(20)?,
                            retrieved_at: row.get(21)?,
                        }),
                        None => None,
                    };
                    let source = match &table {
                        Some(_) => MatchSource::TableCell {
                            snapshot_id: scope,
                            row_number: item,
                            column_name: row.get(15)?,
                        },
                        None => MatchSource::Document {
                            key: DocumentKey {
                                portal_key: scope,
                                document_id: item,
                            },
                            start: row.get(13)?,
                            end: row.get(14)?,
                        },
                    };
                    let source_digest = match &table {
                        Some(table) => table.source_sha256.clone(),
                        None => row.get::<_, String>(17)?,
                    };
                    Ok(SweepCandidate {
                        document,
                        passage_start: row.get(2)?,
                        passage_end: row.get(3)?,
                        quote: row.get(4)?,
                        person_like: row.get(5)?,
                        file_name: row.get(6)?,
                        request_id: row.get(7)?,
                        source,
                        value_sha256: row.get(11)?,
                        name_shaped: row.get(12)?,
                        source_file_name: row.get(16)?,
                        source_digest,
                        table,
                    })
                })
                .map_err(storage)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(storage)?;
            Ok(candidates)
        })
        .await
    }
}
