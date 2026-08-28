use crate::analysis::statute;
use crate::error::AppError;
use crate::types::{AgencyPerformanceProfile, AnalysisConfig, VerificationState};
use duckdb::{params, Connection};
use std::collections::{BTreeSet, HashMap};
use std::convert::TryFrom;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

const SQLITE_DATABASE_ALIAS: &str = "db";
const SECONDS_PER_DAY: f64 = 86_400.0;
pub const STATUTORY_DEADLINE_Z_SCORE: f64 = 2.0;
static SQLITE_EXTENSION_INSTALLATION_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

const SQLITE_LAYOUT_PROBE_QUERY: &str = r#"
SELECT COUNT(*)
FROM (
    SELECT
        requests.id,
        requests.portal_subdomain,
        requests.agency_id,
        requests.title,
        requests.description,
        requests.status,
        requests.created_at,
        requests.updated_at,
        requests.closed_at,
        agencies.name,
        documents.request_id,
        documents.visibility,
        statutory_extensions.notice_at,
        statutory_extensions.expected_determination_at,
        statutory_extensions.reason,
        statutory_extensions.notice_source
    FROM {db}requests AS requests
    LEFT JOIN {db}agencies AS agencies
      ON agencies.id = requests.agency_id
     AND agencies.portal_subdomain = requests.portal_subdomain
    LEFT JOIN {db}documents AS documents
      ON documents.request_id = requests.id
     AND documents.portal_subdomain = requests.portal_subdomain
    LEFT JOIN {db}statutory_extensions AS statutory_extensions
      ON statutory_extensions.request_id = requests.id
     AND statutory_extensions.portal_subdomain = requests.portal_subdomain
    WHERE FALSE
) AS required_layout
"#;

/// Per-agency aggregate over every request, rendered from `analysis::statute`.
///
/// Only the aggregation belongs to this module. The determination proxy, the extension-validity
/// rule, the calendar-day derivation, and every threshold are rendered from `statute::sql`, which
/// is the same model `analysis::timeline` evaluates one request at a time. The two therefore
/// cannot drift apart without failing `statutory_verdicts_match_the_per_request_evaluator`.
///
/// The `parsed` CTE establishes the column names `statute::sql` documents as its contract. Note
/// that instants and calendar dates are derived separately and deliberately: durations and
/// ordering use the `TIMESTAMPTZ`-derived instants, while every day count uses the recorded-offset
/// date, because the statute counts days in the offset the agency wrote down.
const AGENCY_PERFORMANCE_QUERY_TEMPLATE: &str = r#"
WITH parsed AS (
    SELECT
        requests.id,
        requests.portal_subdomain,
        requests.agency_id,
        LOWER(TRIM(requests.status)) AS normalized_status,
        requests.closed_at,
        CAST(TRY_CAST(requests.created_at AS TIMESTAMPTZ) AS TIMESTAMP) AS created_timestamp,
        CAST(TRY_CAST(requests.updated_at AS TIMESTAMPTZ) AS TIMESTAMP) AS updated_timestamp,
        CAST(TRY_CAST(requests.closed_at AS TIMESTAMPTZ) AS TIMESTAMP) AS closed_timestamp,
        CAST(TRY_CAST(requests.determination_at AS TIMESTAMPTZ) AS TIMESTAMP) AS determination_timestamp,
        {calendar_date(requests.created_at)} AS created_date,
        {calendar_date(requests.updated_at)} AS updated_date,
        {calendar_date(requests.closed_at)} AS closed_date,
        {calendar_date(requests.determination_at)} AS explicit_determination_date,
        requests.determination_source,
        CAST(TRY_CAST(statutory_extensions.notice_at AS TIMESTAMPTZ) AS TIMESTAMP) AS extension_notice_timestamp,
        {calendar_date(statutory_extensions.notice_at)} AS extension_notice_date,
        {calendar_date(statutory_extensions.expected_determination_at)} AS extension_expected_date,
        statutory_extensions.reason AS extension_reason,
        statutory_extensions.notice_source AS extension_notice_source
    FROM {db}requests AS requests
    LEFT JOIN {db}statutory_extensions AS statutory_extensions
      ON statutory_extensions.request_id = requests.id
     AND statutory_extensions.portal_subdomain = requests.portal_subdomain
    WHERE requests.agency_id IS NOT NULL
),
milestones AS (
    SELECT
        *,
        {determination_calendar_date} AS determination_date,
        {valid_extension_predicate} AS has_valid_extension
    FROM parsed
),
evaluated AS (
    SELECT
        *,
        {allowed_calendar_days} AS allowed_calendar_days,
        CASE
            WHEN created_timestamp IS NOT NULL
             AND closed_timestamp IS NOT NULL
             AND closed_timestamp >= created_timestamp
            THEN CAST(date_diff('microsecond', created_timestamp, closed_timestamp) AS DOUBLE)
                 / 1000000.0
            ELSE NULL
        END AS latency_seconds
    FROM milestones
),
request_counts AS (
    SELECT
        agency_id,
        portal_subdomain,
        COUNT(*) AS total_requests,
        COUNT(*) FILTER (
            WHERE closed_at IS NOT NULL AND TRIM(closed_at) <> ''
        ) AS total_closed,
        COUNT(*) FILTER (
            WHERE closed_at IS NOT NULL
              AND TRIM(closed_at) <> ''
              AND (
                created_timestamp IS NULL
                OR closed_timestamp IS NULL
                OR closed_timestamp < created_timestamp
              )
        ) AS excluded_invalid_lifetime_count,
        COUNT(*) FILTER (
            WHERE {statutory_violation_predicate}
        ) AS statutory_violation_count,
        COUNT(*) FILTER (
            WHERE {suspicious_fast_closure_predicate}
        ) AS suspicious_fast_closure_count
    FROM evaluated
    GROUP BY agency_id, portal_subdomain
),
agency_latency AS (
    SELECT
        agency_id,
        portal_subdomain,
        COUNT(latency_seconds) AS valid_latency_sample_size,
        AVG(latency_seconds) AS agency_mean_seconds,
        COUNT(*) FILTER (
            WHERE latency_seconds > (?1 * 86400.0)
        ) AS latency_anomaly_count
    FROM evaluated
    WHERE latency_seconds IS NOT NULL
    GROUP BY agency_id, portal_subdomain
),
global_latency AS (
    SELECT
        AVG(latency_seconds) AS global_mean_seconds,
        STDDEV_POP(latency_seconds) AS global_stddev_seconds
    FROM evaluated
    WHERE latency_seconds IS NOT NULL
),
released_documents AS (
    SELECT
        requests.agency_id,
        requests.portal_subdomain,
        COUNT(*) FILTER (
            WHERE LOWER(TRIM(COALESCE(documents.visibility, ''))) = 'public'
        ) AS total_documents_released
    FROM {db}documents AS documents
    INNER JOIN {db}requests AS requests
      ON requests.id = documents.request_id
     AND requests.portal_subdomain = documents.portal_subdomain
    WHERE requests.agency_id IS NOT NULL
    GROUP BY requests.agency_id, requests.portal_subdomain
)
SELECT
    request_counts.agency_id,
    request_counts.portal_subdomain,
    COALESCE(NULLIF(TRIM(agencies.name), ''), 'Unmapped Department') AS agency_name,
    request_counts.total_requests,
    request_counts.total_closed,
    COALESCE(agency_latency.valid_latency_sample_size, 0),
    request_counts.excluded_invalid_lifetime_count,
    -- No COALESCE to 0.0 on any of these three. An agency whose requests are all still open has
    -- no row in `agency_latency`, and 0.0 days would report it as closing every request the same
    -- day -- the fastest possible answer -- rather than as never having closed one. NULL is the
    -- only honest reading, and `valid_latency_sample_size` above says how many closures it rests
    -- on. Same for the global figures when nothing in the workspace has closed.
    agency_latency.agency_mean_seconds / 86400.0,
    global_latency.global_mean_seconds / 86400.0,
    global_latency.global_stddev_seconds / 86400.0,
    CASE
        WHEN COALESCE(agency_latency.valid_latency_sample_size, 0) > 0
         AND COALESCE(global_latency.global_stddev_seconds, 0.0) > 0.0
        THEN (agency_latency.agency_mean_seconds - global_latency.global_mean_seconds)
             / global_latency.global_stddev_seconds
        -- NULL, not 0.0: 0.0 means "exactly at the population mean", which is a measurement.
        ELSE NULL
    END AS performance_z_score,
    COALESCE(agency_latency.latency_anomaly_count, 0),
    request_counts.statutory_violation_count,
    CAST(request_counts.statutory_violation_count AS DOUBLE)
        / NULLIF(CAST(request_counts.total_requests AS DOUBLE), 0.0),
    request_counts.suspicious_fast_closure_count,
    COALESCE(released_documents.total_documents_released, 0)
