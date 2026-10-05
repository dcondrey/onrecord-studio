//! Follow-up letter for a request whose computed determination deadline passed with no recorded
//! response.
//!
//! [`super::statutory_appeal`] covers a determination that arrived late. This covers silence, and
//! it states less: the deadline is computed, the absence is an absence in the sender's records,
//! and the letter asks for the determination. It is a follow-up, not an appeal or a petition, and
//! it draws no conclusion about why nothing is recorded.
//!
//! It refuses whenever a stated date would rest on something the record does not hold: no
//! recorded received date or source for it, an extension record that is not absent or valid (the
//! same two statuses the appeal accepts), any recorded response, or a deadline that has not passed.

use super::agency_record::{AgencyIdentity, AgencyRecord, AgencyRecordOption};
use super::{ArtifactError, ArtifactKind, CitedAuthority, DraftEvidenceBasis, DraftedArtifact};
use crate::profile::DisclosureContext;
use crate::{
    analysis::{
        court_calendar::COURT_CALENDAR_BASIS,
        finding::{FindingConfidence, FindingSubject},
        request_compiler::evaluate_filing,
        statute::{DeterminationBasis, CPRA_INITIAL_DETERMINATION_DAYS, CPRA_STATUTE_CITATION},
        timeline::{StatutoryComplianceAnalysis, StatutoryExtensionStatus},
    },
    error::AppError,
    storage::DuckDbRepository,
    types::CaseworkRequest,
};
use chrono::{DateTime, Days, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

const TEMPLATE_ID: &str = "cpra_overdue_followup_v1";
const JURISDICTION: &str = "US-CA";
/// Read at leginfo.legislature.ca.gov on 2026-10-05: section 7923.000 opens the chapter on
/// judicial enforcement of the right to inspect or receive a copy of a public record. Cited by
/// number only; none of its text is quoted.
const JUDICIAL_RELIEF_CITATION: &str = "California Government Code § 7923.000";
const JUDICIAL_RELIEF_SENTENCE: &str =
    "A requester may seek judicial relief under Government Code section 7923.000 et seq.";

/// What the letter needs beyond the evaluated dates.
pub struct OverdueRequest<'a> {
    /// Stable key for the draft id, e.g. `casework:<case>:<request>`.
    pub subject_id: &'a str,
    pub addressee: &'a str,
    pub label: &'a str,
    /// Where the received date was recorded. Blank refuses the draft.
    pub received_source: &'a str,
    /// A response recorded outside the evaluated dates, in words. Present refuses the draft.
    pub recorded_response: Option<&'a str>,
}

/// The draft, with the agency counts it used and the agencies a user may choose from.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OverdueFollowUpDraft {
    pub artifact: DraftedArtifact,
    /// The counts for the chosen agency. `None` when no agency was chosen.
    pub agency_record: Option<AgencyRecord>,
    /// Whether the letter carries the agency paragraph. False with a record present means the
    /// library holds too few closed requests for it.
    pub agency_paragraph_included: bool,
    pub agency_options: Vec<AgencyRecordOption>,
}

fn unavailable(reason: impl Into<String>) -> ArtifactError {
    ArtifactError::FollowUpNotAvailable(reason.into())
}

fn date_of(instant: &str) -> Result<NaiveDate, ArtifactError> {
    instant
        .get(..10)
        .and_then(|day| NaiveDate::parse_from_str(day, "%Y-%m-%d").ok())
        .ok_or_else(|| ArtifactError::MissingField(format!("calendar date in {instant}")))
}

