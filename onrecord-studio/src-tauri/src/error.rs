//! Central error categorization system tracking data transformations securely.

use thiserror::Error;
use serde::Serialize;

#[derive(Debug, Error, Serialize)]
#[serde(tag = "errorType", content = "message")]
pub enum AppError {
    #[error("Network disruption encountered: {0}")]
    Network(String),
    #[error("File data transformation parser failure: {0}")]
    DataParsing(String),
    #[error("Internal execution boundary breakdown: {0}")]
    Internal(String),
}

// Convert native database errors into standard application wrappers
impl From<rusqlite::Error> for AppError {
    fn from(err: rusqlite::Error) -> Self {
        AppError::Internal(format!("SQLite persistence failure: {}", err))
    }
}