FROM request_counts
CROSS JOIN global_latency
LEFT JOIN agency_latency
  ON agency_latency.agency_id = request_counts.agency_id
 AND agency_latency.portal_subdomain = request_counts.portal_subdomain
LEFT JOIN {db}agencies AS agencies
  ON agencies.id = request_counts.agency_id
 AND agencies.portal_subdomain = request_counts.portal_subdomain
LEFT JOIN released_documents
  ON released_documents.agency_id = request_counts.agency_id
 AND released_documents.portal_subdomain = request_counts.portal_subdomain
ORDER BY performance_z_score DESC NULLS LAST, request_counts.statutory_violation_count DESC,
         request_counts.portal_subdomain, request_counts.agency_id
"#;

/// Where an analytics node reads from. The two modes are not interchangeable: a node that
/// `ATTACH`es a database file observes a snapshot frozen at attach time, so the application's own
/// core database must always be read through a live sibling connection instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AnalyticsSource {
    /// Sibling connection to the live DuckDB core. Tables are unqualified; attaching is rejected.
    Core,
    /// Standalone in-memory node inspecting a foreign SQLite file under the `db` alias.
    ForeignSqlite,
}

impl AnalyticsSource {
    /// Table-name prefix that `qualify` substitutes into every analytical query.
    fn table_prefix(self) -> &'static str {
        match self {
            Self::Core => "",
            Self::ForeignSqlite => "db.",
        }
    }
}

struct AnalyticsState {
    connection: Connection,
    source: AnalyticsSource,
    attached_sqlite_path: Option<PathBuf>,
}

#[derive(Clone)]
pub struct AnalyticsEngine {
    state: Arc<Mutex<AnalyticsState>>,
}

/// Substitutes the `{db}` table qualifier so one query text serves both analytics sources.
fn qualify(template: &str, table_prefix: &str) -> String {
    template.replace("{db}", table_prefix)
}

struct RawAgencyPerformanceProfile {
    agency_id: i64,
    portal_subdomain: String,
    agency_name: String,
    total_requests_processed: i64,
    total_closed: i64,
    valid_latency_sample_size: i64,
    excluded_invalid_lifetime_count: i64,
    /// `None` when this agency has closed nothing measurable, so there is no mean to report.
    average_closure_days: Option<f64>,
    /// `None` when nothing in the workspace has closed, so there is no population to compare to.
    global_mean_closure_days: Option<f64>,
    global_stddev_closure_days: Option<f64>,
    /// `None` when the agency has no valid closure sample, or the population has no spread.
    performance_z_score: Option<f64>,
    anomaly_count: i64,
    statutory_violation_count: i64,
    statutory_violation_rate: f64,
    suspicious_fast_closure_count: i64,
    total_documents_released: i64,
}

