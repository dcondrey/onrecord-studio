//! Sequential lifecycle calculation layers identifying statutory processing delays.

use crate::error::AppError;
use crate::types::StateTransitionLog;

/// Calculates real-time delays across historical state modifications.
pub fn calculate_operational_latency(
    request_id: u64,
    created_at: &str,
    closed_at: Option<&str>,
) -> Result<StateTransitionLog, AppError> {
    let created = chrono::DateTime::parse_from_rfc3339(created_at)
        .map_err(|e| AppError::DataParsing(format!("Invalid timeline created timestamp: {}", e)))?;

    let closed = match closed_at {
        Some(ts) => chrono::DateTime::parse_from_rfc3339(ts)
            .map_err(|e| AppError::DataParsing(format!("Invalid timeline closed timestamp: {}", e)))?,
        None => chrono::Utc::now().into(),
    };

    let delta_seconds = (closed - created).num_seconds();

    Ok(StateTransitionLog {
        request_id,
        transition_timestamp: chrono::Utc::now().to_rfc3339(),
        previous_state: "OPEN".to_string(),
        next_state: if closed_at.is_some() { "CLOSED".to_string() } else { "ACTIVE_STALL".to_string() },
        time_spent_in_previous_state_seconds: delta_seconds,
    })
}
