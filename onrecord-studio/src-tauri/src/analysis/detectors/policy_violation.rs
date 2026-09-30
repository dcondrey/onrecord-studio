//! Stated-commitment versus observed-practice detector.
//!
//! A policy document that states a calendar-day response SLA is compared to the turnaround of the
//! same agency's requests created after the policy took effect. Each finding is a counted
//! comparison against the agency's own words, which is what lets
//! `FindingKind::PolicyViolationDetected::is_legal_claim()` be true, so the observation is held to
//! the statute evaluator's standard:
//!
//! - Only a sourced determination (`DeterminationBasis::SourcedDeterminationEvent`) measures the
//!   period. A closure or activity timestamp is a proxy and is never counted.
//! - A request with a recorded statutory extension is excluded: its allowed period is not the
//!   policy's number.
//! - The policy applies to the agency that issued it; another agency's requests on the same portal
//!   are not evidence about it. A policy whose issuing request has no recorded agency is skipped.
//! - The policy's own stated effective date starts the window. Without one, the upload timestamp
//!   is used, and the finding says so and carries lower confidence.
//!
//! Fee statements are not compared here: a fee in a response against a fee in a policy is one
//! statement against another, which `contradictory_statement` already reports as such.

use super::public_documents::{load_public_documents, PublicDocument};
use crate::{
    analysis::{
        assertion_index::{extract_assertions, AssertionPredicate, AssertionValue},
        commitments::{extract_commitments, Commitment},
        document_class::{classify, DocumentClass},
        finding::{
            finding_id, Finding, FindingCitation, FindingConfidence, FindingKind, FindingSeverity,
            FindingSubject,
        },
        gaps::{EvidenceGap, GapRole},
    },
    error::AppError,
    storage::DuckDbRepository,
};
use chrono::{DateTime, NaiveDate, Utc};
use duckdb::params;
use std::collections::HashMap;

pub const SLUG: &str = "policy_violation";

const MAX_DOCUMENTS: i64 = 5_000;
const MAX_REQUESTS: i64 = 50_000;
const MAX_FINDINGS: usize = 100;
const MAX_CITED_EXAMPLES: usize = 5;
const SERIOUS_BREACH_COUNT: usize = 3;
const STATED_EFFECTIVE_DATE_CONFIDENCE: u16 = 8_000;
const UPLOAD_DATE_CONFIDENCE: u16 = 6_000;
const QUOTE_CHARACTERS: usize = 240;

struct RequestRow {
    request_id: i64,
    agency_id: Option<i64>,
    created_at: DateTime<Utc>,
    /// `Some` only for a sourced determination on a request with no recorded extension.
    measured_days: Option<i64>,
}

#[derive(Default)]
struct Requests {
    by_portal: HashMap<String, Vec<RequestRow>>,
}

impl Requests {
    fn agency_of(&self, portal: &str, request_id: i64) -> Option<i64> {
        self.by_portal
            .get(portal)?
            .iter()
            .find(|row| row.request_id == request_id)?
            .agency_id
    }
}

struct Policy<'a> {
    document: &'a PublicDocument,
    commitments: Vec<Commitment>,
    takes_effect: DateTime<Utc>,
    effective_date_stated: bool,
}

fn policies_in(documents: &[PublicDocument]) -> Vec<Policy<'_>> {
    documents
        .iter()
        .filter_map(|document| {
            if classify(document.file_name.as_deref(), &document.text)? != DocumentClass::Policy {
                return None;
            }
            let commitments = extract_commitments(&document.text);
            if commitments.is_empty() {
                return None;
            }
            let stated = stated_effective_date(&document.text);
            let takes_effect = match stated {
                Some(date) => date,
                None => parse_time(&document.created_at)?,
            };
            Some(Policy {
                document,
                commitments,
                takes_effect,
                effective_date_stated: stated.is_some(),
            })
        })
        .collect()
}

/// Routes this detector identified and could not test: a policy states a calendar-day commitment,
/// and no request from the issuing agency after the policy took effect carries a sourced
/// determination date. `run` returns nothing for these; the caller must not read that as
/// compliance.
pub async fn gaps(repository: &DuckDbRepository) -> Result<Vec<EvidenceGap>, AppError> {
    let documents = load_public_documents(repository, MAX_DOCUMENTS).await?;
    let policies = policies_in(&documents);
    if policies.is_empty() {
        return Ok(Vec::new());
    }
    let requests = load_requests(repository).await?;
    let mut gaps = Vec::new();
    for policy in &policies {
        let portal = &policy.document.portal_key;
        let untestable = match requests.agency_of(portal, policy.document.request_id) {
            None => {
                "the issuing request has no recorded agency, so no request can be attributed to it"
            }
            Some(agency_id) => {
                let measurable = requests.by_portal.get(portal).is_some_and(|rows| {
                    rows.iter().any(|row| {
                        row.agency_id == Some(agency_id)
                            && row.created_at >= policy.takes_effect
                            && row.measured_days.is_some()
                    })
                });
                if measurable {
                    continue;
                }
                "no request to this agency after the policy took effect has a sourced determination date"
            }
        };
        let name = policy_name(policy.document);
        let days = policy
            .commitments
            .iter()
            .map(|commitment| commitment.days)
            .min()
            .unwrap_or_default();
        gaps.push(EvidenceGap {
            detector: SLUG,
            key: format!("{SLUG}:{portal}:{}", policy.document.document_id),
            route: format!("'{name}' states a {days}-day response time"),
            blocked: format!("The commitment cannot be tested against practice: {untestable}."),
            role: GapRole::DeterminationDates,
            request: format!(
                "Provide the existing log of public-records requests received since the effective date of '{name}', with each request's received date, determination date, and any extension."
            ),
        });
    }
    Ok(gaps)
}

