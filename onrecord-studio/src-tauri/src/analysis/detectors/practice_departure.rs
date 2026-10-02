//! Practice policy versus documented practice.
//!
//! A policy that states a duty about a policing practice (camera activation, use-of-force review,
//! pursuit authorization, and the other topics of `analysis::practice`) is paired with a sentence
//! in another document of the same agency that says the duty was not followed. The finding cites
//! both sentences and says only that: the agency's own document states a rule and another
//! document states it was not followed. It never says the conduct was unlawful or wrongful.
//!
//! This detector sees what ingested documents state and nothing else. It does not count incidents,
//! it does not check that a stated count is true, and a corpus with no audit, oversight review or
//! response from the agency yields no finding and an evidence gap, never compliance.
//!
//! Evidence rules:
//!
//! - The policy and the departure must belong to the same agency, through the request each
//!   document was released under. A document whose request has no recorded agency is skipped.
//! - A policy that states it is superseded or rescinded is skipped. When several policies of the
//!   agency state a rule on the topic, the one most recently in force on the departure's date
//!   applies.
//! - The policy must already be in force: the departure's earliest stated date, or the date its
//!   document was posted when it states none, may not precede the policy's effective date, and
//!   neither may the date the document was posted. A policy with no stated effective date is
//!   dated by its posting, and the finding says so.
//! - Two parameters in the same unit that state different values (24 hours against 72 hours)
//!   mean the sentences are not about the same rule, so there is no finding.
//! - Only a departure sentence that states `N of M` cases with `0 < N <= M` is a counted
//!   comparison (`PracticeDepartureCounted`). Everything else is a stated departure with no count
//!   (`PracticeDepartureStated`), which is a pattern signal.
//! - Confidence is derived from what was observed: a stated count starts higher than an
//!   uncounted statement, and each date that rests on a posting time instead of a stated date
//!   lowers it.

use super::public_documents::{load_public_documents, PublicDocument};
use super::text_shared::{load_agencies, Agency, AgencyIndex};
use crate::{
    analysis::{
        assertion_index::{extract_assertions, AssertionPredicate, AssertionValue},
        document_class::{classify, DocumentClass},
        finding::{
            finding_id, Finding, FindingCitation, FindingConfidence, FindingKind, FindingSeverity,
            FindingSubject,
        },
        gaps::{EvidenceGap, GapRole},
        phrase_match::words,
        practice::{
            departures, requirements, states_it_is_superseded, Departure, Requirement, Sentence,
            StatedCount,
        },
    },
    error::AppError,
    storage::DuckDbRepository,
};
use chrono::{DateTime, NaiveDate, Utc};
use std::collections::HashSet;

pub const SLUG: &str = "practice_departure";

const MAX_DOCUMENTS: i64 = 5_000;
const MAX_REQUESTS: i64 = 50_000;
const MAX_FINDINGS: usize = 100;
const MAX_GAPS: usize = 25;
const QUOTE_CHARACTERS: usize = 240;
const COUNTED_CONFIDENCE: u16 = 9_000;
const UNCOUNTED_CONFIDENCE: u16 = 5_000;
const POSTED_DATE_PENALTY: u16 = 1_500;
/// A large stated failure share over a real sample is serious; a small sample is not.
const SERIOUS_MIN_CASES: u128 = 10;
const SERIOUS_NONCOMPLIANT_PERCENT: u128 = 50;

struct Policy<'a> {
    document: &'a PublicDocument,
    agency: &'a Agency,
    effective: NaiveDate,
    effective_stated: bool,
    requirements: Vec<Requirement>,
}

struct Source<'a> {
    document: &'a PublicDocument,
    agency: &'a Agency,
    posted: NaiveDate,
    departures: Vec<Departure>,
}

struct Sources<'a> {
    policies: Vec<Policy<'a>>,
    others: Vec<Source<'a>>,
}

