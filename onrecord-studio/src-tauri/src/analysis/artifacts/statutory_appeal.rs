//! Appeal drafted from a request the statute evaluator counts as past its statutory deadline.
//!
//! This is the other legal-claim source besides `FindingKind::PolicyViolationDetected`. It is
//! built from one request's recorded dates, not from a finding, and it refuses whenever the count
//! would rest on anything the record does not hold:
//!
//! - The determination must be a sourced event. A closure or activity timestamp is a proxy, and an
//!   unfinished period is an observation; neither is counted here.
//! - The extension record must be absent or valid. A defective extension changes which deadline
//!   applies, which is a question for the person reviewing the request, not a drafted assertion.
//!
//! Every date printed is one the request recorded. The deadline is computed, and the draft says so.

use super::{ArtifactError, ArtifactKind, CitedAuthority, DraftEvidenceBasis, DraftedArtifact};
use crate::profile::DisclosureContext;
use crate::{
    analysis::{
        finding::{FindingConfidence, FindingSubject},
        statute::{DeterminationBasis, CPRA_STATUTE_CITATION},
        timeline::{
            evaluate_statutory_compliance_with_determination_at, StatutoryComplianceAnalysis,
            StatutoryExtensionStatus,
        },
    },
    error::AppError,
    storage::DuckDbRepository,
};
use chrono::Utc;

const TEMPLATE_ID: &str = "cpra_appeal_statutory_deadline_v1";
const JURISDICTION: &str = "US-CA";

/// Pure drafting step over an evaluated request.
pub fn draft_statutory_deadline_appeal(
    portal_key: &str,
    agency_name: Option<&str>,
    analysis: &StatutoryComplianceAnalysis,
    jurisdiction: &str,
    disclosure: &DisclosureContext,
) -> Result<DraftedArtifact, ArtifactError> {
    if jurisdiction != JURISDICTION {
        return Err(ArtifactError::AppealNotSupported(format!(
            "no statute is registered for jurisdiction {jurisdiction}"
        )));
    }
    if !analysis.is_violation {
        return Err(ArtifactError::AppealNotSupported(
            "the evaluator does not count this request as past its deadline".to_string(),
        ));
    }
    if analysis.determination_basis != DeterminationBasis::SourcedDeterminationEvent {
        return Err(ArtifactError::AppealNotSupported(format!(
            "the determination date is not a sourced event ({}); a proxy or an unfinished period is not counted",
            analysis.determination_basis.as_str()
        )));
    }
    if !matches!(
        analysis.extension_status,
        StatutoryExtensionStatus::NotInvoked | StatutoryExtensionStatus::Valid
    ) {
        return Err(ArtifactError::AppealNotSupported(format!(
            "the recorded extension is {:?}; review the extension record before drafting",
            analysis.extension_status
        )));
    }
    // Both guards above make `determination_at` and its source present; still not assumed.
    let determination_at = analysis
        .determination_at
        .as_deref()
        .ok_or_else(|| ArtifactError::MissingField("determination timestamp".to_string()))?;
    let determination_source = analysis
        .determination_source
        .as_deref()
        .ok_or_else(|| ArtifactError::MissingField("determination source".to_string()))?;

    let now = Utc::now();
    let request_id = analysis.request_id;
    let (agency_line, mut unknown_facts) = match agency_name {
        Some(name) => (name.to_string(), Vec::new()),
        None => ("[AGENCY NAME]".to_string(), vec!["Agency name".to_string()]),
    };
    unknown_facts.push("Your name and contact details".to_string());
    unknown_facts.push("Whether you are the requester on this request".to_string());
    let extension_line = match (&analysis.extension_reason, analysis.extension_status) {
        (Some(reason), StatutoryExtensionStatus::Valid) => format!(
            "A valid extension is recorded (reason {reason}); the allowed period below includes it."
        ),
        _ => "No extension is recorded for this request.".to_string(),
    };
    let text = format!(
        "Subject: Appeal: determination on request {request_id} issued after the statutory deadline\n\n\
         Date: {today}\n\
         To: {agency_line} (Records Appeals or Custodian of Records)\n\
         Re: Request {request_id} on {portal_key}\n\n\
         I am appealing the timing of the agency's determination on request {request_id}.\n\n\
         Statute: {CPRA_STATUTE_CITATION}.\n\n\
         Recorded facts:\n\
         - Request received: {created}\n\
         - Determination: {determination_at}, from a sourced determination event (source: {determination_source})\n\
         - Calendar days from receipt to determination: {elapsed}\n\
         - Calendar days allowed: {allowed}\n\
         - Deadline (computed from the received date, not a recorded date): {deadline}\n\
         - {extension_line}\n\n\
         Evaluator record: {summary}\n\n\
         {basis}\n\n\
         I ask the agency to acknowledge that the determination was issued after the period the \
         statute allows, and to state what it will do to prevent a recurrence.\n\n\
         Sincerely,\n\
         [YOUR NAME]",
        today = now.format("%Y-%m-%d"),
        created = analysis.created_at,
        elapsed = analysis.calendar_days_elapsed,
        allowed = analysis.allowed_calendar_days,
        deadline = analysis.effective_deadline_at,
        summary = analysis.evidence_summary,
        basis = DraftEvidenceBasis::CountedStatutoryDeadline.statement(),
    );
    let disclosure_text = disclosure.text(&FindingSubject {
        portal_key: Some(portal_key.to_string()),
        ..FindingSubject::default()
    });
    Ok(DraftedArtifact {
        id: DraftedArtifact::build_id(
            ArtifactKind::Appeal,
            &format!("request:{portal_key}:{request_id}"),
            TEMPLATE_ID,
            jurisdiction,
        ),
        kind: ArtifactKind::Appeal,
        subject_finding_id: format!("request:{portal_key}:{request_id}"),
        jurisdiction: jurisdiction.to_string(),
        template_id: TEMPLATE_ID.to_string(),
        generated_text: text,
        disclosure: disclosure_text,
        cited_authorities: vec![CitedAuthority {
            statute: CPRA_STATUTE_CITATION.to_string(),
            quote: None,
        }],
        evidence_basis: DraftEvidenceBasis::CountedStatutoryDeadline,
        // A sourced determination and an absent or valid extension mean every date input to the
        // count was recorded; the project reserves 10000 for exactly that case.
        confidence: FindingConfidence::measured(10_000),
        citations: Vec::new(),
        unknown_facts,
        generated_at: now,
    })
}

