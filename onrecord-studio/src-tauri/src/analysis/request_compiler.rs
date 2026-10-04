//! Compiles a casework records request into a CPRA request scoped to the records that identified
//! it, and schedules its follow-ups from the statutory engine.
//!
//! Two rules bound everything here. The draft is a CPRA request, so the clock is the CPRA rule in
//! [`crate::analysis::statute`] and a draft cites that and names the Act otherwise; no other
//! citation is invented. And no clock exists until a filing date is recorded: a request with no
//! recorded submission has no [`RequestFilingPlan`] at all rather than a plan reading zero days.

use crate::analysis::court_calendar::COURT_CALENDAR_BASIS;
use crate::analysis::statute::{DeterminationBasis, CPRA_DETERMINATION_RULE};
use crate::analysis::timeline::{
    evaluate_statutory_clock, DeterminationEvidence, StatutoryComplianceAnalysis,
    CPRA_INITIAL_DETERMINATION_DAYS, CPRA_MAXIMUM_EXTENSION_DAYS, CPRA_STATUTE_CITATION,
};
use crate::error::AppError;
use serde::{Deserialize, Serialize};

/// What a scheduled follow-up is waiting on. Every variant is reached only from a value the
/// statutory evaluation returned, never from an elapsed-time guess.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FollowUpTrigger {
    /// The determination period is still running; the follow-up is due on the deadline itself.
    DeterminationDue,
    /// The deadline passed with nothing recorded at or after receipt. On
    /// [`DeterminationBasis::ObservationWithoutRecordedActivity`] this states only that the
    /// deadline passed with no recorded response, which is why the rationale carries the basis.
    DeterminationOverdue,
    /// A response was recorded and the production still does not cover the request.
    ProductionIncomplete,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScheduledFollowUp {
    pub trigger: FollowUpTrigger,
    /// The calendar date the follow-up is owed on, in the request's own recorded dates.
    pub scheduled_for: String,
    /// The violation math and its basis, in the words the engine's own evaluation supports.
    pub rationale: String,
}

/// The statutory standing of one filed records request, derived entirely from
/// [`evaluate_statutory_clock`] under the CPRA rule.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RequestFilingPlan {
    pub statute_citation: String,
    /// The calendar date the elapsed period was counted to. The plan is recomputed on save, so
    /// a reader must see when the count was taken rather than read it as current.
    pub evaluated_at: String,
    pub filed_on: String,
    pub determination_due_on: String,
    pub calendar_days_elapsed: i64,
    pub allowed_calendar_days: i64,
    pub determination_basis: DeterminationBasis,
    /// The earliest recorded production date, which is what the elapsed period was measured to
    /// when one exists. Absent means nothing was recorded at or after the filing date.
    pub measured_to: Option<String>,
    pub deadline_passed_without_recorded_response: bool,
    pub follow_ups: Vec<ScheduledFollowUp>,
}

/// The records a request names, and the records already held that identify them.
pub struct RequestScope<'a> {
    pub title: &'a str,
    pub custodian: &'a str,
    pub record_types: &'a [String],
    pub fields: &'a [String],
    pub period: &'a str,
    pub resolves: &'a str,
    /// Each identifying record as `(label, sha256)`: what in hand establishes that the requested
    /// records exist and who holds them.
    pub identified_by: &'a [(String, String)],
}

fn bulleted(values: &[String], absent: &str) -> String {
    let lines = values
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(|value| format!("- {value}"))
        .collect::<Vec<_>>();
    if lines.is_empty() {
        absent.to_string()
    } else {
        lines.join("\n")
    }
}

