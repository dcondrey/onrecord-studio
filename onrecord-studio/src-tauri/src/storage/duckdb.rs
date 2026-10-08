use crate::blocking::spawn_with;
use crate::error::AppError;
#[cfg(test)]
use crate::storage::sqlite_snapshot::import_sqlite_snapshot;
use crate::types::CrossAgencyCorrelation;
use duckdb::{params, Connection, Row};
use std::convert::TryFrom;
#[cfg(test)]
use std::path::Path;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};
use std::time::{Duration, Instant};

const WORKSPACE_LOCK_TIMEOUT: Duration = Duration::from_secs(5);
const WORKSPACE_LOCK_POLL_INTERVAL: Duration = Duration::from_millis(5);
#[cfg(test)]
const CORRELATION_SQLITE_TABLES: [&str; 5] = [
    "requests",
    "agencies",
    "request_keyword_signals",
    "auxiliary_data_sources",
    "auxiliary_events",
];

const CROSS_CORRELATION_QUERY: &str = r#"
WITH request_candidates AS (
    SELECT
        requests.id AS request_id,
        requests.portal_subdomain AS portal_key,
        requests.agency_id,
        COALESCE(agencies.name, 'Unmapped Department') AS agency_name,
        COALESCE(NULLIF(TRIM(requests.title), ''), 'Untitled request') AS request_title,
        CAST(TRY_CAST(requests.created_at AS TIMESTAMPTZ) AS TIMESTAMP) AS request_timestamp,
        LOWER(TRIM(COALESCE(requests.neighborhood_block, ''))) AS normalized_neighborhood,
        COALESCE(SUM(request_keyword_signals.match_count), 0) AS keyword_signal_count
    FROM {db}requests AS requests
    LEFT JOIN {db}agencies AS agencies
      ON agencies.id = requests.agency_id
     AND agencies.portal_subdomain = requests.portal_subdomain
    LEFT JOIN {db}request_keyword_signals AS request_keyword_signals
      ON request_keyword_signals.request_id = requests.id
     AND request_keyword_signals.portal_subdomain = requests.portal_subdomain
    WHERE (?1 = '' OR requests.portal_subdomain = ?1)
    GROUP BY
        requests.id,
        requests.portal_subdomain,
        requests.agency_id,
        agencies.name,
        requests.title,
        requests.created_at,
        requests.neighborhood_block
),
complaint_events AS (
    SELECT
        auxiliary_events.source_id,
        CAST(TRY_CAST(auxiliary_events.occurred_at AS TIMESTAMPTZ) AS TIMESTAMP) AS occurred_timestamp,
        LOWER(TRIM(auxiliary_events.neighborhood_block)) AS normalized_neighborhood
    FROM {db}auxiliary_events AS auxiliary_events
    INNER JOIN {db}auxiliary_data_sources AS auxiliary_data_sources
      ON auxiliary_data_sources.source_id = auxiliary_events.source_id
    WHERE auxiliary_data_sources.source_kind = 'MUNICIPAL_311'
),
correlated AS (
    SELECT
        request_candidates.request_id,
        request_candidates.portal_key,
        request_candidates.agency_id,
        request_candidates.agency_name,
        request_candidates.request_title,
        request_candidates.request_timestamp,
        request_candidates.normalized_neighborhood,
        request_candidates.keyword_signal_count,
        COUNT(*) AS complaint_count,
        COUNT(DISTINCT complaint_events.source_id) AS distinct_source_count,
        MIN(complaint_events.occurred_timestamp) AS first_correlated_event_at,
        MAX(complaint_events.occurred_timestamp) AS last_correlated_event_at
    FROM request_candidates
    INNER JOIN complaint_events
      ON complaint_events.normalized_neighborhood = request_candidates.normalized_neighborhood
     AND complaint_events.occurred_timestamp >= request_candidates.request_timestamp - CAST(?4 AS BIGINT) * INTERVAL '1 hour'
     AND complaint_events.occurred_timestamp <= request_candidates.request_timestamp
    WHERE request_candidates.request_timestamp IS NOT NULL
      AND request_candidates.normalized_neighborhood <> ''
      AND request_candidates.keyword_signal_count > 0
      AND complaint_events.occurred_timestamp IS NOT NULL
    GROUP BY ALL
)
SELECT
    request_id,
    portal_key,
    agency_id,
    agency_name,
    request_title,
    CAST(request_timestamp AS VARCHAR),
    normalized_neighborhood,
    complaint_count,
    CAST(?4 AS BIGINT) AS correlation_window_hours,
    distinct_source_count,
    CAST(first_correlated_event_at AS VARCHAR),
    CAST(last_correlated_event_at AS VARCHAR),
    keyword_signal_count,
    CAST(complaint_count AS DOUBLE) / CAST(?4 AS DOUBLE) AS correlation_density
