//! Just-in-time-compliance detector.
//!
//! Surfaces agencies whose closed requests cluster in the final 24 hours before the statutory
//! deadline at a rate materially above the population base rate. Every closure in the cluster
//! was within the deadline — that is what makes this a pattern signal, not a legal claim. A
//! deadline-hitting practice is not a statutory violation; it is behavioral evidence about how
//! an office prioritises work, which is what the "problems swept under the rug" mandate cares
//! about.
//!
//! **What this is not**: a legal-claim finding. `is_legal_claim = false`. The prose in emitted
//! findings avoids "gaming" / "manipulation" — it states the rate factually and lets the reader
//! draw their own conclusion.

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

pub const SLUG: &str = "just_in_time_compliance";

/// Minimum closed-request count per agency before we compute a rate. Below this the estimate is
/// too noisy to publish.
const MIN_CLOSED_REQUESTS: i64 = 30;

/// A closure is "at the wire" when it lands in this many hours before the deadline (positive =
/// before the deadline).
const AT_THE_WIRE_HOURS: f64 = 24.0;

/// Trigger threshold: the fraction of at-the-wire closures at or above this level fires.
const TRIGGER_FRACTION: f64 = 0.40;

/// Only agencies whose at-the-wire fraction exceeds the population baseline by at least this
/// gap fire. Prevents a global habit from misattributing to individual agencies.
const MIN_LIFT_OVER_BASELINE: f64 = 0.15;

/// Cap the number of representative citations pulled per firing agency.
const CITATIONS_PER_FINDING: i64 = 5;

/// One agency's roll-up. `at_the_wire_count` is the number of closed requests that landed in
/// the [0, AT_THE_WIRE_HOURS] window before the deadline.
struct AgencyStats {
    portal_key: String,
    agency_id: i64,
    agency_name: Option<String>,
    closed_count: i64,
    at_the_wire_count: i64,
}

impl AgencyStats {
    fn fraction_at_the_wire(&self) -> f64 {
        if self.closed_count == 0 {
            return 0.0;
        }
        self.at_the_wire_count as f64 / self.closed_count as f64
    }
}

pub async fn run(repository: &DuckDbRepository) -> Result<Vec<Finding>, AppError> {
    let stats = load_stats(repository).await?;
    if stats.is_empty() {
        return Ok(Vec::new());
    }
    let baseline = population_baseline(&stats);
    let mut findings = Vec::new();
    for stat in &stats {
        if stat.closed_count < MIN_CLOSED_REQUESTS {
            continue;
        }
        let fraction = stat.fraction_at_the_wire();
        if fraction < TRIGGER_FRACTION {
            continue;
        }
        let lift = fraction - baseline;
        if lift < MIN_LIFT_OVER_BASELINE {
            continue;
        }
        let citations = load_representative_citations(repository, stat).await?;
        if citations.is_empty() {
            // A firing agency with no cite-able rows is a data gap, not a finding. Skip rather
            // than emit an accusation the reader cannot verify.
            continue;
        }
        findings.push(build_finding(stat, fraction, baseline, lift, citations));
    }
    Ok(findings)
}

async fn load_stats(repository: &DuckDbRepository) -> Result<Vec<AgencyStats>, AppError> {
    repository
        .query_connection("Just-in-time-compliance aggregation", move |conn| {
            // Compute per-agency counts + at-the-wire hits in one query. Uses the request's own
            // `due_date` column as the deadline; the statute-aware deadline lives in
            // analysis/statute.rs but that path builds its predicates around the extension
            // model, which is CPRA-hardcoded. Using `due_date` keeps this detector jurisdiction-
            // agnostic; every ingested portal populates it.
            let mut statement = conn
                .prepare(
                    "SELECT r.portal_subdomain,
                            r.agency_id,
                            a.name AS agency_name,
                            COUNT(*) AS closed_count,
                            SUM(
                                CASE
                                    WHEN r.determination_at IS NOT NULL
                                     AND r.due_date IS NOT NULL
                                     AND date_diff('hour', CAST(r.determination_at AS TIMESTAMP),
                                                   CAST(r.due_date AS TIMESTAMP)) BETWEEN 0 AND ?1
                                    THEN 1 ELSE 0 END
                            ) AS at_the_wire_count
                     FROM requests r
                     LEFT JOIN agencies a
                        ON a.id = r.agency_id
                       AND a.portal_subdomain = r.portal_subdomain
                     WHERE r.determination_at IS NOT NULL
                       AND r.due_date IS NOT NULL
                       AND r.agency_id IS NOT NULL
                     GROUP BY r.portal_subdomain, r.agency_id, a.name",
                )
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to prepare just-in-time aggregation: {error}"
                    ))
                })?;
            let rows = statement
                .query_map(params![AT_THE_WIRE_HOURS as i64], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, Option<i64>>(4)?,
                    ))
                })
                .map_err(|error| {
                    AppError::Storage(format!("Failed to run just-in-time aggregation: {error}"))
                })?;
            let mut out = Vec::new();
            for row in rows {
                let (portal_key, agency_id, agency_name, closed_count, at_the_wire_count) = row
                    .map_err(|error| {
                        AppError::Storage(format!(
                            "Failed to decode just-in-time aggregation row: {error}"
                        ))
                    })?;
                out.push(AgencyStats {
                    portal_key,
                    agency_id,
                    agency_name,
                    closed_count,
                    at_the_wire_count: at_the_wire_count.unwrap_or(0),
                });
            }
            Ok(out)
        })
        .await
}