/// Drafts the request. Nothing is sent: the operator files it through their own channel and
/// records the confirmed send instant afterwards.
pub fn compile_cpra_request(scope: &RequestScope<'_>) -> String {
    let identified = scope
        .identified_by
        .iter()
        .map(|(label, sha256)| format!("{label} (SHA-256 {sha256})"))
        .collect::<Vec<_>>();
    let period = if scope.period.trim().is_empty() {
        "Period: not stated. No period was recorded with this request.".to_string()
    } else {
        format!("Period: {}", scope.period.trim())
    };
    format!(
        "To: {custodian}\n\
         Subject: California Public Records Act request \u{2014} {title}\n\n\
         Under the California Public Records Act I request the following disclosable public \
         records in your agency's possession, custody or control.\n\n\
         Records requested:\n{records}\n\n\
         Fields or data elements sought within those records:\n{fields}\n\n\
         {period}\n\n\
         Records already in my possession that identify the records requested above:\n{identified}\n\n\
         Determination: {CPRA_STATUTE_CITATION} requires a determination whether this request \
         seeks disclosable records, and prompt notice of that determination and its reasons, \
         within {CPRA_INITIAL_DETERMINATION_DAYS} calendar days of receipt, where \
         {COURT_CALENDAR_BASIS}. A single written extension of up to \
         {CPRA_MAXIMUM_EXTENSION_DAYS} days is available only on written notice stating a reason \
         the statute recognizes.\n\n\
         Please produce records in the electronic format in which they are kept, identify any \
         withheld record or portion and the exemption claimed for it, and provide all reasonably \
         segregable responsive material. If any requested record does not exist, please say so \
         expressly rather than omitting it from the response.\n\n\
         What these records would establish: {resolves}\n\n\
         Draft for review; this application sends nothing.",
        custodian = if scope.custodian.trim().is_empty() {
            "Records custodian"
        } else {
            scope.custodian.trim()
        },
        title = scope.title.trim(),
        records = bulleted(
            scope.record_types,
            "- No record type was recorded with this request; state one before filing."
        ),
        fields = bulleted(
            scope.fields,
            "- No field was recorded with this request; the request is for the records as kept."
        ),
        identified = bulleted(
            &identified,
            "- None recorded. This request does not rest on a record already in hand."
        ),
        resolves = scope.resolves.trim(),
    )
}

/// Evaluates the filed request against the modeled clock and schedules its follow-ups.
///
/// `filed_on` and `responses_on` are the request's own `YYYY-MM-DD` dates; they are widened to
/// instants at the start (filing) and end (responses) of the day in UTC because that is the whole
/// precision the records carry, and the engine measures in calendar days.
///
/// `position` labels the engine's own evidence summary only. A casework request is keyed by a
/// string id the engine has no numeric form for, and nothing here reproduces that summary.
pub fn plan_filing(
    position: u64,
    filed_on: &str,
    responses_on: &[String],
    omissions: &[String],
    observed_at: &str,
) -> Result<RequestFilingPlan, AppError> {
    let earliest_response = responses_on
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .min()
        .map(day_end);
    let analysis = evaluate_statutory_clock(
        &CPRA_DETERMINATION_RULE,
        position,
        &day_start(filed_on),
        // A recorded production is an upper bound on when the agency acted, not a noticed
        // determination, so it enters as the activity proxy and never as a sourced determination.
        DeterminationEvidence {
            updated_at: earliest_response.as_deref(),
            ..DeterminationEvidence::default()
        },
        None,
        observed_at,
    )?;
    Ok(build_plan(&analysis, omissions, observed_at))
}

fn day_start(date: &str) -> String {
    format!("{}T00:00:00Z", date.trim())
}

/// A production is known to its day only. Placing it at the day's end keeps one on the filing day
/// after the filing instant, so it counts as activity after receipt; the day is unchanged.
fn day_end(date: &str) -> String {
    format!("{}T23:59:59Z", date.trim())
}

fn calendar_date(instant: &str) -> String {
    instant.chars().take(10).collect()
}

fn build_plan(
    analysis: &StatutoryComplianceAnalysis,
    omissions: &[String],
    observed_at: &str,
) -> RequestFilingPlan {
    let due_on = calendar_date(&analysis.effective_deadline_at);
    let measured_to = analysis.determination_at.as_deref().map(calendar_date);
    let deadline_passed_without_recorded_response = analysis.is_violation
        && analysis.determination_basis == DeterminationBasis::ObservationWithoutRecordedActivity;
    let mut follow_ups = Vec::with_capacity(2);
    if analysis.is_violation {
        follow_ups.push(ScheduledFollowUp {
            trigger: FollowUpTrigger::DeterminationOverdue,
            scheduled_for: due_on.clone(),
            rationale: format!(
                "{} calendar days elapsed against {} allowed under {}, where {}. {}",
                analysis.calendar_days_elapsed,
                analysis.allowed_calendar_days,
                analysis.statute_citation,
                COURT_CALENDAR_BASIS,
                basis_caveat(analysis.determination_basis),
            ),
        });
    } else {
        follow_ups.push(ScheduledFollowUp {
            trigger: FollowUpTrigger::DeterminationDue,
            scheduled_for: due_on.clone(),
            rationale: format!(
                "{} of {} allowed calendar days have elapsed under {}; the determination period is \
                 within the modeled period. {}",
                analysis.calendar_days_elapsed,
                analysis.allowed_calendar_days,
                analysis.statute_citation,
                basis_caveat(analysis.determination_basis),
            ),
        });
    }
    if measured_to.is_some() && !omissions.is_empty() {
        follow_ups.push(ScheduledFollowUp {
            trigger: FollowUpTrigger::ProductionIncomplete,
            scheduled_for: measured_to.clone().unwrap_or_else(|| due_on.clone()),
            rationale: format!(
                "{} recorded expectation(s) are still not covered by any recorded production. \
                 Unmet expectations are a comparison of this request against what was recorded as \
                 produced, not a finding that anything was unlawfully withheld.",
                omissions.len()
            ),
        });
    }
    RequestFilingPlan {
        statute_citation: analysis.statute_citation.clone(),
        evaluated_at: calendar_date(observed_at),
        filed_on: calendar_date(&analysis.created_at),
        determination_due_on: due_on,
        calendar_days_elapsed: analysis.calendar_days_elapsed,
        allowed_calendar_days: analysis.allowed_calendar_days,
        determination_basis: analysis.determination_basis,
        measured_to,
        deadline_passed_without_recorded_response,
        follow_ups,
    }
}