FROM correlated
WHERE complaint_count >= ?2
ORDER BY complaint_count DESC, portal_key, request_id
LIMIT ?3
"#;

/// Where a workspace reads from. A connection that `ATTACH`es a database file observes a snapshot
/// frozen at attach time, so the application's own core must be read through a live sibling
/// connection instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WorkspaceSource {
    /// Sibling connection to the live DuckDB core. Tables are unqualified; attaching is rejected.
    Core,
    /// Standalone in-memory node inspecting a foreign SQLite file under the `db` alias.
    ForeignSqlite,
}

impl WorkspaceSource {
    fn table_prefix(self) -> &'static str {
        match self {
            Self::Core => "",
            Self::ForeignSqlite => "db.",
        }
    }
}

struct DuckDbState {
    connection: Connection,
    source: WorkspaceSource,
    attached_sqlite_path: Option<PathBuf>,
}

/// Substitutes the `{db}` table qualifier so one query text serves both workspace sources.
fn qualify(template: &str, table_prefix: &str) -> String {
    template.replace("{db}", table_prefix)
}

// LIMIT(concurrency): Writes are serialized by the repository's `Arc<Mutex<Connection>>` and
// analytics reads run on sibling connections to the same instance, so the writer/reader boundary
// is correct. A competing analytics operation returns a retryable-looking busy message after the
// bounded workspace timeout. DuckDB query execution itself is still synchronous once it starts.
// `release_scale_harness_covers_million_rows_large_text_indexes_and_concurrent_reads` exercises
// million-row ingestion, concurrent analytics, a 32 MiB text projection, and index rebuilding;
// `scripts/validate-release-performance.mjs` runs it as an explicit release gate.

struct CorrelationRow {
    request_id: i64,
    portal_key: String,
    agency_id: Option<i64>,
    agency_name: String,
    request_title: String,
    request_timestamp: String,
    neighborhood_block: String,
    complaint_count: i64,
    correlation_window_hours: i64,
    distinct_source_count: i64,
    first_correlated_event_at: String,
    last_correlated_event_at: String,
    keyword_signal_count: i64,
    correlation_density: f64,
}

#[derive(Clone)]
pub struct DuckDbWorkspace {
    state: Arc<Mutex<DuckDbState>>,
}

impl DuckDbWorkspace {
    #[cfg(test)]
    /// Creates a standalone workspace for inspecting a foreign SQLite database supplied by the
    /// investigator. Call `attach_sqlite_read_only` before querying.
    pub fn open_workspace() -> Result<Self, AppError> {
        let connection = Connection::open_in_memory().map_err(|error| {
            AppError::Analysis(format!("Failed to initialize in-memory DuckDB: {error}"))
        })?;
        Ok(Self {
            state: Arc::new(Mutex::new(DuckDbState {
                connection,
                source: WorkspaceSource::ForeignSqlite,
                attached_sqlite_path: None,
            })),
        })
    }

    /// Binds the workspace to a live sibling connection of the application's own DuckDB core,
    /// obtained from `DuckDbRepository::analytics_connection`.
    ///
    /// A sibling connection sees every later write; a separate connection that `ATTACH`ed the core
    /// file would compute correlations against the file as it stood at launch.
    pub fn bind_to_core(connection: Connection) -> Self {
        Self {
            state: Arc::new(Mutex::new(DuckDbState {
                connection,
                source: WorkspaceSource::Core,
                attached_sqlite_path: None,
            })),
        }
    }

