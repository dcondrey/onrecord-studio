//! Source standing the shareable bundle needs beyond a document's visibility.

use crate::error::AppError;
use crate::storage::sqlite::DuckDbRepository;
use duckdb::params;
use std::collections::BTreeSet;

impl DuckDbRepository {
    /// The subset of `portal_keys` whose portal is removed or unknown. An unknown portal cannot be
    /// confirmed public, so it is treated as unavailable rather than assumed fine.
    pub async fn unavailable_portal_keys(
        &self,
        portal_keys: BTreeSet<String>,
    ) -> Result<BTreeSet<String>, AppError> {
        self.query_connection("Check bundle source portals", move |conn| {
            let mut statement = conn
                .prepare("SELECT lifecycle_state FROM portals WHERE subdomain = ?1")
                .map_err(|error| AppError::Storage(error.to_string()))?;
            let mut unavailable = BTreeSet::new();
            for key in portal_keys {
                let state = statement.query_row(params![key], |row| row.get::<_, String>(0));
                match state {
                    Ok(state) if state != "REMOVED" => {}
                    Ok(_) | Err(duckdb::Error::QueryReturnedNoRows) => {
                        unavailable.insert(key);
                    }
                    Err(error) => return Err(AppError::Storage(error.to_string())),
                }
            }
            Ok(unavailable)
        })
        .await
    }
}
