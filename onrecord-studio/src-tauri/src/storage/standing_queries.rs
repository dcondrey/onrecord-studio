//! Standing queries: saved search ASTs re-run against each new corpus state.
//!
//! A run names the corpus it saw (the corpus-state digest) and the match set it found (a digest
//! over the ordered matches), so "query Q against corpus C returned R" is a reproducible record
//! rather than a remembered one. The digest, the query and the write of the run happen inside one
//! connection closure, so a run can never carry the digest of a corpus other than the one it read.
//!
//! Tables carry no foreign keys and no integer key columns, so they create identically on a
//! database file written by an older release.

use duckdb::{params, Connection, OptionalExt};
use sha2::{Digest, Sha256};

use crate::error::AppError;
use crate::integrity::digest_hex;
use crate::storage::search_paging::{build_filters, count_matches, Filters};
use crate::storage::snapshot_export::corpus_state_digest;
use crate::storage::sqlite::DuckDbRepository;
use crate::types::{
    CountBasis, DocumentCluster, Facet, FacetValue, StandingQuery, StandingQueryFilters,
    StandingQueryInput, StandingQueryOutcome, StandingQueryResult, StandingQueryRun,
    StandingQuerySummary,
};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS standing_queries(
    id TEXT PRIMARY KEY, label TEXT NOT NULL, ast_json TEXT NOT NULL, search_term TEXT NOT NULL,
    portal_key TEXT, agency_id BIGINT, status TEXT, policy_deviation TEXT, created_at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS standing_query_runs(
    id TEXT PRIMARY KEY, query_id TEXT NOT NULL, sequence BIGINT NOT NULL, ran_at TEXT NOT NULL,
    corpus_digest TEXT NOT NULL,
    corpus_document_count BIGINT NOT NULL, match_count BIGINT NOT NULL, matches_digest TEXT NOT NULL,
    result_json TEXT NOT NULL);
";

pub const MAX_STANDING_QUERIES: usize = 50;
const MAX_AST_BYTES: usize = 8 * 1024;
const MAX_LABEL_CHARS: usize = 120;
const RUNS_KEPT: i64 = 20;
const FACET_VALUES: usize = 12;
const CLUSTERS_LISTED: usize = 20;
/// Above this many matches the breakdowns are read from a fixed-size sample and scaled; the
/// match count itself is always exact.
#[cfg(not(test))]
const SAMPLING_THRESHOLD: u64 = 10_000;
#[cfg(test)]
const SAMPLING_THRESHOLD: u64 = 3;
const SAMPLE_ROWS: u64 = SAMPLING_THRESHOLD;

fn err(error: impl std::fmt::Display) -> AppError {
    AppError::Storage(error.to_string())
}

fn invalid(message: impl Into<String>) -> AppError {
    AppError::DataParsing(message.into())
}

fn now_stamp() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// The AST is stored verbatim but must be one the flat native query executes exactly. A negated
/// filter, a repeated field, or a field outside the four supported ones would make the saved
/// query run as a looser one than the operator wrote.
fn validate_ast(ast_json: &str) -> Result<(), AppError> {
    if ast_json.len() > MAX_AST_BYTES {
        return Err(invalid("The query is too long to save"));
    }
    let ast: serde_json::Value =
        serde_json::from_str(ast_json).map_err(|_| invalid("The query AST is not valid JSON"))?;
    if ast.get("version").and_then(serde_json::Value::as_u64) != Some(1) {
        return Err(invalid("The query AST version is not supported"));
    }
    if !ast.get("text").is_some_and(serde_json::Value::is_string) {
        return Err(invalid("The query AST has no text"));
    }
    let filters = ast
        .get("filters")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| invalid("The query AST has no filter list"))?;
    let mut seen = std::collections::BTreeSet::new();
    for filter in filters {
        let field = filter
            .get("field")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| invalid("A query filter has no field"))?;
        if !["agency", "portal", "status", "flag"].contains(&field) {
            return Err(invalid(format!(
                "The field '{field}' cannot be executed by a standing query"
            )));
        }
        if filter.get("negated").and_then(serde_json::Value::as_bool) == Some(true) {
            return Err(invalid(format!(
                "A negated '{field}' filter cannot be executed by a standing query"
            )));
        }
        if !seen.insert(field.to_string()) {
            return Err(invalid(format!(
                "A repeated '{field}' filter cannot be executed by a standing query"
            )));
        }
    }
    Ok(())
}