    #[cfg(test)]
    pub async fn attach_sqlite_read_only(
        &self,
        sqlite_path: impl AsRef<Path>,
    ) -> Result<(), AppError> {
        let requested_path = sqlite_path.as_ref().to_path_buf();
        let state = Arc::clone(&self.state);
        spawn_with("DuckDB attachment worker", AppError::Analysis, move || {
            let canonical_path = validate_database_path(&requested_path)?;
            let mut state = lock_state(&state)?;
            if state.source == WorkspaceSource::Core {
                return Err(AppError::Analysis(
                    "The core DuckDB workspace cannot attach a foreign SQLite database".to_string(),
                ));
            }
            if state.attached_sqlite_path.as_ref() == Some(&canonical_path) {
                return validate_layout(&state.connection, state.source.table_prefix());
            }
            let replacement = super::open_duckdb(None).map_err(|error| {
                AppError::Analysis(format!("Failed to initialize SQLite snapshot: {error}"))
            })?;
            import_sqlite_snapshot(&replacement, &canonical_path, &CORRELATION_SQLITE_TABLES)?;
            validate_layout(&replacement, state.source.table_prefix())?;
            state.connection = replacement;
            state.attached_sqlite_path = Some(canonical_path);
            Ok(())
        })
        .await?
    }

    pub async fn correlate_311_with_enforcement_requests(
        &self,
        portal_key: Option<&str>,
        minimum_complaints: usize,
        result_limit: usize,
        correlation_window_hours: usize,
    ) -> Result<Vec<CrossAgencyCorrelation>, AppError> {
        if minimum_complaints == 0 {
            return Err(AppError::DataParsing(
                "Minimum complaint count must be greater than zero".to_string(),
            ));
        }
        if !(1..=10_000).contains(&result_limit) {
            return Err(AppError::DataParsing(
                "Correlation result limit must be between 1 and 10000".to_string(),
            ));
        }
        if !(1..=168).contains(&correlation_window_hours) {
            return Err(AppError::DataParsing(
                "Correlation window must be between 1 and 168 hours".to_string(),
            ));
        }
        let portal_key = portal_key.unwrap_or("").trim().to_string();
        let minimum_complaints = i64::try_from(minimum_complaints).map_err(|_| {
            AppError::DataParsing("Minimum complaint count exceeds DuckDB capacity".to_string())
        })?;
        let result_limit = i64::try_from(result_limit).map_err(|_| {
            AppError::DataParsing("Correlation result limit exceeds DuckDB capacity".to_string())
        })?;
        let correlation_window_hours = i64::try_from(correlation_window_hours).map_err(|_| {
            AppError::DataParsing("Correlation window exceeds DuckDB capacity".to_string())
        })?;
        let state = Arc::clone(&self.state);
        spawn_with("DuckDB correlation worker", AppError::Analysis, move || {
            let state = lock_state(&state)?;
            if state.source == WorkspaceSource::ForeignSqlite
                && state.attached_sqlite_path.is_none()
            {
                return Err(AppError::Analysis(
                    "No SQLite source is attached to DuckDB".to_string(),
                ));
            }
            let query = qualify(CROSS_CORRELATION_QUERY, state.source.table_prefix());
            let mut statement = state.connection.prepare(&query).map_err(|error| {
                AppError::Analysis(format!("Failed to prepare correlation query: {error}"))
            })?;
            let rows = statement
                .query_map(
                    params![
                        portal_key,
                        minimum_complaints,
                        result_limit,
                        correlation_window_hours
                    ],
                    map_correlation_row,
                )
                .map_err(|error| {
                    AppError::Analysis(format!("Failed to execute correlation query: {error}"))
                })?;
            rows.map(|row| {
                let row = row.map_err(|error| {
                    AppError::Analysis(format!("Failed to decode correlation row: {error}"))
                })?;
                Ok(CrossAgencyCorrelation {
                    request_id: unsigned(row.request_id, "request identifier")?,
                    portal_key: row.portal_key,
                    agency_id: row
                        .agency_id
                        .map(|value| unsigned(value, "agency identifier"))
                        .transpose()?,
                    agency_name: row.agency_name,
                    request_title: row.request_title,
                    request_timestamp: row.request_timestamp,
                    neighborhood_block: row.neighborhood_block,
                    complaint_count: count(row.complaint_count, "complaint count")?,
                    correlation_window_hours: count(
                        row.correlation_window_hours,
                        "correlation window",
                    )?,
                    distinct_source_count: count(row.distinct_source_count, "source count")?,
                    first_correlated_event_at: row.first_correlated_event_at,
                    last_correlated_event_at: row.last_correlated_event_at,
                    keyword_signal_count: count(row.keyword_signal_count, "keyword signal count")?,
                    correlation_density: row.correlation_density,
                })
            })
            .collect()
        })
        .await?
    }
}