fn sources<'a>(documents: &'a [PublicDocument], agencies: &'a AgencyIndex) -> Sources<'a> {
    let mut found = Sources {
        policies: Vec::new(),
        others: Vec::new(),
    };
    for document in documents {
        let Some(agency) = agencies.get(&(document.portal_key.clone(), document.request_id)) else {
            continue;
        };
        let Some(posted) = posted_date(&document.created_at) else {
            continue;
        };
        if classify(document.file_name.as_deref(), &document.text) == Some(DocumentClass::Policy) {
            if states_it_is_superseded(&document.text) {
                continue;
            }
            let requirements = requirements(&document.text);
            if requirements.is_empty() {
                continue;
            }
            let stated = stated_effective_date(&document.text);
            found.policies.push(Policy {
                document,
                agency,
                effective: stated.unwrap_or(posted),
                effective_stated: stated.is_some(),
                requirements,
            });
        } else {
            found.others.push(Source {
                document,
                agency,
                posted,
                departures: departures(&document.text),
            });
        }
    }
    found
}

/// The latest effective date the policy states for itself; a later start is the conservative
/// choice because it admits fewer departures.
fn stated_effective_date(text: &str) -> Option<NaiveDate> {
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
}

fn posted_date(value: &str) -> Option<NaiveDate> {
    DateTime::parse_from_rfc3339(value.trim())
        .ok()
        .map(|time| time.with_timezone(&Utc).date_naive())
}

/// Routes this detector identified and could not test: a policy states practice requirements and
/// no other document from the same agency is in the corpus. `run` returns nothing for these; the
/// caller must not read that as compliance.
pub async fn gaps(repository: &DuckDbRepository) -> Result<Vec<EvidenceGap>, AppError> {
    let documents = load_public_documents(repository, MAX_DOCUMENTS).await?;
    let agencies =
        load_agencies(repository, "Practice departure agency sweep", MAX_REQUESTS).await?;
    Ok(untestable_policies(&sources(&documents, &agencies)))
}

fn untestable_policies(sources: &Sources) -> Vec<EvidenceGap> {
    let mut gaps = Vec::new();
    for policy in &sources.policies {
        let checkable = sources.others.iter().any(|other| {
            other.document.portal_key == policy.document.portal_key
                && other.agency.id == policy.agency.id
        });
        if checkable {
            continue;
        }
        let name = document_name(policy.document);
        let topics = topic_labels(&policy.requirements);
        gaps.push(EvidenceGap {
            detector: SLUG,
            key: format!("{SLUG}:{}:{}", policy.document.portal_key, policy.document.document_id),
            route: format!("'{name}' states practice requirements on {topics}"),
            blocked: format!(
                "No audit, oversight review or response from {} is in the corpus, so none of these requirements can be compared with documented practice.",
                policy.agency.name
            ),
            role: GapRole::AgencyRecords,
            request: format!(
                "Provide the most recent audits, inspector general or oversight reviews, after-action reviews and internal affairs summaries covering {topics}, for periods after the effective date of '{name}'."
            ),
        });
        if gaps.len() == MAX_GAPS {
            break;
        }
    }
    gaps
}

fn topic_labels(requirements: &[Requirement]) -> String {
    let mut seen = HashSet::new();
    requirements
        .iter()
        .filter(|requirement| seen.insert(requirement.topic))
        .map(|requirement| requirement.topic.label())
        .collect::<Vec<_>>()
        .join(", ")
}

pub async fn run(repository: &DuckDbRepository) -> Result<Vec<Finding>, AppError> {
    let documents = load_public_documents(repository, MAX_DOCUMENTS).await?;
    let agencies =
        load_agencies(repository, "Practice departure agency sweep", MAX_REQUESTS).await?;
    Ok(findings(&sources(&documents, &agencies)))
}

fn findings(sources: &Sources) -> Vec<Finding> {
    let mut found: Vec<Finding> = sources
        .others
        .iter()
        .flat_map(|source| {
            source
                .departures
                .iter()
                .filter_map(|departure| finding(sources, source, departure))
        })
        .collect();
    found.sort_by(|a, b| {
        b.severity
            .rank_weight()
            .cmp(&a.severity.rank_weight())
            .then_with(|| b.confidence.rank_weight().cmp(&a.confidence.rank_weight()))
            .then_with(|| a.id.cmp(&b.id))
    });
    found.truncate(MAX_FINDINGS);
    found
}

