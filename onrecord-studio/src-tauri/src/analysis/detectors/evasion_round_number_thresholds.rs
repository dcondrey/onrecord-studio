//! Released document counts clustering suspiciously below round-number thresholds.
//!
//! `absence_of_records` catches gaps in a self-labelled numeric sequence within one
//! release. This detector operates one level up: an agency whose release counts across many
//! requests cluster tightly at 9, 24, 49, 99, or 499 — never crossing the 10/25/50/100/500 line
//! that would trigger a heavier review policy — is a pattern that survives sequence-gap scrutiny
//! but suggests responses are being sized to duck a threshold.
//!
//! **Not a legal claim** (`is_legal_claim = false`). Adversarial signal, ≥ 6500 bp confidence
//! floor. Explanation is factual: "N of M releases from agency X sit within 3 of a round-number
//! threshold with none above" — the reader interprets the intent.
//!
//! **Data source**: uses the corpus's per-request public document count. Does not require any
//! agency-published threshold — the round-number heuristic is the signal.

use crate::{
    analysis::finding::{
        finding_id, Finding, FindingCitation, FindingConfidence, FindingKind, FindingSeverity,
        FindingSubject,
    },
    error::AppError,
    storage::DuckDbRepository,
};
use chrono::Utc;
use duckdb::params;
use std::collections::HashMap;

pub const SLUG: &str = "evasion_round_number_thresholds";

const MAX_REQUESTS_INSPECTED: i64 = 10_000;
/// Only consider requests with at least this many released documents — the pattern is about
/// bounded-size releases, and single-document requests can't cluster near a threshold anyway.
const MIN_RELEASE_COUNT: i64 = 5;
/// Round-number thresholds worth checking. Ordered from smallest to largest.
const ROUND_THRESHOLDS: &[i64] = &[10, 25, 50, 100, 250, 500, 1000];
/// A release count is "just under" a threshold if it's within this many of it (below).
const BAND_WIDTH: i64 = 3;
/// Minimum count of just-under releases at one agency+threshold pair to fire.
const MIN_JUST_UNDER: usize = 6;
/// If any release from that same agency exceeds the threshold, the pattern is not a ceiling —
/// suppress. Allow at most this many crossings to tolerate the rare exception.
const MAX_CROSSINGS: usize = 1;
const MAX_FINDINGS: usize = 50;
const MIN_CONFIDENCE_TO_FIRE: u16 = 6500;

pub async fn run(repository: &DuckDbRepository) -> Result<Vec<Finding>, AppError> {
    let releases = load_releases(repository).await?;
    let mut findings = evaluate(&releases);
    findings.sort_by_key(|f| std::cmp::Reverse(f.confidence.rank_weight()));
    findings.truncate(MAX_FINDINGS);
    Ok(findings)
}

#[derive(Debug, Clone)]
struct RequestRelease {
    portal_key: String,
    agency_id: i64,
    request_id: i64,
    release_count: i64,
    sample_document_id: Option<i64>,
}

async fn load_releases(repository: &DuckDbRepository) -> Result<Vec<RequestRelease>, AppError> {
    repository
        .query_connection("Round-number threshold release-count query", move |conn| {
            let mut statement = conn
                .prepare(
                    "SELECT r.portal_subdomain, r.agency_id, r.id,
                            COUNT(d.id) AS release_count,
                            MIN(d.id) AS sample_document_id
                     FROM requests r
                     LEFT JOIN documents d
                        ON d.request_id = r.id
                       AND d.portal_subdomain = r.portal_subdomain
                       AND LOWER(TRIM(d.visibility)) = 'public'
                     WHERE r.agency_id IS NOT NULL
                     GROUP BY r.portal_subdomain, r.agency_id, r.id
                     HAVING COUNT(d.id) >= ?1
                     LIMIT ?2",
                )
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to prepare round-number threshold query: {error}"
                    ))
                })?;
            let rows = statement
                .query_map(params![MIN_RELEASE_COUNT, MAX_REQUESTS_INSPECTED], |row| {
                    Ok(RequestRelease {
                        portal_key: row.get(0)?,
                        agency_id: row.get(1)?,
                        request_id: row.get(2)?,
                        release_count: row.get(3)?,
                        sample_document_id: row.get::<_, Option<i64>>(4)?,
                    })
                })
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to run round-number threshold query: {error}"
                    ))
                })?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row.map_err(|error| {
                    AppError::Storage(format!("Failed to decode round-number row: {error}"))
                })?);
            }
            Ok(out)
        })
        .await
}