/// Baseline = pooled fraction across every agency in the dataset. Weighted by request count so a
/// giant agency isn't outweighed by a tiny one contributing an atypical rate.
fn population_baseline(stats: &[AgencyStats]) -> f64 {
    let total_closed: i64 = stats.iter().map(|s| s.closed_count).sum();
    if total_closed == 0 {
        return 0.0;
    }
    let total_wire: i64 = stats.iter().map(|s| s.at_the_wire_count).sum();
    total_wire as f64 / total_closed as f64
}

async fn load_representative_citations(
    repository: &DuckDbRepository,
    stat: &AgencyStats,
) -> Result<Vec<FindingCitation>, AppError> {
    let portal_key = stat.portal_key.clone();
    let agency_id = stat.agency_id;
    repository
        .query_connection("Just-in-time citation sample", move |conn| {
            let mut statement = conn
                .prepare(
                    "WITH scored AS (
                        SELECT r.id AS request_id,
                               r.portal_subdomain,
                               r.title,
                               date_diff('hour', CAST(r.determination_at AS TIMESTAMP),
                                         CAST(r.due_date AS TIMESTAMP)) AS hours_before_deadline
                        FROM requests r
                        WHERE r.portal_subdomain = ?1
                          AND r.agency_id = ?2
                          AND r.determination_at IS NOT NULL
                          AND r.due_date IS NOT NULL
                          AND date_diff('hour', CAST(r.determination_at AS TIMESTAMP),
                                        CAST(r.due_date AS TIMESTAMP)) BETWEEN 0 AND ?3
                     )
                     SELECT s.request_id, s.portal_subdomain, s.title, s.hours_before_deadline,
                            (SELECT d.id FROM documents d
                              WHERE d.request_id = s.request_id
                                AND d.portal_subdomain = s.portal_subdomain
                                AND LOWER(TRIM(d.visibility)) = 'public'
                              ORDER BY d.id ASC LIMIT 1) AS doc_id
                     FROM scored s
                     ORDER BY s.hours_before_deadline ASC
                     LIMIT ?4",
                )
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to prepare just-in-time citation query: {error}"
                    ))
                })?;
            let rows = statement
                .query_map(
                    params![
                        portal_key,
                        agency_id,
                        AT_THE_WIRE_HOURS as i64,
                        CITATIONS_PER_FINDING
                    ],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, Option<String>>(2)?,
                            row.get::<_, Option<i64>>(3)?,
                            row.get::<_, Option<i64>>(4)?,
                        ))
                    },
                )
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to run just-in-time citation query: {error}"
                    ))
                })?;
            let mut out = Vec::new();
            for row in rows {
                let (request_id, portal_key, title, hours_before_deadline, doc_id) =
                    row.map_err(|error| {
                        AppError::Storage(format!(
                            "Failed to decode just-in-time citation row: {error}"
                        ))
                    })?;
                let label = match (title, hours_before_deadline) {
                    (Some(t), Some(h)) => Some(format!("{t} — closed {h}h before deadline")),
                    (Some(t), None) => Some(t),
                    (None, Some(h)) => Some(format!(
                        "Request {request_id} — closed {h}h before deadline"
                    )),
                    (None, None) => Some(format!("Request {request_id}")),
                };
                out.push(FindingCitation {
                    portal_key,
                    // A request that closed with no released document is still evidence; its
                    // label names the request.
                    document_id: doc_id.unwrap_or(super::REQUEST_LEVEL_DOCUMENT_ID),
                    character_start: None,
                    character_end: None,
                    quote: None,
                    label,
                });
            }
            Ok(out)
        })
        .await
}