/// Pure drafting step over an evaluated request. The letter is dated the evaluation's own
/// observation date, so the day count in it is the one the evaluator made.
pub fn draft_overdue_followup(
    request: &OverdueRequest<'_>,
    analysis: &StatutoryComplianceAnalysis,
    agency_record: Option<&AgencyRecord>,
    jurisdiction: &str,
    disclosure: &DisclosureContext,
) -> Result<DraftedArtifact, ArtifactError> {
    if jurisdiction != JURISDICTION {
        return Err(unavailable(format!(
            "no statute is registered for jurisdiction {jurisdiction}"
        )));
    }
    if request.received_source.trim().is_empty() {
        return Err(unavailable(
            "the received date has no recorded source, so no deadline is counted from it",
        ));
    }
    if let Some(response) = request.recorded_response {
        return Err(unavailable(format!(
            "{response}; this letter covers only a request with no recorded response"
        )));
    }
    match analysis.determination_basis {
        DeterminationBasis::ObservationWithoutRecordedActivity => {}
        DeterminationBasis::SourcedDeterminationEvent => {
            return Err(unavailable(format!(
                "a determination is recorded on {}; this letter covers only a request with no recorded response",
                analysis.determination_at.as_deref().unwrap_or("a recorded date")
            )));
        }
        DeterminationBasis::EarliestRecordedActivityProxy => {
            return Err(unavailable(format!(
                "a response or other agency activity is recorded on {}; this letter covers only a request with no recorded response",
                analysis.determination_at.as_deref().unwrap_or("a recorded date")
            )));
        }
    }
    if !matches!(
        analysis.extension_status,
        StatutoryExtensionStatus::NotInvoked | StatutoryExtensionStatus::Valid
    ) {
        return Err(unavailable(format!(
            "the recorded extension is {:?}; review the extension record before drafting",
            analysis.extension_status
        )));
    }
    let received = date_of(&analysis.created_at)?;
    let deadline = date_of(&analysis.effective_deadline_at)?;
    let elapsed = u64::try_from(analysis.calendar_days_elapsed)
        .map_err(|_| ArtifactError::MissingField("elapsed calendar days".to_string()))?;
    let letter_date = received
        .checked_add_days(Days::new(elapsed))
        .ok_or_else(|| ArtifactError::MissingField("letter date".to_string()))?;
    let days_past = analysis.calendar_days_elapsed - analysis.allowed_calendar_days;
    if !analysis.is_violation || days_past < 1 {
        return Err(unavailable(format!(
            "the determination is due on {deadline} and that date has not passed as of {letter_date}"
        )));
    }

    let (addressee, mut unknown_facts) = if request.addressee.trim().is_empty() {
        ("[AGENCY NAME]".to_string(), vec!["Agency name".to_string()])
    } else {
        (request.addressee.trim().to_string(), Vec::new())
    };
    unknown_facts.push("Your name and contact details".to_string());
    unknown_facts.push(
        "Whether the agency sent a determination or extension notice that is not recorded here"
            .to_string(),
    );
    let extension_line = match (&analysis.extension_reason, analysis.extension_status) {
        (Some(reason), StatutoryExtensionStatus::Valid) => format!(
            "An extension notice is recorded (reason {reason}); the deadline below is computed \
             from the date that notice gave for the determination."
        ),
        _ => "No extension notice is recorded for this request.".to_string(),
    };
    let agency_paragraph = agency_record
        .and_then(AgencyRecord::paragraph)
        .map(|paragraph| format!("{paragraph}\n\n"))
        .unwrap_or_default();
    let day_word = if days_past == 1 {
        "day has"
    } else {
        "days have"
    };
    let label = request.label.trim();
    let text = format!(
        "Subject: Follow-up on public records request: {label}\n\n\
         Date: {letter_date}\n\
         To: {addressee}\n\
         Re: {label}\n\n\
         I am following up on the California Public Records Act request identified above. My \
         records hold no determination and no other response to it.\n\n\
         Recorded facts:\n\
         - Request received: {received} (source: {source})\n\
         - Rule: {CPRA_STATUTE_CITATION} requires a determination within \
         {CPRA_INITIAL_DETERMINATION_DAYS} calendar days of receipt, where {COURT_CALENDAR_BASIS}.\n\
         - {extension_line}\n\
         - Deadline for the determination (computed from the received date under that rule, not \
         a recorded date): {deadline}\n\
         - As of {letter_date}, {days_past} calendar {day_word} passed since that deadline.\n\n\
         Please send the agency's determination on this request and the estimated date on which \
         the records will be made available. If a determination or an extension notice was \
         already sent, please send a copy showing the date it was sent; my records may be \
         incomplete.\n\n\
         {agency_paragraph}\
         {JUDICIAL_RELIEF_SENTENCE}\n\n\
         {basis}\n\n\
         Sincerely,\n\
         [YOUR NAME]",
        source = request.received_source.trim(),
        basis = DraftEvidenceBasis::DeadlinePassedWithoutRecordedResponse.statement(),
    );
    let now = Utc::now();
    Ok(DraftedArtifact {
        id: DraftedArtifact::build_id(
            ArtifactKind::FollowupRequest,
            request.subject_id,
            TEMPLATE_ID,
            jurisdiction,
        ),
        kind: ArtifactKind::FollowupRequest,
        subject_finding_id: request.subject_id.to_string(),
        jurisdiction: jurisdiction.to_string(),
        template_id: TEMPLATE_ID.to_string(),
        generated_text: text,
        disclosure: disclosure.text(&FindingSubject {
            portal_key: agency_record.map(|record| record.portal_key.clone()),
            agency_id: agency_record.map(|record| record.agency_id),
            ..FindingSubject::default()
        }),
        cited_authorities: vec![
            CitedAuthority {
                statute: CPRA_STATUTE_CITATION.to_string(),
                quote: None,
            },
            CitedAuthority {
                statute: JUDICIAL_RELIEF_CITATION.to_string(),
                quote: None,
            },
        ],
        evidence_basis: DraftEvidenceBasis::DeadlinePassedWithoutRecordedResponse,
        // No determination was observed, so not every date input was recorded; 10000 is reserved
        // for that case and nothing else was measured here.
        confidence: FindingConfidence::Unassessed,
        citations: Vec::new(),
        unknown_facts,
        generated_at: now,
    })
}