/// Loads one request, evaluates it with the shared statute evaluator, and drafts the appeal.
pub async fn draft_for_request(
    repository: &DuckDbRepository,
    portal_key: &str,
    request_id: u64,
    jurisdiction: &str,
    disclosure: &DisclosureContext,
) -> Result<DraftedArtifact, AppError> {
    let evidence = repository
        .load_legal_brief_evidence(request_id, portal_key)
        .await?;
    let analysis = evaluate_statutory_compliance_with_determination_at(
        request_id,
        &evidence.created_at,
        Some(&evidence.updated_at),
        evidence.closed_at.as_deref(),
        evidence.determination_at.as_deref(),
        evidence.determination_source.as_deref(),
        evidence.extension.as_ref(),
        &Utc::now().to_rfc3339(),
    )?;
    let agency = repository
        .agency_label_for_request(portal_key, request_id)
        .await?;
    draft_statutory_deadline_appeal(
        portal_key,
        agency.as_deref(),
        &analysis,
        jurisdiction,
        disclosure,
    )
    .map_err(|error| AppError::Analysis(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{StatutoryExtensionReason, StatutoryExtensionRecord};

    fn analyze(
        determination: Option<(&str, &str)>,
        closed: Option<&str>,
        extension: Option<&StatutoryExtensionRecord>,
    ) -> StatutoryComplianceAnalysis {
        evaluate_statutory_compliance_with_determination_at(
            7,
            "2026-03-02T00:00:00Z",
            Some("2026-03-30T00:00:00Z"),
            closed,
            determination.map(|(at, _)| at),
            determination.map(|(_, source)| source),
            extension,
            "2026-04-30T00:00:00Z",
        )
        .expect("evaluates")
    }

    #[test]
    fn a_sourced_late_determination_is_drafted_with_recorded_dates_only() {
        let analysis = analyze(
            Some(("2026-03-20T00:00:00Z", "portal event log")),
            None,
            None,
        );
        let artifact = draft_statutory_deadline_appeal(
            "sd",
            Some("City Clerk"),
            &analysis,
            "US-CA",
            &DisclosureContext::unset(),
        )
        .unwrap();
        let text = &artifact.generated_text;
        assert!(text.contains(CPRA_STATUTE_CITATION));
        assert!(text.contains("2026-03-20T00:00:00+00:00"));
        assert!(text.contains("portal event log"));
        assert!(text.contains("Calendar days from receipt to determination: 18"));
        assert!(text.contains("computed from the received date, not a recorded date"));
        assert!(text.contains("City Clerk"));
        assert!(!text.contains('{'));
        assert_eq!(
            artifact.evidence_basis,
            DraftEvidenceBasis::CountedStatutoryDeadline
        );
    }

    #[test]
    fn a_missing_agency_name_is_a_placeholder_and_a_declared_unknown() {
        let analysis = analyze(Some(("2026-03-20T00:00:00Z", "log")), None, None);
        let artifact = draft_statutory_deadline_appeal(
            "sd",
            None,
            &analysis,
            "US-CA",
            &DisclosureContext::unset(),
        )
        .unwrap();
        assert!(artifact.generated_text.contains("[AGENCY NAME]"));
        assert!(artifact.unknown_facts.contains(&"Agency name".to_string()));
    }

    #[test]
    fn a_proxy_determination_or_an_unfinished_period_is_refused() {
        // Closed 18 days after receipt but with no sourced determination: a proxy.
        let proxy = analyze(None, Some("2026-03-20T00:00:00Z"), None);
        assert!(proxy.is_violation);
        assert!(matches!(
            draft_statutory_deadline_appeal(
                "sd",
                None,
                &proxy,
                "US-CA",
                &DisclosureContext::unset()
            ),
            Err(ArtifactError::AppealNotSupported(_))
        ));
    }

    #[test]
    fn an_on_time_determination_is_refused() {
        let on_time = analyze(Some(("2026-03-12T00:00:00Z", "log")), None, None);
        assert!(!on_time.is_violation);
        assert!(draft_statutory_deadline_appeal(
            "sd",
            None,
            &on_time,
            "US-CA",
            &DisclosureContext::unset()
        )
        .is_err());
    }

    #[test]
    fn a_defective_extension_is_refused_and_a_valid_one_moves_the_allowed_period() {
        let defective = StatutoryExtensionRecord {
            notice_at: "2026-03-05T00:00:00Z".into(),
            expected_determination_at: "2026-03-20T00:00:00Z".into(),
            reason: StatutoryExtensionReason::VoluminousRecords,
            notice_source: String::new(),
        };
        let analysis = analyze(
            Some(("2026-03-25T00:00:00Z", "log")),
            None,
            Some(&defective),
        );
        assert!(matches!(
            draft_statutory_deadline_appeal(
                "sd",
                None,
                &analysis,
                "US-CA",
                &DisclosureContext::unset()
            ),
            Err(ArtifactError::AppealNotSupported(_))
        ));
        let valid = StatutoryExtensionRecord {
            notice_source: "extension letter".into(),
            ..defective
        };
        let analysis = analyze(Some(("2026-03-25T00:00:00Z", "log")), None, Some(&valid));
        let artifact = draft_statutory_deadline_appeal(
            "sd",
            None,
            &analysis,
            "US-CA",
            &DisclosureContext::unset(),
        )
        .unwrap();
        assert!(artifact
            .generated_text
            .contains("Calendar days allowed: 18"));
        assert!(artifact.generated_text.contains("VOLUMINOUS_RECORDS"));
    }

    #[test]
    fn an_unregistered_jurisdiction_is_refused() {
        let analysis = analyze(Some(("2026-03-20T00:00:00Z", "log")), None, None);
        assert!(draft_statutory_deadline_appeal(
            "sd",
            None,
            &analysis,
            "EU-DE",
            &DisclosureContext::unset()
        )
        .is_err());
    }
}