fn build_finding(
    stat: &AgencyStats,
    fraction: f64,
    baseline: f64,
    lift: f64,
    citations: Vec<FindingCitation>,
) -> Finding {
    let pct = (fraction * 100.0).round() as u32;
    let baseline_pct = (baseline * 100.0).round() as u32;
    let lift_pct = (lift * 100.0).round() as u32;

    // Severity: single-tier for this detector — Serious in every case. The threshold gates
    // ensured the pattern is materially above baseline; there is no reader-useful distinction
    // between 41% at the wire and 65% for the outcome we're surfacing. Keeping it single-tier
    // avoids inventing a distinction the numbers don't support.
    let severity = FindingSeverity::Serious;

    // Confidence: scaled by (sample size, effect size). Both are needed — a giant lift on a
    // small sample is noisy and a tiny lift on a huge sample is unremarkable.
    let sample_component = ((stat.closed_count as f64 / 200.0).min(1.0)) * 4_000.0;
    let effect_component = ((lift / 0.40).min(1.0)) * 6_000.0;
    let bp = (sample_component + effect_component).round() as u16;
    let confidence = FindingConfidence::measured(bp);

    let subject = FindingSubject {
        portal_key: Some(stat.portal_key.clone()),
        agency_id: Some(stat.agency_id),
        ..Default::default()
    };
    // Salient bits: bucket the fraction into 10-point bins so a re-run with essentially the
    // same rate keeps the same id, and only a real change in behavior recomputes to a new one.
    let pct_bucket = format!("wire_pct_bucket={}", pct / 10);
    let sample_bucket = format!("sample_bucket={}", stat.closed_count / 100);
    let id = finding_id(
        FindingKind::JustInTimeCompliance,
        SLUG,
        &subject,
        &[&pct_bucket, &sample_bucket],
    );

    let display_name = stat
        .agency_name
        .clone()
        .unwrap_or_else(|| format!("Agency {}", stat.agency_id));
    let headline = format!(
        "{display_name} closes {pct}% of requests within the final 24 hours before the deadline"
    );
    let explanation = format!(
        "Across {closed_count} closed requests, {at_the_wire_count} ({pct}%) closed with 24 hours \
         or less remaining on the statutory clock. The population baseline for the same measure is \
         {baseline_pct}% — a {lift_pct}-point lift. Every closure was within the deadline; this is \
         a behavioral pattern about timing, not a statutory violation.",
        closed_count = stat.closed_count,
        at_the_wire_count = stat.at_the_wire_count,
    );

    let now = Utc::now();
    Finding {
        id,
        kind: FindingKind::JustInTimeCompliance,
        severity,
        confidence,
        subject,
        headline,
        explanation,
        citations,
        discovered_at: now,
        last_seen_at: now,
        dismissed_at: None,
        source_detector: SLUG.into(),
    }
}