/// Drafts the letter for one saved casework request, counted to `observed_at`.
pub fn draft_for_casework_request(
    case_id: &str,
    request: &CaseworkRequest,
    observed_at: &str,
    agency_record: Option<&AgencyRecord>,
    disclosure: &DisclosureContext,
) -> Result<DraftedArtifact, ArtifactError> {
    let Some(filed_on) = request.submitted_at.as_deref() else {
        return Err(unavailable(
            "no submission date is recorded for this request; record the date it was filed and save the case",
        ));
    };
    let recorded_response = match request.state.as_str() {
        "SUBMITTED" => None,
        "DRAFT" => {
            return Err(unavailable(
                "the request is still recorded as a draft, not as submitted",
            ))
        }
        state => Some(format!("the request's recorded state is {state}")),
    };
    let responses = request
        .productions
        .iter()
        .map(|production| production.received_at.clone())
        .collect::<Vec<_>>();
    let analysis = evaluate_filing(0, filed_on, &responses, observed_at)
        .map_err(|error| unavailable(error.to_string()))?;
    let label = if request.external_reference.trim().is_empty() {
        request.title.trim().to_string()
    } else {
        format!(
            "{} (agency reference {})",
            request.title.trim(),
            request.external_reference.trim()
        )
    };
    draft_overdue_followup(
        &OverdueRequest {
            subject_id: &format!("casework:{case_id}:{}", request.id),
            addressee: &request.custodian,
            label: &label,
            received_source: "the submission date I recorded when I filed the request; the count \
                              treats that date as the date of receipt",
            recorded_response: recorded_response.as_deref(),
        },
        &analysis,
        agency_record,
        JURISDICTION,
        disclosure,
    )
}