fn read_query(row: &duckdb::Row<'_>) -> duckdb::Result<StandingQuery> {
    Ok(StandingQuery {
        id: row.get(0)?,
        label: row.get(1)?,
        ast_json: row.get(2)?,
        filters: StandingQueryFilters {
            search_term: row.get(3)?,
            portal_key: row.get(4)?,
            agency_id: row
                .get::<_, Option<i64>>(5)?
                .and_then(|value| u64::try_from(value).ok()),
            status: row.get(6)?,
            policy_deviation: row.get(7)?,
        },
        created_at: row.get(8)?,
    })
}

const QUERY_COLUMNS: &str = "id, label, ast_json, search_term, portal_key, agency_id, status,
                             policy_deviation, created_at";

fn read_run(row: &duckdb::Row<'_>) -> duckdb::Result<(StandingQueryRun, String)> {
    let result_json: String = row.get(6)?;
    let result: StandingQueryResult = serde_json::from_str(&result_json).map_err(|error| {
        duckdb::Error::FromSqlConversionFailure(6, duckdb::types::Type::Text, Box::new(error))
    })?;
    Ok((
        StandingQueryRun {
            run_id: row.get(0)?,
            query_id: row.get(1)?,
            ran_at: row.get(2)?,
            corpus_digest: row.get(3)?,
            corpus_document_count: row.get::<_, i64>(4)?.max(0) as u64,
            result,
            changed_since_previous: None,
            previous_match_count: None,
        },
        row.get::<_, String>(5)?,
    ))
}

const RUN_COLUMNS: &str =
    "id, query_id, ran_at, corpus_digest, corpus_document_count, matches_digest, result_json";

fn latest_run(
    conn: &Connection,
    query_id: &str,
) -> Result<Option<(StandingQueryRun, String)>, AppError> {
    conn.query_row(
        &format!(
            "SELECT {RUN_COLUMNS} FROM standing_query_runs WHERE query_id = ?1
             ORDER BY sequence DESC LIMIT 1"
        ),
        params![query_id],
        read_run,
    )
    .optional()
    .map_err(err)
}

fn filters_for(conn: &Connection, query: &StandingQuery) -> Result<Filters, AppError> {
    let _ = conn;
    build_filters(
        &query.filters.search_term,
        query.filters.portal_key.as_deref(),
        query.filters.agency_id,
        query.filters.status.as_deref(),
        query.filters.policy_deviation.as_deref(),
    )
}

/// The match set as a digest over its ordered (portal, id) rows.
fn matches_digest(conn: &Connection, filters: &Filters) -> Result<String, AppError> {
    let mut statement = conn
        .prepare(&format!(
            "SELECT requests.portal_subdomain, requests.id FROM requests WHERE {}
             ORDER BY requests.portal_subdomain, requests.id",
            filters.sql
        ))
        .map_err(err)?;
    let rows = statement
        .query_map(filters.params(), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(err)?;
    let mut hasher = Sha256::new();
    for row in rows {
        let (portal, id) = row.map_err(err)?;
        hasher.update(portal.as_bytes());
        hasher.update(b"\t");
        hasher.update(id.to_string().as_bytes());
        hasher.update(b"\n");
    }
    Ok(digest_hex(hasher.finalize()))
}

/// The matched requests, whole or sampled, as a CTE body. The seed makes a sampled breakdown
/// repeatable against the same corpus state.
fn matched_cte(filters: &Filters, sampled: bool) -> String {
    let sample = if sampled {
        format!(" USING SAMPLE reservoir({SAMPLE_ROWS} ROWS) REPEATABLE (7)")
    } else {
        String::new()
    };
    format!(
        "matched AS (
            SELECT requests.id, requests.portal_subdomain, requests.agency_id, requests.status,
                   requests.created_at
            FROM requests WHERE {}{sample}
         )",
        filters.sql
    )
}

