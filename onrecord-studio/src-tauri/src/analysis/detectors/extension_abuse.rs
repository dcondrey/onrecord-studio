//! Extension-abuse detector.
//!
//! Surfaces agencies whose statutory extensions are overwhelmingly concentrated on a single
//! reason. A single valid extension is legitimate; a systemic pattern of invoking the same
//! ground on the great majority of extended requests is a pretext signal the app should surface
//! without an analyst having to notice.
//!
//! **What this is not**: a legal claim. `is_legal_claim = false`. The prose in emitted findings
//! is factual — an extension reason and a rate. It does not label any specific extension as
//! defective; that is the timeline detector's job, and its findings live under
//! `TimelineAnomalyMarker::StatutoryExtensionDefective`.

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

pub const SLUG: &str = "extension_abuse";

/// Minimum count of extended requests per agency before we compute per-reason fractions.
const MIN_EXTENDED_REQUESTS: i64 = 20;

/// Minimum absolute count of a single reason before it fires.
const MIN_REASON_COUNT: i64 = 15;

/// Trigger threshold: fraction of an agency's extensions on one reason at or above this fires.
const TRIGGER_FRACTION: f64 = 0.70;

/// Cap the number of representative citations pulled per firing agency/reason pair.
const CITATIONS_PER_FINDING: i64 = 5;

/// One (agency, reason) aggregate.
struct ReasonStats {
    portal_key: String,
    agency_id: i64,
    agency_name: Option<String>,
    reason: String,
    reason_count: i64,
    agency_total: i64,
}

impl ReasonStats {
    fn fraction(&self) -> f64 {
        if self.agency_total == 0 {
            return 0.0;
        }
        self.reason_count as f64 / self.agency_total as f64
    }
}

pub async fn run(repository: &DuckDbRepository) -> Result<Vec<Finding>, AppError> {
    let stats = load_stats(repository).await?;
    let mut findings = Vec::new();
    for stat in &stats {
        if stat.agency_total < MIN_EXTENDED_REQUESTS {
            continue;
        }
        if stat.reason_count < MIN_REASON_COUNT {
            continue;
        }
        let fraction = stat.fraction();
        if fraction < TRIGGER_FRACTION {
            continue;
        }
        let citations = load_representative_citations(repository, stat).await?;
        if citations.is_empty() {
            // Aggregate says N extensions on this ground, but we could not cite one. That's a
            // schema/join gap, not a finding to surface as fact.
            continue;
        }
        findings.push(build_finding(stat, fraction, citations));
    }
    Ok(findings)
}

async fn load_stats(repository: &DuckDbRepository) -> Result<Vec<ReasonStats>, AppError> {
    repository
        .query_connection("Extension-abuse per-reason aggregation", move |conn| {
            // One aggregation over statutory_extensions joined to requests for agency identity.
            // The window function gives us each (agency, reason) count alongside the agency
            // total, which is what the fraction calculation needs.
            let mut statement = conn
                .prepare(
                    "WITH per_reason AS (
                        SELECT e.portal_subdomain,
                               r.agency_id,
                               e.reason,
                               COUNT(*) AS reason_count,
                               SUM(COUNT(*)) OVER (
                                   PARTITION BY e.portal_subdomain, r.agency_id
                               ) AS agency_total
                        FROM statutory_extensions e
                        JOIN requests r
                          ON r.id = e.request_id
                         AND r.portal_subdomain = e.portal_subdomain
                        WHERE r.agency_id IS NOT NULL
                        GROUP BY e.portal_subdomain, r.agency_id, e.reason
                     )
                     SELECT p.portal_subdomain,
                            p.agency_id,
                            a.name,
                            p.reason,
                            p.reason_count,
                            p.agency_total
                     FROM per_reason p
                     LEFT JOIN agencies a
                        ON a.id = p.agency_id
                       AND a.portal_subdomain = p.portal_subdomain",
                )
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to prepare extension-abuse aggregation: {error}"
                    ))
                })?;
            let rows = statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, i64>(5)?,
                    ))
                })
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to run extension-abuse aggregation: {error}"
                    ))
                })?;
            let mut out = Vec::new();
            for row in rows {
                let (portal_key, agency_id, agency_name, reason, reason_count, agency_total) = row
                    .map_err(|error| {
                        AppError::Storage(format!(
                            "Failed to decode extension-abuse aggregation row: {error}"
                        ))
                    })?;
                out.push(ReasonStats {
                    portal_key,
                    agency_id,
                    agency_name,
                    reason,
                    reason_count,
                    agency_total,
                });
            }
            Ok(out)
        })
        .await
}

