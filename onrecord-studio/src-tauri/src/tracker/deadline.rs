//! The statutory deadline for one tracked request, from the dates the user confirmed.
//!
//! The rules belong to `analysis::timeline` and `analysis::statute`; this module only maps the
//! tracker's events onto the evaluator's inputs and words the result. It adds no rule of its own.
//! Suggestions never reach it: they live outside `ClaimedRequest::events`.

use crate::analysis::statute::DeterminationBasis;
use crate::analysis::timeline::{
    evaluate_statutory_compliance_with_determination_at, StatutoryComplianceAnalysis,
    StatutoryExtensionStatus, CPRA_STATUTE_CITATION,
};
use crate::types::tracker::*;
use crate::types::StatutoryExtensionRecord;
use chrono::{DateTime, FixedOffset, Utc};

fn parsed(value: &str) -> Option<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(value).ok()
}

fn earliest<'a>(events: impl Iterator<Item = &'a TimelineEvent>) -> Option<&'a TimelineEvent> {
    events.min_by_key(|event| parsed(&event.occurred_at))
}

fn of_kind(
    claim: &ClaimedRequest,
    kind: TimelineEventKind,
    basis: Option<EventBasis>,
) -> impl Iterator<Item = &TimelineEvent> {
    claim
        .events
        .iter()
        .filter(move |event| event.kind == kind && basis.is_none_or(|basis| event.basis == basis))
}

fn day(value: &str) -> &str {
    value.get(..10).unwrap_or(value)
}

fn no_analysis(
    claim: &ClaimedRequest,
    model: DeadlineModel,
    kind: CountdownKind,
    statement: &str,
    notes: Vec<String>,
) -> RequestDeadline {
    RequestDeadline {
        claim_id: claim.id.clone(),
        model,
        analysis: None,
        countdown: Countdown { kind, days: None },
        determination_basis: None,
        violation_asserted: false,
        basis_statement: statement.to_string(),
        notes,
        pending_suggestion_count: claim.suggestions.len() as u32,
    }
}

pub fn request_deadline(claim: &ClaimedRequest, now: DateTime<Utc>) -> RequestDeadline {
    let mut notes = Vec::new();
    if !claim.suggestions.is_empty() {
        notes.push(format!(
            "{} suggested event(s) from emails await your confirmation and are not counted.",
            claim.suggestions.len()
        ));
    }
    if claim.jurisdiction.as_deref() != Some(CALIFORNIA_CPRA_JURISDICTION) {
        let why = if claim.jurisdiction.is_some() {
            "This app models only California CPRA deadlines"
        } else {
            "No jurisdiction is set, and this app models only California CPRA deadlines"
        };
        return no_analysis(
            claim,
            DeadlineModel::NoModel,
            CountdownKind::NoDeadlineModel,
            &format!("No deadline model for this jurisdiction. {why}, so no deadline is computed."),
            notes,
        );
    }
    let Some(submitted) = earliest(of_kind(claim, TimelineEventKind::Submitted, None)) else {
        return no_analysis(
            claim,
            DeadlineModel::CaliforniaCpra,
            CountdownKind::NoSubmissionDate,
            "No submission date is recorded, so no deadline is computed.",
            notes,
        );
    };
    let Some(created) = parsed(&submitted.occurred_at) else {
        return no_analysis(
            claim,
            DeadlineModel::CaliforniaCpra,
            CountdownKind::Unavailable,
            "The recorded submission date cannot be read.",
            notes,
        );
    };
    // Observed in the submission's own offset so calendar days match how the statute counts them.
    let observed = now.with_timezone(created.offset());
    if observed < created {
        return no_analysis(
            claim,
            DeadlineModel::CaliforniaCpra,
            CountdownKind::Unavailable,
            &format!(
                "The submission date {} is in the future, so no deadline is computed.",
                day(&submitted.occurred_at)
            ),
            notes,
        );
    }

    let sourced = earliest(of_kind(
        claim,
        TimelineEventKind::Determination,
        Some(EventBasis::EmailConfirmed),
    ));
    let entered = earliest(of_kind(
        claim,
        TimelineEventKind::Determination,
        Some(EventBasis::UserEntered),
    ));
    if sourced.is_some() && entered.is_some() {
        notes.push(
            "A determination confirmed from an email is used instead of the one you entered by hand."
                .to_string(),
        );
    }
    let determination = sourced.or(entered);
    if let Some(event) = determination {
        if parsed(&event.occurred_at).is_none_or(|date| date < created) {
            return no_analysis(
                claim,
                DeadlineModel::CaliforniaCpra,
                CountdownKind::Unavailable,
                &format!(
                    "The determination dated {} is before the submission dated {}. Correct one of them.",
                    day(&event.occurred_at),
                    day(&submitted.occurred_at)
                ),
                notes,
            );
        }
    }

    let extension_event = [EventBasis::EmailConfirmed, EventBasis::UserEntered]
        .into_iter()
        .find_map(|basis| {
            earliest(
                of_kind(claim, TimelineEventKind::ExtensionNotice, Some(basis)).filter(|event| {
                    event.extension_reason.is_some() && event.expected_determination_at.is_some()
                }),
            )
        });
    if of_kind(claim, TimelineEventKind::ExtensionNotice, None)
        .next()
        .is_some()
        && extension_event.is_none()
    {
        notes.push(
            "An extension notice is recorded without an expected determination date, so the initial 10-day deadline is shown."
                .to_string(),
        );
    }
    let extension = extension_event.and_then(|event| {
        Some(StatutoryExtensionRecord {
            notice_at: event.occurred_at.clone(),
            expected_determination_at: event.expected_determination_at.clone()?,
            reason: event.extension_reason?,
            // Only an email supplies a notice source. A hand-entered notice has none, and the
            // evaluator then declines to honor it rather than trusting an unsourced extension.
            notice_source: event
                .evidence
                .as_ref()
                .map(|evidence| evidence.message_id_hash.clone())
                .unwrap_or_default(),
        })
    });

    let (updated_at, determination_at, determination_source) = match determination {
        Some(event) if event.basis == EventBasis::EmailConfirmed => (
            None,
            Some(event.occurred_at.as_str()),
            event
                .evidence
                .as_ref()
                .map(|evidence| evidence.message_id_hash.as_str()),
        ),
        Some(event) => (Some(event.occurred_at.as_str()), None, None),
        None => (None, None, None),
    };
    let analysis = match evaluate_statutory_compliance_with_determination_at(
        0,
        &submitted.occurred_at,
        updated_at,
        None,
        determination_at,
        determination_source,
        extension.as_ref(),
        &observed.to_rfc3339(),
    ) {
        Ok(analysis) => analysis,
        Err(error) => {
            return no_analysis(
                claim,
                DeadlineModel::CaliforniaCpra,
                CountdownKind::Unavailable,
                &format!("The recorded dates cannot be evaluated together: {error}"),
                notes,
            )
        }
    };

    notes.extend(extension_note(analysis.extension_status));
    if determination.is_none()
        && of_kind(claim, TimelineEventKind::Closed, None)
            .next()
            .is_some()
    {
        notes.push(
            "A closure is recorded, but a closure is not a determination, so the count continues."
                .to_string(),
        );
    }

    let basis = analysis.determination_basis;
    let violation_asserted = analysis.is_violation && basis.is_sourced_determination();
    let countdown = countdown(&analysis, basis);
    let basis_statement = basis_statement(&analysis, basis, violation_asserted);
    RequestDeadline {
        claim_id: claim.id.clone(),
        model: DeadlineModel::CaliforniaCpra,
        determination_basis: Some(basis),
        violation_asserted,
        countdown,
        basis_statement,
        notes,
        pending_suggestion_count: claim.suggestions.len() as u32,
        analysis: Some(analysis),
    }
}