/// The requirement a departure is measured against, or `None` when no policy of the same agency
/// that was already in force states a compatible rule on the topic.
fn governing_requirement<'a>(
    sources: &'a Sources,
    source: &Source,
    departure: &Departure,
) -> Option<(&'a Policy<'a>, &'a Requirement)> {
    let in_force_from = departure.dates.first().copied().unwrap_or(source.posted);
    let policy = sources
        .policies
        .iter()
        .filter(|policy| {
            policy.document.portal_key == source.document.portal_key
                && policy.agency.id == source.agency.id
                && policy.effective <= in_force_from
                && policy.effective <= source.posted
                && policy
                    .requirements
                    .iter()
                    .any(|requirement| requirement.topic == departure.topic)
        })
        .max_by_key(|policy| {
            (
                policy.effective,
                std::cmp::Reverse(policy.document.document_id),
            )
        })?;
    let departure_words: HashSet<String> = content_words(&departure.sentence.text);
    policy
        .requirements
        .iter()
        .filter(|requirement| requirement.topic == departure.topic)
        .filter(
            |requirement| match (requirement.parameter, departure.parameter) {
                (Some(required), Some(stated)) => !required.conflicts_with(stated),
                _ => true,
            },
        )
        .max_by_key(|requirement| {
            (
                content_words(&requirement.sentence.text)
                    .intersection(&departure_words)
                    .count(),
                std::cmp::Reverse(requirement.sentence.character_start),
            )
        })
        .map(|requirement| (policy, requirement))
}

fn content_words(text: &str) -> HashSet<String> {
    words(text)
        .into_iter()
        .map(|word| word.text)
        .filter(|word| word.chars().count() > 3)
        .collect()
}

fn finding(sources: &Sources, source: &Source, departure: &Departure) -> Option<Finding> {
    let (policy, requirement) = governing_requirement(sources, source, departure)?;
    let counted = match departure.count {
        StatedCount::Stated {
            noncompliant,
            total,
        } => Some((noncompliant, total)),
        StatedCount::Unstated => None,
    };
    let label = departure.topic.label();
    let agency = &policy.agency.name;
    let policy_name = document_name(policy.document);
    let source_name = document_name(source.document);

    let mut basis_points = if counted.is_some() {
        COUNTED_CONFIDENCE
    } else {
        UNCOUNTED_CONFIDENCE
    };
    let mut caveats = Vec::new();
    if !policy.effective_stated {
        basis_points -= POSTED_DATE_PENALTY;
        caveats.push(format!(
            "'{policy_name}' states no effective date, so the date it was posted ({}) is used and the rule may have taken effect earlier.",
            policy.effective
        ));
    }
    if departure.dates.is_empty() {
        basis_points -= POSTED_DATE_PENALTY;
        caveats.push(format!(
            "The departure sentence states no date, so its timing rests on the date '{source_name}' was posted ({}); the conduct it describes may predate the policy.",
            source.posted
        ));
    }

    let (kind, severity, headline, finding_statement) = match counted {
        Some((noncompliant, total)) => {
            let serious = u128::from(total) >= SERIOUS_MIN_CASES
                && u128::from(noncompliant) * 100 >= u128::from(total) * SERIOUS_NONCOMPLIANT_PERCENT;
            let compliant_tenths = u128::from(total - noncompliant) * 1000 / u128::from(total);
            (
                FindingKind::PracticeDepartureCounted,
                if serious {
                    FindingSeverity::Serious
                } else {
                    FindingSeverity::Notable
                },
                format!(
                    "{agency}: a document states {noncompliant} of {total} cases did not follow the agency's own written {label} rule"
                ),
                format!(
                    "'{source_name}' states {noncompliant} of {total} cases did not follow it. \
                     Compliance computed from those stated counts alone is {}.{}%; no count was inferred and \
                     the stated count was not checked against incident records.",
                    compliant_tenths / 10,
                    compliant_tenths % 10
                ),
            )
        }
        None => (
            FindingKind::PracticeDepartureStated,
            FindingSeverity::Notable,
            format!(
                "{agency}: a document states the agency's own written {label} rule was not followed"
            ),
            format!(
                "'{source_name}' states the rule was not followed, without a count, so the extent is unknown."
            ),
        ),
    };

    let parameter = requirement
        .parameter
        .map(|parameter| format!(" The rule states {}.", parameter.display()))
        .unwrap_or_default();
    let explanation = format!(
        "'{policy_name}' (issued by {agency}, in force from {}) states: \"{}\".{parameter} {finding_statement} \
         This compares what two documents state. It does not establish that the conduct was unlawful or wrongful, \
         and it does not say why the rule was not followed. {}",
        policy.effective,
        excerpt(&requirement.sentence.text),
        caveats.join(" ")
    );

    let subject = FindingSubject {
        portal_key: Some(source.document.portal_key.clone()),
        agency_id: Some(source.agency.id),
        request_id: Some(source.document.request_id),
        document_id: Some(source.document.document_id),
        ..Default::default()
    };
    let salient = format!(
        "topic={};policy={}:{};departure={}",
        departure.topic.as_str(),
        policy.document.document_id,
        requirement.sentence.character_start,
        departure.sentence.character_start
    );
    let now = Utc::now();
    Some(Finding {
        id: finding_id(kind, SLUG, &subject, &[&salient]),
        kind,
        severity,
        confidence: FindingConfidence::measured(basis_points),
        subject,
        headline,
        explanation: explanation.trim_end().to_string(),
        citations: vec![
            citation(
                policy.document,
                &requirement.sentence,
                "Agency policy requirement",
            ),
            citation(source.document, &departure.sentence, "Documented departure"),
        ],
        discovered_at: now,
        last_seen_at: now,
        dismissed_at: None,
        source_detector: SLUG.to_string(),
    })
}

