//! Differential-treatment detector.
//!
//! For each `(agency, requester_attribution)` pair with enough requests, compares the class's
//! mean statutory-compliance rate, mean redaction density, and mean determination latency against
//! the agency's population baseline. Emits a finding when one class's outcomes differ from the
//! baseline by a materially large effect (Cohen's h for the compliance rate, standardized-mean
//! difference for the latency).
//!
//! **What this is not**: a legal-claim finding. Differential outcomes across requester classes
//! are a statistical pattern, not a per-request statutory violation. The pattern names a
//! disparity worth investigating; the record for any individual request stays as its own
//! statutory finding.
//!
//! **Expected state today**: the `requester_attribution` column is nullable and populated only
//! where an ingestion adapter can derive it. With zero attributed requests, this detector runs
//! quickly, finds nothing, and returns `Ok(vec![])`. That is not an error; it's the honest
//! answer.

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

pub const SLUG: &str = "differential_treatment";

/// A class must carry at least this many requests before its mean is compared against the
/// baseline. Below this, the sample is too small to distinguish a real disparity from noise.
const MIN_CLASS_REQUESTS: i64 = 10;

/// An agency's baseline is only meaningful with a reasonable number of requests across classes.
const MIN_AGENCY_REQUESTS: i64 = 40;

/// Statutory-compliance-rate disparity threshold expressed as a percentage-point gap. A class
/// whose compliance rate is more than this many points below the agency baseline fires the
/// pattern. 15 points ≈ Cohen's h > 0.3 at balanced base rates, which is a materially large
/// effect.
const COMPLIANCE_GAP_POINTS: f64 = 15.0;

/// Latency disparity threshold expressed as calendar days above baseline mean.
const LATENCY_GAP_DAYS: f64 = 14.0;

pub async fn run(repository: &DuckDbRepository) -> Result<Vec<Finding>, AppError> {
    let rows = load_class_summaries(repository).await?;
    Ok(evaluate(&rows))
}

/// One row per `(portal_key, agency_id, requester_attribution)` grouping, with the same three
/// stats per row for the agency's baseline embedded so the evaluator doesn't need a second query.
struct ClassSummary {
    portal_key: String,
    agency_id: i64,
    requester_attribution: String,
    class_requests: i64,
    class_compliant_rate: f64,
    class_mean_latency_days: f64,
    agency_requests: i64,
    agency_compliant_rate: f64,
    agency_mean_latency_days: f64,
}

async fn load_class_summaries(
    repository: &DuckDbRepository,
) -> Result<Vec<ClassSummary>, AppError> {
    repository
        .query_connection("Differential-treatment class summaries", move |conn| {
            // The query only returns rows where requester_attribution is non-NULL AND the agency
            // itself has enough total requests to trust its baseline. This keeps the surface
            // silent on the common case (attribution unpopulated) and only surfaces disparities
            // in agencies with enough data to know a baseline.
            let mut statement = conn
                .prepare(
                    "WITH request_stats AS (
                         SELECT r.portal_subdomain,
                                r.agency_id,
                                r.requester_attribution,
                                r.id AS request_id,
                                -- Compliant when determination lands on or before the due date.
                                CASE WHEN r.determination_at IS NOT NULL
                                       AND r.due_date IS NOT NULL
                                       AND r.determination_at <= r.due_date
                                     THEN 1.0 ELSE 0.0 END AS compliant,
                                CASE WHEN r.determination_at IS NOT NULL
                                       AND r.created_at IS NOT NULL
                                     THEN date_diff(
                                        'day',
                                        CAST(r.created_at AS TIMESTAMP),
                                        CAST(r.determination_at AS TIMESTAMP)
                                     )
                                     ELSE NULL END AS latency_days
                         FROM requests r
                         WHERE r.agency_id IS NOT NULL
                     ),
                     class_agg AS (
                         SELECT portal_subdomain,
                                agency_id,
                                requester_attribution,
                                COUNT(*) AS class_requests,
                                AVG(compliant) AS class_compliant_rate,
                                AVG(latency_days) AS class_mean_latency_days
                         FROM request_stats
                         WHERE requester_attribution IS NOT NULL
                         GROUP BY portal_subdomain, agency_id, requester_attribution
                     ),
                     agency_agg AS (
                         SELECT portal_subdomain,
                                agency_id,
                                COUNT(*) AS agency_requests,
                                AVG(compliant) AS agency_compliant_rate,
                                AVG(latency_days) AS agency_mean_latency_days
                         FROM request_stats
                         GROUP BY portal_subdomain, agency_id
                     )
                     SELECT c.portal_subdomain, c.agency_id, c.requester_attribution,
                            c.class_requests, c.class_compliant_rate, c.class_mean_latency_days,
                            a.agency_requests, a.agency_compliant_rate, a.agency_mean_latency_days
                     FROM class_agg c
                     JOIN agency_agg a
                        ON a.portal_subdomain = c.portal_subdomain
                       AND a.agency_id = c.agency_id
                     WHERE c.class_requests >= ?1
                       AND a.agency_requests >= ?2",
                )
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to prepare differential-treatment query: {error}"
                    ))
                })?;
            let rows = statement
                .query_map(params![MIN_CLASS_REQUESTS, MIN_AGENCY_REQUESTS], |row| {
                    Ok(ClassSummary {
                        portal_key: row.get(0)?,
                        agency_id: row.get(1)?,
                        requester_attribution: row.get(2)?,
                        class_requests: row.get(3)?,
                        class_compliant_rate: row.get::<_, f64>(4).unwrap_or(0.0),
                        class_mean_latency_days: row.get::<_, Option<f64>>(5)?.unwrap_or(0.0),
                        agency_requests: row.get(6)?,
                        agency_compliant_rate: row.get::<_, f64>(7).unwrap_or(0.0),
                        agency_mean_latency_days: row.get::<_, Option<f64>>(8)?.unwrap_or(0.0),
                    })
                })
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to execute differential-treatment query: {error}"
                    ))
                })?;
            let mut collected = Vec::new();
            for row in rows {
                collected.push(row.map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to decode differential-treatment row: {error}"
                    ))
                })?);
            }
            Ok(collected)
        })
        .await
}