fn extension_note(status: StatutoryExtensionStatus) -> Option<String> {
    let problem = match status {
        StatutoryExtensionStatus::NotInvoked | StatutoryExtensionStatus::Valid => return None,
        StatutoryExtensionStatus::MissingNoticeSource => {
            return Some(
                "The extension notice you entered has no email source, so it is not applied. Import the notice email and confirm it to apply the extension."
                    .to_string(),
            )
        }
        StatutoryExtensionStatus::NoticeBeforeReceipt => "the notice predates the request",
        StatutoryExtensionStatus::NoticeAfterInitialDeadline => {
            "the notice came after the initial 10 days"
        }
        StatutoryExtensionStatus::ExpectedDateNotExtended => {
            "the expected date is not later than the initial deadline"
        }
        StatutoryExtensionStatus::ExtensionExceedsFourteenDays => {
            "the expected date is more than 14 days past the initial deadline"
        }
    };
    Some(format!(
        "The extension notice does not meet the statute's conditions ({problem}), so the initial deadline is shown."
    ))
}

fn countdown(analysis: &StatutoryComplianceAnalysis, basis: DeterminationBasis) -> Countdown {
    if basis.is_recorded_activity() {
        return Countdown {
            kind: CountdownKind::DeterminationRecorded,
            days: None,
        };
    }
    let remaining = analysis.allowed_calendar_days - analysis.calendar_days_elapsed;
    if remaining >= 0 {
        Countdown {
            kind: CountdownKind::DaysRemaining,
            days: Some(remaining),
        }
    } else {
        Countdown {
            kind: CountdownKind::DeadlinePassed,
            days: Some(-remaining),
        }
    }
}

fn basis_statement(
    analysis: &StatutoryComplianceAnalysis,
    basis: DeterminationBasis,
    violation_asserted: bool,
) -> String {
    let determined = analysis
        .determination_at
        .as_deref()
        .map(day)
        .unwrap_or_default();
    let counted = format!(
        "{} of {} allowed calendar days from submission",
        analysis.calendar_days_elapsed, analysis.allowed_calendar_days
    );
    match basis {
        DeterminationBasis::SourcedDeterminationEvent => {
            let verdict = if violation_asserted {
                format!("This exceeds the allowed period: a violation of {CPRA_STATUTE_CITATION} is counted.")
            } else {
                "This is within the allowed period.".to_string()
            };
            format!(
                "Measured to a determination on {determined} that you confirmed from an email: {counted}. {verdict}"
            )
        }
        DeterminationBasis::EarliestRecordedActivityProxy => format!(
            "Measured to {determined}, a determination date you entered by hand. It has no notice source, so it is a proxy, not a sourced determination, and no violation is asserted: {counted}. Confirm the determination from the agency's email for a sourced result."
        ),
        DeterminationBasis::ObservationWithoutRecordedActivity => format!(
            "No determination is recorded, so the period is measured to today: {counted}. This is provisional: the agency may have responded outside this record."
        ),
    }
}
