//! Bounded local, closed-set context hypotheses and held-out evaluation.
//!
//! This module does not download a model, contact a service, or invent candidates. Its versioned
//! local ranker scores only investigator-supplied candidates and emits hypothesis-class evidence.

use crate::error::AppError;
use crate::parsers::forensic_evidence::{
    finding_for_text, EvidenceBundle, EvidenceClass, EvidenceLocator, ForensicAuditOptions,
};
use crate::parsers::hypothesis::{
    rank_supplied_candidates, CandidateRanking, CandidateRankingPolicy, CandidateText,
};
use std::collections::{BTreeMap, HashSet};

const LOCAL_MODEL_ID: &str = "onrecord-closed-set-token-overlap";
const LOCAL_MODEL_VERSION: &str = "1";
const MAXIMUM_CONTEXT_CHARACTERS: usize = 32_768;
const MAXIMUM_TOP_K: usize = 20;
const MAXIMUM_EVALUATION_CASES: usize = 256;
const MAXIMUM_EVALUATION_TEXT_CHARACTERS: usize = 512;

#[derive(Debug, Clone)]
pub struct LocalHypothesisRequest {
    pub model_id: String,
    pub model_version: String,
    pub supplied_corpus_id: String,
    pub candidates: Vec<CandidateText>,
    pub top_k: usize,
    pub maximum_width_error: Option<f64>,
    pub minimum_grammar_score: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct HeldOutHypothesisCase {
    pub case_id: String,
    pub subgroup: String,
    pub context: String,
    pub geometry: Option<[f64; 4]>,
    pub candidates: Vec<CandidateText>,
    pub expected: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HypothesisEvaluation {
    pub model_id: String,
    pub model_version: String,
    pub case_count: usize,
    pub expected_case_count: usize,
    pub character_error_rate_basis_points: u16,
    pub abstention_rate_basis_points: u16,
    pub mean_calibration_error_basis_points: u16,
    pub privacy_leakage_count: usize,
    pub subgroup_metrics: Vec<HypothesisSubgroupMetrics>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HypothesisSubgroupMetrics {
    pub subgroup: String,
    pub case_count: usize,
    pub correct_count: usize,
    pub abstained_count: usize,
}

pub fn generate_context_hypotheses(
    context: &str,
    geometry: Option<[f64; 4]>,
    request: LocalHypothesisRequest,
    options: ForensicAuditOptions,
) -> Result<EvidenceBundle, AppError> {
    validate_request(context, &request)?;
    if !options.include_hypotheses {
        return Ok(EvidenceBundle {
            coverage_complete: true,
            warnings: vec!["HYPOTHESIS_ANALYSIS_NOT_AUTHORIZED".to_string()],
            ..EvidenceBundle::default()
        });
    }
    let ranking = rank_request(context, request.candidates.clone(), &request)?;
    let mut bundle = EvidenceBundle {
        coverage_complete: true,
        ..EvidenceBundle::default()
    };
    if ranking.abstained {
        bundle
            .warnings
            .push(format!("HYPOTHESIS_ABSTAINED:{}", ranking.explanation));
        return Ok(bundle);
    }
    for (rank, candidate) in ranking.candidates.iter().take(request.top_k).enumerate() {
        let value = format!(
            "candidate={};rank={};modelId={};modelVersion={};corpusId={};contextScore={:.6};grammarScore={:.6};widthError={};shapingProfile={}",
            candidate.value,
            rank + 1,
            request.model_id,
            request.model_version,
            request.supplied_corpus_id,
            candidate.context_score.unwrap_or(0.0),
            candidate.grammar_score.unwrap_or(0.0),
            candidate.width_error.map(|value| format!("{value:.6}")).unwrap_or_else(|| "unavailable".to_string()),
            candidate.shaping_profile.as_deref().unwrap_or("unavailable"),
        );
        bundle.findings.push(finding_for_text(
            EvidenceClass::Hypothesis,
            ranking.confidence,
            "LOCAL_CLOSED_SET_TEXT_HYPOTHESIS",
            EvidenceLocator {
                bounds: geometry,
                ..EvidenceLocator::default()
            },
            &value,
            format!(
                "This candidate came only from investigator-supplied corpus '{}'. The versioned local ranker used token overlap plus supplied grammar and font-shaped width measurements. It is a ranked suggestion, never recovered text. {}",
                request.supplied_corpus_id, ranking.explanation
            ),
            options,
        )?);
    }
    Ok(bundle)
}

pub fn evaluate_local_hypothesis_model(
    supplied_corpus_id: &str,
    cases: Vec<HeldOutHypothesisCase>,
    maximum_width_error: Option<f64>,
    minimum_grammar_score: Option<f64>,
) -> Result<HypothesisEvaluation, AppError> {
    if cases.is_empty() || cases.len() > MAXIMUM_EVALUATION_CASES {
        return Err(AppError::DataParsing(format!(
            "HYPOTHESIS_EVALUATION_CASE_LIMIT: expected 1 to {MAXIMUM_EVALUATION_CASES} cases"
        )));
    }
    validate_identifier("supplied corpus", supplied_corpus_id)?;
    let mut expected_case_count = 0_usize;
    let mut abstained_count = 0_usize;
    let mut character_errors = 0_usize;
    let mut expected_characters = 0_usize;
    let mut calibration_error = 0.0_f64;
    let mut privacy_leakage_count = 0_usize;
    let mut subgroup_metrics = BTreeMap::<String, (usize, usize, usize)>::new();
    for case in &cases {
        validate_identifier("evaluation case", &case.case_id)?;
        validate_identifier("evaluation subgroup", &case.subgroup)?;
        if case
            .expected
            .as_ref()
            .is_some_and(|value| value.chars().count() > MAXIMUM_EVALUATION_TEXT_CHARACTERS)
            || case.candidates.iter().any(|candidate| {
                candidate.value.chars().count() > MAXIMUM_EVALUATION_TEXT_CHARACTERS
            })
        {
            return Err(AppError::DataParsing(format!(
                "HYPOTHESIS_EVALUATION_TEXT_LIMIT:{}",
                case.case_id
            )));
        }
        if let Some(expected) = &case.expected {
            expected_case_count += 1;
            if !case.candidates.iter().any(|candidate| {
                candidate.value == expected.as_str() && candidate.closed_set_member
            }) {
                return Err(AppError::DataParsing(format!(
                    "HYPOTHESIS_EVALUATION_EXPECTED_OUTSIDE_CLOSED_SET:{}",
                    case.case_id
                )));
            }
        }
        let request = LocalHypothesisRequest {
            model_id: LOCAL_MODEL_ID.to_string(),
            model_version: LOCAL_MODEL_VERSION.to_string(),
            supplied_corpus_id: supplied_corpus_id.to_string(),
            candidates: Vec::new(),
            top_k: 1,
            maximum_width_error,
            minimum_grammar_score,
        };
        validate_request(&case.context, &request)?;
        let supplied = case
            .candidates
            .iter()
            .map(|candidate| candidate.value.as_str())
            .collect::<HashSet<_>>();
        let ranking = rank_request(&case.context, case.candidates.clone(), &request)?;
        let selected = (!ranking.abstained)
            .then(|| ranking.candidates.first())
            .flatten();
        let correct = selected
            .zip(case.expected.as_ref())
            .is_some_and(|(candidate, expected)| candidate.value == expected.as_str());
        if ranking.abstained {
            abstained_count += 1;
        }
        if selected.is_some_and(|candidate| !supplied.contains(candidate.value.as_str())) {
            privacy_leakage_count += 1;
        }
        if let Some(expected) = &case.expected {
            let predicted = selected
                .map(|candidate| candidate.value.as_str())
                .unwrap_or("");
            character_errors += levenshtein_distance(predicted, expected);
            expected_characters += expected.chars().count().max(1);
            let predicted_probability = ranking.leading_score.unwrap_or(0.0).clamp(0.0, 1.0);
            calibration_error += (predicted_probability - if correct { 1.0 } else { 0.0 }).abs();
        }
        let metrics = subgroup_metrics.entry(case.subgroup.clone()).or_default();
        metrics.0 += 1;
        metrics.1 += usize::from(correct);
        metrics.2 += usize::from(ranking.abstained);
    }
    let case_count = cases.len();
    Ok(HypothesisEvaluation {
        model_id: LOCAL_MODEL_ID.to_string(),
        model_version: LOCAL_MODEL_VERSION.to_string(),
        case_count,
        expected_case_count,
        character_error_rate_basis_points: basis_points(character_errors, expected_characters),
        abstention_rate_basis_points: basis_points(abstained_count, case_count),
        mean_calibration_error_basis_points: if expected_case_count == 0 {
            0
        } else {
            ((calibration_error / expected_case_count as f64) * 10_000.0)
                .round()
                .clamp(0.0, 10_000.0) as u16
        },
        privacy_leakage_count,
        subgroup_metrics: subgroup_metrics
            .into_iter()
            .map(|(subgroup, (case_count, correct_count, abstained_count))| {
                HypothesisSubgroupMetrics {
                    subgroup,
                    case_count,
                    correct_count,
                    abstained_count,
                }
            })
            .collect(),
    })
}

fn rank_request(
    context: &str,
    mut candidates: Vec<CandidateText>,
    request: &LocalHypothesisRequest,
) -> Result<CandidateRanking, AppError> {
    for candidate in &mut candidates {
        candidate.context_score = Some(token_overlap_score(context, &candidate.value));
        candidate.grammar_score = Some(
            candidate
                .grammar_score
                .unwrap_or_else(|| bounded_grammar_score(&candidate.value)),
        );
    }
    rank_supplied_candidates(
        candidates,
        true,
        CandidateRankingPolicy {
            maximum_width_error: request.maximum_width_error,
            minimum_grammar_score: request.minimum_grammar_score,
            require_closed_set: true,
        },
    )
}

fn validate_request(context: &str, request: &LocalHypothesisRequest) -> Result<(), AppError> {
    if context.chars().count() > MAXIMUM_CONTEXT_CHARACTERS {
        return Err(AppError::DataParsing(format!(
            "HYPOTHESIS_CONTEXT_LIMIT_EXCEEDED:{MAXIMUM_CONTEXT_CHARACTERS}"
        )));
    }
    if request.model_id != LOCAL_MODEL_ID || request.model_version != LOCAL_MODEL_VERSION {
        return Err(AppError::DataParsing(format!(
            "HYPOTHESIS_MODEL_NOT_AVAILABLE: expected {LOCAL_MODEL_ID}@{LOCAL_MODEL_VERSION}"
        )));
    }
    validate_identifier("supplied corpus", &request.supplied_corpus_id)?;
    if request.top_k == 0 || request.top_k > MAXIMUM_TOP_K {
        return Err(AppError::DataParsing(format!(
            "HYPOTHESIS_TOP_K_LIMIT: expected 1 to {MAXIMUM_TOP_K}"
        )));
    }
    Ok(())
}

fn validate_identifier(label: &str, value: &str) -> Result<(), AppError> {
    if value.trim().is_empty()
        || value.chars().count() > 256
        || !value
            .chars()
            .all(|character| character.is_alphanumeric() || matches!(character, '-' | '_' | '.'))
    {
        return Err(AppError::DataParsing(format!(
            "HYPOTHESIS_IDENTIFIER_INVALID:{label}"
        )));
    }
    Ok(())
}

fn token_overlap_score(context: &str, candidate: &str) -> f64 {
    let context_tokens = lexical_tokens(context).collect::<HashSet<_>>();
    let candidate_tokens = lexical_tokens(candidate).collect::<Vec<_>>();
    if candidate_tokens.is_empty() {
        return 0.0;
    }
    let matches = candidate_tokens
        .iter()
        .filter(|token| context_tokens.contains(token.as_str()))
        .count();
    matches as f64 / candidate_tokens.len() as f64
}

fn lexical_tokens(value: &str) -> impl Iterator<Item = String> + '_ {
    value
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| token.chars().count() >= 2)
        .map(str::to_lowercase)
}

fn bounded_grammar_score(value: &str) -> f64 {
    let character_count = value.chars().count();
    if character_count == 0 {
        return 0.0;
    }
    let accepted = value
        .chars()
        .filter(|character| {
            character.is_alphanumeric()
                || character.is_whitespace()
                || character.is_ascii_punctuation()
        })
        .count();
    let token_count = lexical_tokens(value).count();
    let character_score = accepted as f64 / character_count as f64;
    let token_score = (token_count as f64 / 4.0).min(1.0);
    character_score * 0.75 + token_score * 0.25
}

fn levenshtein_distance(left: &str, right: &str) -> usize {
    let right = right.chars().collect::<Vec<_>>();
    let mut previous = (0..=right.len()).collect::<Vec<_>>();
    for (left_index, left_character) in left.chars().enumerate() {
        let mut current = Vec::with_capacity(right.len() + 1);
        current.push(left_index + 1);
        for (right_index, right_character) in right.iter().enumerate() {
            current.push(
                (previous[right_index + 1] + 1)
                    .min(current[right_index] + 1)
                    .min(previous[right_index] + usize::from(left_character != *right_character)),
            );
        }
        previous = current;
    }
    previous[right.len()]
}

fn basis_points(numerator: usize, denominator: usize) -> u16 {
    if denominator == 0 {
        0
    } else {
        ((numerator as u128 * 10_000) / denominator as u128).min(10_000) as u16
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(value: &str, width_error: f64) -> CandidateText {
        CandidateText {
            value: value.to_string(),
            width_error: Some(width_error),
            context_score: None,
            grammar_score: None,
            closed_set_member: true,
            shaping_profile: Some("fixture-font-v1".to_string()),
        }
    }

    fn request(candidates: Vec<CandidateText>) -> LocalHypothesisRequest {
        LocalHypothesisRequest {
            model_id: LOCAL_MODEL_ID.to_string(),
            model_version: LOCAL_MODEL_VERSION.to_string(),
            supplied_corpus_id: "fixture-corpus-v1".to_string(),
            candidates,
            top_k: 1,
            maximum_width_error: Some(20.0),
            minimum_grammar_score: Some(0.5),
        }
    }

    #[test]
    fn emits_only_authorized_closed_set_hypotheses() {
        let result = generate_context_hypotheses(
            "The municipal budget hearing minutes were approved.",
            Some([10.0, 20.0, 120.0, 18.0]),
            request(vec![
                candidate("budget hearing", 1.0),
                candidate("weather report", 1.0),
            ]),
            ForensicAuditOptions {
                include_hypotheses: true,
                preserve_raw_values: true,
            },
        )
        .expect("rank");
        assert!(result.findings.iter().any(|finding| {
            finding.class == EvidenceClass::Hypothesis
                && finding
                    .masked_preview
                    .as_deref()
                    .is_some_and(|value| value.contains("candidate=budget hearing"))
                && finding.locator.bounds == Some([10.0, 20.0, 120.0, 18.0])
        }));
        assert!(result.findings.iter().all(|finding| {
            !finding
                .masked_preview
                .as_deref()
                .is_some_and(|value| value.contains("candidate=weather report"))
        }));
    }

    #[test]
    fn reports_held_out_error_abstention_calibration_and_privacy_metrics() {
        let evaluation = evaluate_local_hypothesis_model(
            "fixture-corpus-v1",
            vec![HeldOutHypothesisCase {
                case_id: "case-1".to_string(),
                subgroup: "meeting-records".to_string(),
                context: "The municipal budget hearing minutes were approved.".to_string(),
                geometry: None,
                candidates: vec![
                    candidate("budget hearing", 1.0),
                    candidate("weather report", 1.0),
                ],
                expected: Some("budget hearing".to_string()),
            }],
            Some(20.0),
            Some(0.5),
        )
        .expect("evaluate");
        assert_eq!(evaluation.case_count, 1);
        assert_eq!(evaluation.character_error_rate_basis_points, 0);
        assert_eq!(evaluation.abstention_rate_basis_points, 0);
        assert_eq!(evaluation.privacy_leakage_count, 0);
        assert_eq!(evaluation.subgroup_metrics[0].correct_count, 1);
    }
}
