//! Opening the core database and the serialized writer every repository call goes through.

use super::*;

impl DuckDbRepository {
    /// Establishes a persistent DuckDB connection and configures conservative local analytics
    /// settings. All mutations are serialized through this connection because DuckDB's durable
    /// single-process writer model is the reliability boundary for the desktop application.
    pub fn new<P: AsRef<Path>>(db_path: P) -> Result<Self, AppError> {
        let supplied_path = db_path.as_ref();
        let database_path = if supplied_path == Path::new(":memory:") || supplied_path.is_absolute()
        {
            supplied_path.to_path_buf()
        } else {
            std::env::current_dir()
                .map_err(|error| {
                    AppError::Storage(format!("Failed to resolve working directory: {error}"))
                })?
                .join(supplied_path)
        };
        let conn = if database_path == Path::new(":memory:") {
            crate::storage::open_duckdb(None)
        } else {
            crate::storage::open_duckdb(Some(&database_path))
        }
        .map_err(|error| {
            AppError::Internal(format!(
                "Failed to open DuckDB database '{}': {error}",
                database_path.display()
            ))
        })?;
        // Every timestamp this repository writes is formatted with a literal `Z`. That is true
        // because `open_duckdb` never loads the ICU extension: without it a `TIMESTAMPTZ` has no
        // session zone and `strftime` renders it in UTC on every machine.
        conn.execute_batch(
            "SET preserve_insertion_order = false;
             SET threads = 1;
             SET enable_progress_bar = false;",
        )
        .map_err(|error| AppError::Internal(format!("Failed to configure DuckDB: {error}")))?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            database_path: Arc::new(database_path),
            search_index_pending: Arc::new(AtomicU64::new(0)),
            search_index_stalled: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Flushes DuckDB's write-ahead log into the main database file. Called once at startup,
    /// after every other `initialize_*` call, so a session that made heavy writes (casework
    /// revisions, document ingestion, purges) does not leave the `.wal` file to grow unbounded
    /// across the next launch — `legacy_import.rs`'s one-off post-copy `CHECKPOINT` is the proven
    /// precedent this mirrors. Deliberately does *not* run `VACUUM`: this DuckDB version's actual
    /// space-reclamation behavior for `VACUUM` was not verified against current documentation, and
    /// CLAUDE.md requires verifying unfamiliar API behavior rather than assuming it, so an
    /// unconfirmed call was left out rather than shipped as a guess.
    pub async fn checkpoint(&self) -> Result<(), AppError> {
        self.query_connection("Checkpoint the database file", |conn| {
            conn.execute_batch("CHECKPOINT;").map_err(|error| {
                AppError::Storage(format!("Failed to checkpoint the database: {error}"))
            })
        })
        .await
    }

    /// Opens an additional connection to the *same live* DuckDB instance.
    ///
    /// Analytics engines must obtain their connection here. A connection that instead opens or
    /// `ATTACH`es the database file independently observes a snapshot frozen at attach time and
    /// never sees a later write, so every analytical result would be stale from process start.
    pub async fn analytics_connection(&self) -> Result<Connection, AppError> {
        let connection = self.conn.lock().await;
        connection.try_clone().map_err(|error| {
            AppError::Internal(format!("Failed to open an analytics connection: {error}"))
        })
    }

    pub(crate) async fn query_connection<T, F>(
        &self,
        operation: &'static str,
        query: F,
    ) -> Result<T, AppError>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T, AppError> + Send + 'static,
    {
        let connection = Arc::clone(&self.conn);
        spawn_with(operation, AppError::Internal, move || {
            let mut connection = connection.blocking_lock();
            query(&mut connection)
        })
        .await?
    }

    pub fn database_path(&self) -> &Path {
        self.database_path.as_path()
    }
}

pub(super) fn execute_transaction<T, F>(
    connection: &mut Connection,
    operation_name: &str,
    operation: F,
) -> Result<T, AppError>
where
    F: FnOnce(&Transaction<'_>) -> Result<T, AppError>,
{
    let transaction = connection.transaction().map_err(|error| {
        AppError::Storage(format!(
            "Failed to open {operation_name} transaction: {error}"
        ))
    })?;
    let result = operation(&transaction)?;
    transaction.commit().map_err(|error| {
        AppError::Storage(format!(
            "Failed to commit {operation_name} transaction: {error}"
        ))
    })?;
    Ok(result)
}
