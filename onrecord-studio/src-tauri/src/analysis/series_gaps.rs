//! Missing numbers in labelled document series ("Resolution No. 2024-012").
//!
//! A gap is a lead: the corpus holds 2024-011 and 2024-013 and nothing numbered 2024-012. It is
//! not a finding that the agency withheld anything; the corpus is whatever was requested and
//! released, and a number can be skipped, voided, or filed elsewhere.

use std::collections::BTreeMap;

use crate::types::{SeriesGap, SeriesGapReport, SeriesMember};

/// Members needed before a series is examined; two members always "bound" everything between.
const MINIMUM_SERIES_MEMBERS: usize = 3;
/// A run of missing numbers longer than this is more likely two series than one with a hole.
const MAXIMUM_MISSING_RUN: u64 = 25;

pub struct SeriesObservation {
    pub entity_id: String,
    /// `label:VALUE`, as the extractor normalizes it.
    pub normalized_value: String,
    pub display_value: String,
    pub document_count: u64,
}

struct Parsed {
    label: String,
    prefix: String,
    number: u64,
    width: usize,
}

/// Splits `label:PREFIX-NNN` into its series key and trailing number.
fn parse(normalized: &str) -> Option<Parsed> {
    let (label, value) = normalized.split_once(':')?;
    let digit_start = value
        .rfind(|character: char| !character.is_ascii_digit())
        .map_or(0, |index| index + 1);
    let digits = &value[digit_start..];
    if digits.is_empty() || digits.len() > 12 {
        return None;
    }
    Some(Parsed {
        label: label.to_string(),
        prefix: value[..digit_start].to_string(),
        number: digits.parse().ok()?,
        width: digits.len(),
    })
}

pub fn series_gaps(observations: Vec<SeriesObservation>, limit: usize) -> SeriesGapReport {
    let mut series: BTreeMap<(String, String), BTreeMap<u64, (usize, SeriesMember)>> =
        BTreeMap::new();
    for observation in observations {
        let Some(parsed) = parse(&observation.normalized_value) else {
            continue;
        };
        let member = SeriesMember {
            entity_id: observation.entity_id,
            display_value: observation.display_value,
            document_count: observation.document_count,
        };
        series
            .entry((parsed.label, parsed.prefix))
            .or_default()
            // The same number written at two widths ("12" and "012") is one member; keep the
            // better-documented spelling.
            .entry(parsed.number)
            .and_modify(|existing| {
                if member.document_count > existing.1.document_count {
                    *existing = (parsed.width, member.clone());
                }
            })
            .or_insert((parsed.width, member));
    }

    let mut gaps = Vec::new();
    let mut series_count = 0_u64;
    for ((label, prefix), members) in series {
        if members.len() < MINIMUM_SERIES_MEMBERS {
            continue;
        }
        series_count += 1;
        let width = members.values().map(|(width, _)| *width).max().unwrap_or(1);
        let ordered: Vec<(&u64, &(usize, SeriesMember))> = members.iter().collect();
        for pair in ordered.windows(2) {
            let (before_number, before) = pair[0];
            let (after_number, after) = pair[1];
            let missing = after_number - before_number - 1;
            if missing == 0 || missing > MAXIMUM_MISSING_RUN {
                continue;
            }
            gaps.push(SeriesGap {
                label: label.clone(),
                prefix: prefix.clone(),
                from_number: before_number + 1,
                to_number: after_number - 1,
                width,
                before: before.1.clone(),
                after: after.1.clone(),
                series_member_count: members.len() as u64,
            });
        }
    }
    // Shortest holes in the best-documented series first: one missing number between two
    // well-attested neighbours is the strongest lead.
    gaps.sort_by(|left, right| {
        (left.to_number - left.from_number)
            .cmp(&(right.to_number - right.from_number))
            .then(right.series_member_count.cmp(&left.series_member_count))
            .then(left.label.cmp(&right.label))
            .then(left.prefix.cmp(&right.prefix))
            .then(left.from_number.cmp(&right.from_number))
    });
    let truncated = gaps.len() > limit;
    gaps.truncate(limit);
    SeriesGapReport {
        gaps,
        series_count,
        truncated,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(normalized: &str, documents: u64) -> SeriesObservation {
        SeriesObservation {
            entity_id: normalized.to_string(),
            normalized_value: normalized.to_string(),
            display_value: normalized.to_string(),
            document_count: documents,
        }
    }

    /// A gap needs a bounded series: three members, a hole short enough to be one series, and
    /// numbers compared as numbers so `012` and `12` are one member rather than a false pair.
    #[test]
    fn a_gap_is_bounded_short_and_numeric() {
        let report = series_gaps(
            vec![
                seen("resolution:2024-011", 2),
                seen("resolution:2024-013", 1),
                seen("resolution:2024-13", 3),
                seen("resolution:2024-016", 1),
                // Two members only: nothing between them can be called missing.
                seen("ordinance:100", 1),
                seen("ordinance:102", 1),
                // A different prefix is a different series, not a hole of thousands.
                seen("resolution:2023-001", 1),
                seen("resolution:2023-002", 1),
                seen("resolution:2023-050", 1),
            ],
            10,
        );
        assert_eq!(report.series_count, 2);
        let summary: Vec<(String, u64, u64, String)> = report
            .gaps
            .iter()
            .map(|gap| {
                (
                    gap.prefix.clone(),
                    gap.from_number,
                    gap.to_number,
                    gap.after.display_value.clone(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                (
                    "2024-".to_string(),
                    12,
                    12,
                    "resolution:2024-13".to_string()
                ),
                (
                    "2024-".to_string(),
                    14,
                    15,
                    "resolution:2024-016".to_string()
                ),
            ]
        );
        assert_eq!(report.gaps[0].width, 3);
        assert!(!report.truncated);
        assert!(series_gaps(vec![], 10).gaps.is_empty());
    }
}
