//! The single authoritative model of California Public Records Act determination timing.
//!
//! Two evaluation paths consume this module and neither of them owns the rules:
//!
//! * [`crate::analysis::timeline`] evaluates one request in Rust. Its
//!   [`StatutoryComplianceAnalysis`](crate::analysis::StatutoryComplianceAnalysis) is what the
//!   legal-brief export and the repository's timeline-audit refresh are built from.
//! * [`crate::analysis::anomaly`] evaluates every request at once inside DuckDB to produce the
//!   per-agency aggregates.
//!
//! The aggregate path cannot call the Rust evaluator row by row without giving up vectorized
//! execution, so it is *generated* from the constants and SQL fragments below instead. Every
//! threshold, the determination proxy, the extension-validity rule, and the calendar-day
//! derivation therefore have exactly one definition. `analysis::anomaly`'s parity test drives both
//! renderings from one table of cases so they cannot diverge silently.
//!
//! # Column contract for the SQL fragments
//!
//! A query that renders [`sql::render`] must open with `WITH {court_holidays_cte},` and expose
//! these columns in scope of the fragment:
//!
//! | column | meaning |
//! |---|---|
//! | `created_timestamp` | receipt instant |
//! | `updated_timestamp` / `closed_timestamp` | activity instants, nullable |
//! | `created_date` / `updated_date` / `closed_date` | the *recorded-offset* calendar dates |
//! | `determination_timestamp` / `explicit_determination_date` | sourced determination, nullable |
//! | `determination_source` | notice/source identifier required for the explicit date |
//! | `extension_notice_timestamp` | extension notice instant, nullable |
//! | `extension_notice_date` / `extension_expected_date` | extension calendar dates |
//! | `extension_reason` / `extension_notice_source` | extension provenance |
//! | `determination_date` | output of [`sql::determination_calendar_date`] |
//! | `initial_deadline_date` | output of [`sql::initial_deadline_date`] |
//! | `has_valid_extension` | output of [`sql::valid_extension_predicate`] |
//! | `allowed_calendar_days` | output of [`sql::allowed_calendar_days`] |
//! | `normalized_status` / `latency_seconds` | lifecycle aggregates |

use crate::types::StatutoryExtensionReason;
use serde::{Deserialize, Serialize};

/// Calendar days an agency has to determine whether a request seeks disclosable records.
pub const CPRA_INITIAL_DETERMINATION_DAYS: u64 = 10;
/// Calendar days a single written extension may add to the initial determination period.
pub const CPRA_MAXIMUM_EXTENSION_DAYS: u64 = 14;
/// Citation recorded on every finding this model produces.
pub const CPRA_STATUTE_CITATION: &str = "California Government Code § 7922.535";
/// A closure this soon after receipt is recorded as a lifecycle anomaly, not a statutory finding.
pub const SUSPICIOUS_CLOSURE_THRESHOLD_SECONDS: i64 = 300;

/// What a compliance evaluation actually measured the elapsed period against.
///
/// The statute runs to the agency's *determination*, which is a distinct, noticed event. Sources
/// may carry that event and its provenance explicitly. When absent, both evaluation paths report
/// which substitute they used, and no consumer may present a proxy as a measured event.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DeterminationBasis {
    /// MEASURED. The source carried a distinct determination timestamp and a non-empty notice or
    /// source identifier, both retained with the request.
    SourcedDeterminationEvent,
    /// PROXY. The earliest recorded portal activity at or after receipt (`updated_at`, else
    /// `closed_at`) stood in for the determination. Activity is an upper bound on when the agency
    /// acted, not evidence that the act was a determination: a request touched for an unrelated
    /// reason reports an earlier date than the determination it actually issued, and a request
    /// whose determination was communicated off-portal is not represented here at all.
    EarliestRecordedActivityProxy,
    /// PROVISIONAL. Nothing was recorded at or after receipt, so the elapsed period was measured
    /// to the observation instant. The period is still running; a violation reported on this basis
    /// states only that the deadline has passed with no recorded response.
    ObservationWithoutRecordedActivity,
}

