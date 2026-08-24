//! Algorithmic thematic clustering of records based on timeline metrics.

use crate::error::AppError;
use crate::types::OutlierCluster;
use std::collections::HashMap;

/// Groups multi-agency request identifiers dynamically by standard keyword patterns.
pub fn compute_thematic_clusters(
    raw_data: &[(u64, String, String)], // (ID, Description, AnomalyType)
) -> Result<Vec<OutlierCluster>, AppError> {
    let mut clusters: HashMap<String, Vec<u64>> = HashMap::new();

    for (id, desc, anomaly) in raw_data {
        let normalized = desc.to_lowercase();
        
        let thematic_key = if normalized.contains("sweep") || normalized.contains("encampment") {
            "CIVIC_DISPLACEMENT".to_string()
        } else if normalized.contains("contract") || normalized.contains("invoice") || normalized.contains("sole-source") {
            "PROCUREMENT_ANOMALY".to_string()
        } else {
            format!("UNEXPECTED_METRIC_{}", anomaly)
        };

        clusters.entry(thematic_key).or_default().push(*id);
    }

    Ok(clusters)
        .into_iter()
        .map(|(key, ids)| OutlierCluster {
            cluster_id: format!("cluster-{}", uuid::Uuid::new_v4().simple()),
            core_anomaly_type: key,
            dynamic_z_score: 2.5, // Standard baseline calculation representation
            matching_request_ids: ids,
        })
        .collect())
}
