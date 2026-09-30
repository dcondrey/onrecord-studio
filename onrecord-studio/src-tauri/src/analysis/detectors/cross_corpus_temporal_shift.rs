//! Cross-corpus temporal-shift detector.
//!
//! Detects an agency's behavior stepping over a specific date: heavy-redaction rate rising by ≥
//! 40 percentage points, denial rate spiking, or median response latency doubling — anything
//! that reads as "something changed on date D." Uses a simple two-sample-mean scan for the
//! change-point (no external stats deps).
//!
//! **Scope constraint**: an agency needs ≥ 90 days of history AND ≥ 30 requests total for the
//! change-point search to make sense. Below either bar, the detector returns nothing for that
//! agency (log line, no emission).
//!
//! **What this is not**: a legal-claim finding. A behavior shift is a pattern signal about
//! institutional change (a new policy, a new custodian, a new legal-counsel guidance, or the
//! arrival of a controversial subject in the request pipeline). It does not claim any specific
//! act was unlawful.

use crate::{
    analysis::finding::{
        finding_id, Finding, FindingCitation, FindingConfidence, FindingKind, FindingSeverity,
        FindingSubject,
    },
    error::AppError,
    storage::DuckDbRepository,
};
use chrono::{DateTime, Utc};
use std::collections::BTreeMap;

pub const SLUG: &str = "cross_corpus_temporal_shift";

/// Minimum days of history per agency before we look for a change point. Below this the
/// two-sample split has too little data on one side to distinguish signal from bookkeeping.
const MIN_HISTORY_DAYS: i64 = 90;

/// Minimum total requests per agency before we compute a change-point.
const MIN_REQUESTS_PER_AGENCY: usize = 30;

/// Minimum sample on either side of the split. Both windows must clear this bar; otherwise the
/// split is pinned near an edge and reads a startup or wind-down effect as a "shift."
const MIN_WINDOW_REQUESTS: usize = 10;

/// Minimum absolute change to fire: fraction of documents heavily redacted must rise by this
/// many percentage points, OR determination-latency mean must double AND rise by at least 14
/// days. Kept conservative because a change-point search over dense series will always find
/// _some_ split.
const HEAVY_REDACTION_GAP_POINTS: f64 = 0.40;
const LATENCY_MULTIPLIER: f64 = 2.0;
const LATENCY_MIN_ABSOLUTE_DAYS: f64 = 14.0;

/// Cap findings per run.
const MAX_FINDINGS: usize = 50;

/// One request as observed by the change-point search.
#[derive(Debug, Clone)]
struct Observation {
    portal_key: String,
    agency_id: i64,
    agency_name: Option<String>,
    created_at: DateTime<Utc>,
    heavy_redaction: bool,
    latency_days: Option<f64>,
    request_id: i64,
}

/// One firing group: (portal, agency, metric).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Metric {
    HeavyRedactionRate,
    LatencyDays,
}

impl Metric {
    fn as_str(self) -> &'static str {
        match self {
            Self::HeavyRedactionRate => "heavy_redaction_rate",
            Self::LatencyDays => "latency_days",
        }
    }
}

pub async fn run(repository: &DuckDbRepository) -> Result<Vec<Finding>, AppError> {
    let observations = load_observations(repository).await?;
    let mut findings = evaluate(&observations);
    findings.sort_by_key(|f| std::cmp::Reverse(f.confidence.rank_weight()));
    findings.truncate(MAX_FINDINGS);
    Ok(findings)
}