struct KeywordPattern {
    label: String,
    tokens: Vec<String>,
}

impl AnalyticsEngine {
    /// Creates a standalone node for inspecting a foreign SQLite database supplied by the
    /// investigator. Call `attach_sqlite_source_path` before querying.
    pub fn new() -> Result<Self, AppError> {
        let connection = Connection::open_in_memory().map_err(|error| {
            AppError::Analysis(format!(
                "Failed to initialize DuckDB analytics node: {error}"
            ))
        })?;
        Ok(Self {
            state: Arc::new(Mutex::new(AnalyticsState {
                connection,
                source: AnalyticsSource::ForeignSqlite,
                attached_sqlite_path: None,
            })),
        })
    }

    /// Binds the node to a live sibling connection of the application's own DuckDB core, obtained
    /// from `DuckDbRepository::analytics_connection`.
    ///
    /// The previous implementation opened a separate in-memory connection and `ATTACH`ed the core
    /// file. That connection observed the database as it stood at process start and never saw a
    /// subsequent write, so every profile was computed against a launch-time snapshot.
    pub fn bind_to_core(connection: Connection) -> Self {
        Self {
            state: Arc::new(Mutex::new(AnalyticsState {
                connection,
                source: AnalyticsSource::Core,
                attached_sqlite_path: None,
            })),
        }
    }

    pub async fn attach_sqlite_source(&self, sqlite_path: &str) -> Result<(), AppError> {
        self.attach_sqlite_source_path(sqlite_path).await
    }

    pub async fn attach_sqlite_source_path(
        &self,
        sqlite_path: impl AsRef<Path>,
    ) -> Result<(), AppError> {
        let requested_path = sqlite_path.as_ref().to_path_buf();
        let state = Arc::clone(&self.state);
        tokio::task::spawn_blocking(move || {
            let canonical_path = validate_sqlite_path(&requested_path)?;
            let mut state = lock_analytics_state(&state)?;
            if state.source == AnalyticsSource::Core {
                return Err(AppError::Analysis(
                    "The core analytics node cannot attach a foreign SQLite database".to_string(),
                ));
            }
            initialize_sqlite_extension(&state.connection)?;
            if state.attached_sqlite_path.as_ref() == Some(&canonical_path) {
                return validate_attached_sqlite_layout(&state.connection);
            }
            replace_sqlite_attachment(&mut state, canonical_path)
        })
        .await
        .map_err(|error| AppError::Analysis(format!("DuckDB attachment worker failed: {error}")))?
    }

    pub async fn calculate_agency_outlier_profiles(
        &self,
        config: &AnalysisConfig,
    ) -> Result<Vec<AgencyPerformanceProfile>, AppError> {
        validate_latency_threshold(config.latency_threshold_days)?;
        let patterns = keyword_patterns(&config.target_policy_keywords);
        let threshold = config.latency_threshold_days;
        let state = Arc::clone(&self.state);
        tokio::task::spawn_blocking(move || {
            let state = lock_analytics_state(&state)?;
            if state.source == AnalyticsSource::ForeignSqlite
                && state.attached_sqlite_path.is_none()
            {
                return Err(AppError::Analysis(
                    "No SQLite source is attached to the analytics node".to_string(),
                ));
            }
            let table_prefix = state.source.table_prefix();
            let mut profiles =
                query_agency_performance_profiles(&state.connection, threshold, table_prefix)?;
            hydrate_keyword_frequency_maps(
                &state.connection,
                &mut profiles,
                &patterns,
                table_prefix,
            )?;
            Ok(profiles)
        })
        .await
        .map_err(|error| AppError::Analysis(format!("DuckDB aggregation worker failed: {error}")))?
    }
}

/// Renders the statutory model into the aggregate query, then applies the source's table prefix.
///
/// Statutory placeholders are expanded before `{db}` so the two substitution passes stay
/// independent: `statute::sql` never sees a table qualifier and `qualify` never sees a rule.
fn agency_performance_query(table_prefix: &str) -> String {
    qualify(
        &statute::sql::render(AGENCY_PERFORMANCE_QUERY_TEMPLATE),
        table_prefix,
    )
}

fn query_agency_performance_profiles(
    connection: &Connection,
    latency_threshold_days: f64,
    table_prefix: &str,
) -> Result<Vec<AgencyPerformanceProfile>, AppError> {
    let mut statement = connection
        .prepare(&agency_performance_query(table_prefix))
        .map_err(|error| AppError::Analysis(format!("Failed to prepare agency query: {error}")))?;
    let rows = statement
        .query_map(params![latency_threshold_days], |row| {
            Ok(RawAgencyPerformanceProfile {
                agency_id: row.get(0)?,
                portal_subdomain: row.get(1)?,
                agency_name: row.get(2)?,
                total_requests_processed: row.get(3)?,
                total_closed: row.get(4)?,
                valid_latency_sample_size: row.get(5)?,
                excluded_invalid_lifetime_count: row.get(6)?,
                average_closure_days: row.get(7)?,
                global_mean_closure_days: row.get(8)?,
                global_stddev_closure_days: row.get(9)?,
                performance_z_score: row.get(10)?,
                anomaly_count: row.get(11)?,
                statutory_violation_count: row.get(12)?,
                statutory_violation_rate: row.get(13)?,
                suspicious_fast_closure_count: row.get(14)?,
                total_documents_released: row.get(15)?,
            })
        })
        .map_err(|error| AppError::Analysis(format!("Failed to execute agency query: {error}")))?;
    rows.map(|row| {
        convert_raw_profile(
            row.map_err(|error| {
                AppError::Analysis(format!("Failed to decode agency row: {error}"))
            })?,
        )
    })
    .collect()
}

