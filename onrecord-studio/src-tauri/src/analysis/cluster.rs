use crate::error::AppError;
use crate::types::OutlierCluster;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const HEAVY_REDACTION_DENSITY_THRESHOLD: f64 = 0.45;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ClusterAlertState {
    HeavyRedactionDensity,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RedactionPatternKind {
    BlackUnicodeBlock,
    ExplicitRedactedLabel,
    WithholdingExemption,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RedactionMatchEvidence {
    pub pattern_kind: RedactionPatternKind,
    pub start_character_offset: usize,
    pub end_character_offset: usize,
    pub matched_character_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RedactionDensityMetric {
    pub pattern_match_count: usize,
    pub match_count: usize,
    pub total_string_length: usize,
    pub redaction_ratio: f64,
    pub alert_states: Vec<ClusterAlertState>,
    pub matches: Vec<RedactionMatchEvidence>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TextLayerInput {
    pub layer_id: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TextLayerRedactionMetric {
    pub layer_id: String,
    pub metric: RedactionDensityMetric,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RedactionLayerAnalysis {
    pub layers: Vec<TextLayerRedactionMetric>,
    pub combined_pattern_match_count: usize,
    pub combined_match_count: usize,
    pub combined_string_length: usize,
    pub combined_redaction_ratio: f64,
    pub alert_states: Vec<ClusterAlertState>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NeighborhoodTextRecord {
    pub request_id: u64,
    pub neighborhood_block: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NeighborhoodKeywordDensity {
    pub neighborhood_block: String,
    pub request_count: usize,
    pub total_word_count: usize,
    pub total_keyword_matches: usize,
    pub matching_request_count: usize,
    pub keyword_match_counts: BTreeMap<String, usize>,
    pub keyword_density_ratio: f64,
    pub keyword_density_percent: f64,
    pub per_keyword_density_percent: BTreeMap<String, f64>,
    pub matching_request_ids: Vec<u64>,
}

#[derive(Debug, Clone)]
struct WordToken<'a> {
    _source: &'a str,
    normalized: String,
    start: usize,
    end: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RedactionSpan {
    kind: RedactionPatternKind,
    start: usize,
    end: usize,
}

#[derive(Debug, Default)]
struct NeighborhoodAccumulator {
    display_name: String,
    request_ids: BTreeSet<u64>,
    matching_request_ids: BTreeSet<u64>,
    total_word_count: usize,
    keyword_match_counts: BTreeMap<String, usize>,
}

pub fn analyze_redaction_density(text: &str) -> Result<RedactionDensityMetric, AppError> {
    let total_string_length = text.chars().count();
    let spans = redaction_spans(text);
    let mut matches = Vec::with_capacity(spans.len());
    let mut match_count = 0_usize;

    for span in spans {
        if span.start > span.end
            || span.end > text.len()
            || !text.is_char_boundary(span.start)
            || !text.is_char_boundary(span.end)
        {
            return Err(AppError::Internal(
                "Redaction scanner produced an invalid boundary".to_string(),
            ));
        }
        let start_character_offset = text[..span.start].chars().count();
        let matched_character_count = text[span.start..span.end].chars().count();
        match_count = match_count
            .checked_add(matched_character_count)
            .ok_or_else(|| AppError::Analysis("Redaction coverage count overflowed".to_string()))?;
        matches.push(RedactionMatchEvidence {
            pattern_kind: span.kind,
            start_character_offset,
            end_character_offset: start_character_offset + matched_character_count,
            matched_character_count,
        });
    }

    let redaction_ratio = ratio(match_count, total_string_length);
    let alert_states = if redaction_ratio > HEAVY_REDACTION_DENSITY_THRESHOLD {
        vec![ClusterAlertState::HeavyRedactionDensity]
    } else {
        Vec::new()
    };
    Ok(RedactionDensityMetric {
        pattern_match_count: matches.len(),
        match_count,
        total_string_length,
        redaction_ratio,
        alert_states,
        matches,
    })
}

pub fn analyze_redaction_layers(
    layers: &[TextLayerInput],
) -> Result<RedactionLayerAnalysis, AppError> {
    let mut seen_layer_ids = BTreeSet::new();
    let mut layer_metrics = Vec::with_capacity(layers.len());
    let mut combined_pattern_match_count = 0_usize;
    let mut combined_match_count = 0_usize;
    let mut combined_string_length = 0_usize;
    for layer in layers {
        let layer_id = layer.layer_id.trim();
        if layer_id.is_empty() || !seen_layer_ids.insert(layer_id.to_string()) {
            return Err(AppError::DataParsing(format!(
                "Text layer identifier '{}' is empty or duplicated",
                layer.layer_id
            )));
        }
        let metric = analyze_redaction_density(&layer.text)?;
        combined_pattern_match_count = combined_pattern_match_count
            .checked_add(metric.pattern_match_count)
            .ok_or_else(|| AppError::Analysis("Combined match count overflowed".to_string()))?;
        combined_match_count = combined_match_count
            .checked_add(metric.match_count)
            .ok_or_else(|| AppError::Analysis("Combined coverage count overflowed".to_string()))?;
        combined_string_length = combined_string_length
            .checked_add(metric.total_string_length)
            .ok_or_else(|| AppError::Analysis("Combined text length overflowed".to_string()))?;
        layer_metrics.push(TextLayerRedactionMetric {
            layer_id: layer_id.to_string(),
            metric,
        });
    }
    let combined_redaction_ratio = ratio(combined_match_count, combined_string_length);
    let alert_states = if combined_redaction_ratio > HEAVY_REDACTION_DENSITY_THRESHOLD {
        vec![ClusterAlertState::HeavyRedactionDensity]
    } else {
        Vec::new()
    };
    Ok(RedactionLayerAnalysis {
        layers: layer_metrics,
        combined_pattern_match_count,
        combined_match_count,
        combined_string_length,
        combined_redaction_ratio,
        alert_states,
    })
}

pub fn compute_neighborhood_keyword_densities(
    records: &[NeighborhoodTextRecord],
) -> Result<Vec<NeighborhoodKeywordDensity>, AppError> {
    let mut accumulators: BTreeMap<String, NeighborhoodAccumulator> = BTreeMap::new();
    for record in records {
        let display_name = normalize_neighborhood_block(&record.neighborhood_block);
        let accumulator = accumulators
            .entry(display_name.to_lowercase())
            .or_insert_with(|| NeighborhoodAccumulator {
                display_name,
                ..NeighborhoodAccumulator::default()
            });
        if !accumulator.request_ids.insert(record.request_id) {
            continue;
        }
        let words = word_tokens(&record.text);
        accumulator.total_word_count = accumulator
            .total_word_count
            .checked_add(words.len())
            .ok_or_else(|| AppError::Analysis("Neighborhood word count overflowed".to_string()))?;
        let mut matched = false;
        for word in words {
            if let Some(keyword) = enforcement_keyword(&word.normalized) {
                let count = accumulator
                    .keyword_match_counts
                    .entry(keyword.to_string())
                    .or_insert(0);
                *count = count.checked_add(1).ok_or_else(|| {
                    AppError::Analysis(format!("Keyword count for '{keyword}' overflowed"))
                })?;
                matched = true;
            }
        }
        if matched {
            accumulator.matching_request_ids.insert(record.request_id);
        }
    }

    accumulators
        .into_values()
        .map(|mut accumulator| {
            for keyword in ["sweep", "encampment", "eviction"] {
                accumulator
                    .keyword_match_counts
                    .entry(keyword.to_string())
                    .or_insert(0);
            }
            let total_keyword_matches = accumulator
                .keyword_match_counts
                .values()
                .try_fold(0_usize, |total, count| total.checked_add(*count))
                .ok_or_else(|| {
                    AppError::Analysis("Neighborhood keyword total overflowed".to_string())
                })?;
            let per_keyword_density_percent = accumulator
                .keyword_match_counts
                .iter()
                .map(|(keyword, count)| {
                    (
                        keyword.clone(),
                        percentage(*count, accumulator.total_word_count),
                    )
                })
                .collect();
            Ok(NeighborhoodKeywordDensity {
                neighborhood_block: accumulator.display_name,
                request_count: accumulator.request_ids.len(),
                total_word_count: accumulator.total_word_count,
                total_keyword_matches,
                matching_request_count: accumulator.matching_request_ids.len(),
                keyword_match_counts: accumulator.keyword_match_counts,
                keyword_density_ratio: ratio(total_keyword_matches, accumulator.total_word_count),
                keyword_density_percent: percentage(
                    total_keyword_matches,
                    accumulator.total_word_count,
                ),
                per_keyword_density_percent,
                matching_request_ids: accumulator.matching_request_ids.into_iter().collect(),
            })
        })
        .collect()
}

pub fn extract_enforcement_keyword_counts(text: &str) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for token in word_tokens(text) {
        if let Some(keyword) = enforcement_keyword(&token.normalized) {
            let count = counts.entry(keyword.to_string()).or_insert(0_usize);
            *count = count.saturating_add(1);
        }
    }
    counts
}

pub fn compute_thematic_clusters(
    raw_data: &[(u64, String, String)],
) -> Result<Vec<OutlierCluster>, AppError> {
    let mut cluster_members: BTreeMap<String, BTreeSet<u64>> = BTreeMap::new();
    for (request_id, description, anomaly_type) in raw_data {
        let words = word_tokens(description);
        let cluster_key = if words
            .iter()
            .any(|word| enforcement_keyword(&word.normalized).is_some())
        {
            "CIVIC_DISPLACEMENT".to_string()
        } else if contains_procurement_signal(&words) {
            "PROCUREMENT_ANOMALY".to_string()
        } else {
            format!("UNEXPECTED_METRIC_{}", normalize_anomaly_type(anomaly_type))
        };
        cluster_members
            .entry(cluster_key)
            .or_default()
            .insert(*request_id);
    }
    if cluster_members.is_empty() {
        return Ok(Vec::new());
    }
    let mean = cluster_members
        .values()
        .map(|members| members.len() as f64)
        .sum::<f64>()
        / cluster_members.len() as f64;
    let standard_deviation = (cluster_members
        .values()
        .map(|members| {
            let difference = members.len() as f64 - mean;
            difference * difference
        })
        .sum::<f64>()
        / cluster_members.len() as f64)
        .sqrt();
    Ok(cluster_members
        .into_iter()
        .map(|(cluster_key, members)| OutlierCluster {
            cluster_id: format!(
                "cluster-{}",
                cluster_key.to_ascii_lowercase().replace('_', "-")
            ),
            core_anomaly_type: cluster_key,
            dynamic_z_score: if standard_deviation > 0.0 {
                (members.len() as f64 - mean) / standard_deviation
            } else {
                0.0
            },
            matching_request_ids: members.into_iter().collect(),
        })
        .collect())
}

fn redaction_spans(text: &str) -> Vec<RedactionSpan> {
    let mut spans = black_block_spans(text);
    let words = word_tokens(text);
    for (index, word) in words.iter().enumerate() {
        if word.normalized == "redacted" {
            if let Some((start, end)) = expand_brackets(text, word.start, word.end) {
                spans.push(RedactionSpan {
                    kind: RedactionPatternKind::ExplicitRedactedLabel,
                    start,
                    end,
                });
            }
        }
        if word.normalized != "withheld"
            || words.get(index + 1).map(|value| value.normalized.as_str()) != Some("under")
        {
            continue;
        }
        let mut next = index + 2;
        if words
            .get(next)
            .is_some_and(|value| matches!(value.normalized.as_str(), "a" | "an"))
        {
            next += 1;
        }
        if let Some(exemption) = words
            .get(next)
            .filter(|value| value.normalized == "exemption")
        {
            let (start, end) = expand_brackets(text, word.start, exemption.end)
                .unwrap_or((word.start, exemption.end));
            spans.push(RedactionSpan {
                kind: RedactionPatternKind::WithholdingExemption,
                start,
                end,
            });
        }
    }
    spans.sort_by(|left, right| {
        left.start
            .cmp(&right.start)
            .then_with(|| right.end.cmp(&left.end))
            .then_with(|| left.kind.cmp(&right.kind))
    });
    let mut selected: Vec<RedactionSpan> = Vec::new();
    for span in spans {
        if selected
            .last()
            .is_some_and(|previous| span.start < previous.end)
        {
            continue;
        }
        selected.push(span);
    }
    selected
}

fn black_block_spans(text: &str) -> Vec<RedactionSpan> {
    let mut spans = Vec::new();
    let mut run_start = None;
    let mut run_length = 0_usize;
    for (index, character) in text
        .char_indices()
        .chain(std::iter::once((text.len(), '\0')))
    {
        if character == '\u{2588}' {
            run_start.get_or_insert(index);
            run_length += 1;
        } else if let Some(start) = run_start.take() {
            if run_length >= 3 {
                spans.push(RedactionSpan {
                    kind: RedactionPatternKind::BlackUnicodeBlock,
                    start,
                    end: index,
                });
            }
            run_length = 0;
        }
    }
    spans
}

fn expand_brackets(text: &str, start: usize, end: usize) -> Option<(usize, usize)> {
    let prefix = &text[..start];
    let suffix = &text[end..];
    let opening = prefix
        .char_indices()
        .rev()
        .find(|(_, character)| !character.is_whitespace())?;
    let closing = suffix
        .char_indices()
        .find(|(_, character)| !character.is_whitespace())?;
    if opening.1 == '[' && closing.1 == ']' {
        Some((opening.0, end + closing.0 + closing.1.len_utf8()))
    } else {
        None
    }
}

fn word_tokens(text: &str) -> Vec<WordToken<'_>> {
    let mut words = Vec::new();
    let mut start = None;
    for (index, character) in text
        .char_indices()
        .chain(std::iter::once((text.len(), '\0')))
    {
        if character.is_alphanumeric() || matches!(character, '\'' | '’' | '-') {
            start.get_or_insert(index);
        } else if let Some(word_start) = start.take() {
            let value = &text[word_start..index];
            words.push(WordToken {
                _source: value,
                normalized: value
                    .trim_matches(|value: char| matches!(value, '\'' | '’' | '-'))
                    .to_lowercase(),
                start: word_start,
                end: index,
            });
        }
    }
    words
}

fn enforcement_keyword(word: &str) -> Option<&'static str> {
    match word {
        "sweep" | "sweeps" | "sweeping" | "sweeped" | "swept" => Some("sweep"),
        "encampment" | "encampments" => Some("encampment"),
        "evict" | "evicts" | "evicted" | "evicting" | "eviction" | "evictions" => Some("eviction"),
        _ => None,
    }
}

fn contains_procurement_signal(words: &[WordToken<'_>]) -> bool {
    words.iter().enumerate().any(|(index, word)| {
        matches!(
            word.normalized.as_str(),
            "contract" | "contracts" | "invoice" | "invoices"
        ) || (word.normalized == "sole"
            && words
                .get(index + 1)
                .is_some_and(|next| next.normalized == "source"))
    })
}

fn normalize_neighborhood_block(value: &str) -> String {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        "UNSPECIFIED_NEIGHBORHOOD".to_string()
    } else {
        normalized
    }
}

fn normalize_anomaly_type(value: &str) -> String {
    let mut normalized = String::new();
    for character in value.trim().chars() {
        if character.is_alphanumeric() {
            normalized.extend(character.to_uppercase());
        } else if !normalized.is_empty() && !normalized.ends_with('_') {
            normalized.push('_');
        }
    }
    while normalized.ends_with('_') {
        normalized.pop();
    }
    if normalized.is_empty() {
        "UNSPECIFIED".to_string()
    } else {
        normalized
    }
}

fn ratio(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

fn percentage(numerator: usize, denominator: usize) -> f64 {
    ratio(numerator, denominator) * 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measures_structural_redaction_coverage() {
        let metric = analyze_redaction_density("[REDACTED]████").unwrap();
        assert_eq!(metric.pattern_match_count, 2);
        assert_eq!(metric.redaction_ratio, 1.0);
        assert_eq!(
            metric.alert_states,
            vec![ClusterAlertState::HeavyRedactionDensity]
        );
    }

    #[test]
    fn computes_stemmed_density_without_substring_false_positives() {
        let records = vec![NeighborhoodTextRecord {
            request_id: 1,
            neighborhood_block: " Downtown ".to_string(),
            text: "Sweeping encampments and evictions, not sweepstakes.".to_string(),
        }];
        let density = compute_neighborhood_keyword_densities(&records).unwrap();
        assert_eq!(density[0].total_keyword_matches, 3);
        assert_eq!(density[0].matching_request_ids, vec![1]);
    }
}