fn scaled(count: i64, total: u64, sample_size: u64) -> u64 {
    let count = count.max(0) as u64;
    if sample_size == 0 {
        return 0;
    }
    ((count as u128 * total as u128) / sample_size as u128) as u64
}

fn execute(conn: &Connection, filters: &Filters) -> Result<StandingQueryResult, AppError> {
    let match_count = count_matches(conn, filters)?;
    let digest = matches_digest(conn, filters)?;
    let sampled = match_count > SAMPLING_THRESHOLD;
    let cte = matched_cte(filters, sampled);

    let facet_sql = format!(
        "WITH {cte}
         SELECT dimension, value, n FROM (
            SELECT 'status' AS dimension, COALESCE(status, '') AS value, COUNT(*) AS n
            FROM matched GROUP BY 2
            UNION ALL
            SELECT 'portal', portal_subdomain, COUNT(*) FROM matched GROUP BY 2
            UNION ALL
            SELECT 'year', LEFT(COALESCE(created_at, ''), 4), COUNT(*) FROM matched GROUP BY 2
            UNION ALL
            SELECT 'agency', COALESCE(agencies.name, 'Unmapped Department'), COUNT(*)
            FROM matched
            LEFT JOIN agencies ON agencies.id = matched.agency_id
                              AND agencies.portal_subdomain = matched.portal_subdomain
            GROUP BY 2
         ) ORDER BY dimension, n DESC, value"
    );
    let mut statement = conn.prepare(&facet_sql).map_err(err)?;
    let rows = statement
        .query_map(filters.params(), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })
        .map_err(err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(err)?;
    // Every matched (or sampled) row has exactly one status, so this is the sample size.
    let sample_size: u64 = rows
        .iter()
        .filter(|(dimension, _, _)| dimension == "status")
        .map(|(_, _, n)| n.max(&0).unsigned_abs())
        .sum();
    let count_basis = if sampled {
        CountBasis::EstimatedFromSample { sample_size }
    } else {
        CountBasis::Exact
    };
    let mut facets: Vec<Facet> = Vec::new();
    for (dimension, value, n) in rows {
        let count = if sampled {
            scaled(n, match_count, sample_size)
        } else {
            n.max(0) as u64
        };
        let facet = match facets.last_mut() {
            Some(facet) if facet.dimension == dimension => facet,
            _ => {
                facets.push(Facet {
                    dimension,
                    values: Vec::new(),
                    truncated: false,
                });
                facets.last_mut().expect("just pushed")
            }
        };
        if facet.values.len() < FACET_VALUES {
            facet.values.push(FacetValue { value, count });
        } else {
            facet.truncated = true;
        }
    }

    let cluster_sql = format!(
        "WITH {cte}
         SELECT documents.extracted_text_sha256,
                COUNT(DISTINCT documents.portal_subdomain || ':' || CAST(documents.request_id AS VARCHAR)),
                COUNT(*),
                MIN(COALESCE(documents.file_name, ''))
         FROM documents
         JOIN matched ON matched.id = documents.request_id
                     AND matched.portal_subdomain = documents.portal_subdomain
         WHERE documents.extracted_text_sha256 IS NOT NULL
           AND LOWER(TRIM(documents.visibility)) = 'public'
         GROUP BY 1
         HAVING COUNT(DISTINCT documents.portal_subdomain || ':' || CAST(documents.request_id AS VARCHAR)) >= 2
         ORDER BY 2 DESC, 3 DESC, 1
         LIMIT {CLUSTERS_LISTED}"
    );
    let mut statement = conn.prepare(&cluster_sql).map_err(err)?;
    let clusters = statement
        .query_map(filters.params(), |row| {
            Ok(DocumentCluster {
                extracted_text_sha256: row.get(0)?,
                request_count: row.get::<_, i64>(1)?.max(0) as u64,
                document_count: row.get::<_, i64>(2)?.max(0) as u64,
                example_file_name: row.get(3)?,
            })
        })
        .map_err(err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(err)?;

    Ok(StandingQueryResult {
        match_count,
        matches_digest: digest,
        count_basis,
        facets,
        clusters,
    })
}

fn record_run(
    conn: &Connection,
    query: &StandingQuery,
    corpus_digest: &str,
    corpus_document_count: u64,
    result: StandingQueryResult,
) -> Result<StandingQueryRun, AppError> {
    let previous = latest_run(conn, &query.id)?;
    let run = StandingQueryRun {
        run_id: uuid::Uuid::new_v4().to_string(),
        query_id: query.id.clone(),
        ran_at: now_stamp(),
        corpus_digest: corpus_digest.to_string(),
        corpus_document_count,
        changed_since_previous: previous
            .as_ref()
            .map(|(_, matches)| *matches != result.matches_digest),
        previous_match_count: previous.as_ref().map(|(run, _)| run.result.match_count),
        result,
    };
    let result_json = serde_json::to_string(&run.result).map_err(err)?;
    // Instants are second-resolution, so two runs in one second need an order of their own;
    // the connection is serialized, so the next sequence number cannot race.
    let sequence: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(sequence), 0) + 1 FROM standing_query_runs WHERE query_id = ?1",
            params![run.query_id],
            |row| row.get(0),
        )
        .map_err(err)?;
    conn.execute(
        "INSERT INTO standing_query_runs
            (id, query_id, sequence, ran_at, corpus_digest, corpus_document_count, match_count,
             matches_digest, result_json)
         VALUES (?1, ?2, ?9, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            run.run_id,
            run.query_id,
            run.ran_at,
            run.corpus_digest,
            i64::try_from(run.corpus_document_count).unwrap_or(i64::MAX),
            i64::try_from(run.result.match_count).unwrap_or(i64::MAX),
            run.result.matches_digest,
            result_json,
            sequence,
        ],
    )
    .map_err(err)?;
    conn.execute(
        "DELETE FROM standing_query_runs WHERE query_id = ?1 AND id NOT IN (
            SELECT id FROM standing_query_runs WHERE query_id = ?1
            ORDER BY sequence DESC LIMIT ?2)",
        params![run.query_id, RUNS_KEPT],
    )
    .map_err(err)?;
    Ok(run)
}

