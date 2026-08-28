//! Placeholder for explicitly opt-in constrained reconstruction experiments.

use crate::error::AppError;
use crate::parsers::forensic_evidence::EvidenceConfidence;

#[derive(Debug, Clone)]
pub struct CandidateText {
    pub value: String,
    pub width_error: Option<f64>,
    pub context_score: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct CandidateRanking {
    pub candidates: Vec<CandidateText>,
    pub confidence: EvidenceConfidence,
    pub abstained: bool,
    pub explanation: String,
}

pub fn rank_supplied_candidates(
    _candidates: Vec<CandidateText>,
    _allow_hypotheses: bool,
) -> Result<CandidateRanking, AppError> {
    // TODO(hypothesis): Add font shaping, geometry tolerances, grammar, and closed-set entity
    // constraints only after deterministic evidence collection is complete.
    // NOTE(hypothesis): Never generate or enumerate sensitive identifiers, and never report a
    // candidate as recovered text. Require abstention when scores are close or inputs are weak.
    Err(AppError::DataParsing(
        "HYPOTHESIS_RANKER_NOT_IMPLEMENTED".to_string(),
    ))
}

// FEATURE(hypothesis): Coordinate this narrow candidate API with layout_side_channel.rs,
// pixelation.rs, and neural_inference.rs only through typed, confidence-labelled findings.
// NOTE(hypothesis): Research accuracy on historical inscriptions must not be generalized to
// modern legal, medical, financial, or personally identifying documents without held-out tests.
