use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DataMaintenancePolicy {
    pub enabled: bool,
    pub auto_import: bool,
    pub auto_refresh: bool,
    pub refresh_hours: u32,
    /// ORIGINALS or REFERENCE_ONLY. Reference mode applies only after effective_at.
    pub retention_mode: String,
    pub retained_file_budget_bytes: u64,
}
impl Default for DataMaintenancePolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            auto_import: true,
            auto_refresh: true,
            refresh_hours: 24,
            retention_mode: "ORIGINALS".into(),
            retained_file_budget_bytes: 10 * 1024 * 1024 * 1024,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataMaintenanceEvent {
    pub action: String,
    pub detail: String,
    pub recorded_at: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataMaintenanceStatus {
    pub policy: DataMaintenancePolicy,
    pub effective_at: String,
    pub retained_file_bytes: u64,
    pub referenced_documents: u64,
    pub reclaimed_file_bytes: u64,
    pub events: Vec<DataMaintenanceEvent>,
}
