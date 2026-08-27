//! Placeholder for opt-in, provenance-aware text restoration hypotheses.

use crate::error::AppError;
use crate::parsers::forensic_evidence::{EvidenceBundle, ForensicAuditOptions};

pub fn generate_context_hypotheses(
    _context: &str,
    _geometry: Option<[f64; 4]>,
    _options: ForensicAuditOptions,
) -> Result<EvidenceBundle, AppError> {
    // FEATURE(hypothesis): Permit only locally configured models and explicitly supplied corpus
    // boundaries. Store model/version, top-k scores, calibration data, and abstention reasons.
    // TODO(hypothesis): Evaluate on held-out fixtures with character error rate, abstention rate,
    // calibration, and subgroup/privacy leakage metrics.
    // NOTE(hypothesis): Ithaca-style restoration is ranked suggestion, not proof of modern text.
    Err(AppError::DataParsing(
        "NEURAL_RESTORATION_NOT_IMPLEMENTED".to_string(),
    ))
}