fn corpus_identity(conn: &Connection) -> Result<(String, u64), AppError> {
    let digest = corpus_state_digest(conn)?;
    let documents: i64 = conn
        .query_row("SELECT count(*) FROM documents", [], |row| row.get(0))
        .map_err(err)?;
    Ok((digest, u64::try_from(documents).unwrap_or(0)))
}

impl DuckDbRepository {
    pub async fn initialize_standing_queries(&self) -> Result<(), AppError> {
        self.query_connection("Initialize standing queries", |conn| {
            conn.execute_batch(SCHEMA).map_err(err)
        })
        .await
    }

    pub async fn save_standing_query(
        &self,
        input: StandingQueryInput,
    ) -> Result<StandingQuery, AppError> {
        let label: String = input.label.trim().chars().take(MAX_LABEL_CHARS).collect();
        validate_ast(&input.ast_json)?;
        // The flat filters must themselves be executable, which the filter builder decides.
        build_filters(
            &input.filters.search_term,
            input.filters.portal_key.as_deref(),
            input.filters.agency_id,
            input.filters.status.as_deref(),
            input.filters.policy_deviation.as_deref(),
        )?;
        let query = StandingQuery {
            id: uuid::Uuid::new_v4().to_string(),
            label: if label.is_empty() {
                input
                    .filters
                    .search_term
                    .trim()
                    .chars()
                    .take(MAX_LABEL_CHARS)
                    .collect()
            } else {
                label
            },
            ast_json: input.ast_json,
            filters: input.filters,
            created_at: now_stamp(),
        };
        self.query_connection("Save standing query", move |conn| {
            let existing: i64 = conn
                .query_row("SELECT COUNT(*) FROM standing_queries", [], |row| {
                    row.get(0)
                })
                .map_err(err)?;
            if existing >= MAX_STANDING_QUERIES as i64 {
                return Err(invalid(format!(
                    "At most {MAX_STANDING_QUERIES} standing queries can be kept"
                )));
            }
            conn.execute(
                &format!(
                    "INSERT INTO standing_queries ({QUERY_COLUMNS})
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)"
                ),
                params![
                    query.id,
                    query.label,
                    query.ast_json,
                    query.filters.search_term,
                    query.filters.portal_key,
                    query
                        .filters
                        .agency_id
                        .map(|value| i64::try_from(value).unwrap_or(i64::MAX)),
                    query.filters.status,
                    query.filters.policy_deviation,
                    query.created_at,
                ],
            )
            .map_err(err)?;
            Ok(query)
        })
        .await
    }

    pub async fn delete_standing_query(&self, id: String) -> Result<(), AppError> {
        self.query_connection("Delete standing query", move |conn| {
            conn.execute(
                "DELETE FROM standing_query_runs WHERE query_id = ?1",
                params![id],
            )
            .map_err(err)?;
            conn.execute("DELETE FROM standing_queries WHERE id = ?1", params![id])
                .map_err(err)?;
            Ok(())
        })
        .await
    }

    pub async fn list_standing_queries(&self) -> Result<Vec<StandingQuerySummary>, AppError> {
        self.query_connection("List standing queries", |conn| {
            let (current_corpus_digest, _) = corpus_identity(conn)?;
            let mut statement = conn
                .prepare(&format!(
                    "SELECT {QUERY_COLUMNS} FROM standing_queries ORDER BY created_at, id"
                ))
                .map_err(err)?;
            let queries = statement
                .query_map([], read_query)
                .map_err(err)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(err)?;
            queries
                .into_iter()
                .map(|query| {
                    let last_run = latest_run(conn, &query.id)?.map(|(run, _)| run);
                    let due = last_run
                        .as_ref()
                        .is_none_or(|run| run.corpus_digest != current_corpus_digest);
                    Ok(StandingQuerySummary {
                        query,
                        last_run,
                        current_corpus_digest: current_corpus_digest.clone(),
                        due,
                    })
                })
                .collect()
        })
        .await
    }

    /// Runs every standing query (or the one named) whose last run saw a different corpus state,
    /// or all of them when `force` is set. `search_index_pending` defers queries that read the
    /// token projection, which would otherwise report fewer matches than the corpus holds.
    pub async fn run_standing_queries(
        &self,
        query_id: Option<String>,
        force: bool,
        search_index_pending: Option<String>,
    ) -> Result<Vec<StandingQueryOutcome>, AppError> {
        self.query_connection("Run standing queries", move |conn| {
            let (corpus_digest, corpus_document_count) = corpus_identity(conn)?;
            let mut statement = conn
                .prepare(&format!(
                    "SELECT {QUERY_COLUMNS} FROM standing_queries
                     WHERE ?1 IS NULL OR id = ?1 ORDER BY created_at, id"
                ))
                .map_err(err)?;
            let queries = statement
                .query_map(params![query_id], read_query)
                .map_err(err)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(err)?;
            let mut outcomes = Vec::with_capacity(queries.len());
            for query in queries {
                let filters = filters_for(conn, &query)?;
                if !force
                    && latest_run(conn, &query.id)?
                        .is_some_and(|(run, _)| run.corpus_digest == corpus_digest)
                {
                    outcomes.push(StandingQueryOutcome::NotDue { query_id: query.id });
                    continue;
                }
                if let (Some(reason), true) = (&search_index_pending, filters.reads_tokens()) {
                    outcomes.push(StandingQueryOutcome::Deferred {
                        query_id: query.id,
                        reason: reason.clone(),
                    });
                    continue;
                }
                let result = execute(conn, &filters)?;
                let run = record_run(conn, &query, &corpus_digest, corpus_document_count, result)?;
                outcomes.push(StandingQueryOutcome::Ran { run });
            }
            Ok(outcomes)
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup() -> DuckDbRepository {
        let db = DuckDbRepository::new(":memory:").unwrap();
        db.run_migrations(include_str!("../../migrations/0001_initial_schema.sql"))
            .await
            .unwrap();
        // Twice: the tables must already exist on a database an older release created and left
        // without them, and a second initialization must be a no-op.
        db.initialize_standing_queries().await.unwrap();
        db.initialize_standing_queries().await.unwrap();
        let conn = db.analytics_connection().await.unwrap();
        conn.execute_batch(
            "INSERT INTO portals(subdomain,display_name,base_url,lifecycle_state)
             VALUES ('city','City','https://city.example','ACTIVE');
             INSERT INTO agencies(id,portal_subdomain,name) VALUES (7,'city','Housing');
             INSERT INTO requests(id,portal_subdomain,title,status,created_at,updated_at,agency_id)
             VALUES (1,'city','Shelter beds','closed','2025-01-02','2025-01-02',7),
                    (2,'city','Shelter audit','open','2026-03-04','2026-03-04',7),
                    (3,'city','Parks','closed','2026-05-06','2026-05-06',NULL);",
        )
        .unwrap();
        db
    }

    async fn add_document(db: &DuckDbRepository, id: i64, request: i64, text: &str) {
        let conn = db.analytics_connection().await.unwrap();
        conn.execute(
            "INSERT INTO documents(id,request_id,portal_subdomain,file_name,download_url,visibility,
                 created_at,updated_at,raw_file_sha256,extracted_text_sha256,extraction_status)
             VALUES (?1,?2,'city',?3,?4,'public','2026-01-01','2026-01-01',?5,?6,'SEARCHABLE')",
            params![
                id,
                request,
                format!("doc{id}.pdf"),
                format!("https://city.example/doc{id}.pdf"),
                "a".repeat(64),
                crate::integrity::sha256_hex(text.as_bytes())
            ],
        )
        .unwrap();
    }

    fn input(search_term: &str, status: Option<&str>) -> StandingQueryInput {
        StandingQueryInput {
            label: "shelter".into(),
            ast_json: serde_json::json!({
                "version": 1, "source": "qualifier", "text": search_term,
                "filters": status.map(|value| vec![serde_json::json!({"field": "status", "operator": "equals", "value": value})]).unwrap_or_default(),
                "issues": []
            })
            .to_string(),
            filters: StandingQueryFilters {
                search_term: search_term.into(),
                status: status.map(str::to_string),
                ..Default::default()
            },
        }
    }

    fn ran(outcome: &StandingQueryOutcome) -> &StandingQueryRun {
        match outcome {
            StandingQueryOutcome::Ran { run } => run,
            other => panic!("expected a run, got {other:?}"),
        }
    }

    /// A query runs only against a corpus state it has not seen, every run names that state, and
    /// `changed_since_previous` compares match sets rather than corpus states: a new document on a
    /// request that already matched changes the corpus but not the answer.
    #[tokio::test]
    async fn a_query_reruns_on_a_new_corpus_state_and_reports_whether_its_answer_changed() {
        let db = setup().await;
        let saved = db
            .save_standing_query(input("", Some("closed")))
            .await
            .unwrap();
        let first = db.run_standing_queries(None, false, None).await.unwrap();
        let run = ran(&first[0]);
        assert_eq!(run.result.match_count, 2);
        assert_eq!(run.changed_since_previous, None);
        assert_eq!(run.result.count_basis, CountBasis::Exact);
        let status = run
            .result
            .facets
            .iter()
            .find(|facet| facet.dimension == "status")
            .unwrap();
        assert_eq!(
            status.values,
            vec![FacetValue {
                value: "closed".into(),
                count: 2
            }]
        );

        assert_eq!(
            db.run_standing_queries(None, false, None).await.unwrap(),
            vec![StandingQueryOutcome::NotDue {
                query_id: saved.id.clone()
            }]
        );
        assert!(!db.list_standing_queries().await.unwrap()[0].due);

        add_document(&db, 10, 1, "beds").await;
        let summary = &db.list_standing_queries().await.unwrap()[0];
        assert!(summary.due);
        let second = ran(&db.run_standing_queries(None, false, None).await.unwrap()[0]).clone();
        assert_ne!(second.corpus_digest, run.corpus_digest);
        assert_eq!(
            second.corpus_digest,
            db.list_standing_queries().await.unwrap()[0].current_corpus_digest
        );
        assert_eq!(second.changed_since_previous, Some(false));
        assert_eq!(second.previous_match_count, Some(2));

        let conn = db.analytics_connection().await.unwrap();
        conn.execute_batch(
            "INSERT INTO requests(id,portal_subdomain,title,status,created_at,updated_at)
             VALUES (4,'city','More','closed','2026-07-08','2026-07-08')",
        )
        .unwrap();
        drop(conn);
        add_document(&db, 11, 4, "beds").await;
        let third = ran(&db
            .run_standing_queries(Some(saved.id.clone()), false, None)
            .await
            .unwrap()[0])
        .clone();
        assert_eq!(third.changed_since_previous, Some(true));
        assert_eq!(third.result.match_count, 3);
        // Identical extracted text under two requests is one cluster, found by digest.
        assert_eq!(third.result.clusters.len(), 1);
        assert_eq!(
            (
                third.result.clusters[0].request_count,
                third.result.clusters[0].document_count
            ),
            (2, 2)
        );

        let forced = db.run_standing_queries(None, true, None).await.unwrap();
        assert!(
            matches!(&forced[0], StandingQueryOutcome::Ran { run } if run.changed_since_previous == Some(false))
        );
        assert_eq!(
            db.list_standing_queries().await.unwrap()[0]
                .last_run
                .as_ref()
                .unwrap()
                .run_id,
            ran(&forced[0]).run_id
        );
    }

    /// Above the sampling threshold the breakdowns are scaled from a sample and say so; the match
    /// count stays exact. At the threshold they are exact.
    #[tokio::test]
    async fn breakdowns_above_the_threshold_are_labelled_estimates_and_the_total_stays_exact() {
        let db = setup().await;
        db.save_standing_query(input("", None)).await.unwrap();
        let at_threshold =
            ran(&db.run_standing_queries(None, true, None).await.unwrap()[0]).clone();
        assert_eq!(at_threshold.result.match_count, SAMPLING_THRESHOLD);
        assert_eq!(at_threshold.result.count_basis, CountBasis::Exact);

        let conn = db.analytics_connection().await.unwrap();
        conn.execute_batch(
            "INSERT INTO requests(id,portal_subdomain,title,status,created_at,updated_at)
             VALUES (4,'city','Fourth','closed','2026-07-08','2026-07-08')",
        )
        .unwrap();
        drop(conn);
        let above = ran(&db.run_standing_queries(None, true, None).await.unwrap()[0]).clone();
        assert_eq!(above.result.match_count, SAMPLING_THRESHOLD + 1);
        assert_eq!(
            above.result.count_basis,
            CountBasis::EstimatedFromSample {
                sample_size: SAMPLE_ROWS
            }
        );
        let status = above
            .result
            .facets
            .iter()
            .find(|facet| facet.dimension == "status")
            .unwrap();
        let estimated: u64 = status.values.iter().map(|value| value.count).sum();
        assert!(
            (SAMPLING_THRESHOLD - 1..=SAMPLING_THRESHOLD + 1).contains(&estimated),
            "{estimated}"
        );
    }

    /// A query the flat native search cannot execute exactly is refused, and a query that reads
    /// the token projection is deferred, not run against a partial index.
    #[tokio::test]
    async fn an_inexact_query_is_refused_and_a_text_query_is_deferred_while_indexing() {
        let db = setup().await;
        let mut negated = input("", Some("closed"));
        negated.ast_json = negated
            .ast_json
            .replace("\"operator\"", "\"negated\":true,\"operator\"");
        let error = db.save_standing_query(negated).await.unwrap_err();
        assert!(error.to_string().contains("negated"), "{error}");
        assert!(db.list_standing_queries().await.unwrap().is_empty());

        let text = db.save_standing_query(input("beds", None)).await.unwrap();
        let outcomes = db
            .run_standing_queries(None, false, Some("index building".into()))
            .await
            .unwrap();
        assert_eq!(
            outcomes,
            vec![StandingQueryOutcome::Deferred {
                query_id: text.id,
                reason: "index building".into()
            }]
        );
    }
}
