use crate::error::AppError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TextLayerRedactionMetric {
    pub layer_id: String,
    pub metric: RedactionDensityMetric,
}

#[derive(Debug, Clone)]
pub(crate) struct WordToken<'a> {
    _source: &'a str,
    pub(crate) normalized: String,
    pub(crate) start: usize,
    pub(crate) end: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RedactionSpan {
    pub(crate) kind: RedactionPatternKind,
    pub(crate) start: usize,
    pub(crate) end: usize,
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

pub fn extract_enforcement_keyword_counts(text: &str) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    let words = word_tokens(text);
    for keyword in enforcement_keywords(&words) {
        let count = counts.entry(keyword.to_string()).or_insert(0_usize);
        *count = count.saturating_add(1);
    }
    counts
}

pub(crate) fn redaction_spans(text: &str) -> Vec<RedactionSpan> {
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

pub(crate) fn word_tokens(text: &str) -> Vec<WordToken<'_>> {
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
        "clean" | "cleans" | "cleaned" | "cleaning" | "cleanup" | "cleanups" => Some("clean"),
        "displace" | "displaces" | "displaced" | "displacing" | "displacement"
        | "displacements" => Some("displace"),
        _ => None,
    }
}

fn enforcement_keywords(words: &[WordToken<'_>]) -> Vec<&'static str> {
    let mut matches = Vec::new();
    for (index, word) in words.iter().enumerate() {
        if let Some(keyword) = enforcement_keyword(&word.normalized) {
            matches.push(keyword);
        }
        if word.normalized == "property"
            && words
                .get(index + 1)
                .is_some_and(|next| matches!(next.normalized.as_str(), "storage" | "stored"))
        {
            matches.push("property storage");
        }
    }
    matches
}

fn ratio(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
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
    fn matches_cleanup_displacement_and_bounded_property_storage_phrases() {
        let counts = extract_enforcement_keyword_counts(
            "Cleanup crews displaced residents while property storage was arranged. A property tax storage rule is unrelated.",
        );
        assert_eq!(counts.get("clean"), Some(&1));
        assert_eq!(counts.get("displace"), Some(&1));
        assert_eq!(counts.get("property storage"), Some(&1));
    }
}