fn hydrate_keyword_frequency_maps(
    connection: &Connection,
    profiles: &mut [AgencyPerformanceProfile],
    patterns: &[KeywordPattern],
    table_prefix: &str,
) -> Result<(), AppError> {
    for profile in profiles.iter_mut() {
        profile.keyword_frequency_map = patterns
            .iter()
            .map(|pattern| (pattern.label.clone(), 0_usize))
            .collect();
    }
    if profiles.is_empty() || patterns.is_empty() {
        return Ok(());
    }
    let profile_index = profiles
        .iter()
        .enumerate()
        .map(|(index, profile)| ((profile.agency_id, profile.portal_subdomain.clone()), index))
        .collect::<HashMap<_, _>>();
    let mut statement = connection
        .prepare(&qualify(
            "SELECT agency_id, portal_subdomain,
                COALESCE(title, '') || ' ' || COALESCE(description, '')
             FROM {db}requests
             WHERE agency_id IS NOT NULL",
            table_prefix,
        ))
        .map_err(|error| AppError::Analysis(format!("Failed to prepare keyword scan: {error}")))?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|error| AppError::Analysis(format!("Failed to query keyword text: {error}")))?;
    for row in rows {
        let (agency_id, portal_key, text) = row.map_err(|error| {
            AppError::Analysis(format!("Failed to decode keyword text: {error}"))
        })?;
        let agency_id = u64::try_from(agency_id)
            .map_err(|_| AppError::DataParsing(format!("Invalid agency identifier {agency_id}")))?;
        let Some(index) = profile_index.get(&(agency_id, portal_key)) else {
            continue;
        };
        let tokens = normalized_tokens(&text);
        for pattern in patterns {
            let count = count_token_pattern(&tokens, &pattern.tokens);
            let existing = profiles[*index]
                .keyword_frequency_map
                .get_mut(&pattern.label)
                .ok_or_else(|| {
                    AppError::Internal(format!(
                        "Keyword map omitted initialized pattern '{}'",
                        pattern.label
                    ))
                })?;
            *existing = existing.checked_add(count).ok_or_else(|| {
                AppError::Analysis(format!("Keyword count for '{}' overflowed", pattern.label))
            })?;
        }
    }
    Ok(())
}

fn keyword_patterns(keywords: &[String]) -> Vec<KeywordPattern> {
    let defaults = if keywords.is_empty() {
        vec![
            "sweep".to_string(),
            "encampment".to_string(),
            "eviction".to_string(),
        ]
    } else {
        keywords.to_vec()
    };
    defaults
        .into_iter()
        .map(|keyword| {
            keyword
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase()
        })
        .filter(|keyword| !keyword.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter_map(|label| {
            let tokens = normalized_tokens(&label);
            (!tokens.is_empty()).then_some(KeywordPattern { label, tokens })
        })
        .collect()
}

fn normalized_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for character in text.chars().chain(std::iter::once('\0')) {
        if character.is_alphanumeric() {
            current.extend(character.to_lowercase());
        } else if !current.is_empty() {
            tokens.push(normalize_stem(&current));
            current.clear();
        }
    }
    tokens
}

fn normalize_stem(token: &str) -> String {
    match token {
        "sweeps" | "sweeping" | "sweeped" | "swept" => "sweep".to_string(),
        "encampments" => "encampment".to_string(),
        "evict" | "evicts" | "evicted" | "evicting" | "evictions" => "eviction".to_string(),
        other => other.to_string(),
    }
}

fn count_token_pattern(tokens: &[String], pattern: &[String]) -> usize {
    if pattern.is_empty() || tokens.len() < pattern.len() {
        return 0;
    }
    tokens
        .windows(pattern.len())
        .filter(|window| *window == pattern)
        .count()
}