impl DeterminationBasis {
    /// Whether the evaluation measured to something the agency actually did.
    ///
    /// False means the elapsed period was measured to "now" and will keep growing.
    pub const fn is_recorded_activity(self) -> bool {
        matches!(
            self,
            Self::SourcedDeterminationEvent | Self::EarliestRecordedActivityProxy
        )
    }

    pub const fn is_sourced_determination(self) -> bool {
        matches!(self, Self::SourcedDeterminationEvent)
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SourcedDeterminationEvent => "SOURCED_DETERMINATION_EVENT",
            Self::EarliestRecordedActivityProxy => "EARLIEST_RECORDED_ACTIVITY_PROXY",
            Self::ObservationWithoutRecordedActivity => "OBSERVATION_WITHOUT_RECORDED_ACTIVITY",
        }
    }
}

/// Every extension reason the statute recognizes, in the order the SQL fragment renders them.
///
/// [`StatutoryExtensionReason`] is a closed enum, so the Rust evaluator never has to test
/// membership; the SQL path reads free text out of the database and does. Keeping the list here
/// means adding a variant updates both paths at once.
pub const RECOGNIZED_EXTENSION_REASONS: [StatutoryExtensionReason; 6] = [
    StatutoryExtensionReason::SeparateFieldFacilities,
    StatutoryExtensionReason::VoluminousRecords,
    StatutoryExtensionReason::InteragencyConsultation,
    StatutoryExtensionReason::DataCompilation,
    StatutoryExtensionReason::CyberattackSystemsUnavailable,
    StatutoryExtensionReason::DeclaredEmergencyOperationalImpact,
];

/// SQL renderings of the rules above, for the vectorized aggregate path.
///
/// Every fragment is deliberately expressed over *calendar dates* rather than instants. The
/// statute counts days, and a day is a day in the offset the agency recorded, so the fragments
/// derive dates from the RFC 3339 date prefix rather than converting to a session time zone
/// first. That matches [`chrono::DateTime::date_naive`] on the Rust side and, unlike a
/// `TIMESTAMPTZ` conversion, produces the same answer on every machine.
pub mod sql {
    use super::{
        CPRA_INITIAL_DETERMINATION_DAYS, CPRA_MAXIMUM_EXTENSION_DAYS, RECOGNIZED_EXTENSION_REASONS,
        SUSPICIOUS_CLOSURE_THRESHOLD_SECONDS,
    };
    use crate::analysis::court_calendar::{
        modeled_weekday_holidays, FIRST_MODELED_YEAR, LAST_MODELED_YEAR,
        MAXIMUM_CONSECUTIVE_CLOSED_DAYS,
    };

    /// Observation date used when nothing was recorded after receipt.
    ///
    /// `CURRENT_TIMESTAMP` is a `TIMESTAMPTZ`; the inner cast pins it to the session's civil time
    /// the same way every other timestamp in the schema is written, so the aggregate and the Rust
    /// evaluator agree whenever the session runs in UTC, which is DuckDB's default here.
    const OBSERVATION_CALENDAR_DATE: &str = "CAST(CAST(CURRENT_TIMESTAMP AS TIMESTAMP) AS DATE)";

    const CALENDAR_DATE_OPEN: &str = "{calendar_date(";
    const CALENDAR_DATE_CLOSE: &str = ")}";

    /// The calendar date of an RFC 3339 timestamp string, in the offset it was recorded with.
    ///
    /// Reading the date prefix is what makes this offset-preserving. `CAST(TRY_CAST(x AS
    /// TIMESTAMPTZ) AS TIMESTAMP)` would first shift the instant into the session time zone, so
    /// `2026-01-01T23:00:00-08:00` would land on 2026-01-02 under a UTC session and on 2026-01-01
    /// under a Pacific one. The statute's clock does not move with the reader's machine.
    ///
    /// `TRY_CAST` keeps malformed input NULL, which every consuming predicate already excludes.
    pub fn calendar_date(timestamp_expression: &str) -> String {
        format!("TRY_CAST(substr({timestamp_expression}, 1, 10) AS DATE)")
    }