fn map_correlation_row(row: &Row<'_>) -> duckdb::Result<CorrelationRow> {
    Ok(CorrelationRow {
        request_id: row.get(0)?,
        portal_key: row.get(1)?,
        agency_id: row.get(2)?,
        agency_name: row.get(3)?,
        request_title: row.get(4)?,
        request_timestamp: row.get(5)?,
        neighborhood_block: row.get(6)?,
        complaint_count: row.get(7)?,
        correlation_window_hours: row.get(8)?,
        distinct_source_count: row.get(9)?,
        first_correlated_event_at: row.get(10)?,
        last_correlated_event_at: row.get(11)?,
        keyword_signal_count: row.get(12)?,
        correlation_density: row.get(13)?,
    })
}

#[cfg(test)]
fn validate_layout(connection: &Connection, table_prefix: &str) -> Result<(), AppError> {
    validate_layout_query(
        connection,
        "SQLite analytical layout",
        table_prefix,
        "SELECT COUNT(*) FROM (
                SELECT requests.id, requests.portal_subdomain, requests.agency_id,
                    requests.title, requests.description, requests.created_at,
                    requests.neighborhood_block, agencies.name,
                    request_keyword_signals.keyword, request_keyword_signals.match_count
                FROM {db}requests AS requests
                LEFT JOIN {db}agencies AS agencies
                  ON agencies.id = requests.agency_id
                 AND agencies.portal_subdomain = requests.portal_subdomain
                LEFT JOIN {db}request_keyword_signals AS request_keyword_signals
                  ON request_keyword_signals.request_id = requests.id
                 AND request_keyword_signals.portal_subdomain = requests.portal_subdomain
                WHERE FALSE
             );",
    )?;
    validate_layout_query(
        connection,
        "Auxiliary analytical layout",
        table_prefix,
        "SELECT COUNT(*) FROM (
                SELECT auxiliary_events.source_id, auxiliary_events.occurred_at,
                    auxiliary_events.neighborhood_block, auxiliary_data_sources.source_kind
                FROM {db}auxiliary_events AS auxiliary_events
                INNER JOIN {db}auxiliary_data_sources AS auxiliary_data_sources
                  ON auxiliary_data_sources.source_id = auxiliary_events.source_id
                WHERE FALSE
             );",
    )?;
    Ok(())
}

#[cfg(test)]
fn validate_layout_query(
    connection: &Connection,
    layout_name: &str,
    table_prefix: &str,
    query: &str,
) -> Result<(), AppError> {
    connection
        .query_row(&qualify(query, table_prefix), [], |row| {
            row.get::<_, i64>(0)
        })
        .map(|_| ())
        .map_err(|error| AppError::Analysis(format!("{layout_name} is incompatible: {error}")))
}

#[cfg(test)]
fn validate_database_path(path: &Path) -> Result<PathBuf, AppError> {
    if path.as_os_str().is_empty() || path == Path::new(":memory:") {
        return Err(AppError::DataParsing(
            "DuckDB requires a file-backed SQLite database".to_string(),
        ));
    }
    let canonical = std::fs::canonicalize(path).map_err(|error| {
        AppError::Storage(format!(
            "SQLite path '{}' is inaccessible: {error}",
            path.display()
        ))
    })?;
    if !canonical.is_file() {
        return Err(AppError::Storage(format!(
            "SQLite path '{}' is not a regular file",
            canonical.display()
        )));
    }
    Ok(canonical)
}