/// Loads the saved request and, when an agency was chosen, its counted record, then drafts.
pub async fn draft_for_saved_casework_request(
    repository: &DuckDbRepository,
    case_id: String,
    request_id: String,
    agency: Option<AgencyIdentity>,
    disclosure: &DisclosureContext,
    now: DateTime<Utc>,
) -> Result<OverdueFollowUpDraft, AppError> {
    let request = repository
        .saved_casework_request(case_id.clone(), request_id)
        .await?;
    let observed_at = now.to_rfc3339();
    let agency_record = match agency {
        Some(identity) => Some(
            repository
                .agency_record(identity, observed_at.clone())
                .await?,
        ),
        None => None,
    };
    let artifact = draft_for_casework_request(
        &case_id,
        &request,
        &observed_at,
        agency_record.as_ref(),
        disclosure,
    )
    .map_err(|error| AppError::Analysis(error.to_string()))?;
    let agency_paragraph_included = agency_record
        .as_ref()
        .is_some_and(|record| record.paragraph().is_some());
    Ok(OverdueFollowUpDraft {
        artifact,
        agency_record,
        agency_paragraph_included,
        agency_options: repository.agency_record_options().await?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::artifacts::agency_record::MINIMUM_CLOSED_REQUESTS;
    use crate::analysis::timeline::evaluate_statutory_compliance_with_determination_at;
    use crate::types::{CaseworkProduction, StatutoryExtensionReason, StatutoryExtensionRecord};

    fn request(state: &str, submitted_at: Option<&str>) -> CaseworkRequest {
        CaseworkRequest {
            id: "r1".into(),
            title: "Monthly bed-night reports".into(),
            custodian: "Housing Commission".into(),
            state: state.into(),
            submitted_at: submitted_at.map(str::to_string),
            external_reference: "PR-42".into(),
            ..CaseworkRequest::default()
        }
    }

    fn draft(
        request: &CaseworkRequest,
        observed_on: &str,
        record: Option<&AgencyRecord>,
    ) -> Result<DraftedArtifact, ArtifactError> {
        draft_for_casework_request(
            "case-a",
            request,
            &format!("{observed_on}T15:00:00Z"),
            record,
            &DisclosureContext::unset(),
        )
    }

    fn refusal(result: Result<DraftedArtifact, ArtifactError>) -> String {
        match result {
            Err(ArtifactError::FollowUpNotAvailable(reason)) => reason,
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    fn agency(closed: u64) -> AgencyRecord {
        AgencyRecord {
            portal_key: "sd".into(),
            agency_id: 7,
            agency_name: Some("Purchasing".into()),
            closed_requests: closed,
            sourced_determinations: 6,
            late_sourced_determinations: 2,
            no_records_closures: 3,
            no_records_with_recorded_earlier_release: 1,
            period: Some(("2025-01-02".into(), "2026-03-04".into())),
        }
    }

    #[test]
    fn no_letter_on_the_deadline_day_and_one_day_counted_the_day_after() {
        // Received Monday 2026-03-02; ten calendar days later is Thursday 2026-03-12.
        let filed = request("SUBMITTED", Some("2026-03-02"));
        let on_the_day = refusal(draft(&filed, "2026-03-12", None));
        assert!(on_the_day.contains("due on 2026-03-12"), "{on_the_day}");
        let text = draft(&filed, "2026-03-13", None).unwrap().generated_text;
        assert!(text.contains("Date: 2026-03-13"));
        assert!(text.contains("Request received: 2026-03-02 (source: the submission date"));
        assert!(text.contains("not a recorded date): 2026-03-12"));
        assert!(text.contains("1 calendar day has passed since that deadline"));
        assert!(text.contains(CPRA_STATUTE_CITATION));
        assert!(text.contains("No extension notice is recorded"));
        assert!(text.contains("Monthly bed-night reports (agency reference PR-42)"));
        assert!(!text.contains('{'));
        let later = draft(&filed, "2026-03-20", None).unwrap().generated_text;
        assert!(later.contains("8 calendar days have passed"));
    }

    #[test]
    fn a_deadline_landing_on_a_court_holiday_moves_to_the_next_court_day() {
        // Ten days from 2025-12-22 is 2026-01-01, a judicial holiday; the deadline is Friday the 2nd.
        let filed = request("SUBMITTED", Some("2025-12-22"));
        assert!(refusal(draft(&filed, "2026-01-02", None)).contains("due on 2026-01-02"));
        let text = draft(&filed, "2026-01-03", None).unwrap().generated_text;
        assert!(text.contains("not a recorded date): 2026-01-02"));
        assert!(text.contains("1 calendar day has passed"));
        assert!(text.contains(COURT_CALENDAR_BASIS));
    }

    fn evaluated(
        extension: Option<&StatutoryExtensionRecord>,
        observed_on: &str,
    ) -> StatutoryComplianceAnalysis {
        evaluate_statutory_compliance_with_determination_at(
            0,
            "2026-03-02T00:00:00Z",
            None,
            None,
            None,
            None,
            extension,
            &format!("{observed_on}T00:00:00Z"),
        )
        .expect("evaluates")
    }

    fn overdue(source: &'static str) -> OverdueRequest<'static> {
        OverdueRequest {
            subject_id: "tracked:1",
            addressee: "",
            label: "Request 24-1",
            received_source: source,
            recorded_response: None,
        }
    }

    fn from_analysis(
        analysis: &StatutoryComplianceAnalysis,
        source: &'static str,
        jurisdiction: &str,
    ) -> Result<DraftedArtifact, ArtifactError> {
        draft_overdue_followup(
            &overdue(source),
            analysis,
            None,
            jurisdiction,
            &DisclosureContext::unset(),
        )
    }

    #[test]
    fn a_valid_recorded_extension_moves_the_boundary_and_a_defective_one_refuses() {
        let valid = StatutoryExtensionRecord {
            notice_at: "2026-03-05T00:00:00Z".into(),
            expected_determination_at: "2026-03-20T00:00:00Z".into(),
            reason: StatutoryExtensionReason::VoluminousRecords,
            notice_source: "extension letter".into(),
        };
        // Past the initial deadline of 2026-03-12 but on the extended one: still no letter.
        let on_the_day = from_analysis(&evaluated(Some(&valid), "2026-03-20"), "portal", "US-CA");
        assert!(refusal(on_the_day).contains("due on 2026-03-20"));
        let artifact =
            from_analysis(&evaluated(Some(&valid), "2026-03-21"), "portal", "US-CA").unwrap();
        let text = &artifact.generated_text;
        assert!(text.contains("An extension notice is recorded (reason VOLUMINOUS_RECORDS)"));
        assert!(text.contains("not a recorded date): 2026-03-20"));
        assert!(text.contains("1 calendar day has passed"));
        assert!(text.contains("To: [AGENCY NAME]"));
        assert!(artifact.unknown_facts.contains(&"Agency name".to_string()));

        let unsourced = StatutoryExtensionRecord {
            notice_source: String::new(),
            ..valid
        };
        let reason = refusal(from_analysis(
            &evaluated(Some(&unsourced), "2026-04-30"),
            "portal",
            "US-CA",
        ));
        assert!(reason.contains("MissingNoticeSource"), "{reason}");
    }

    #[test]
    fn each_missing_or_unaccepted_input_refuses_with_its_own_reason() {
        let observed = "2026-04-30";
        let reason = refusal(draft(&request("SUBMITTED", None), observed, None));
        assert!(
            reason.contains("no submission date is recorded"),
            "{reason}"
        );
        let reason = refusal(draft(&request("DRAFT", Some("2026-03-02")), observed, None));
        assert!(reason.contains("still recorded as a draft"), "{reason}");
        let reason = refusal(draft(
            &request("ACKNOWLEDGED", Some("2026-03-02")),
            observed,
            None,
        ));
        assert!(
            reason.contains("recorded state is ACKNOWLEDGED"),
            "{reason}"
        );

        let mut produced = request("SUBMITTED", Some("2026-03-02"));
        produced.productions.push(CaseworkProduction {
            received_at: "2026-03-25".into(),
            ..CaseworkProduction::default()
        });
        let reason = refusal(draft(&produced, observed, None));
        assert!(
            reason.contains("activity is recorded on 2026-03-25"),
            "{reason}"
        );

        // A submission date after the letter date is not counted from.
        let reason = refusal(draft(
            &request("SUBMITTED", Some("2026-05-10")),
            observed,
            None,
        ));
        assert!(reason.contains("observed before"), "{reason}");

        let analysis = evaluated(None, observed);
        let reason = refusal(from_analysis(&analysis, "  ", "US-CA"));
        assert!(reason.contains("no recorded source"), "{reason}");
        let reason = refusal(from_analysis(&analysis, "portal", "EU-DE"));
        assert!(reason.contains("EU-DE"), "{reason}");

        let determined = evaluate_statutory_compliance_with_determination_at(
            0,
            "2026-03-02T00:00:00Z",
            None,
            None,
            Some("2026-03-25T00:00:00Z"),
            Some("portal event log"),
            None,
            "2026-04-30T00:00:00Z",
        )
        .unwrap();
        let reason = refusal(from_analysis(&determined, "portal", "US-CA"));
        assert!(reason.contains("a determination is recorded"), "{reason}");
    }

    #[test]
    fn the_agency_paragraph_appears_only_from_the_minimum_and_the_letter_draws_no_conclusion() {
        let filed = request("SUBMITTED", Some("2026-03-02"));
        let below = agency(MINIMUM_CLOSED_REQUESTS - 1);
        let without = draft(&filed, "2026-03-20", Some(&below)).unwrap();
        assert!(!without.generated_text.contains("my own library"));
        assert_eq!(
            without.generated_text,
            draft(&filed, "2026-03-20", None).unwrap().generated_text
        );
        let at = agency(MINIMUM_CLOSED_REQUESTS);
        let with = draft(&filed, "2026-03-20", Some(&at)).unwrap();
        let text = with.generated_text.to_lowercase();
        assert!(text.contains("2 of those 6 are dated after the deadline"));
        assert!(text.contains("for 1 of those 3"));
        assert!(text.contains("not from all requests the agency received"));
        for word in [
            "violat",
            "unlawful",
            "illegal",
            "breach",
            "intent",
            "deliberate",
            "willful",
            "refus",
            "ignor",
            "bad faith",
            "appeal",
            "petition",
        ] {
            assert!(!text.contains(word), "letter contains '{word}'");
        }
        assert_eq!(with.kind, ArtifactKind::FollowupRequest);
        assert_eq!(with.confidence, FindingConfidence::Unassessed);
    }
}
