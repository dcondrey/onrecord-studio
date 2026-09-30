//! Embedded OLAP, lifecycle anomaly, and local text-analysis services.

pub mod anomaly;
pub mod cluster;
pub mod reconstruction;
pub(crate) mod sql;
pub mod statute;
pub mod timeline;

pub use anomaly::{AnalyticsEngine, STATUTORY_DEADLINE_Z_SCORE};
pub use cluster::{
    analyze_redaction_density, extract_enforcement_keyword_counts, ClusterAlertState,
    RedactionDensityMetric, RedactionMatchEvidence, RedactionPatternKind, TextLayerRedactionMetric,
    HEAVY_REDACTION_DENSITY_THRESHOLD,
};
pub use statute::{DeterminationBasis, CPRA_MAXIMUM_EXTENDED_DETERMINATION_DAYS};
pub use timeline::{
    calculate_operational_latency_for_status, detect_fast_drop_for_status,
    evaluate_statutory_compliance_at, evaluate_statutory_compliance_with_determination_at,
    StatutoryComplianceAnalysis, StatutoryExtensionStatus, CPRA_INITIAL_DETERMINATION_DAYS,
    CPRA_MAXIMUM_EXTENSION_DAYS, CPRA_STATUTE_CITATION, SUSPICIOUS_CLOSURE_THRESHOLD_SECONDS,
};

pub mod review;

pub mod financial;

pub mod facility_coverage;
pub mod budget_lines;

pub mod monitoring;

pub mod assertion_index;

pub mod claim_validation;
pub mod lead_dossier;
pub mod story_desk;

pub mod document_class;

pub mod practice;

pub mod commitments;

pub mod spending_cases;
pub mod spending_rules;

pub(crate) mod phrase_match;

pub mod intent;

pub mod typed_edges;

pub mod finding;
pub mod gaps;
pub mod topic_packs;

pub mod detectors;

pub mod non_c2pa_provenance;

pub mod redaction_inference;

pub mod artifacts;
