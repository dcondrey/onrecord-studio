//! Explicitly opt-in, closed-set reconstruction ranking.

use crate::error::AppError;
use crate::parsers::forensic_evidence::EvidenceConfidence;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidateText {
    pub value: String,
    /// Absolute error from a caller-supplied, font-shaped width measurement.
    pub width_error: Option<f64>,
    pub context_score: Option<f64>,
    pub grammar_score: Option<f64>,
    pub closed_set_member: bool,
    pub shaping_profile: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct CandidateRankingPolicy {
    pub maximum_width_error: Option<f64>,
    pub minimum_grammar_score: Option<f64>,
    pub require_closed_set: bool,
}

impl Default for CandidateRankingPolicy {
    fn default() -> Self {
        Self {
            maximum_width_error: None,
            minimum_grammar_score: None,
            require_closed_set: true,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidateRanking {
    pub candidates: Vec<CandidateText>,
    pub confidence: EvidenceConfidence,
    pub abstained: bool,
    pub rejected_candidate_count: usize,
    pub leading_score: Option<f64>,
    pub score_gap: Option<f64>,
    pub explanation: String,
}

pub fn rank_supplied_candidates(
    mut candidates: Vec<CandidateText>,
    allow_hypotheses: bool,
    policy: CandidateRankingPolicy,
) -> Result<CandidateRanking, AppError> {
    // Candidates are never generated here: the caller supplies a closed set, optional exact
    // font-shaped geometry error and grammar/context scores. The ranker validates those
    // constraints and abstains on weak or close inputs.
    if policy
        .maximum_width_error
        .is_some_and(|value| !value.is_finite() || value < 0.0)
        || policy
            .minimum_grammar_score
            .is_some_and(|value| !value.is_finite() || !(0.0..=1.0).contains(&value))
    {
        return Err(AppError::DataParsing(
            "HYPOTHESIS_POLICY_INVALID: geometry and grammar thresholds must be finite and in range"
                .to_string(),
        ));
    }
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
            || candidate
                .grammar_score
                .is_some_and(|value| !value.is_finite() || !(0.0..=1.0).contains(&value))
            || candidate.shaping_profile.as_ref().is_some_and(|value| {
                value.trim().is_empty() || value.chars().count() > 256 || value.contains('\0')
            })
        {
            return Err(AppError::DataParsing(
                "HYPOTHESIS_SCORE_INVALID: width error must be non-negative; context and grammar scores must be between 0 and 1; shaping profiles must be bounded"
                    .to_string(),
            ));
        }
    }
    if !allow_hypotheses {
        let rejected_candidate_count = candidates.len();
        return Ok(CandidateRanking {
            candidates: Vec::new(),
            confidence: EvidenceConfidence::Limited,
            abstained: true,
            rejected_candidate_count,
            leading_score: None,
            score_gap: None,
            explanation:
                "Hypothesis ranking was not authorized; no supplied candidate was evaluated."
                    .to_string(),
        });
    }
    let supplied_count = candidates.len();
    candidates.retain(|candidate| {
        (!policy.require_closed_set || candidate.closed_set_member)
            && policy
                .maximum_width_error
                .is_none_or(|limit| candidate.width_error.is_some_and(|value| value <= limit))
            && policy.minimum_grammar_score.is_none_or(|minimum| {
                candidate
                    .grammar_score
                    .is_some_and(|value| value >= minimum)
            })
    });
    let rejected_candidate_count = supplied_count - candidates.len();
    candidates.sort_by(|left, right| candidate_score(right).total_cmp(&candidate_score(left)));
    let Some(top) = candidates.first() else {
        return Ok(CandidateRanking {
            candidates,
            confidence: EvidenceConfidence::Limited,
            abstained: true,
            rejected_candidate_count,
            leading_score: None,
            score_gap: None,
            explanation: if supplied_count == 0 {
                "No candidates were supplied.".to_string()
            } else {
                "Every supplied candidate failed the closed-set, geometry, or grammar policy; the analyzer abstained.".to_string()
            },
        });
    };
    if top.width_error.is_none() && top.context_score.is_none() {
        return Ok(CandidateRanking {
            candidates,
            confidence: EvidenceConfidence::Speculative,
            abstained: true,
            rejected_candidate_count,
            leading_score: None,
            score_gap: None,
            explanation:
                "Candidates contain no deterministic geometry or caller-supplied context scores."
                    .to_string(),
        });
    }
    let score_gap = candidates
        .get(1)
        .map(|runner_up| candidate_score(top) - candidate_score(runner_up))
        .unwrap_or(1.0);
    let leading_score = candidate_score(top);
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
        rejected_candidate_count,
        leading_score: Some(leading_score),
        score_gap: Some(score_gap),
        explanation: explanation.to_string(),
    })
}

fn candidate_score(candidate: &CandidateText) -> f64 {
    candidate.context_score.unwrap_or(0.0) * 0.55
        + candidate.grammar_score.unwrap_or(0.0) * 0.25
        + (1.0 - candidate.width_error.unwrap_or(1_000.0).min(1_000.0) / 1_000.0) * 0.20
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
                    grammar_score: Some(0.8),
                    closed_set_member: true,
                    shaping_profile: Some("fixture-font-v1".into()),
                },
                CandidateText {
                    value: "beta".into(),
                    width_error: Some(1.0),
                    context_score: Some(0.5),
                    grammar_score: Some(0.8),
                    closed_set_member: true,
                    shaping_profile: Some("fixture-font-v1".into()),
                },
            ],
            true,
            CandidateRankingPolicy::default(),
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
                grammar_score: Some(1.0),
                closed_set_member: true,
                shaping_profile: Some("fixture-font-v1".into()),
            }],
            false,
            CandidateRankingPolicy::default(),
        )
        .expect("ranking");
        assert!(result.abstained);
        assert!(result.candidates.is_empty());
    }
}