fn basis_caveat(basis: DeterminationBasis) -> &'static str {
    match basis {
        DeterminationBasis::SourcedDeterminationEvent => {
            "The period was measured to a sourced determination event."
        }
        DeterminationBasis::EarliestRecordedActivityProxy => {
            "The period was measured to the earliest recorded production, which bounds when the \
             agency acted but is not itself evidence that the act was a determination."
        }
        DeterminationBasis::ObservationWithoutRecordedActivity => {
            "Nothing was recorded at or after filing, so the period was measured to today and is \
             still running; this states only that the deadline passed with no recorded response."
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_draft_scopes_to_the_identifying_records_and_cites_only_the_modeled_statute() {
        let identified = vec![("Shelter contract".to_string(), "a".repeat(64))];
        let draft = compile_cpra_request(&RequestScope {
            title: "Monthly bed-night reports",
            custodian: "Housing Commission",
            record_types: &["monthly performance report".to_string()],
            fields: &["occupied bed nights".to_string()],
            period: "FY2025",
            resolves: "delivered service volume",
            identified_by: &identified,
        });
        assert!(draft.contains("Shelter contract (SHA-256 aaaa"));
        assert!(draft.contains("occupied bed nights"));
        assert!(draft.contains(CPRA_STATUTE_CITATION));
        assert!(draft.contains(COURT_CALENDAR_BASIS));
        // Only the determination clock and its court-calendar rule are modeled; any section sign
        // beyond those two constants would be invented law.
        let modeled = CPRA_STATUTE_CITATION.matches('\u{a7}').count()
            + COURT_CALENDAR_BASIS.matches('\u{a7}').count();
        assert_eq!(draft.matches('\u{a7}').count(), modeled);
    }

    #[test]
    fn an_unscoped_draft_states_the_absence_instead_of_an_empty_list() {
        let draft = compile_cpra_request(&RequestScope {
            title: "Correspondence",
            custodian: "",
            record_types: &[],
            fields: &[],
            period: "   ",
            resolves: "whether the department was warned",
            identified_by: &[],
        });
        assert!(draft.contains("No record type was recorded"));
        assert!(draft.contains("Period: not stated"));
        assert!(draft.contains("None recorded. This request does not rest on a record"));
    }

    #[test]
    fn an_unanswered_request_past_its_deadline_schedules_an_overdue_follow_up_without_claiming_a_response(
    ) {
        let plan = plan_filing(1, "2026-09-01", &[], &[], "2026-10-01T00:00:00Z").unwrap();
        assert_eq!(plan.determination_due_on, "2026-09-11");
        assert!(plan.measured_to.is_none());
        assert!(plan.deadline_passed_without_recorded_response);
        assert_eq!(plan.evaluated_at, "2026-10-01");
        assert_eq!(
            plan.follow_ups[0].trigger,
            FollowUpTrigger::DeterminationOverdue
        );
        assert!(plan.follow_ups[0].rationale.contains("30 calendar days"));
        assert!(plan.follow_ups[0].rationale.contains("still running"));
    }

    #[test]
    fn a_recorded_production_measures_the_period_and_an_omission_adds_a_production_follow_up() {
        let plan = plan_filing(
            1,
            "2026-09-01",
            &["2026-09-20".to_string(), "2026-09-08".to_string()],
            &["Expected field not yet recorded as produced: invoice month".to_string()],
            "2026-10-01T00:00:00Z",
        )
        .unwrap();
        assert_eq!(plan.measured_to.as_deref(), Some("2026-09-08"));
        assert!(!plan.deadline_passed_without_recorded_response);
        assert_eq!(
            plan.determination_basis,
            DeterminationBasis::EarliestRecordedActivityProxy
        );
        assert_eq!(
            plan.follow_ups
                .iter()
                .map(|follow_up| follow_up.trigger)
                .collect::<Vec<_>>(),
            vec![
                FollowUpTrigger::DeterminationDue,
                FollowUpTrigger::ProductionIncomplete
            ]
        );
    }
}