async fn load_representative_citations(
    repository: &DuckDbRepository,
    stat: &ReasonStats,
) -> Result<Vec<FindingCitation>, AppError> {
    let portal_key = stat.portal_key.clone();
    let agency_id = stat.agency_id;
    let reason = stat.reason.clone();
    repository
        .query_connection("Extension-abuse citation sample", move |conn| {
            let mut statement = conn
                .prepare(
                    "SELECT e.request_id,
                            e.portal_subdomain,
                            e.notice_at,
                            r.title,
                            (SELECT d.id FROM documents d
                              WHERE d.request_id = e.request_id
                                AND d.portal_subdomain = e.portal_subdomain
                                AND LOWER(TRIM(d.visibility)) = 'public'
                              ORDER BY d.id ASC LIMIT 1) AS doc_id
                     FROM statutory_extensions e
                     JOIN requests r
                       ON r.id = e.request_id
                      AND r.portal_subdomain = e.portal_subdomain
                     WHERE e.portal_subdomain = ?1
                       AND r.agency_id = ?2
                       AND e.reason = ?3
                     ORDER BY e.notice_at DESC
                     LIMIT ?4",
                )
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to prepare extension-abuse citation query: {error}"
                    ))
                })?;
            let rows = statement
                .query_map(
                    params![portal_key, agency_id, reason, CITATIONS_PER_FINDING],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, Option<String>>(3)?,
                            row.get::<_, Option<i64>>(4)?,
                        ))
                    },
                )
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to run extension-abuse citation query: {error}"
                    ))
                })?;
            let mut out = Vec::new();
            for row in rows {
                let (request_id, portal_key, notice_at, title, doc_id) = row.map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to decode extension-abuse citation row: {error}"
                    ))
                })?;
                let short_notice = notice_at
                    .split('T')
                    .next()
                    .unwrap_or(&notice_at)
                    .to_string();
                let label = match title {
                    Some(t) => Some(format!("{t} — extension noticed {short_notice}")),
                    None => Some(format!(
                        "Request {request_id} — extension noticed {short_notice}"
                    )),
                };
                out.push(FindingCitation {
                    portal_key,
                    // As with the just-in-time detector: fall back to request_id when no
                    // released document is attached, so a request with zero visible documents
                    // is not silently omitted.
                    document_id: doc_id.unwrap_or(request_id),
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

fn build_finding(stat: &ReasonStats, fraction: f64, citations: Vec<FindingCitation>) -> Finding {
    let pct = (fraction * 100.0).round() as u32;

    // Severity: single-tier Serious. Threshold gates ensured this is >=70% concentration on ≥15
    // extensions; the reader-useful distinction is already made by the confidence score below.
    let severity = FindingSeverity::Serious;

    // Confidence: scaled by (sample size, concentration). A 70% concentration on 15 extensions
    // is weaker than a 90% concentration on 100.
    let sample_component = ((stat.agency_total as f64 / 100.0).min(1.0)) * 4_000.0;
    let concentration_component =
        ((fraction - TRIGGER_FRACTION) / (1.0 - TRIGGER_FRACTION)).clamp(0.0, 1.0) * 6_000.0;
    let bp = (sample_component + concentration_component).round() as u16;
    let confidence = FindingConfidence::measured(bp);

    let subject = FindingSubject {
        portal_key: Some(stat.portal_key.clone()),
        agency_id: Some(stat.agency_id),
        ..Default::default()
    };
    let pct_bucket = format!("pct_bucket={}", pct / 10);
    let sample_bucket = format!("sample_bucket={}", stat.agency_total / 25);
    let reason_bit = format!("reason={}", stat.reason);
    let id = finding_id(
        FindingKind::ExtensionAbuse,
        SLUG,
        &subject,
        &[&reason_bit, &pct_bucket, &sample_bucket],
    );

    let display_name = stat
        .agency_name
        .clone()
        .unwrap_or_else(|| format!("Agency {}", stat.agency_id));
    let headline = format!(
        "{display_name} invoked {reason} on {pct}% of extended requests",
        reason = stat.reason,
    );
    let explanation = format!(
        "{reason_count} of {agency_total} extensions ({pct}%) cited the {reason} ground. \
         A single valid extension is legitimate; concentration at this level across an agency's \
         extended requests is a pattern signal about whether that ground is being used as \
         individualized justification or applied as a default. Whether any individual extension \
         is defective is a separate question — see per-request extension findings in the \
         timeline for those.",
        reason_count = stat.reason_count,
        agency_total = stat.agency_total,
        reason = stat.reason,
    );

    let now = Utc::now();
    Finding {
        id,
        kind: FindingKind::ExtensionAbuse,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn stat(reason: &str, reason_count: i64, agency_total: i64) -> ReasonStats {
        ReasonStats {
            portal_key: "test".into(),
            agency_id: 1,
            agency_name: Some("Records Bureau".into()),
            reason: reason.into(),
            reason_count,
            agency_total,
        }
    }

    #[test]
    fn agency_below_min_extended_requests_would_not_fire() {
        let s = stat("VOLUMINOUS_RECORDS", 15, 19); // 15 hits, but only 19 total (below MIN_EXTENDED_REQUESTS=20)
        assert!(s.agency_total < MIN_EXTENDED_REQUESTS);
    }

    #[test]
    fn concentration_below_trigger_fraction_would_not_fire() {
        // 60% concentration — below TRIGGER_FRACTION (70%).
        let s = stat("VOLUMINOUS_RECORDS", 18, 30);
        assert!(s.fraction() < TRIGGER_FRACTION);
    }

    #[test]
    fn absolute_count_below_minimum_would_not_fire() {
        // 100% concentration on the reason, but only 10 hits — below MIN_REASON_COUNT (15).
        let s = stat("VOLUMINOUS_RECORDS", 10, 10);
        assert_eq!(s.fraction(), 1.0);
        assert!(s.reason_count < MIN_REASON_COUNT);
    }

    #[test]
    fn firing_agency_reason_produces_finding_with_scaled_confidence_and_stable_id() {
        // 40 total extensions, 32 on the same reason = 80% — comfortably firing.
        let s = stat("INTERAGENCY_CONSULTATION", 32, 40);
        assert!(s.fraction() >= TRIGGER_FRACTION);
        assert!(s.reason_count >= MIN_REASON_COUNT);
        assert!(s.agency_total >= MIN_EXTENDED_REQUESTS);
        let citations = vec![FindingCitation {
            portal_key: "test".into(),
            document_id: 42,
            character_start: None,
            character_end: None,
            quote: None,
            label: Some("test row".into()),
        }];
        let a = build_finding(&s, s.fraction(), citations.clone());
        let b = build_finding(&s, s.fraction(), citations);
        assert_eq!(a.id, b.id, "same aggregate must produce the same id");
        assert_eq!(a.kind, FindingKind::ExtensionAbuse);
        assert_eq!(a.severity, FindingSeverity::Serious);
        assert_eq!(a.subject.agency_id, Some(1));
        assert!(a.headline.contains("INTERAGENCY_CONSULTATION"));
        assert!(a.headline.contains("80%"));
        // Explanation must reference both raw and pct.
        assert!(a.explanation.contains("32"));
        assert!(a.explanation.contains("40"));
        assert!(!a.explanation.to_ascii_lowercase().contains("gaming"));
        assert!(!a.explanation.to_ascii_lowercase().contains("manipulation"));
    }

    #[test]
    fn confidence_grows_with_sample_size_and_concentration() {
        let low = build_finding(
            &stat("VOLUMINOUS_RECORDS", 15, 20),
            0.75,
            vec![FindingCitation {
                portal_key: "test".into(),
                document_id: 1,
                character_start: None,
                character_end: None,
                quote: None,
                label: None,
            }],
        );
        let high = build_finding(
            &stat("VOLUMINOUS_RECORDS", 95, 100),
            0.95,
            vec![FindingCitation {
                portal_key: "test".into(),
                document_id: 1,
                character_start: None,
                character_end: None,
                quote: None,
                label: None,
            }],
        );
        assert!(
            high.confidence.stored_basis_points().unwrap()
                > low.confidence.stored_basis_points().unwrap()
        );
    }

    #[test]
    fn changing_the_reason_changes_the_finding_id() {
        let s1 = stat("VOLUMINOUS_RECORDS", 32, 40);
        let s2 = stat("INTERAGENCY_CONSULTATION", 32, 40);
        let cit = vec![FindingCitation {
            portal_key: "test".into(),
            document_id: 1,
            character_start: None,
            character_end: None,
            quote: None,
            label: None,
        }];
        let a = build_finding(&s1, s1.fraction(), cit.clone());
        let b = build_finding(&s2, s2.fraction(), cit);
        assert_ne!(a.id, b.id);
    }
}