fn lock_state(state: &Arc<Mutex<DuckDbState>>) -> Result<MutexGuard<'_, DuckDbState>, AppError> {
    lock_state_with_timeout(state, WORKSPACE_LOCK_TIMEOUT)
}

fn lock_state_with_timeout(
    state: &Arc<Mutex<DuckDbState>>,
    timeout: Duration,
) -> Result<MutexGuard<'_, DuckDbState>, AppError> {
    let started_at = Instant::now();
    loop {
        match state.try_lock() {
            Ok(guard) => return Ok(guard),
            Err(TryLockError::Poisoned(_)) => {
                return Err(AppError::Analysis(
                    "DuckDB workspace lock was poisoned".to_string(),
                ));
            }
            Err(TryLockError::WouldBlock) => {
                let elapsed = started_at.elapsed();
                if elapsed >= timeout {
                    return Err(AppError::Analysis(format!(
                        "DuckDB workspace remained busy for {} ms; retry the operation",
                        timeout.as_millis()
                    )));
                }
                std::thread::sleep(
                    WORKSPACE_LOCK_POLL_INTERVAL.min(timeout.saturating_sub(elapsed)),
                );
            }
        }
    }
}

fn unsigned(value: i64, label: &str) -> Result<u64, AppError> {
    u64::try_from(value)
        .map_err(|_| AppError::DataParsing(format!("DuckDB produced invalid {label}: {value}")))
}

