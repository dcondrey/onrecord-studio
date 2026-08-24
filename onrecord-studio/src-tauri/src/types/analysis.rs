//! Structures for statistical analysis execution and backend analytics profiles.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisConfig {
    pub latency_threshold_days: f64,
    pub min_ocr_character_density: usize,
    pub suspicious_closure_window_minutes: i64,
    pub target_policy_keywords: Vec<String>,
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
        }
    }
}

#[derive(Debug, Clone, Serialize, Boxed)]
#[serde(rename_all = "camelCase")]
pub struct AgencyPerformanceProfile {
    pub agency_id: u64,
    pub agency_name: String,
    pub total_requests_processed: usize,
    pub total_closed: usize,
    pub average_closure_days: f64,
    pub anomaly_count: usize,
    pub total_documents_released: usize,
    pub keyword_frequency_map: HashMap<String, usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OutlierCluster {
    pub cluster_id: String,
    pub core_anomaly_type: String,
    pub dynamic_z_score: f64,
    pub matching_request_ids: Vec<u64>,
}