pub async fn run(repository: &DuckDbRepository) -> Result<Vec<Finding>, AppError> {
    let documents = load_public_documents(repository, MAX_DOCUMENTS).await?;
    let policies = policies_in(&documents);
    if policies.is_empty() {
        return Ok(Vec::new());
    }
    let requests = load_requests(repository).await?;
    let mut findings: Vec<Finding> = policies
        .iter()
        .flat_map(|policy| {
            policy
                .commitments
                .iter()
                .filter_map(|commitment| sla_finding(policy, commitment, &requests))
                .collect::<Vec<_>>()
        })
        .collect();
    findings.sort_by(|a, b| {
        b.severity
            .rank_weight()
            .cmp(&a.severity.rank_weight())
            .then_with(|| b.confidence.rank_weight().cmp(&a.confidence.rank_weight()))
    });
    findings.truncate(MAX_FINDINGS);
    Ok(findings)
}

async fn load_requests(repository: &DuckDbRepository) -> Result<Requests, AppError> {
    repository
        .query_connection("Policy violation request sweep", move |conn| {
            let mut statement = conn
                .prepare(
                    "SELECT r.portal_subdomain, r.id, r.agency_id, r.created_at,
                            r.determination_at,
                            COALESCE(TRIM(r.determination_source), '') <> '' AS sourced,
                            EXISTS (SELECT 1 FROM statutory_extensions e
                                    WHERE e.request_id = r.id
                                      AND e.portal_subdomain = r.portal_subdomain) AS extended
                     FROM requests r
                     ORDER BY r.created_at DESC
                     LIMIT ?1",
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to prepare request sweep: {error}"))
                })?;
            let rows = statement
                .query_map(params![MAX_REQUESTS], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, bool>(5)?,
                        row.get::<_, bool>(6)?,
                    ))
                })
                .map_err(|error| {
                    AppError::Storage(format!("Failed to run request sweep: {error}"))
                })?;
            let mut requests = Requests::default();
            for row in rows {
                let (portal, request_id, agency_id, created, determined, sourced, extended) = row
                    .map_err(
                    |error| AppError::Storage(format!("Failed to decode request row: {error}")),
                )?;
                let Some(created_at) = parse_time(&created) else {
                    continue;
                };
                let measured_days =
                    measured_days(created_at, determined.as_deref(), sourced, extended);
                requests
                    .by_portal
                    .entry(portal)
                    .or_default()
                    .push(RequestRow {
                        request_id,
                        agency_id,
                        created_at,
                        measured_days,
                    });
            }
            Ok(requests)
        })
        .await
}

