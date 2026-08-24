//! Schema definitions mapping audit metrics and historical timeline modifications.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaseAuditRecord {
    pub audit_id: String,
    pub target_request_id: u64,
    pub target_subdomain: String,
    pub auditor_notes: Option<String>,
    pub verified_at: String,
    pub verification_status: VerificationState,
    pub document_integrity_flags: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum VerificationState {
    Unaudited,
    Compliant,
    PolicyViolationDetected,
    StatutoryDeadlineExceeded,
    DataAnomalyFlagged,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateTransitionLog {
    pub request_id: u64,
    pub transition_timestamp: String,
    pub previous_state: String,
    pub next_state: String,
    pub time_spent_in_previous_state_seconds: i64,
}