fn convert_raw_profile(
    raw: RawAgencyPerformanceProfile,
) -> Result<AgencyPerformanceProfile, AppError> {
    // Only a value that exists can be non-finite. An absent one is checked by being absent.
    for (label, value) in [
        ("agency mean", raw.average_closure_days),
        ("global mean", raw.global_mean_closure_days),
        ("global standard deviation", raw.global_stddev_closure_days),
        ("performance Z-score", raw.performance_z_score),
        (
            "statutory violation rate",
            Some(raw.statutory_violation_rate),
        ),
    ] {
        if let Some(value) = value {
            if !value.is_finite() {
                return Err(AppError::Analysis(format!(
                    "DuckDB produced a non-finite {label}: {value}"
                )));
            }
        }
    }
    let statutory_violation_count =
        count(raw.statutory_violation_count, "statutory violation count")?;
    // Two different claims, kept apart. `statutory_violation_count > 0` is a legal conclusion
    // drawn from the statute's calendar-day rule and is the only thing that may produce
    // `StatutoryDeadlineExceeded`. `performance_z_score >= STATUTORY_DEADLINE_Z_SCORE` is a
    // statistical statement about this agency's closure times relative to every other agency,
    // which no statute speaks to: an agency that answered every request lawfully but more slowly
    // than its peers is an outlier, not a violator. The statutory verdict is checked first
    // because an agency can be both, and the legal finding is the stronger claim.
    let compliance_indicator = if statutory_violation_count > 0 {
        VerificationState::StatutoryDeadlineExceeded
    } else if raw
        .performance_z_score
        .is_some_and(|score| score >= STATUTORY_DEADLINE_Z_SCORE)
    {
        VerificationState::LatencyOutlier
    } else {
        VerificationState::Compliant
    };
    Ok(AgencyPerformanceProfile {
        agency_id: u64::try_from(raw.agency_id).map_err(|_| {
            AppError::DataParsing(format!("Invalid agency identifier {}", raw.agency_id))
        })?,
        portal_subdomain: raw.portal_subdomain,
        agency_name: raw.agency_name,
        total_requests_processed: count(raw.total_requests_processed, "request count")?,
        total_closed: count(raw.total_closed, "closed request count")?,
        valid_latency_sample_size: count(raw.valid_latency_sample_size, "latency sample count")?,
        excluded_invalid_lifetime_count: count(
            raw.excluded_invalid_lifetime_count,
            "invalid lifetime count",
        )?,
        average_closure_days: raw.average_closure_days,
        global_mean_closure_days: raw.global_mean_closure_days,
        global_stddev_closure_days: raw.global_stddev_closure_days,
        performance_z_score: raw.performance_z_score,
        compliance_indicator,
        anomaly_count: count(raw.anomaly_count, "latency anomaly count")?,
        statutory_violation_count,
        statutory_violation_rate: raw.statutory_violation_rate,
        suspicious_fast_closure_count: count(
            raw.suspicious_fast_closure_count,
            "fast closure count",
        )?,
        total_documents_released: count(raw.total_documents_released, "document count")?,
        keyword_frequency_map: HashMap::new(),
    })
}

fn initialize_sqlite_extension(connection: &Connection) -> Result<(), AppError> {
    if connection.execute_batch("LOAD sqlite;").is_ok() {
        return Ok(());
    }
    let lock = SQLITE_EXTENSION_INSTALLATION_LOCK.get_or_init(|| Mutex::new(()));
    let _guard = lock.lock().map_err(|_| {
        AppError::Analysis("SQLite extension installation lock was poisoned".to_string())
    })?;
    if connection.execute_batch("LOAD sqlite;").is_ok() {
        return Ok(());
    }
    connection
        .execute_batch("INSTALL sqlite; LOAD sqlite;")
        .map_err(|error| {
            AppError::Analysis(format!("Failed to load DuckDB SQLite extension: {error}"))
        })
}

fn replace_sqlite_attachment(
    state: &mut AnalyticsState,
    new_path: PathBuf,
) -> Result<(), AppError> {
    let previous_path = state.attached_sqlite_path.take();
    if previous_path.is_some() {
        state
            .connection
            .execute_batch(&format!("DETACH {SQLITE_DATABASE_ALIAS};"))
            .map_err(|error| {
                AppError::Analysis(format!("Failed to detach prior SQLite source: {error}"))
            })?;
    }
    let attachment_result = attach_database(&state.connection, &new_path)
        .and_then(|()| validate_attached_sqlite_layout(&state.connection));
    if let Err(new_error) = attachment_result {
        // The alias must be released before anything is attached under it again. If the release
        // fails the connection still owns `db` while the tracked state says nothing is attached,
        // which would reject every later attach; discard the node and start from a clean one.
        if let Err(detach_error) = state
            .connection
            .execute_batch(&format!("DETACH {SQLITE_DATABASE_ALIAS};"))
        {
            reset_analytics_connection(state).map_err(|reset_error| {
                AppError::Analysis(format!(
                    "New SQLite source was rejected ({new_error}); releasing the alias failed \
                     ({detach_error}) and rebuilding the analytics node also failed ({reset_error})"
                ))
            })?;
            return Err(new_error);
        }
        if let Some(previous_path) = previous_path {
            match attach_database(&state.connection, &previous_path)
                .and_then(|()| validate_attached_sqlite_layout(&state.connection))
            {
                Ok(()) => {
                    state.attached_sqlite_path = Some(previous_path);
                    return Err(new_error);
                }
                Err(restore_error) => {
                    return Err(AppError::Analysis(format!(
                        "New SQLite source was rejected ({new_error}); restoring the prior source also failed ({restore_error})"
                    )));
                }
            }
        }
        return Err(new_error);
    }
    state.attached_sqlite_path = Some(new_path);
    Ok(())
}

/// Replaces a foreign-SQLite node's connection with a fresh one, discarding any alias the old
/// connection could not release. Only valid for `AnalyticsSource::ForeignSqlite`, whose connection
/// is an in-memory node holding no durable state of its own.
fn reset_analytics_connection(state: &mut AnalyticsState) -> Result<(), AppError> {
    debug_assert_eq!(state.source, AnalyticsSource::ForeignSqlite);
    state.attached_sqlite_path = None;
    state.connection = Connection::open_in_memory().map_err(|error| {
        AppError::Analysis(format!("Failed to rebuild the analytics node: {error}"))
    })?;
    Ok(())
}

fn attach_database(connection: &Connection, path: &Path) -> Result<(), AppError> {
    connection
        .execute_batch(&format!(
            "ATTACH {} AS {SQLITE_DATABASE_ALIAS} (TYPE SQLITE, READ_ONLY);",
            quote_sql_path(path)?
        ))
        .map_err(|error| AppError::Analysis(format!("Failed to attach SQLite database: {error}")))
}

fn validate_attached_sqlite_layout(connection: &Connection) -> Result<(), AppError> {
    connection
        .query_row(
            &qualify(
                SQLITE_LAYOUT_PROBE_QUERY,
                AnalyticsSource::ForeignSqlite.table_prefix(),
            ),
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| {
            AppError::Analysis(format!("Attached SQLite layout is incompatible: {error}"))
        })?;
    Ok(())
}