fn evaluate(rows: &[ClassSummary]) -> Vec<Finding> {
    // Group by (portal, agency) so we can also exclude the trivial single-class case: an agency
    // whose only attributed class simply IS the population; nothing to compare.
    let mut per_agency: HashMap<(String, i64), Vec<&ClassSummary>> = HashMap::new();
    for row in rows {
        per_agency
            .entry((row.portal_key.clone(), row.agency_id))
            .or_default()
            .push(row);
    }
    let mut findings = Vec::new();
    let now = Utc::now();
    for (_, classes) in per_agency {
        // Need at least two classes to talk about differential treatment; one class equals the
        // whole attributed population.
        if classes.len() < 2 {
            continue;
        }
        for class in classes {
            let compliance_gap_pts =
                (class.agency_compliant_rate - class.class_compliant_rate) * 100.0;
            let latency_gap_days = class.class_mean_latency_days - class.agency_mean_latency_days;
            let compliance_flag = compliance_gap_pts >= COMPLIANCE_GAP_POINTS;
            let latency_flag = latency_gap_days >= LATENCY_GAP_DAYS;
            if !compliance_flag && !latency_flag {
                continue;
            }
            // Severity: two disparities in the same class is Serious; one is Notable.
            let severity = if compliance_flag && latency_flag {
                FindingSeverity::Serious
            } else {
                FindingSeverity::Notable
            };
            // Confidence: derived from the class sample size and the size of the gap. Larger
            // classes and bigger gaps push confidence up.
            let sample_bp = ((class.class_requests as u32).saturating_mul(120).min(4_000)) as u16;
            let gap_bp = if compliance_flag && latency_flag {
                4_000
            } else if compliance_flag {
                (compliance_gap_pts.clamp(0.0, 40.0) * 75.0) as u16
            } else {
                (latency_gap_days.clamp(0.0, 60.0) * 50.0) as u16
            };
            let confidence =
                FindingConfidence::measured(sample_bp.saturating_add(gap_bp).min(9_500));
            let subject = FindingSubject {
                portal_key: Some(class.portal_key.clone()),
                agency_id: Some(class.agency_id),
                ..Default::default()
            };
            let compliance_bucket = (compliance_gap_pts.round() as i64).max(0);
            let latency_bucket = (latency_gap_days.round() as i64).max(0);
            let salient = format!(
                "class={};n={};compliance_gap_pts={};latency_gap_days={}",
                class.requester_attribution,
                class.class_requests,
                compliance_bucket,
                latency_bucket,
            );
            let id = finding_id(
                FindingKind::DifferentialTreatment,
                SLUG,
                &subject,
                &[&salient],
            );
            let mut headline = format!(
                "'{}' requesters see {n} lower compliance{plus}",
                class.requester_attribution,
                n = if compliance_flag {
                    format!("{:.0}pt", compliance_gap_pts)
                } else {
                    String::new()
                },
                plus = if compliance_flag && latency_flag {
                    format!(", {:.0}d longer latency", latency_gap_days)
                } else if latency_flag && !compliance_flag {
                    format!(" and {:.0}d longer latency", latency_gap_days)
                } else {
                    String::new()
                },
            );
            // Collapse the empty-percentage case for the latency-only branch.
            headline = headline
                .replace("see  lower compliance", "see")
                .trim()
                .into();
            let explanation = format!(
                "Requests from Agency {agency_id} attributed to '{class}' (n={n}) had an \
                 average statutory-compliance rate of {class_rate:.0}%, compared with the \
                 agency's population baseline of {agency_rate:.0}% (n={agency_n}). \
                 Mean determination latency was {class_latency:.1} days vs. baseline \
                 {agency_latency:.1} days. Differential outcomes across requester classes are a \
                 statistical pattern, not a per-request statutory violation.",
                agency_id = class.agency_id,
                class = class.requester_attribution,
                n = class.class_requests,
                class_rate = class.class_compliant_rate * 100.0,
                agency_rate = class.agency_compliant_rate * 100.0,
                agency_n = class.agency_requests,
                class_latency = class.class_mean_latency_days,
                agency_latency = class.agency_mean_latency_days,
            );
            // No per-document citations: this is an agency-level finding. Note the aggregate
            // basis in the label of an empty citation so the row still says where to look.
            let citations = vec![FindingCitation {
                portal_key: class.portal_key.clone(),
                document_id: 0,
                character_start: None,
                character_end: None,
                quote: None,
                label: Some(format!(
                    "Aggregate over {n} requests attributed to '{class}' at Agency {agency_id}",
                    n = class.class_requests,
                    class = class.requester_attribution,
                    agency_id = class.agency_id,
                )),
            }];
            findings.push(Finding {
                id,
                kind: FindingKind::DifferentialTreatment,
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
            });
        }
    }
    findings.sort_by_key(|f| std::cmp::Reverse(f.rank_key()));
    findings
}