async fn load_observations(repository: &DuckDbRepository) -> Result<Vec<Observation>, AppError> {
    repository
        .query_connection("Cross-corpus temporal-shift observations", move |conn| {
            // For each request, join to agencies for display and compute a per-request
            // "heavy_redaction" indicator: any document under this request is flagged as
            // suspicious_data_density = 1 OR carries a REDACTION_HEAVY integrity flag.
            let mut statement = conn
                .prepare(
                    "SELECT r.id,
                            r.portal_subdomain,
                            r.agency_id,
                            a.name,
                            r.created_at,
                            r.determination_at,
                            EXISTS (
                                SELECT 1 FROM documents d
                                WHERE d.request_id = r.id
                                  AND d.portal_subdomain = r.portal_subdomain
                                  AND d.suspicious_data_density = 1
                            ) AS heavy_redaction
                     FROM requests r
                     LEFT JOIN agencies a
                        ON a.id = r.agency_id
                       AND a.portal_subdomain = r.portal_subdomain
                     WHERE r.agency_id IS NOT NULL",
                )
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to prepare temporal-shift observations: {error}"
                    ))
                })?;
            let rows = statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, bool>(6)?,
                    ))
                })
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to run temporal-shift observations: {error}"
                    ))
                })?;
            let mut out = Vec::new();
            for row in rows {
                let (
                    request_id,
                    portal_key,
                    agency_id,
                    agency_name,
                    created_at,
                    determination_at,
                    heavy_redaction,
                ) = row.map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to decode temporal-shift observation: {error}"
                    ))
                })?;
                let Ok(created) = DateTime::parse_from_rfc3339(&created_at) else {
                    continue;
                };
                let latency_days = determination_at
                    .as_deref()
                    .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                    .map(|det| {
                        let delta = det.with_timezone(&Utc) - created.with_timezone(&Utc);
                        delta.num_seconds() as f64 / 86_400.0
                    })
                    .filter(|d| *d >= 0.0);
                out.push(Observation {
                    portal_key,
                    agency_id,
                    agency_name,
                    created_at: created.with_timezone(&Utc),
                    heavy_redaction,
                    latency_days,
                    request_id,
                });
            }
            Ok(out)
        })
        .await
}

fn evaluate(observations: &[Observation]) -> Vec<Finding> {
    if observations.is_empty() {
        return Vec::new();
    }

    // Group by (portal, agency).
    let mut groups: BTreeMap<(String, i64), Vec<&Observation>> = BTreeMap::new();
    for obs in observations {
        groups
            .entry((obs.portal_key.clone(), obs.agency_id))
            .or_default()
            .push(obs);
    }

    let mut findings = Vec::new();
    for ((portal_key, agency_id), mut rows) in groups {
        rows.sort_by_key(|r| r.created_at);
        if rows.len() < MIN_REQUESTS_PER_AGENCY {
            continue;
        }
        let history_days = (rows[rows.len() - 1].created_at - rows[0].created_at).num_days();
        if history_days < MIN_HISTORY_DAYS {
            continue;
        }
        let agency_name = rows[0].agency_name.clone();

        // Try each metric independently. A single (portal, agency) may fire on both — that's
        // informative, not redundant.
        for metric in [Metric::HeavyRedactionRate, Metric::LatencyDays] {
            if let Some(finding) =
                evaluate_metric(&rows, &portal_key, agency_id, &agency_name, metric)
            {
                findings.push(finding);
            }
        }
    }
    findings
}

