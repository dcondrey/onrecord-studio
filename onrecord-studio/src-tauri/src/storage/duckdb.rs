//! Structural column-oriented analytic tracking context.

use duckdb::Connection;
use std::sync::Arc;
use tokio::sync::Mutex;
use crate::error::AppError;

#[derive(Clone)]
pub struct DuckDbWorkspace {
    pub inner: Arc<Mutex<Connection>>,
}

impl DuckDbWorkspace {
    /// Bootstraps an embedded, highly accelerated in-memory calculation node.
    pub fn open_workspace() -> Result<Self, AppError> {
        let conn = Connection::open_in_memory()
            .map_err(|e| AppError::Internal(format!("Failed to initialize DuckDB handle: {}", e)))?;
        Ok(Self { inner: Arc::new(Mutex::new(conn)) })
    }
}