fn excerpt(text: &str) -> String {
    text.chars().take(QUOTE_CHARACTERS).collect()
}

fn citation(document: &PublicDocument, sentence: &Sentence, label: &str) -> FindingCitation {
    FindingCitation {
        portal_key: document.portal_key.clone(),
        document_id: document.document_id,
        character_start: i64::try_from(sentence.character_start).ok(),
        character_end: i64::try_from(sentence.character_end).ok(),
        quote: Some(excerpt(&sentence.text)),
        label: Some(label.to_string()),
    }
}

fn document_name(document: &PublicDocument) -> String {
    document
        .file_name
        .clone()
        .unwrap_or_else(|| format!("document {}", document.document_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::detectors::policy_violation;

    const POLICY: &str = "Body Worn Camera Policy\n\nThe body worn camera policy is effective on March 1, 2025.\n\n\
        Officers shall activate their body-worn cameras before arriving at any call for service. \
        Recordings from body-worn cameras must be uploaded within 24 hours of the end of the shift. \
        A supervisor shall review each use of force report within 72 hours.";
    const DEPARTURE: &str = "Audit of Patrol Operations\n\nAudit period: January 1, 2026 to June 30, 2026.\n\n\
        Body-worn cameras were not activated in 14 of 40 sampled incidents between March 3, 2026 and June 30, 2026.";

    fn agency(id: i64) -> Agency {
        Agency {
            id,
            name: "Police Department".into(),
        }
    }

    fn document(id: i64, request: i64, name: &str, posted: &str, text: &str) -> PublicDocument {
        PublicDocument {
            portal_key: "sd".into(),
            document_id: id,
            request_id: request,
            file_name: Some(name.into()),
            created_at: format!("{posted}T00:00:00Z"),
            text: text.into(),
        }
    }

    /// One policy (request 1, document 10) and one other document (request 2, document 20).
    fn evaluate(
        policy: &str,
        policy_posted: &str,
        other: &str,
        other_posted: &str,
        other_agency: Option<i64>,
    ) -> Vec<Finding> {
        let documents = [
            document(10, 1, "bwc-policy.pdf", policy_posted, policy),
            document(20, 2, "patrol-audit.pdf", other_posted, other),
        ];
        let mut agencies = AgencyIndex::new();
        agencies.insert(("sd".into(), 1), agency(3));
        if let Some(id) = other_agency {
            agencies.insert(("sd".into(), 2), agency(id));
        }
        findings(&sources(&documents, &agencies))
    }

    fn departure_in(text: &str) -> String {
        format!("Audit of Patrol Operations\n\n{text}")
    }

    #[test]
    fn a_counted_departure_cites_both_sentences_and_computes_the_rate_from_the_stated_counts() {
        let found = evaluate(POLICY, "2025-03-02", DEPARTURE, "2026-08-01", Some(3));
        assert_eq!(found.len(), 1, "{found:#?}");
        let finding = &found[0];
        assert_eq!(finding.kind, FindingKind::PracticeDepartureCounted);
        assert!(finding.kind.is_legal_claim() && finding.check_recordable().is_ok());
        assert_eq!(finding.confidence, FindingConfidence::measured(9_000));
        assert!(finding.headline.contains("14 of 40"));
        assert!(
            finding.explanation.contains("65.0%"),
            "{}",
            finding.explanation
        );
        assert_eq!(finding.subject.agency_id, Some(3));
        assert_eq!(finding.subject.document_id, Some(20));

        let labels: Vec<_> = finding
            .citations
            .iter()
            .map(|citation| (citation.document_id, citation.label.as_deref()))
            .collect();
        assert_eq!(
            labels,
            [
                (10, Some("Agency policy requirement")),
                (20, Some("Documented departure"))
            ]
        );
        for (citation, text) in finding.citations.iter().zip([POLICY, DEPARTURE]) {
            let start = usize::try_from(citation.character_start.unwrap()).unwrap();
            let end = usize::try_from(citation.character_end.unwrap()).unwrap();
            let cited: String = text.chars().skip(start).take(end - start).collect();
            assert_eq!(Some(cited), citation.quote);
        }
    }

    #[test]
    fn the_policy_must_be_in_force_on_the_departures_date_at_the_boundary() {
        for (day, expected) in [("February 28", 0), ("March 1", 1), ("March 2", 1)] {
            let text = departure_in(&format!(
                "Body-worn cameras were not activated in 14 of 40 incidents on {day}, 2025."
            ));
            let found = evaluate(POLICY, "2025-03-02", &text, "2026-08-01", Some(3));
            assert_eq!(found.len(), expected, "{day}");
        }
    }

    #[test]
    fn an_undated_departure_posted_before_the_policy_took_effect_is_excluded() {
        let text = departure_in("Body-worn cameras were not activated in 14 of 40 incidents.");
        for (posted, expected) in [("2025-02-28", 0), ("2025-03-01", 1), ("2025-03-02", 1)] {
            let found = evaluate(POLICY, "2025-03-02", &text, posted, Some(3));
            assert_eq!(found.len(), expected, "{posted}");
        }
        let found = evaluate(POLICY, "2025-03-02", &text, "2025-03-01", Some(3));
        assert_eq!(found[0].confidence, FindingConfidence::measured(7_500));
        assert!(found[0].explanation.contains("states no date"));
    }

    #[test]
    fn a_policy_without_a_stated_effective_date_is_dated_by_its_posting_and_says_so() {
        let policy = POLICY.replace(
            "The body worn camera policy is effective on March 1, 2025.",
            "",
        );
        let found = evaluate(&policy, "2025-03-02", DEPARTURE, "2026-08-01", Some(3));
        assert_eq!(found[0].confidence, FindingConfidence::measured(7_500));
        assert!(found[0].explanation.contains("states no effective date"));
        let before = evaluate(&policy, "2026-09-01", DEPARTURE, "2026-08-01", Some(3));
        assert!(before.is_empty(), "posted after the departure's own date");
    }

    #[test]
    fn another_agencys_or_an_unattributed_document_is_not_evidence_about_the_policy() {
        assert!(evaluate(POLICY, "2025-03-02", DEPARTURE, "2026-08-01", Some(9)).is_empty());
        assert!(evaluate(POLICY, "2025-03-02", DEPARTURE, "2026-08-01", None).is_empty());
    }

    #[test]
    fn sentences_that_negate_or_do_not_depart_produce_no_finding() {
        for text in [
            "Activation of body-worn cameras was not required during transport.",
            "Body-worn cameras were activated in 40 of 40 incidents.",
            "Body-worn cameras were not activated in 0 of 40 incidents.",
            "Body-worn cameras were not activated in 41 of 40 incidents.",
            "The audit found no instances in which officers failed to activate body-worn cameras.",
            "Officers who fail to activate body-worn cameras will be disciplined.",
            "Patrol officers received new radios.",
        ] {
            let found = evaluate(
                POLICY,
                "2025-03-02",
                &departure_in(text),
                "2026-08-01",
                Some(3),
            );
            assert!(found.is_empty(), "{text}: {found:#?}");
        }
    }

    #[test]
    fn a_departure_on_a_topic_the_policy_does_not_state_is_not_compared() {
        let text =
            departure_in("Pursuits were not authorized by a supervisor in 6 of 20 incidents.");
        assert!(evaluate(POLICY, "2025-03-02", &text, "2026-08-01", Some(3)).is_empty());
    }

    #[test]
    fn a_differing_parameter_in_the_same_unit_means_a_different_rule() {
        let upload = |hours: u32| {
            departure_in(&format!(
                "Officers did not upload body-worn camera recordings within {hours} hours in 5 of 20 incidents on June 2, 2026."
            ))
        };
        assert_eq!(
            evaluate(POLICY, "2025-03-02", &upload(24), "2026-08-01", Some(3)).len(),
            1
        );
        assert!(evaluate(POLICY, "2025-03-02", &upload(72), "2026-08-01", Some(3)).is_empty());
    }

    #[test]
    fn a_policy_that_states_it_is_superseded_is_not_applied() {
        let policy = format!("{POLICY}\n\nThis policy has been superseded by General Order 40.");
        assert!(evaluate(&policy, "2025-03-02", DEPARTURE, "2026-08-01", Some(3)).is_empty());
    }

    #[test]
    fn the_policy_most_recently_in_force_on_the_departures_date_governs() {
        let older = POLICY.replace("March 1, 2025", "January 1, 2024");
        let audit = |date: &str| {
            departure_in(&format!(
                "Body-worn cameras were not activated in 14 of 40 incidents on {date}."
            ))
        };
        let documents = |departure: &str| {
            [
                document(10, 1, "bwc-policy.pdf", "2024-01-02", &older),
                document(11, 1, "bwc-policy-2025.pdf", "2025-03-02", POLICY),
                document(20, 2, "patrol-audit.pdf", "2026-08-01", departure),
            ]
        };
        let mut agencies = AgencyIndex::new();
        agencies.insert(("sd".into(), 1), agency(3));
        agencies.insert(("sd".into(), 2), agency(3));
        for (date, policy_id) in [("June 2, 2025", 11), ("June 2, 2024", 10)] {
            let documents = documents(&audit(date));
            let found = findings(&sources(&documents, &agencies));
            assert_eq!(found.len(), 1);
            assert_eq!(found[0].citations[0].document_id, policy_id, "{date}");
        }
    }

    #[test]
    fn an_uncounted_departure_is_a_pattern_signal_with_lower_confidence() {
        let text = departure_in(
            "Officers failed to activate body-worn cameras during several stops on June 2, 2026.",
        );
        let found = evaluate(POLICY, "2025-03-02", &text, "2026-08-01", Some(3));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, FindingKind::PracticeDepartureStated);
        assert!(!found[0].kind.is_legal_claim());
        assert_eq!(found[0].confidence, FindingConfidence::measured(5_000));
        assert_eq!(found[0].severity, FindingSeverity::Notable);
    }

    #[test]
    fn serious_needs_a_real_sample_and_at_least_half_the_cases() {
        for (noncompliant, total, serious) in [
            (5, 9, false),
            (4, 10, false),
            (5, 10, true),
            (5, 11, false),
            (6, 11, true),
        ] {
            let text = departure_in(&format!(
                "Body-worn cameras were not activated in {noncompliant} of {total} incidents on June 2, 2026."
            ));
            let found = evaluate(POLICY, "2025-03-02", &text, "2026-08-01", Some(3));
            assert_eq!(
                found[0].severity == FindingSeverity::Serious,
                serious,
                "{noncompliant} of {total}"
            );
        }
    }

    #[test]
    fn a_policy_with_no_document_from_its_agency_to_check_is_a_gap_not_compliance() {
        let documents = [document(10, 1, "bwc-policy.pdf", "2025-03-02", POLICY)];
        let mut agencies = AgencyIndex::new();
        agencies.insert(("sd".into(), 1), agency(3));
        let built = sources(&documents, &agencies);
        assert!(findings(&built).is_empty());
        let gaps = untestable_policies(&built);
        assert_eq!(gaps.len(), 1);
        assert!(gaps[0].route.contains("body-worn camera activation"));

        // A checkable corpus with nothing stated is "checked, nothing found", not a gap.
        let documents = [
            document(10, 1, "bwc-policy.pdf", "2025-03-02", POLICY),
            document(
                20,
                2,
                "patrol-audit.pdf",
                "2026-08-01",
                "Patrol officers received new radios.",
            ),
        ];
        agencies.insert(("sd".into(), 2), agency(3));
        assert!(untestable_policies(&sources(&documents, &agencies)).is_empty());
    }

    async fn seeded(practice_documents: bool) -> DuckDbRepository {
        let repository = DuckDbRepository::new(":memory:").expect("open");
        repository
            .run_migrations(include_str!("../../../migrations/0001_initial_schema.sql"))
            .await
            .expect("migrate");
        let records_policy = "Records Request Policy\n\nThis policy governs how staff respond. Response will be provided within 10 days.";
        let mut sql = String::from(
            "INSERT INTO portals (subdomain, display_name, base_url) VALUES ('sd', 'SD', 'https://sd.example');
             INSERT INTO agencies (id, portal_subdomain, name) VALUES (3, 'sd', 'Police Department');
             INSERT INTO requests (id, portal_subdomain, title, status, created_at, updated_at, agency_id)
               VALUES (1, 'sd', 'Policies', 'closed', '2026-01-01T00:00:00Z', '2026-01-02T00:00:00Z', 3);
             INSERT INTO requests (id, portal_subdomain, title, status, created_at, updated_at, agency_id, determination_at, determination_source)
               VALUES (2, 'sd', 'Audits', 'closed', '2026-02-01T00:00:00Z', '2026-03-05T00:00:00Z', 3, '2026-03-05T00:00:00Z', 'portal');",
        );
        let mut add = |id: i64, request: i64, name: &str, posted: &str, text: &str| {
            sql.push_str(&format!(
                "INSERT INTO documents (id, request_id, portal_subdomain, file_name, visibility, created_at, updated_at, extracted_text_sha256)
                   VALUES ({id}, {request}, 'sd', '{name}', 'public', '{posted}T00:00:00Z', '{posted}T00:00:00Z', 'h{id}');
                 INSERT INTO documents_fts (document_id, portal_subdomain, file_name, extracted_text)
                   VALUES ({id}, 'sd', '{name}', '{text}');"
            ));
        };
        add(1, 1, "records-policy.pdf", "2026-01-01", records_policy);
        if practice_documents {
            add(2, 1, "bwc-policy.pdf", "2025-03-02", POLICY);
            add(3, 2, "patrol-audit.pdf", "2026-08-01", DEPARTURE);
        }
        repository
            .query_connection("practice fixture", move |connection| {
                connection
                    .execute_batch(&sql)
                    .map_err(|error| AppError::Storage(format!("fixture: {error}")))
            })
            .await
            .expect("seed");
        repository
    }

    #[tokio::test]
    async fn adding_practice_documents_leaves_the_response_time_detector_output_unchanged() {
        let summarize = |found: Vec<Finding>| {
            let mut rows: Vec<_> = found
                .into_iter()
                .map(|finding| {
                    (
                        finding.id,
                        finding.kind,
                        finding.headline,
                        finding.explanation,
                    )
                })
                .collect();
            rows.sort_by(|a, b| a.0.cmp(&b.0));
            rows
        };
        let without = seeded(false).await;
        let baseline = summarize(policy_violation::run(&without).await.unwrap());
        assert_eq!(
            baseline.len(),
            1,
            "the fixture must make the response-time detector fire"
        );
        assert_eq!(baseline[0].1, FindingKind::PolicyViolationDetected);
        assert!(run(&without).await.unwrap().is_empty());

        let with = seeded(true).await;
        assert_eq!(
            summarize(policy_violation::run(&with).await.unwrap()),
            baseline
        );
        let practice = run(&with).await.unwrap();
        assert_eq!(practice.len(), 1, "{practice:#?}");
        assert_eq!(practice[0].kind, FindingKind::PracticeDepartureCounted);
        assert_eq!(practice[0].subject.agency_id, Some(3));
    }
}
