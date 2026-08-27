//! Embedded OLAP, lifecycle anomaly, and local text-analysis services.

pub mod anomaly;
pub mod cluster;
pub mod statute;
pub mod timeline;

pub use anomaly::{AnalyticsEngine, STATUTORY_DEADLINE_Z_SCORE};
pub use cluster::{
    analyze_redaction_density, analyze_redaction_layers, compute_neighborhood_keyword_densities,
    compute_thematic_clusters, extract_enforcement_keyword_counts, ClusterAlertState,
    NeighborhoodKeywordDensity, NeighborhoodTextRecord, RedactionDensityMetric,
    RedactionLayerAnalysis, RedactionMatchEvidence, RedactionPatternKind, TextLayerInput,
    TextLayerRedactionMetric, HEAVY_REDACTION_DENSITY_THRESHOLD,
};
pub use statute::{DeterminationBasis, CPRA_MAXIMUM_EXTENDED_DETERMINATION_DAYS};
pub use timeline::{
    calculate_operational_latency, calculate_operational_latency_at,
    calculate_operational_latency_for_status, detect_fast_drop, detect_fast_drop_for_status,
    evaluate_statutory_compliance, evaluate_statutory_compliance_at, StatutoryComplianceAnalysis,
    StatutoryExtensionStatus, CPRA_INITIAL_DETERMINATION_DAYS, CPRA_MAXIMUM_EXTENSION_DAYS,
    CPRA_STATUTE_CITATION, SUSPICIOUS_CLOSURE_THRESHOLD_SECONDS,
};