fn validate_sqlite_path(path: &Path) -> Result<PathBuf, AppError> {
    if path.as_os_str().is_empty() || path == Path::new(":memory:") {
        return Err(AppError::DataParsing(
            "Analytics requires a file-backed SQLite path".to_string(),
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

fn quote_sql_path(path: &Path) -> Result<String, AppError> {
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

fn validate_latency_threshold(value: f64) -> Result<(), AppError> {
    if !value.is_finite() || value < 0.0 || value > i64::MAX as f64 / SECONDS_PER_DAY {
        return Err(AppError::DataParsing(format!(
            "Latency threshold must be a finite, non-negative, representable number of days; received {value}"
        )));
    }
    Ok(())
}

fn count(value: i64, label: &str) -> Result<usize, AppError> {
    usize::try_from(value)
        .map_err(|_| AppError::DataParsing(format!("DuckDB produced invalid {label}: {value}")))
}

fn lock_analytics_state(
    state: &Arc<Mutex<AnalyticsState>>,
) -> Result<MutexGuard<'_, AnalyticsState>, AppError> {
    state
        .lock()
        .map_err(|_| AppError::Analysis("DuckDB analytics lock was poisoned".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::statute::DeterminationBasis;
    use crate::analysis::timeline::evaluate_statutory_compliance_at;
    use crate::types::{StatutoryExtensionReason, StatutoryExtensionRecord};

    fn connection() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                r#"
                CREATE SCHEMA db;
                CREATE TABLE db.requests (
                    id BIGINT, portal_subdomain VARCHAR, agency_id BIGINT,
                    title VARCHAR, description VARCHAR, status VARCHAR,
                    created_at VARCHAR, updated_at VARCHAR, closed_at VARCHAR,
                    determination_at VARCHAR, determination_source VARCHAR
                );
                CREATE TABLE db.agencies (id BIGINT, portal_subdomain VARCHAR, name VARCHAR);
                CREATE TABLE db.documents (
                    id BIGINT, request_id BIGINT, portal_subdomain VARCHAR, visibility VARCHAR
                );
                CREATE TABLE db.statutory_extensions (
                    request_id BIGINT, portal_subdomain VARCHAR, notice_at VARCHAR,
                    expected_determination_at VARCHAR, reason VARCHAR, notice_source VARCHAR
                );
                INSERT INTO db.agencies VALUES (1, 'city', 'Routine'), (2, 'city', 'Slow');
                INSERT INTO db.requests
                SELECT i, 'city', 1, 'Routine', 'encampment sweeping', 'closed',
                    '2024-01-01T00:00:00Z', '2024-01-05T00:00:00Z', '2024-01-11T00:00:00Z', NULL, NULL
                FROM range(1, 10) rows(i);
                INSERT INTO db.requests VALUES
                    (10, 'city', 2, 'Slow', 'eviction', 'closed',
                     '2024-01-01T00:00:00Z', '2024-01-20T00:00:00Z', '2024-04-10T00:00:00Z', NULL, NULL);
                INSERT INTO db.documents VALUES (1, 10, 'city', 'public');
                "#,
            )
            .unwrap();
        connection
    }

    #[test]
    fn computes_vectorized_profiles_and_statutory_counts() {
        let profiles = query_agency_performance_profiles(
            &connection(),
            90.0,
            AnalyticsSource::ForeignSqlite.table_prefix(),
        )
        .unwrap();
        let slow = &profiles[0];
        assert_eq!(slow.agency_id, 2);
        assert_eq!(slow.performance_z_score, Some(3.0));
        assert_eq!(slow.statutory_violation_count, 1);
        assert_eq!(
            slow.compliance_indicator,
            VerificationState::StatutoryDeadlineExceeded
        );
    }

    /// One request the parity fixture drives through both statutory implementations.
    struct ParityCase {
        label: &'static str,
        created_at: &'static str,
        updated_at: &'static str,
        status: &'static str,
        closed_at: Option<&'static str>,
        extension: Option<StatutoryExtensionRecord>,
    }

    fn parity_cases() -> Vec<ParityCase> {
        vec![
            ParityCase {
                label: "utc timely determination",
                created_at: "2024-01-01T00:00:00Z",
                updated_at: "2024-01-05T00:00:00Z",
                status: "closed",
                closed_at: Some("2024-01-11T00:00:00Z"),
                extension: None,
            },
            ParityCase {
                label: "utc late determination",
                created_at: "2024-01-01T00:00:00Z",
                updated_at: "2024-01-20T00:00:00Z",
                status: "closed",
                closed_at: Some("2024-04-10T00:00:00Z"),
                extension: None,
            },
            ParityCase {
                label: "tenth calendar day is still timely",
                created_at: "2026-01-01T00:01:00-08:00",
                updated_at: "2026-01-11T23:59:00-08:00",
                status: "open",
                closed_at: None,
                extension: None,
            },
            ParityCase {
                // Receipt late on 1 January Pacific is 2 January in UTC. Converting to a session
                // time zone before counting days would clear this agency at ten days; counting in
                // the recorded offset reports the eleventh day it actually took.
                label: "near-midnight receipt in a western offset",
                created_at: "2026-01-01T23:00:00-08:00",
                updated_at: "2026-01-12T10:00:00-08:00",
                status: "open",
                closed_at: None,
                extension: None,
            },
            ParityCase {
                label: "valid written extension",
                created_at: "2026-01-01T09:00:00-08:00",
                updated_at: "2026-01-20T12:00:00-08:00",
                status: "closed",
                closed_at: Some("2026-01-20T12:00:00-08:00"),
                extension: Some(StatutoryExtensionRecord {
                    notice_at: "2026-01-08T12:00:00-08:00".to_string(),
                    expected_determination_at: "2026-01-20T12:00:00-08:00".to_string(),
                    reason: StatutoryExtensionReason::VoluminousRecords,
                    notice_source: "Portal message 17".to_string(),
                }),
            },
            ParityCase {
                label: "extension beyond fourteen days is defective",
                created_at: "2026-01-01T09:00:00-08:00",
                updated_at: "2026-01-20T12:00:00-08:00",
                status: "closed",
                closed_at: Some("2026-01-20T12:00:00-08:00"),
                extension: Some(StatutoryExtensionRecord {
                    notice_at: "2026-01-08T12:00:00-08:00".to_string(),
                    expected_determination_at: "2026-01-26T12:00:00-08:00".to_string(),
                    reason: StatutoryExtensionReason::DataCompilation,
                    notice_source: "Portal notice".to_string(),
                }),
            },
            ParityCase {
                // A legacy row: the command path rejects a blank notice source, but a database
                // written before that check still has to be evaluated the same way by both paths.
                label: "extension without a notice source is defective",
                created_at: "2026-01-01T09:00:00-08:00",
                updated_at: "2026-01-20T12:00:00-08:00",
                status: "closed",
                closed_at: Some("2026-01-20T12:00:00-08:00"),
                extension: Some(StatutoryExtensionRecord {
                    notice_at: "2026-01-08T12:00:00-08:00".to_string(),
                    expected_determination_at: "2026-01-20T12:00:00-08:00".to_string(),
                    reason: StatutoryExtensionReason::InteragencyConsultation,
                    notice_source: "   ".to_string(),
                }),
            },
        ]
    }

    /// Session time zones the aggregate must produce identical verdicts under.
    ///
    /// The statutory clock is a calendar-day clock in the offset the agency recorded. Deriving
    /// dates by converting instants into the reader's session time zone makes the verdict depend
    /// on where the application happens to run, which is why the aggregate reads the RFC 3339 date
    /// prefix instead. These zones straddle the boundary in both directions.
    const PARITY_SESSION_TIME_ZONES: [&str; 3] = ["UTC", "America/Los_Angeles", "Australia/Sydney"];

    fn parity_connection(cases: &[ParityCase]) -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                r#"
                CREATE SCHEMA db;
                CREATE TABLE db.requests (
                    id BIGINT, portal_subdomain VARCHAR, agency_id BIGINT,
                    title VARCHAR, description VARCHAR, status VARCHAR,
                    created_at VARCHAR, updated_at VARCHAR, closed_at VARCHAR,
                    determination_at VARCHAR, determination_source VARCHAR
                );
                CREATE TABLE db.agencies (id BIGINT, portal_subdomain VARCHAR, name VARCHAR);
                CREATE TABLE db.documents (
                    id BIGINT, request_id BIGINT, portal_subdomain VARCHAR, visibility VARCHAR
                );
                CREATE TABLE db.statutory_extensions (
                    request_id BIGINT, portal_subdomain VARCHAR, notice_at VARCHAR,
                    expected_determination_at VARCHAR, reason VARCHAR, notice_source VARCHAR
                );
                INSERT INTO db.agencies VALUES (1, 'city', 'Parity');
                "#,
            )
            .unwrap();
        for (index, case) in cases.iter().enumerate() {
            let request_id = index as i64 + 1;
            let closed = case
                .closed_at
                .map(|value| format!("'{value}'"))
                .unwrap_or_else(|| "NULL".to_string());
            connection
                .execute_batch(&format!(
                    "INSERT INTO db.requests VALUES ({request_id}, 'city', 1, '{}', '', '{}', '{}', '{}', {closed}, NULL, NULL);",
                    case.label, case.status, case.created_at, case.updated_at
                ))
                .unwrap();
            if let Some(extension) = &case.extension {
                connection
                    .execute_batch(&format!(
                        "INSERT INTO db.statutory_extensions VALUES ({request_id}, 'city', '{}', '{}', '{}', '{}');",
                        extension.notice_at,
                        extension.expected_determination_at,
                        extension.reason.as_str(),
                        extension.notice_source
                    ))
                    .unwrap();
            }
        }
        connection
    }

    /// The aggregate SQL and the per-request Rust evaluator must reach the same verdict.
    ///
    /// Both sides are driven from one table of cases, so this fails when either implementation
    /// changes alone. Hard-coding the expected count twice would only document today's answer.
    #[test]
    fn statutory_verdicts_match_the_per_request_evaluator() {
        let cases = parity_cases();
        let observation_at = "2026-06-01T00:00:00Z";
        let rust_verdicts = cases
            .iter()
            .enumerate()
            .map(|(index, case)| {
                let analysis = evaluate_statutory_compliance_at(
                    index as u64 + 1,
                    case.created_at,
                    Some(case.updated_at),
                    case.closed_at,
                    case.extension.as_ref(),
                    observation_at,
                )
                .unwrap_or_else(|error| {
                    panic!("case '{}' failed to evaluate: {error}", case.label)
                });
                // Every case records activity after receipt, so neither path falls back to "now"
                // and the comparison stays deterministic.
                assert_eq!(
                    analysis.determination_basis,
                    DeterminationBasis::EarliestRecordedActivityProxy,
                    "case '{}' must measure to recorded activity",
                    case.label
                );
                (case.label, analysis.is_violation)
            })
            .collect::<Vec<_>>();

        let rust_violation_count = rust_verdicts
            .iter()
            .filter(|(_, is_violation)| *is_violation)
            .count();
        // A fixture where every case agrees regardless of the day-count rule would pin nothing.
        assert!(
            rust_verdicts.iter().any(|(label, is_violation)| *label
                == "near-midnight receipt in a western offset"
                && *is_violation),
            "the offset-sensitive case must be a violation or the fixture does not discriminate"
        );
        assert!(rust_violation_count > 0 && rust_violation_count < rust_verdicts.len());

        let mut applied_time_zones = Vec::new();
        for time_zone in PARITY_SESSION_TIME_ZONES {
            let connection = parity_connection(&cases);
            // A build without the ICU extension cannot change session time zone at all. Record
            // which zones were exercised rather than reporting untested zones as passing.
            if connection
                .execute_batch(&format!("SET TimeZone = '{time_zone}';"))
                .is_ok()
            {
                applied_time_zones.push(time_zone);
            }
            let profiles = query_agency_performance_profiles(
                &connection,
                90.0,
                AnalyticsSource::ForeignSqlite.table_prefix(),
            )
            .unwrap();
            assert_eq!(profiles.len(), 1);
            assert_eq!(
                profiles[0].statutory_violation_count, rust_violation_count,
                "under session time zone '{time_zone}' the aggregate disagreed with the \
                 per-request evaluator; per-request verdicts were {rust_verdicts:?}"
            );
            assert_eq!(profiles[0].total_requests_processed, cases.len());
        }
        assert!(
            applied_time_zones.len() >= 2,
            "the day-count rule must be proven independent of the session time zone, but only \
             {applied_time_zones:?} could be applied"
        );
    }

    #[test]
    fn sourced_determination_event_takes_precedence_in_vectorized_analysis() {
        let connection = connection();
        connection
            .execute(
                "UPDATE db.requests
                 SET determination_at = '2024-01-05T00:00:00Z',
                     determination_source = 'Agency determination notice'
                 WHERE id = 10",
                [],
            )
            .expect("the sourced event fixture must update");
        let profiles = query_agency_performance_profiles(
            &connection,
            365.0,
            AnalyticsSource::ForeignSqlite.table_prefix(),
        )
        .expect("the aggregate must evaluate the sourced event");
        let slow = profiles
            .iter()
            .find(|profile| profile.agency_id == 2)
            .expect("the slow agency must be present");
        assert_eq!(slow.statutory_violation_count, 0);
    }

    /// Pins the separation `convert_raw_profile` draws: an agency that answered every request
    /// inside the statutory deadline but far more slowly than its peers is a statistical outlier
    /// and nothing more. Reporting that as a statutory breach states a legal conclusion the
    /// evidence does not support, which is what this labelling used to do.
    #[test]
    fn latency_outlier_without_a_violation_is_not_labeled_statutory() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                r#"
                CREATE SCHEMA db;
                CREATE TABLE db.requests (
                    id BIGINT, portal_subdomain VARCHAR, agency_id BIGINT,
                    title VARCHAR, description VARCHAR, status VARCHAR,
                    created_at VARCHAR, updated_at VARCHAR, closed_at VARCHAR,
                    determination_at VARCHAR, determination_source VARCHAR
                );
                CREATE TABLE db.agencies (id BIGINT, portal_subdomain VARCHAR, name VARCHAR);
                CREATE TABLE db.documents (
                    id BIGINT, request_id BIGINT, portal_subdomain VARCHAR, visibility VARCHAR
                );
                CREATE TABLE db.statutory_extensions (
                    request_id BIGINT, portal_subdomain VARCHAR, notice_at VARCHAR,
                    expected_determination_at VARCHAR, reason VARCHAR, notice_source VARCHAR
                );
                INSERT INTO db.agencies VALUES (1, 'city', 'Peers'), (2, 'city', 'Slow but lawful');
                -- Peer agency: determination and closure both well inside the deadline.
                INSERT INTO db.requests
                SELECT i, 'city', 1, 'Peer', '', 'closed',
                    '2024-01-01T00:00:00Z', '2024-01-02T00:00:00Z', '2024-01-02T00:00:00Z', NULL, NULL
                FROM range(1, 10) rows(i);
                -- Determined on day four, so no statutory violation, but the file stayed open for
                -- a hundred days, which makes the agency a latency outlier.
                INSERT INTO db.requests VALUES
                    (10, 'city', 2, 'Slow but lawful', '', 'closed',
                     '2024-01-01T00:00:00Z', '2024-01-05T00:00:00Z', '2024-04-10T00:00:00Z', NULL, NULL);
                "#,
            )
            .unwrap();
        let profiles = query_agency_performance_profiles(
            &connection,
            365.0,
            AnalyticsSource::ForeignSqlite.table_prefix(),
        )
        .unwrap();
        let outlier = profiles
            .iter()
            .find(|profile| profile.agency_id == 2)
            .expect("the outlier agency must be profiled");
        assert_eq!(outlier.statutory_violation_count, 0);
        assert!(
            outlier
                .performance_z_score
                .expect("the outlier agency has a measurable closure sample")
                >= STATUTORY_DEADLINE_Z_SCORE
        );
        assert_eq!(
            outlier.compliance_indicator,
            VerificationState::LatencyOutlier,
            "a z-score alone is a statistical outlier, not a statutory violation"
        );
    }

    #[test]
    fn token_matcher_respects_stems_and_boundaries() {
        let tokens = normalized_tokens("Sweeping an encampment, not sweepstakes; evicted.");
        assert_eq!(count_token_pattern(&tokens, &["sweep".to_string()]), 1);
        assert_eq!(count_token_pattern(&tokens, &["eviction".to_string()]), 1);
    }
}