fn evaluate(releases: &[RequestRelease]) -> Vec<Finding> {
    // Group by (agency_id, portal_key). Portal matters because an agency across two portals may
    // have distinct release cadences; we prefer per-portal fidelity to over-aggregation.
    let mut per_agency: HashMap<(String, i64), Vec<&RequestRelease>> = HashMap::new();
    for release in releases {
        per_agency
            .entry((release.portal_key.clone(), release.agency_id))
            .or_default()
            .push(release);
    }
    let mut findings = Vec::new();
    for ((portal_key, agency_id), group) in per_agency {
        // For each threshold, count how many releases fall in [threshold - BAND_WIDTH, threshold - 1]
        // vs. how many strictly exceed threshold. A ceiling shows as many just-under, ≤ MAX_CROSSINGS
        // above.
        for &threshold in ROUND_THRESHOLDS {
            let just_under: Vec<&&RequestRelease> = group
                .iter()
                .filter(|r| {
                    r.release_count >= threshold - BAND_WIDTH && r.release_count < threshold
                })
                .collect();
            if just_under.len() < MIN_JUST_UNDER {
                continue;
            }
            // "Crossings" of THIS threshold are releases that cleared the ceiling but did NOT
            // land in the NEXT threshold's just-under band. Releases inside the next band are
            // an independent ceiling pattern, not evidence that this ceiling was crossed at
            // will. So a release at 98 is inside the 100-band, not a crossing of 50.
            let next_threshold = ROUND_THRESHOLDS
                .iter()
                .copied()
                .find(|&t| t > threshold)
                .unwrap_or(i64::MAX);
            let next_band_start = next_threshold.saturating_sub(BAND_WIDTH);
            let crossings = group
                .iter()
                .filter(|r| r.release_count >= threshold && r.release_count < next_band_start)
                .count();
            if crossings > MAX_CROSSINGS {
                continue;
            }
            // Confidence: scale by (just-under count, group size, tightness).
            let size_bp = ((just_under.len() as f64 - MIN_JUST_UNDER as f64) * 350.0)
                .clamp(0.0, 2100.0) as u16;
            let group_bp = ((group.len() as f64).ln() * 700.0).clamp(0.0, 1400.0) as u16;
            let avg_gap: f64 = just_under
                .iter()
                .map(|r| (threshold - r.release_count) as f64)
                .sum::<f64>()
                / just_under.len() as f64;
            let tight_bp = ((BAND_WIDTH as f64 - avg_gap + 0.5) * 800.0).clamp(0.0, 1600.0) as u16;
            // Base = MIN_CONFIDENCE_TO_FIRE. Just-under counts + group size + tightness are
            // the ONLY signals that push confidence beyond the floor; the crossings/count
            // gates above are the sufficiency check for firing at all.
            let raw_bp = MIN_CONFIDENCE_TO_FIRE
                .saturating_add(size_bp)
                .saturating_add(group_bp)
                .saturating_add(tight_bp);
            let bp = raw_bp.min(10_000);
            if bp < MIN_CONFIDENCE_TO_FIRE {
                continue;
            }
            let subject = FindingSubject {
                portal_key: Some(portal_key.clone()),
                agency_id: Some(agency_id),
                ..Default::default()
            };
            let salient = format!(
                "threshold={},just_under={},crossings={}",
                threshold,
                just_under.len(),
                crossings
            );
            let id = finding_id(
                FindingKind::EvasionRoundNumberThresholds,
                SLUG,
                &subject,
                &[&salient],
            );
            let headline = format!(
                "Agency {} released just under {}: {} times, {} above",
                agency_id,
                threshold,
                just_under.len(),
                crossings
            );
            let counts_preview: String = {
                let mut sizes: Vec<i64> = just_under.iter().map(|r| r.release_count).collect();
                sizes.sort_unstable_by(|a, b| b.cmp(a));
                let preview: Vec<String> = sizes.iter().take(6).map(|n| n.to_string()).collect();
                preview.join(", ")
            };
            let explanation = format!(
                "Of agency {agency}'s {total} multi-document releases inspected, {n} landed \
                 within {band} documents of {threshold} without crossing it (release counts: \
                 {counts_preview}). {crossings} releases from this agency exceed {threshold}. \
                 A ceiling that reads as a boundary rather than a coincidence is the pattern; \
                 the intent behind it is not something the corpus alone establishes.",
                agency = agency_id,
                total = group.len(),
                n = just_under.len(),
                band = BAND_WIDTH,
                threshold = threshold,
                crossings = crossings,
            );
            let citations: Vec<FindingCitation> = just_under
                .iter()
                .take(5)
                .filter_map(|r| {
                    r.sample_document_id.map(|document_id| FindingCitation {
                        portal_key: r.portal_key.clone(),
                        document_id,
                        character_start: None,
                        character_end: None,
                        quote: None,
                        label: Some(format!(
                            "request {} released {} documents",
                            r.request_id, r.release_count
                        )),
                    })
                })
                .collect();
            let now = Utc::now();
            findings.push(Finding {
                id,
                kind: FindingKind::EvasionRoundNumberThresholds,
                severity: FindingSeverity::Serious,
                confidence: FindingConfidence::measured(bp),
                subject,
                headline,
                explanation,
                citations,
                discovered_at: now,
                last_seen_at: now,
                dismissed_at: None,
                source_detector: SLUG.into(),
            });
        }
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(agency: i64, request: i64, count: i64) -> RequestRelease {
        RequestRelease {
            portal_key: "test".into(),
            agency_id: agency,
            request_id: request,
            release_count: count,
            sample_document_id: Some(request * 100),
        }
    }

    #[test]
    fn ceiling_at_100_fires_when_six_just_under_and_no_crossings() {
        let releases: Vec<RequestRelease> = (0..6).map(|i| release(7, i, 99 - (i % 3))).collect(); // counts of 99, 98, 97, 99, 98, 97 — all inside band, none crossing
        let findings = evaluate(&releases);
        assert!(!findings.is_empty(), "expected a finding: {findings:?}");
        assert!(findings[0].headline.contains("100"));
    }

    #[test]
    fn many_crossings_suppress_the_ceiling_finding() {
        let mut releases: Vec<RequestRelease> =
            (0..6).map(|i| release(7, i, 99 - (i % 3))).collect();
        // Add many releases above 100 — this agency clearly isn't ceiling-capped.
        for i in 6..12 {
            releases.push(release(7, i, 150));
        }
        let findings = evaluate(&releases);
        assert!(
            findings.is_empty(),
            "many crossings should suppress; got: {findings:?}"
        );
    }

    #[test]
    fn count_below_minimum_just_under_does_not_fire() {
        // Only 4 just-under; MIN_JUST_UNDER is 6.
        let releases: Vec<RequestRelease> = (0..4).map(|i| release(7, i, 99 - i)).collect();
        assert!(evaluate(&releases).is_empty());
    }

    #[test]
    fn ceiling_finding_stable_across_reruns() {
        let releases: Vec<RequestRelease> = (0..6).map(|i| release(7, i, 99 - (i % 3))).collect();
        let a = evaluate(&releases);
        let b = evaluate(&releases);
        assert_eq!(a[0].id, b[0].id);
    }

    #[test]
    fn multiple_thresholds_can_fire_independently_for_one_agency() {
        // Requests at ~49 AND at ~99 — both hit ceilings with enough tightness for
        // MIN_CONFIDENCE_TO_FIRE. avg_gap 1.5 for each band puts tight_bp at the 1600 clamp,
        // yielding raw ≈ 7200 for each threshold.
        let mut releases: Vec<RequestRelease> =
            (0..6).map(|i| release(7, i, 49 - (i % 2))).collect();
        releases.extend((0..6).map(|i| release(7, i + 100, 99 - (i % 2))));
        let findings = evaluate(&releases);
        let thresholds: std::collections::HashSet<&str> = findings
            .iter()
            .map(|f| {
                if f.headline.contains("50") {
                    "50"
                } else if f.headline.contains("100") {
                    "100"
                } else {
                    "?"
                }
            })
            .collect();
        assert!(thresholds.contains("50"));
        assert!(thresholds.contains("100"));
    }
}
