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
    mut candidates: Vec<CandidateText>,
    allow_hypotheses: bool,
) -> Result<CandidateRanking, AppError> {
    // TODO(hypothesis): Add font shaping, geometry tolerances, grammar, and closed-set entity
    // constraints only after deterministic evidence collection is complete.
    // NOTE(hypothesis): Never generate or enumerate sensitive identifiers, and never report a
    // candidate as recovered text. Require abstention when scores are close or inputs are weak.
    if candidates.len() > 1_000 {
        return Err(AppError::DataParsing(
            "HYPOTHESIS_CANDIDATE_LIMIT_EXCEEDED: at most 1000 supplied candidates are accepted"
                .to_string(),
        ));
    }
    for candidate in &candidates {
        if candidate.value.is_empty() || candidate.value.chars().count() > 4_096 {
            return Err(AppError::DataParsing(
                "HYPOTHESIS_CANDIDATE_INVALID: values must contain 1 to 4096 characters"
                    .to_string(),
            ));
        }
        if candidate
            .width_error
            .is_some_and(|value| !value.is_finite() || value < 0.0)
            || candidate
                .context_score
                .is_some_and(|value| !value.is_finite() || !(0.0..=1.0).contains(&value))
        {
            return Err(AppError::DataParsing(
                "HYPOTHESIS_SCORE_INVALID: width error must be non-negative and context score must be between 0 and 1"
                    .to_string(),
            ));
        }
    }
    if !allow_hypotheses {
        return Ok(CandidateRanking {
            candidates: Vec::new(),
            confidence: EvidenceConfidence::Limited,
            abstained: true,
            explanation:
                "Hypothesis ranking was not authorized; no supplied candidate was evaluated."
                    .to_string(),
        });
    }
    candidates.sort_by(|left, right| candidate_score(right).total_cmp(&candidate_score(left)));
    let Some(top) = candidates.first() else {
        return Ok(CandidateRanking {
            candidates,
            confidence: EvidenceConfidence::Limited,
            abstained: true,
            explanation: "No candidates were supplied.".to_string(),
        });
    };
    if top.width_error.is_none() && top.context_score.is_none() {
        return Ok(CandidateRanking {
            candidates,
            confidence: EvidenceConfidence::Speculative,
            abstained: true,
            explanation:
                "Candidates contain no deterministic geometry or caller-supplied context scores."
                    .to_string(),
        });
    }
    let score_gap = candidates
        .get(1)
        .map(|runner_up| candidate_score(top) - candidate_score(runner_up))
        .unwrap_or(1.0);
    let (confidence, abstained, explanation) = if score_gap >= 0.25
        && top.width_error.is_some()
        && top.context_score.is_some()
    {
        (EvidenceConfidence::Strong, false, "The leading supplied candidate is separated on both geometry and context scores; it remains a hypothesis, not recovered text.")
    } else if score_gap >= 0.1 {
        (EvidenceConfidence::Limited, false, "The leading supplied candidate has limited score separation and must not be treated as recovered text.")
    } else {
        (
            EvidenceConfidence::Speculative,
            true,
            "The leading candidates are too close to rank responsibly; the analyzer abstained.",
        )
    };
    Ok(CandidateRanking {
        candidates,
        confidence,
        abstained,
        explanation: explanation.to_string(),
    })
}

fn candidate_score(candidate: &CandidateText) -> f64 {
    candidate.context_score.unwrap_or(0.0)
        - candidate.width_error.unwrap_or(1_000.0).min(1_000.0) / 1_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abstains_when_candidates_are_tied() {
        let result = rank_supplied_candidates(
            vec![
                CandidateText {
                    value: "alpha".into(),
                    width_error: Some(1.0),
                    context_score: Some(0.5),
                },
                CandidateText {
                    value: "beta".into(),
                    width_error: Some(1.0),
                    context_score: Some(0.5),
                },
            ],
            true,
        )
        .expect("ranking");
        assert!(result.abstained);
        assert_eq!(result.confidence, EvidenceConfidence::Speculative);
    }

    #[test]
    fn refuses_ranking_without_authorization() {
        let result = rank_supplied_candidates(
            vec![CandidateText {
                value: "alpha".into(),
                width_error: Some(0.0),
                context_score: Some(1.0),
            }],
            false,
        )
        .expect("ranking");
        assert!(result.abstained);
        assert!(result.candidates.is_empty());
    }
}

// FEATURE(hypothesis): Coordinate this narrow candidate API with layout_side_channel.rs,
// pixelation.rs, and neural_inference.rs only through typed, confidence-labelled findings.
// NOTE(hypothesis): Research accuracy on historical inscriptions must not be generalized to
// modern legal, medical, financial, or personally identifying documents without held-out tests.