    /// Calendar date of a sourced determination, or otherwise the earliest recorded activity at
    /// or after receipt, mirroring `timeline`.
    ///
    /// The *instants* pick the winner and the winner's own date is returned, so the choice is made
    /// on real ordering while the arithmetic stays on calendar dates.
    pub fn determination_calendar_date() -> String {
        "CASE
            WHEN determination_timestamp >= created_timestamp
             AND TRIM(COALESCE(determination_source, '')) <> ''
                THEN explicit_determination_date
            WHEN updated_timestamp >= created_timestamp AND closed_timestamp >= created_timestamp
                THEN CASE
                    WHEN updated_timestamp <= closed_timestamp THEN updated_date
                    ELSE closed_date
                END
            WHEN updated_timestamp >= created_timestamp THEN updated_date
            WHEN closed_timestamp >= created_timestamp THEN closed_date
            ELSE NULL
        END"
        .to_string()
    }

    /// The `court_holidays(day)` CTE, rendered from `court_calendar` so the holiday rule has one
    /// definition. Weekends are tested by day of week in [`court_day_on_or_after`].
    pub fn court_holidays_cte() -> String {
        let rows = modeled_weekday_holidays()
            .iter()
            .map(|day| format!("(DATE '{day}')"))
            .collect::<Vec<_>>()
            .join(", ");
        format!("court_holidays(day) AS (VALUES {rows})")
    }

