use crate::blocking::spawn_with;
use crate::error::AppError;
use crate::types::CrossAgencyCorrelation;
use duckdb::{params, Connection, Row};
use std::convert::TryFrom;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

static SQLITE_EXTENSION_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

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
        COUNT(*) AS complaint_count_48h,
        COUNT(DISTINCT complaint_events.source_id) AS distinct_source_count,
        MIN(complaint_events.occurred_timestamp) AS first_correlated_event_at,
        MAX(complaint_events.occurred_timestamp) AS last_correlated_event_at
    FROM request_candidates
    INNER JOIN complaint_events
      ON complaint_events.normalized_neighborhood = request_candidates.normalized_neighborhood
     AND complaint_events.occurred_timestamp >= request_candidates.request_timestamp - INTERVAL 48 HOURS
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
    complaint_count_48h,
    distinct_source_count,
    CAST(first_correlated_event_at AS VARCHAR),
    CAST(last_correlated_event_at AS VARCHAR),
    keyword_signal_count,
    CAST(complaint_count_48h AS DOUBLE) / 48.0 AS correlation_density
FROM correlated
WHERE complaint_count_48h >= ?2
ORDER BY complaint_count_48h DESC, portal_key, request_id
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

// TODO(concurrency): Writes are serialized by the repository's `Arc<Mutex<Connection>>` and
// analytics reads run on sibling connections to the same instance, so the writer/reader boundary is
// correct. Backpressure is not: an analytics query issued during heavy ingest simply waits on the
// mutex with no timeout, no queue depth signal, and no way for the caller to cancel.
// TODO(performance-validation): Add a benchmark corpus covering million-row ingestion, concurrent
// analytics, large extracted text, and index rebuild latency before tuning production pragmas.

struct CorrelationRow {
    request_id: i64,
    portal_key: String,
    agency_id: Option<i64>,
    agency_name: String,
    request_title: String,
    request_timestamp: String,
    neighborhood_block: String,
    complaint_count_48h: i64,
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
    /// The previous implementation opened a separate in-memory connection and `ATTACH`ed the core
    /// file. That connection observed the database as it stood at process start and never saw a
    /// subsequent write, so every correlation was computed against a launch-time snapshot.
    pub fn bind_to_core(connection: Connection) -> Self {
        Self {
            state: Arc::new(Mutex::new(DuckDbState {
                connection,
                source: WorkspaceSource::Core,
                attached_sqlite_path: None,
            })),
        }
    }

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
            initialize_sqlite_extension(&state.connection)?;
            if state.attached_sqlite_path.as_ref() == Some(&canonical_path) {
                return validate_layout(&state.connection, state.source.table_prefix());
            }
            if state.attached_sqlite_path.is_some() {
                state
                    .connection
                    .execute_batch("DETACH db;")
                    .map_err(|error| {
                        AppError::Analysis(format!("Failed to detach SQLite source: {error}"))
                    })?;
                state.attached_sqlite_path = None;
            }
            let attachment = format!(
                "ATTACH {} AS db (TYPE SQLITE, READ_ONLY);",
                quote_path(&canonical_path)?
            );
            state
                .connection
                .execute_batch(&attachment)
                .map_err(|error| {
                    AppError::Analysis(format!("Failed to attach SQLite source: {error}"))
                })?;
            if let Err(error) = validate_layout(&state.connection, state.source.table_prefix()) {
                // The alias must be released before anything is attached under it again. If the
                // release fails the connection still owns `db` while the tracked state says
                // nothing is attached, which would reject every later attach; discard the node.
                if let Err(detach_error) = state.connection.execute_batch("DETACH db;") {
                    reset_workspace_connection(&mut state).map_err(|reset_error| {
                        AppError::Analysis(format!(
                            "Attached SQLite layout was rejected ({error}); releasing the alias \
                             failed ({detach_error}) and rebuilding the workspace also failed \
                             ({reset_error})"
                        ))
                    })?;
                }
                return Err(error);
            }
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
        let portal_key = portal_key.unwrap_or("").trim().to_string();
        let minimum_complaints = i64::try_from(minimum_complaints).map_err(|_| {
            AppError::DataParsing("Minimum complaint count exceeds DuckDB capacity".to_string())
        })?;
        let result_limit = i64::try_from(result_limit).map_err(|_| {
            AppError::DataParsing("Correlation result limit exceeds DuckDB capacity".to_string())
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
                    params![portal_key, minimum_complaints, result_limit],
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
                    complaint_count_48h: count(row.complaint_count_48h, "complaint count")?,
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
        complaint_count_48h: row.get(7)?,
        distinct_source_count: row.get(8)?,
        first_correlated_event_at: row.get(9)?,
        last_correlated_event_at: row.get(10)?,
        keyword_signal_count: row.get(11)?,
        correlation_density: row.get(12)?,
    })
}

fn initialize_sqlite_extension(connection: &Connection) -> Result<(), AppError> {
    if connection.execute_batch("LOAD sqlite;").is_ok() {
        return Ok(());
    }
    let lock = SQLITE_EXTENSION_LOCK.get_or_init(|| Mutex::new(()));
    let _guard = lock
        .lock()
        .map_err(|_| AppError::Analysis("SQLite extension lock was poisoned".to_string()))?;
    if connection.execute_batch("LOAD sqlite;").is_ok() {
        return Ok(());
    }
    // TODO(distribution): Bundle and integrity-check the DuckDB SQLite extension so first-run
    // analytics remains deterministic in offline and restricted-network forensic environments.
    connection
        .execute_batch("INSTALL sqlite; LOAD sqlite;")
        .map_err(|error| {
            AppError::Analysis(format!("Failed to load DuckDB SQLite extension: {error}"))
        })
}

/// Replaces a foreign-SQLite workspace's connection with a fresh one, discarding any alias the old
/// connection could not release. Only valid for `WorkspaceSource::ForeignSqlite`, whose connection
/// is an in-memory node holding no durable state of its own.
fn reset_workspace_connection(state: &mut DuckDbState) -> Result<(), AppError> {
    debug_assert_eq!(state.source, WorkspaceSource::ForeignSqlite);
    state.attached_sqlite_path = None;
    state.connection = Connection::open_in_memory().map_err(|error| {
        AppError::Analysis(format!("Failed to rebuild the DuckDB workspace: {error}"))
    })?;
    Ok(())
}

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

fn quote_path(path: &Path) -> Result<String, AppError> {
    let value = path.to_str().ok_or_else(|| {
        AppError::DataParsing(format!("SQLite path '{}' is not UTF-8", path.display()))
    })?;
    if value.contains('\0') {
        return Err(AppError::DataParsing(
            "SQLite path contains a NUL character".to_string(),
        ));
    }
    Ok(format!("'{}'", value.replace('\'', "''")))
}

fn lock_state(state: &Arc<Mutex<DuckDbState>>) -> Result<MutexGuard<'_, DuckDbState>, AppError> {
    state
        .lock()
        .map_err(|_| AppError::Analysis("DuckDB workspace lock was poisoned".to_string()))
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
            .correlate_311_with_enforcement_requests(Some("city-a"), 2, 100)
            .await
            .unwrap();
        assert_eq!(correlations.len(), 1);
        assert_eq!(correlations[0].complaint_count_48h, 2);
        assert_eq!(correlations[0].keyword_signal_count, 2);
    }
}