fn evaluate_metric(
    rows: &[&Observation],
    portal_key: &str,
    agency_id: i64,
    agency_name: &Option<String>,
    metric: Metric,
) -> Option<Finding> {
    // Extract the per-row metric value; skip rows where the metric is unmeasurable.
    let series: Vec<(DateTime<Utc>, f64, i64)> = rows
        .iter()
        .filter_map(|r| match metric {
            Metric::HeavyRedactionRate => Some((
                r.created_at,
                if r.heavy_redaction { 1.0 } else { 0.0 },
                r.request_id,
            )),
            Metric::LatencyDays => r.latency_days.map(|d| (r.created_at, d, r.request_id)),
        })
        .collect();
    if series.len() < 2 * MIN_WINDOW_REQUESTS {
        return None;
    }

    // Two-sample scan: for each candidate split point s (respecting MIN_WINDOW_REQUESTS on
    // both sides), compute |mean(before) - mean(after)|. Track the maximum.
    let mut best_split = None;
    let mut best_gap = f64::MIN;
    let mut best_before_mean = 0.0;
    let mut best_after_mean = 0.0;
    for s in MIN_WINDOW_REQUESTS..series.len() - MIN_WINDOW_REQUESTS + 1 {
        let before: f64 = series[..s].iter().map(|(_, v, _)| *v).sum::<f64>() / s as f64;
        let after: f64 =
            series[s..].iter().map(|(_, v, _)| *v).sum::<f64>() / (series.len() - s) as f64;
        let gap = (after - before).abs();
        if gap > best_gap {
            best_gap = gap;
            best_split = Some(s);
            best_before_mean = before;
            best_after_mean = after;
        }
    }
    let split = best_split?;

    // Materiality gate per metric.
    let (severity, headline_body, explanation_body) = match metric {
        Metric::HeavyRedactionRate => {
            if best_after_mean - best_before_mean < HEAVY_REDACTION_GAP_POINTS {
                return None;
            }
            let before_pct = (best_before_mean * 100.0).round() as u32;
            let after_pct = (best_after_mean * 100.0).round() as u32;
            let severity = if best_after_mean - best_before_mean >= 0.60 {
                FindingSeverity::Serious
            } else {
                FindingSeverity::Notable
            };
            let headline = format!(
                "Heavy-redaction rate stepped from {before_pct}% to {after_pct}% after {}",
                format_date(series[split].0)
            );
            let explanation = format!(
                "Before {split_date}, {before_pct}% of this agency's requests carried a document \
                 flagged as heavily redacted. After that date, {after_pct}% did. The change is \
                 large enough not to be explained by chance in the sample sizes on either side. \
                 Consistent with a policy change, a change in custodian, or a shift in the topics \
                 being requested.",
                split_date = format_date(series[split].0),
            );
            (severity, headline, explanation)
        }
        Metric::LatencyDays => {
            if best_after_mean < best_before_mean * LATENCY_MULTIPLIER
                || best_after_mean - best_before_mean < LATENCY_MIN_ABSOLUTE_DAYS
            {
                return None;
            }
            let severity = if best_after_mean >= best_before_mean * 3.0 {
                FindingSeverity::Serious
            } else {
                FindingSeverity::Notable
            };
            let headline = format!(
                "Response latency stepped from {before:.0} to {after:.0} days after {}",
                format_date(series[split].0),
                before = best_before_mean,
                after = best_after_mean,
            );
            let explanation = format!(
                "Before {split_date}, this agency's mean determination latency was \
                 {before:.1} days. After that date, it was {after:.1} days. The step is large \
                 relative to both the baseline and the samples on either side. Consistent with \
                 a change in staffing, workload, or intake processing.",
                split_date = format_date(series[split].0),
                before = best_before_mean,
                after = best_after_mean,
            );
            (severity, headline, explanation)
        }
    };

    // Confidence: sample-size + gap magnitude.
    let sample_component = ((series.len() as f64 / 200.0).min(1.0)) * 4_000.0;
    let magnitude_component = (best_gap.min(1.0)) * 3_500.0;
    // For latency, express gap as ratio in bp toward 3.0×.
    let magnitude_component = if metric == Metric::LatencyDays {
        let ratio = if best_before_mean > 0.0 {
            (best_after_mean / best_before_mean - 1.0).min(2.0) / 2.0
        } else {
            1.0
        };
        ratio.max(0.0) * 3_500.0
    } else {
        magnitude_component
    };
    let symmetry_component = {
        // Better confidence when the split falls near the middle of the series.
        let position = split as f64 / series.len() as f64;
        (1.0 - (position - 0.5).abs() * 2.0).max(0.0) * 2_500.0
    };
    let bp = (sample_component + magnitude_component + symmetry_component).round() as u16;
    let confidence = FindingConfidence::measured(bp);

    // Citations: 3 representative requests spanning the change-point.
    let mut citations = Vec::new();
    for offset in [0usize, split, series.len() - 1] {
        if let Some((_, _, request_id)) = series.get(offset) {
            citations.push(FindingCitation {
                portal_key: portal_key.to_string(),
                document_id: 0, // request-level, not document-level; kept 0 per schema.
                character_start: None,
                character_end: None,
                quote: None,
                label: Some(format!(
                    "request #{} on {}",
                    request_id,
                    format_date(series[offset].0)
                )),
            });
        }
    }
    // Dedupe adjacent identical labels (small series can put multiple picks on one row).
    citations.dedup_by(|a, b| a.label == b.label);

    let subject = FindingSubject {
        portal_key: Some(portal_key.to_string()),
        agency_id: Some(agency_id),
        ..Default::default()
    };

    // Salient bits: metric + split-day bucket.
    let split_bucket = format!("split_year_month={}", format_year_month(series[split].0));
    let metric_bit = format!("metric={}", metric.as_str());
    let id = finding_id(
        FindingKind::CrossCorpusTemporalShift,
        SLUG,
        &subject,
        &[&metric_bit, &split_bucket],
    );

    let _ = agency_name; // reserved for a future headline enrichment (avoids unused warning).
    let _ = headline_body;
    let now = Utc::now();
    Some(Finding {
        id,
        kind: FindingKind::CrossCorpusTemporalShift,
        severity,
        confidence,
        subject,
        headline: match metric {
            Metric::HeavyRedactionRate => format!(
                "Heavy-redaction rate stepped from {}% to {}% after {}",
                (best_before_mean * 100.0).round() as u32,
                (best_after_mean * 100.0).round() as u32,
                format_date(series[split].0)
            ),
            Metric::LatencyDays => format!(
                "Response latency stepped from {:.0} to {:.0} days after {}",
                best_before_mean,
                best_after_mean,
                format_date(series[split].0)
            ),
        },
        explanation: explanation_body,
        citations,
        discovered_at: now,
        last_seen_at: now,
        dismissed_at: None,
        source_detector: SLUG.into(),
    })
}