    /// The first court day on or after `date_expression`, mirroring
    /// `court_calendar::court_day_on_or_after`, and NULL outside the modeled years as the Rust
    /// side returns an error there.
    pub fn court_day_on_or_after(date_expression: &str) -> String {
        format!(
            "(CASE WHEN year({date_expression}) BETWEEN {FIRST_MODELED_YEAR} AND {LAST_MODELED_YEAR} THEN (
                SELECT MIN(candidate)
                FROM (
                    SELECT CAST({date_expression} AS DATE) + CAST(offset_days AS INTEGER) AS candidate
                    FROM range(0, {}) AS offsets(offset_days)
                ) AS candidates
                WHERE dayofweek(candidate) NOT IN (0, 6)
                  AND candidate NOT IN (SELECT day FROM court_holidays)
            ) END)",
            MAXIMUM_CONSECUTIVE_CLOSED_DAYS + 1
        )
    }

    /// Last day of the initial period, mirroring `timeline`.
    pub fn initial_deadline_date() -> String {
        court_day_on_or_after(&format!("created_date + {CPRA_INITIAL_DETERMINATION_DAYS}"))
    }

    /// Whether a recorded extension actually moves the deadline, mirroring
    /// `timeline::evaluate_extension`.
    ///
    /// The Rust evaluator distinguishes *why* an extension is defective because a single request's
    /// report names the defect. The aggregate only needs the deadline, and every defective status
    /// leaves the initial deadline in place, so this renders as one predicate.
    pub fn valid_extension_predicate() -> String {
        format!(
            "CASE
            WHEN extension_notice_timestamp >= created_timestamp
             AND extension_notice_date <= initial_deadline_date
             AND extension_expected_date > initial_deadline_date
             AND extension_expected_date <= {}
             AND TRIM(COALESCE(extension_notice_source, '')) <> ''
             AND extension_reason IN ({})
            THEN TRUE ELSE FALSE
        END",
            court_day_on_or_after(&format!(
                "initial_deadline_date + {CPRA_MAXIMUM_EXTENSION_DAYS}"
            )),
            recognized_extension_reason_list()
        )
    }

    /// Days the agency was entitled to, given the extension verdict.
    pub fn allowed_calendar_days() -> String {
        format!(
            "CASE
            WHEN has_valid_extension
            THEN date_diff('day', created_date, {})
            ELSE date_diff('day', created_date, initial_deadline_date)
        END",
            court_day_on_or_after("extension_expected_date")
        )
    }

    /// Whether the elapsed period exceeded the entitlement, mirroring `timeline`'s `is_violation`.
    ///
    /// A row with no parseable receipt date is excluded rather than counted either way: the model
    /// has nothing to measure from, and asserting compliance would be as wrong as asserting a
    /// violation.
    pub fn statutory_violation_predicate() -> String {
        format!(
            "created_timestamp IS NOT NULL
          AND created_date IS NOT NULL
          AND date_diff(
                'day',
                created_date,
                COALESCE(determination_date, {OBSERVATION_CALENDAR_DATE})
              ) > allowed_calendar_days"
        )
    }

    /// Lifecycle anomaly, not a statutory finding: a closure too fast to represent real review.
    pub fn suspicious_fast_closure_predicate() -> String {
        format!(
            "normalized_status = 'closed'
          AND latency_seconds BETWEEN 0.0 AND {SUSPICIOUS_CLOSURE_THRESHOLD_SECONDS}.0"
        )
    }

    /// The `IN` list of statutorily recognized extension reasons.
    pub fn recognized_extension_reason_list() -> String {
        RECOGNIZED_EXTENSION_REASONS
            .iter()
            .map(|reason| format!("'{}'", reason.as_str()))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Substitutes every statutory placeholder in a query template.
    ///
    /// Templates keep DuckDB's own `{db}` table qualifier untouched; the caller applies that
    /// afterwards.
    pub fn render(template: &str) -> String {
        let rendered = substitute_calendar_dates(template);
        rendered
            .replace("{court_holidays_cte}", &court_holidays_cte())
            .replace("{initial_deadline_date}", &initial_deadline_date())
            .replace(
                "{determination_calendar_date}",
                &determination_calendar_date(),
            )
            .replace("{valid_extension_predicate}", &valid_extension_predicate())
            .replace("{allowed_calendar_days}", &allowed_calendar_days())
            .replace(
                "{statutory_violation_predicate}",
                &statutory_violation_predicate(),
            )
            .replace(
                "{suspicious_fast_closure_predicate}",
                &suspicious_fast_closure_predicate(),
            )
    }

    /// Expands every `{calendar_date(<expression>)}` placeholder.
    fn substitute_calendar_dates(template: &str) -> String {
        let mut rendered = String::with_capacity(template.len());
        let mut remainder = template;
        while let Some(start) = remainder.find(CALENDAR_DATE_OPEN) {
            let (before, rest) = remainder.split_at(start);
            rendered.push_str(before);
            let rest = &rest[CALENDAR_DATE_OPEN.len()..];
            let Some(end) = rest.find(CALENDAR_DATE_CLOSE) else {
                // An unterminated placeholder is a template bug, not user input. Emit it verbatim
                // so DuckDB reports the offending text instead of silently dropping a column.
                rendered.push_str(CALENDAR_DATE_OPEN);
                remainder = rest;
                continue;
            };
            rendered.push_str(&calendar_date(rest[..end].trim()));
            remainder = &rest[end + CALENDAR_DATE_CLOSE.len()..];
        }
        rendered.push_str(remainder);
        rendered
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sql_extension_reason_list_covers_every_modeled_reason() {
        // The SQL path tests free text for membership while the Rust path relies on the enum being
        // closed. A variant added without updating the list would make the aggregate silently
        // treat a lawful extension as defective.
        let rendered = sql::recognized_extension_reason_list();
        for reason in [
            StatutoryExtensionReason::SeparateFieldFacilities,
            StatutoryExtensionReason::VoluminousRecords,
            StatutoryExtensionReason::InteragencyConsultation,
            StatutoryExtensionReason::DataCompilation,
            StatutoryExtensionReason::CyberattackSystemsUnavailable,
            StatutoryExtensionReason::DeclaredEmergencyOperationalImpact,
        ] {
            assert!(
                rendered.contains(reason.as_str()),
                "reason {} is missing from the SQL membership list",
                reason.as_str()
            );
        }
        assert_eq!(RECOGNIZED_EXTENSION_REASONS.len(), 6);
    }

    #[test]
    fn calendar_date_placeholders_expand_and_survive_malformed_templates() {
        assert_eq!(
            sql::render("SELECT {calendar_date(requests.created_at)} AS created_date"),
            "SELECT TRY_CAST(substr(requests.created_at, 1, 10) AS DATE) AS created_date"
        );
        // An unterminated placeholder must reach DuckDB intact rather than vanish.
        assert!(sql::render("SELECT {calendar_date(oops").contains("{calendar_date("));
    }

    #[test]
    fn only_the_recorded_activity_basis_measures_a_completed_period() {
        assert!(DeterminationBasis::EarliestRecordedActivityProxy.is_recorded_activity());
        assert!(!DeterminationBasis::ObservationWithoutRecordedActivity.is_recorded_activity());
    }
}