// Deterministic sort helper used by tests / callers that inspect the raw vec.
#[cfg(test)]
fn sort_findings_for_inspection(mut findings: Vec<Finding>) -> Vec<Finding> {
    findings.sort_by_key(|f| f.subject.agency_id);
    findings
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stat(agency_id: i64, closed: i64, at_wire: i64) -> AgencyStats {
        AgencyStats {
            portal_key: "test".into(),
            agency_id,
            agency_name: Some(format!("Agency {agency_id}")),
            closed_count: closed,
            at_the_wire_count: at_wire,
        }
    }

    #[test]
    fn population_baseline_pools_across_agencies() {
        let stats = vec![stat(1, 100, 40), stat(2, 100, 20)];
        let baseline = population_baseline(&stats);
        // 60/200 = 0.30
        assert!((baseline - 0.30).abs() < 1e-9);
    }

    #[test]
    fn population_baseline_zero_when_no_data() {
        let stats: Vec<AgencyStats> = Vec::new();
        assert_eq!(population_baseline(&stats), 0.0);
    }

    #[test]
    fn agency_below_min_sample_does_not_fire() {
        // 29 closed requests, all at the wire. Below MIN_CLOSED_REQUESTS.
        let stats = [stat(1, 29, 29)];
        let baseline = 0.10;
        let s = &stats[0];
        assert!(s.closed_count < MIN_CLOSED_REQUESTS);
        // We don't call `run` here (needs a repo); we validate the branch that skips.
        assert!(s.fraction_at_the_wire() >= TRIGGER_FRACTION);
        // The predicate that gates emission:
        let would_gate = s.closed_count < MIN_CLOSED_REQUESTS
            || s.fraction_at_the_wire() < TRIGGER_FRACTION
            || s.fraction_at_the_wire() - baseline < MIN_LIFT_OVER_BASELINE;
        assert!(would_gate);
    }

    #[test]
    fn agency_below_trigger_fraction_does_not_fire() {
        // 100 closed, 30 at wire (30%) — below TRIGGER_FRACTION (40%).
        let s = stat(1, 100, 30);
        let would_gate =
            s.closed_count < MIN_CLOSED_REQUESTS || s.fraction_at_the_wire() < TRIGGER_FRACTION;
        assert!(would_gate);
    }

    #[test]
    fn agency_at_baseline_does_not_fire() {
        // 100 closed, 45 at wire (45%). Baseline is also 45%. Lift = 0, below MIN_LIFT.
        let s = stat(1, 100, 45);
        let baseline = 0.45;
        assert!(s.fraction_at_the_wire() >= TRIGGER_FRACTION);
        assert!(s.fraction_at_the_wire() - baseline < MIN_LIFT_OVER_BASELINE);
    }

    #[test]
    fn firing_agency_produces_a_serious_finding_with_scaled_confidence() {
        let s = stat(1, 200, 120); // 60% at wire
        let baseline = 0.20;
        let fraction = s.fraction_at_the_wire();
        let lift = fraction - baseline;
        assert!(fraction >= TRIGGER_FRACTION);
        assert!(lift >= MIN_LIFT_OVER_BASELINE);
        let citations = vec![FindingCitation {
            portal_key: "test".into(),
            document_id: 42,
            character_start: None,
            character_end: None,
            quote: None,
            label: Some("test row".into()),
        }];
        let finding = build_finding(&s, fraction, baseline, lift, citations);
        assert_eq!(finding.kind, FindingKind::JustInTimeCompliance);
        assert_eq!(finding.severity, FindingSeverity::Serious);
        // Sample maxed (200 hits saturation at 200), effect scales toward saturation too.
        let bp = finding.confidence.stored_basis_points().unwrap();
        assert!(
            bp >= 5_000,
            "confidence should be at least mid-range for this signal"
        );
        assert_eq!(finding.subject.agency_id, Some(1));
        assert!(finding.headline.contains("60%"));
        assert!(!finding.explanation.contains("gaming"));
        assert!(!finding.explanation.contains("manipulation"));
    }

    #[test]
    fn same_agency_same_bucket_produces_stable_id_across_runs() {
        let s = stat(1, 200, 120);
        let baseline = 0.20;
        let fraction = s.fraction_at_the_wire();
        let lift = fraction - baseline;
        let citations = vec![FindingCitation {
            portal_key: "test".into(),
            document_id: 1,
            character_start: None,
            character_end: None,
            quote: None,
            label: None,
        }];
        let a = build_finding(&s, fraction, baseline, lift, citations.clone());
        let b = build_finding(&s, fraction, baseline, lift, citations);
        assert_eq!(a.id, b.id);
    }

    #[test]
    fn sort_helper_orders_by_agency() {
        // Just to make sure the sort helper the tests use is stable and predictable.
        let citations = vec![FindingCitation {
            portal_key: "test".into(),
            document_id: 1,
            character_start: None,
            character_end: None,
            quote: None,
            label: None,
        }];
        let mut input = Vec::new();
        for id in [3, 1, 2] {
            let s = stat(id, 200, 120);
            input.push(build_finding(&s, 0.6, 0.2, 0.4, citations.clone()));
        }
        let sorted = sort_findings_for_inspection(input);
        assert_eq!(
            sorted
                .iter()
                .map(|f| f.subject.agency_id.unwrap())
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
    }
}