fn sla_finding(policy: &Policy, commitment: &Commitment, requests: &Requests) -> Option<Finding> {
    let committed_days = i64::from(commitment.days);
    let portal = &policy.document.portal_key;
    let agency_id = requests.agency_of(portal, policy.document.request_id)?;
    let eligible: Vec<(&RequestRow, i64)> = requests
        .by_portal
        .get(portal)?
        .iter()
        .filter(|row| row.agency_id == Some(agency_id) && row.created_at >= policy.takes_effect)
        .filter_map(|row| Some((row, row.measured_days?)))
        .collect();
    let breached: Vec<&(&RequestRow, i64)> = eligible
        .iter()
        .filter(|(_, elapsed)| *elapsed > committed_days)
        .collect();
    let (worst, worst_days) = breached.iter().max_by_key(|(_, elapsed)| *elapsed)?;
    let severity = if breached.len() >= SERIOUS_BREACH_COUNT {
        FindingSeverity::Serious
    } else {
        FindingSeverity::Notable
    };
    let confidence = FindingConfidence::measured(if policy.effective_date_stated {
        STATED_EFFECTIVE_DATE_CONFIDENCE
    } else {
        UPLOAD_DATE_CONFIDENCE
    });
    let examples = breached
        .iter()
        .take(MAX_CITED_EXAMPLES)
        .map(|(row, _)| row.request_id.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let window_basis = if policy.effective_date_stated {
        "the effective date the policy states"
    } else {
        "the date the policy document was posted, because it states no effective date"
    };
    let subject = FindingSubject {
        portal_key: Some(portal.clone()),
        agency_id: Some(agency_id),
        request_id: Some(worst.request_id),
        document_id: Some(policy.document.document_id),
        ..Default::default()
    };
    let salient = format!(
        "sla={committed_days};breached={};eligible={}",
        breached.len(),
        eligible.len()
    );
    let now = Utc::now();
    Some(Finding {
        id: finding_id(FindingKind::PolicyViolationDetected, SLUG, &subject, &[&salient]),
        kind: FindingKind::PolicyViolationDetected,
        severity,
        confidence,
        subject,
        headline: format!(
            "{} of {} requests exceeded the {committed_days}-day response time the agency's own policy states",
            breached.len(),
            eligible.len()
        ),
        explanation: format!(
            "'{}' states a response within {committed_days} calendar days. Of {} requests to this \
             agency created after {window_basis}, with a sourced determination and no recorded \
             extension, {} took longer; the slowest, request {}, took {worst_days} days. \
             Examples: {examples}. Requests with only a closure or activity timestamp, or with an \
             extension, are not counted.",
            policy_name(policy.document),
            eligible.len(),
            breached.len(),
            worst.request_id,
        ),
        citations: vec![citation(policy.document, commitment)],
        discovered_at: now,
        last_seen_at: now,
        dismissed_at: None,
        source_detector: SLUG.to_string(),
    })
}

/// Elapsed calendar days to a sourced determination on a request with no recorded extension.
/// Everything else is a proxy or an unknown allowed period, and counts nothing.
fn measured_days(
    created_at: DateTime<Utc>,
    determined: Option<&str>,
    sourced: bool,
    extended: bool,
) -> Option<i64> {
    let determined_at = parse_time(determined?)?;
    (sourced && !extended && determined_at >= created_at)
        .then(|| (determined_at - created_at).num_days())
}

/// The latest effective date the policy states for itself. The latest is the conservative choice:
/// a later start counts fewer requests.
pub(super) fn stated_effective_date(text: &str) -> Option<DateTime<Utc>> {
    extract_assertions(text)
        .into_iter()
        .filter(|assertion| {
            assertion.predicate == AssertionPredicate::EffectiveDate
                && assertion.subject.starts_with("policy::")
        })
        .filter_map(|assertion| match assertion.value {
            AssertionValue::Date(iso) => NaiveDate::parse_from_str(&iso, "%Y-%m-%d").ok(),
            _ => None,
        })
        .max()
        .and_then(|date| date.and_hms_opt(0, 0, 0))
        .map(|start| start.and_utc())
}

fn citation(document: &PublicDocument, commitment: &Commitment) -> FindingCitation {
    let start = document.text[..commitment.byte_start].chars().count();
    let end = document.text[..commitment.byte_end].chars().count();
    FindingCitation {
        portal_key: document.portal_key.clone(),
        document_id: document.document_id,
        character_start: i64::try_from(start).ok(),
        character_end: i64::try_from(end).ok(),
        quote: document
            .text
            .get(commitment.byte_start..commitment.byte_end)
            .map(|quote| quote.chars().take(QUOTE_CHARACTERS).collect()),
        label: Some("Stated commitment".to_string()),
    }
}

fn policy_name(document: &PublicDocument) -> String {
    document
        .file_name
        .clone()
        .unwrap_or_else(|| format!("document {}", document.document_id))
}

fn parse_time(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value.trim())
        .ok()
        .map(|time| time.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(day: u32) -> DateTime<Utc> {
        NaiveDate::from_ymd_opt(2026, 3, day)
            .and_then(|date| date.and_hms_opt(0, 0, 0))
            .expect("valid date")
            .and_utc()
    }

    #[test]
    fn only_a_sourced_unextended_determination_measures_a_period() {
        let end = Some("2026-03-20T00:00:00Z");
        assert_eq!(measured_days(at(1), end, true, false), Some(19));
        assert_eq!(measured_days(at(1), end, false, false), None);
        assert_eq!(measured_days(at(1), end, true, true), None);
        assert_eq!(measured_days(at(1), None, true, false), None);
    }

    #[test]
    fn a_breach_counts_only_the_issuing_agencys_measured_requests_after_the_policy() {
        let row = |request_id, agency_id, day, measured_days| RequestRow {
            request_id,
            agency_id,
            created_at: at(day),
            measured_days,
        };
        let mut requests = Requests::default();
        requests.by_portal.insert(
            "sd".into(),
            vec![
                row(1, Some(9), 1, Some(30)),
                row(2, Some(9), 10, Some(30)),
                row(3, Some(9), 10, None),
                row(4, Some(5), 10, Some(30)),
                row(5, Some(9), 10, Some(5)),
            ],
        );
        let document = PublicDocument {
            portal_key: "sd".into(),
            document_id: 100,
            request_id: 1,
            file_name: Some("policy.pdf".into()),
            created_at: "2026-03-05T00:00:00Z".into(),
            text: "Response will be provided within 10 days".into(),
        };
        let commitments = extract_commitments(&document.text);
        let policy = Policy {
            document: &document,
            commitments: commitments.clone(),
            takes_effect: at(5),
            effective_date_stated: false,
        };
        let finding = sla_finding(&policy, &commitments[0], &requests).expect("one breach");
        assert!(finding.headline.starts_with("1 of 2 requests"));
        assert_eq!(finding.subject.request_id, Some(2));
    }
}
