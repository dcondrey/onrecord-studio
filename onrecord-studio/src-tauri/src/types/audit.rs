//! Schema definitions mapping audit metrics and historical timeline modifications.

use serde::{Deserialize, Serialize};

/// What an audit or profile concluded about a request or an agency.
///
/// `scripts/generate-contracts.mjs` derives the TypeScript counterpart from this enum, and
/// `npm run check:ipc` fails when the checked-in generated contract is stale.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum VerificationState {
    Unaudited,
    Compliant,
    PolicyViolationDetected,
    /// A legal conclusion: the statute's calendar-day rule was exceeded on at least one request.
    /// Only a counted statutory violation may produce this.
    StatutoryDeadlineExceeded,
    /// A statistical conclusion: this agency's closure times sit far enough from the population
    /// mean to be an outlier. No statute speaks to that, so it is never a violation on its own.
    /// Computed by `analysis::anomaly` and never written to `case_audits`.
    LatencyOutlier,
    DataAnomalyFlagged,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TimelineAnomalyMarker {
    SuspiciouslyFastClosure,
    UpdateBeforeCreation,
    CloseBeforeCreation,
    ObservationBeforeCreation,
    StatutoryLatencyViolation,
    StatutoryExtensionDefective,
}

impl TimelineAnomalyMarker {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SuspiciouslyFastClosure => "SUSPICIOUSLY_FAST_CLOSURE",
            Self::UpdateBeforeCreation => "UPDATE_BEFORE_CREATION",
            Self::CloseBeforeCreation => "CLOSE_BEFORE_CREATION",
            Self::ObservationBeforeCreation => "OBSERVATION_BEFORE_CREATION",
            Self::StatutoryLatencyViolation => "STATUTORY_LATENCY_VIOLATION",
            Self::StatutoryExtensionDefective => "STATUTORY_EXTENSION_DEFECTIVE",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StateTransitionLog {
    pub request_id: u64,
    pub transition_timestamp: String,
    pub previous_state: String,
    pub next_state: String,
    pub time_spent_in_previous_state_seconds: i64,
    #[serde(default)]
    pub anomaly_markers: Vec<TimelineAnomalyMarker>,
}
