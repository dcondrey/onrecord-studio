//! Structures for statistical analysis execution and backend analytics profiles.

use crate::types::VerificationState;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AnalysisConfig {
    pub latency_threshold_days: f64,
    pub min_ocr_character_density: usize,
    pub suspicious_closure_window_minutes: i64,
    pub target_policy_keywords: Vec<String>,
    #[serde(default)]
    pub created_from: Option<String>,
    #[serde(default)]
    pub created_to: Option<String>,
}

impl Default for AnalysisConfig {
    fn default() -> Self {
        Self {
            latency_threshold_days: 90.0,
            min_ocr_character_density: 15,
            suspicious_closure_window_minutes: 5,
            target_policy_keywords: vec![
                "sweep".to_string(),
                "encampment".to_string(),
                "eviction".to_string(),
                "displace".to_string(),
                "clean up".to_string(),
                "property storage".to_string(),
            ],
            created_from: None,
            created_to: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgencyPerformanceProfile {
    pub agency_id: u64,
    pub portal_subdomain: String,
    pub agency_name: String,
    pub total_requests_processed: usize,
    pub total_closed: usize,
    pub valid_latency_sample_size: usize,
    pub excluded_invalid_lifetime_count: usize,
    /// `None` when the agency has closed nothing measurable. Never 0.0 for that case: 0.0 days
    /// is the fastest possible answer, and an agency that has never closed a request is not fast.
    pub average_closure_days: Option<f64>,
    pub global_mean_closure_days: Option<f64>,
    pub global_stddev_closure_days: Option<f64>,
    /// `None` when there is no closure sample to place against the population, or the population
    /// has no spread. Never 0.0 for that case: 0.0 means exactly at the mean.
    pub performance_z_score: Option<f64>,
    pub compliance_indicator: VerificationState,
    pub anomaly_count: usize,
    /// Requests measured against a statutory clock. A request whose source names no
    /// jurisdiction is not, and counts toward neither this nor the violations.
    pub statutory_evaluated_count: usize,
    pub statutory_violation_count: usize,
    /// Violations over evaluated requests; `None` when none was evaluated, never 0.0 for that.
    pub statutory_violation_rate: Option<f64>,
    pub suspicious_fast_closure_count: usize,
    pub total_documents_released: usize,
    pub keyword_frequency_map: HashMap<String, usize>,
}