fn count(value: i64, label: &str) -> Result<usize, AppError> {
    usize::try_from(value)
        .map_err(|_| AppError::DataParsing(format!("DuckDB produced invalid {label}: {value}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_lock_reports_contention_after_the_requested_timeout() {
        let workspace = DuckDbWorkspace::open_workspace().expect("workspace must open");
        let _held = workspace
            .state
            .lock()
            .expect("fixture lock must be available");
        let error = match lock_state_with_timeout(&workspace.state, Duration::from_millis(10)) {
            Ok(_) => panic!("a held workspace lock must time out"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("remained busy for 10 ms"));
    }

    #[tokio::test]
    async fn correlates_persisted_keyword_signals_with_prior_311_events() {
        let database_file = tempfile::NamedTempFile::new().unwrap();
        let connection = rusqlite::Connection::open(database_file.path()).unwrap();
        // This test exercises the external SQLite attachment adapter, not the
        // DuckDB production migration. Keep the fixture intentionally minimal
        // and SQLite-compatible so the two storage engines remain decoupled.
        connection
            .execute_batch(
                "CREATE TABLE portals (subdomain TEXT PRIMARY KEY, display_name TEXT, base_url TEXT);
                 CREATE TABLE agencies (id INTEGER, portal_subdomain TEXT, name TEXT, PRIMARY KEY (id, portal_subdomain));
                 CREATE TABLE requests (id INTEGER, portal_subdomain TEXT, title TEXT, description TEXT, status TEXT, created_at TEXT, updated_at TEXT, neighborhood_block TEXT, agency_id INTEGER, PRIMARY KEY (id, portal_subdomain));
                 CREATE TABLE request_keyword_signals (request_id INTEGER, portal_subdomain TEXT, keyword TEXT, match_count INTEGER);
                 CREATE TABLE auxiliary_data_sources (source_id TEXT PRIMARY KEY, source_kind TEXT, display_name TEXT, source_sha256 TEXT, imported_at TEXT, record_count INTEGER);
                 CREATE TABLE auxiliary_events (source_id TEXT, external_id TEXT, occurred_at TEXT, neighborhood_block TEXT, event_type TEXT, description TEXT, source_payload_json TEXT);",
            )
            .unwrap();
        connection
            .execute_batch(
                "INSERT INTO portals (subdomain, display_name, base_url)
                 VALUES ('city-a', 'City A', 'https://city-a.example');
                 INSERT INTO agencies (id, portal_subdomain, name)
                 VALUES (7, 'city-a', 'Neighborhood Services');
                 INSERT INTO requests (
                    id, portal_subdomain, title, description, status, created_at,
                    updated_at, neighborhood_block, agency_id
                 ) VALUES (
                    41, 'city-a', 'Encampment field operation', '', 'open',
                    '2026-06-03T12:00:00Z', '2026-06-03T12:00:00Z', 'Block 12', 7
                 );
                 INSERT INTO request_keyword_signals (
                    request_id, portal_subdomain, keyword, match_count
                 ) VALUES (41, 'city-a', 'encampment', 2);
                 INSERT INTO auxiliary_data_sources (
                    source_id, source_kind, display_name, source_sha256, imported_at, record_count
                 ) VALUES (
                    'source-311', 'MUNICIPAL_311', '311 log',
                    'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                    '2026-06-04T00:00:00Z', 3
                 );
                 INSERT INTO auxiliary_events (
                    source_id, external_id, occurred_at, neighborhood_block,
                    event_type, description, source_payload_json
                 ) VALUES
                    ('source-311', 'a', '2026-06-02T10:00:00Z', 'Block 12', 'complaint', '', '{}'),
                    ('source-311', 'b', '2026-06-03T11:00:00Z', 'Block 12', 'complaint', '', '{}'),
                    ('source-311', 'c', '2026-05-25T11:00:00Z', 'Block 12', 'complaint', '', '{}');",
            )
            .unwrap();
        drop(connection);

        let workspace = DuckDbWorkspace::open_workspace().unwrap();
        workspace
            .attach_sqlite_read_only(database_file.path())
            .await
            .unwrap();
        let correlations = workspace
            .correlate_311_with_enforcement_requests(Some("city-a"), 2, 100, 48)
            .await
            .unwrap();
        assert_eq!(correlations.len(), 1);
        assert_eq!(correlations[0].complaint_count, 2);
        assert_eq!(correlations[0].correlation_window_hours, 48);
        assert_eq!(correlations[0].keyword_signal_count, 2);
        let one_day = workspace
            .correlate_311_with_enforcement_requests(Some("city-a"), 1, 100, 24)
            .await
            .unwrap();
        assert_eq!(one_day[0].complaint_count, 1);
        assert_eq!(one_day[0].correlation_window_hours, 24);
        assert!((one_day[0].correlation_density - (1.0 / 24.0)).abs() < f64::EPSILON);
    }

    #[test]
    fn release_scale_harness_covers_million_rows_large_text_indexes_and_concurrent_reads() {
        const ROW_COUNT: i64 = 1_000_000;
        const LARGE_TEXT_BYTES: i64 = 32 * 1024 * 1024;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("release-performance.duckdb");
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "SET threads = 1;
                 SET memory_limit = '512MB';
                 CREATE TABLE perf_requests AS
                 SELECT value AS id, value % 100 AS agency_id,
                        CASE WHEN value % 17 = 0 THEN 'closed' ELSE 'open' END AS status
                 FROM range(1000000) rows(value);
                 CREATE INDEX idx_perf_requests_agency ON perf_requests(agency_id, status);",
            )
            .unwrap();
        let indexed_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM perf_requests WHERE agency_id = 42 AND status = 'closed'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(indexed_count > 0);
        connection
            .execute_batch(
                "DROP INDEX idx_perf_requests_agency;
                 CREATE INDEX idx_perf_requests_agency ON perf_requests(agency_id, status);
                 CREATE TABLE perf_documents AS
                 SELECT 1 AS document_id, repeat('e', 33554432) AS extracted_text;",
            )
            .unwrap();
        let large_text_bytes: i64 = connection
            .query_row(
                "SELECT length(extracted_text) FROM perf_documents",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(large_text_bytes, LARGE_TEXT_BYTES);
        // Concurrent readers share the one database instance, as the app's do: a second
        // instance of the same file is refused on Windows, which locks the file per instance.
        std::thread::scope(|scope| {
            let mut readers = Vec::new();
            for _ in 0..4 {
                let reader = connection.try_clone().unwrap();
                readers.push(scope.spawn(move || {
                    reader
                        .query_row("SELECT COUNT(*) FROM perf_requests", [], |row| {
                            row.get::<_, i64>(0)
                        })
                        .unwrap()
                }));
            }
            for reader in readers {
                assert_eq!(reader.join().unwrap(), ROW_COUNT);
            }
        });
    }
}