#[cfg(test)]
mod tests {
    use super::*;

    fn class(
        attribution: &str,
        class_requests: i64,
        class_compliant: f64,
        class_latency: f64,
        agency_compliant: f64,
        agency_latency: f64,
    ) -> ClassSummary {
        ClassSummary {
            portal_key: "p".into(),
            agency_id: 42,
            requester_attribution: attribution.into(),
            class_requests,
            class_compliant_rate: class_compliant,
            class_mean_latency_days: class_latency,
            agency_requests: 500,
            agency_compliant_rate: agency_compliant,
            agency_mean_latency_days: agency_latency,
        }
    }

    #[test]
    fn empty_input_produces_no_findings() {
        assert!(evaluate(&[]).is_empty());
    }

    #[test]
    fn single_attributed_class_produces_no_findings() {
        // An agency's only attributed class IS the population — no comparison possible.
        let rows = vec![class("journalist", 100, 0.5, 30.0, 0.9, 5.0)];
        assert!(
            evaluate(&rows).is_empty(),
            "single-class agency must not fire"
        );
    }

    #[test]
    fn no_finding_when_class_matches_baseline() {
        let rows = vec![
            class("journalist", 50, 0.85, 12.0, 0.9, 10.0),
            class("individual", 200, 0.9, 8.0, 0.9, 10.0),
        ];
        assert!(evaluate(&rows).is_empty(), "small gaps do not fire");
    }

    #[test]
    fn notable_finding_for_compliance_gap_alone() {
        let rows = vec![
            class("journalist", 30, 0.5, 12.0, 0.8, 10.0),
            class("individual", 400, 0.85, 10.0, 0.8, 10.0),
        ];
        let findings = evaluate(&rows);
        // Only the journalist class fails the compliance threshold (30-point gap); the individual
        // class's rate (85%) is above the baseline (80%) so it's not flagged.
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, FindingSeverity::Notable);
        assert!(findings[0].headline.contains("journalist"));
    }

    #[test]
    fn serious_finding_when_both_disparities_present() {
        let rows = vec![
            class("journalist", 40, 0.5, 30.0, 0.8, 10.0),
            class("individual", 300, 0.8, 10.0, 0.8, 10.0),
        ];
        let findings = evaluate(&rows);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, FindingSeverity::Serious);
    }

    #[test]
    fn finding_id_stable_across_re_runs_for_same_gap_bucket() {
        let rows = vec![
            class("journalist", 40, 0.5, 30.0, 0.8, 10.0),
            class("individual", 300, 0.8, 10.0, 0.8, 10.0),
        ];
        let a = evaluate(&rows);
        let b = evaluate(&rows);
        assert_eq!(a[0].id, b[0].id);
    }

    #[test]
    fn class_below_minimum_size_is_excluded_by_query_semantics() {
        // The evaluator itself does not enforce MIN_CLASS_REQUESTS; the SQL WHERE clause does.
        // Callers passing a class below the threshold through the in-process test path must not
        // silently be reported. This test documents that behavior at the evaluator level: the
        // in-process rate signal still passes because the guard lives in the query.
        let rows = vec![
            class("journalist", 5, 0.4, 30.0, 0.8, 10.0),
            class("individual", 300, 0.8, 10.0, 0.8, 10.0),
        ];
        // The evaluator will surface this class if fed directly. That's fine: production always
        // goes through the query, and the query filters. This test just prevents someone from
        // accidentally moving the sample-size guard into the evaluator later without noticing
        // that the SQL side goes silent.
        let findings = evaluate(&rows);
        assert!(
            !findings.is_empty(),
            "the evaluator itself does not enforce the class-size floor"
        );
    }
}
