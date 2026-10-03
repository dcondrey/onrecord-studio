//! Requests whose records were released in installments spread over months.
//!
//! Reads the release events a portal timeline records (`request_release_events`). An installment
//! is a calendar date, in the portal's own wall-clock time, on which at least one release event is
//! recorded; several uploads on one day are one installment. The finding describes how a release
//! was spread out. It is not a legal claim, and the release dates never become the request's
//! determination date.

use std::collections::BTreeMap;

use chrono::{DateTime, NaiveDate, Utc};

use crate::{
    analysis::finding::{
        finding_id, Finding, FindingConfidence, FindingKind, FindingSeverity, FindingSubject,
    },
    error::AppError,
    storage::{sqlite::RequestReleaseHistory, DuckDbRepository},
};

pub const SLUG: &str = "rolling_production";

/// Measured on 142 sandiego.nextrequest.com "encampment" requests (2026-10-03): 23 released on two
/// or more dates. Two dates are most often a release and one supplement; three or more include at
/// least one interim installment.
pub const MINIMUM_INSTALLMENTS: usize = 3;
/// On the same corpus the spans of multi-date releases fall at 74 days or less, then jump to 132
/// and beyond. A threshold inside that gap does not move a request across it on a day's drift.
pub const SPAN_EXCEEDING_DAYS: i64 = 90;

const MAX_REQUESTS: usize = 5_000;
const MAX_FINDINGS: usize = 200;

pub async fn run(repository: &DuckDbRepository) -> Result<Vec<Finding>, AppError> {
    let histories = repository
        .request_release_histories(MINIMUM_INSTALLMENTS, MAX_REQUESTS)
        .await?;
    evaluate(&histories, Utc::now())
}

/// Installment dates with the file names listed on each, `None` when no event that day listed any.
fn installments(
    history: &RequestReleaseHistory,
) -> Result<BTreeMap<NaiveDate, Option<u32>>, AppError> {
    let mut dates: BTreeMap<NaiveDate, Option<u32>> = BTreeMap::new();
    for release in &history.releases {
        let date = DateTime::parse_from_rfc3339(&release.released_at)
            .map_err(|_| {
                AppError::Storage(format!(
                    "Stored release time '{}' is not RFC 3339",
                    release.released_at
                ))
            })?
            .date_naive();
        let listed = dates.entry(date).or_insert(None);
        if let Some(count) = release.listed_document_count {
            *listed = Some(listed.unwrap_or(0).saturating_add(count));
        }
    }
    Ok(dates)
}

fn evaluate(
    histories: &[RequestReleaseHistory],
    now: DateTime<Utc>,
) -> Result<Vec<Finding>, AppError> {
    let mut findings = Vec::new();
    for history in histories {
        let dates = installments(history)?;
        let (Some(first), Some(last)) = (dates.keys().next(), dates.keys().next_back()) else {
            continue;
        };
        let span = (*last - *first).num_days();
        if dates.len() < MINIMUM_INSTALLMENTS || span <= SPAN_EXCEEDING_DAYS {
            continue;
        }
        let subject = FindingSubject {
            portal_key: Some(history.portal_key.clone()),
            agency_id: history.agency_id,
            request_id: Some(history.request_id),
            ..Default::default()
        };
        let basis: Vec<String> = dates
            .iter()
            .map(|(date, listed)| match listed {
                Some(1) => format!("{date} (1 file listed)"),
                Some(count) => format!("{date} ({count} files listed)"),
                None => format!("{date} (no files listed)"),
            })
            .collect();
        let salient = dates
            .keys()
            .map(NaiveDate::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let label = history.rpid.as_deref().map_or_else(
            || "this request".to_string(),
            |rpid| format!("request {rpid}"),
        );
        let explanation = format!(
            "The portal timeline for {label} records documents released on {} separate dates \
             from {first} to {last}, {span} days apart: {}. This describes how the release was \
             spread out. It does not say any deadline was missed, and these dates are not \
             treated as the agency's determination date.",
            dates.len(),
            basis.join("; ")
        );
        findings.push((
            span,
            Finding {
                id: finding_id(FindingKind::RollingProduction, SLUG, &subject, &[&salient]),
                kind: FindingKind::RollingProduction,
                severity: FindingSeverity::Info,
                // The dates are restated as the portal recorded them; nothing is inferred to score.
                confidence: FindingConfidence::Unassessed,
                subject,
                headline: format!(
                    "Released on {} separate dates over {span} days",
                    dates.len()
                ),
                explanation,
                citations: Vec::new(),
                discovered_at: now,
                last_seen_at: now,
                dismissed_at: None,
                source_detector: SLUG.to_string(),
            },
        ));
    }
    // Longest spans first, so the cap keeps the most drawn-out releases.
    findings.sort_by(|(a_span, a), (b_span, b)| b_span.cmp(a_span).then_with(|| a.id.cmp(&b.id)));
    Ok(findings
        .into_iter()
        .take(MAX_FINDINGS)
        .map(|(_, finding)| finding)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::sqlite::{StoredRelease, RELEASED};

    fn history(released_at: &[&str]) -> RequestReleaseHistory {
        RequestReleaseHistory {
            portal_key: "sandiego".to_string(),
            request_id: 7,
            rpid: Some("20-2226".to_string()),
            agency_id: None,
            releases: released_at
                .iter()
                .map(|at| StoredRelease {
                    event_name: RELEASED.to_string(),
                    released_at: (*at).to_string(),
                    listed_document_count: Some(2),
                })
                .collect(),
        }
    }

    /// Three dates 91 days apart fire; 90 days, or 91 days on only two dates (several uploads on
    /// one day are one installment), do not.
    #[test]
    fn installments_over_the_threshold_yield_one_finding_listing_the_dates() {
        let under_span = history(&[
            "2022-06-19T09:45:00-07:00",
            "2022-07-10T11:16:00-07:00",
            "2022-09-17T09:00:00-07:00",
        ]);
        let two_days = history(&[
            "2022-06-19T09:45:00-07:00",
            "2022-06-19T17:00:00-07:00",
            "2022-09-18T09:00:00-07:00",
        ]);
        let rolling = history(&[
            "2022-06-19T09:45:00-07:00",
            "2022-07-10T11:16:00-07:00",
            "2022-09-18T09:00:00-07:00",
        ]);
        assert!(evaluate(&[under_span, two_days], Utc::now())
            .unwrap()
            .is_empty());
        let findings = evaluate(&[rolling], Utc::now()).unwrap();
        assert_eq!(findings.len(), 1);
        let finding = &findings[0];
        assert!(!finding.kind.is_legal_claim());
        assert_eq!(
            finding.headline,
            "Released on 3 separate dates over 91 days"
        );
        for date in ["2022-06-19", "2022-07-10", "2022-09-18"] {
            assert!(finding.explanation.contains(date), "{date} missing");
        }
        assert!(finding.explanation.contains("(2 files listed)"));
    }
}