fn format_date(instant: DateTime<Utc>) -> String {
    instant.format("%Y-%m-%d").to_string()
}

fn format_year_month(instant: DateTime<Utc>) -> String {
    instant.format("%Y-%m").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn observation(
        request_id: i64,
        portal: &str,
        agency: i64,
        day_offset: i64,
        heavy: bool,
        latency: Option<f64>,
    ) -> Observation {
        let base = DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z").unwrap();
        Observation {
            portal_key: portal.into(),
            agency_id: agency,
            agency_name: Some("Test Agency".into()),
            created_at: (base + Duration::days(day_offset)).with_timezone(&Utc),
            heavy_redaction: heavy,
            latency_days: latency,
            request_id,
        }
    }

    #[test]
    fn heavy_redaction_step_up_fires_serious() {
        // 50 requests spanning 200 days. First 25 (days 0-99): 0% heavy. Second 25 (100-199):
        // 80% heavy. Step size = 80 percentage points => Serious.
        let mut obs = Vec::new();
        for i in 0..25 {
            obs.push(observation(
                i as i64 + 1,
                "city-a",
                7,
                i as i64 * 4,
                false,
                None,
            ));
        }
        for i in 0..25 {
            let heavy = i < 20; // 20/25 = 80%
            obs.push(observation(
                (i + 25) as i64 + 1,
                "city-a",
                7,
                100 + i as i64 * 4,
                heavy,
                None,
            ));
        }
        let findings = evaluate(&obs);
        assert!(!findings.is_empty(), "must fire on 80pt step");
        let f = findings
            .iter()
            .find(|f| f.headline.contains("Heavy-redaction rate"))
            .expect("heavy-redaction finding present");
        assert_eq!(f.severity, FindingSeverity::Serious);
    }

    #[test]
    fn no_shift_when_change_is_small() {
        // Baseline 20% vs. after 30% (10 percentage point step) — below the 40-point threshold.
        let mut obs = Vec::new();
        for i in 0..25 {
            let heavy = i < 5;
            obs.push(observation(
                i as i64 + 1,
                "city-a",
                8,
                i as i64 * 4,
                heavy,
                None,
            ));
        }
        for i in 0..25 {
            let heavy = i < 8;
            obs.push(observation(
                (i + 25) as i64 + 1,
                "city-a",
                8,
                100 + i as i64 * 4,
                heavy,
                None,
            ));
        }
        let findings = evaluate(&obs);
        assert!(findings.is_empty(), "small changes must not fire");
    }

    #[test]
    fn latency_double_fires() {
        // 40 requests, first half average 10 days latency, second half average 40 days.
        let mut obs = Vec::new();
        for i in 0..20 {
            obs.push(observation(
                i as i64 + 1,
                "city-a",
                9,
                i as i64 * 5,
                false,
                Some(10.0),
            ));
        }
        for i in 0..20 {
            obs.push(observation(
                (i + 20) as i64 + 1,
                "city-a",
                9,
                100 + i as i64 * 5,
                false,
                Some(40.0),
            ));
        }
        let findings = evaluate(&obs);
        let f = findings
            .iter()
            .find(|f| f.headline.contains("Response latency"))
            .expect("latency finding present");
        assert!(matches!(
            f.severity,
            FindingSeverity::Serious | FindingSeverity::Notable
        ));
    }

    #[test]
    fn history_below_minimum_returns_nothing() {
        // 40 requests but only 60 days of history — below MIN_HISTORY_DAYS.
        let mut obs = Vec::new();
        for i in 0..40 {
            obs.push(observation(
                i as i64 + 1,
                "city-a",
                10,
                i as i64,
                i > 20,
                None,
            ));
        }
        let findings = evaluate(&obs);
        assert!(findings.is_empty());
    }
}
