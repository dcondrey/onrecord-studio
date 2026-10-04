use crate::analysis::statute::{DeterminationBasis, StatutoryClockRule, StatutoryJurisdiction};
use crate::error::AppError;
use crate::types::{StateTransitionLog, StatutoryExtensionRecord, TimelineAnomalyMarker};
use chrono::{DateTime, FixedOffset, NaiveDate, TimeDelta, Utc};
use serde::{Deserialize, Serialize};

// Thresholds and the citation live in `analysis::statute` because the vectorized aggregate in
// `analysis::anomaly` renders its SQL from the same values. They are re-exported here so every
// existing `analysis::timeline` import path keeps working.
pub use crate::analysis::statute::{
    CPRA_INITIAL_DETERMINATION_DAYS, CPRA_MAXIMUM_EXTENSION_DAYS, CPRA_STATUTE_CITATION,
    SUSPICIOUS_CLOSURE_THRESHOLD_SECONDS,
};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum StatutoryExtensionStatus {
    NotInvoked,
    Valid,
    NoticeBeforeReceipt,
    NoticeAfterInitialDeadline,
    /// The stated ground is not one the governing rule recognizes.
    GroundNotRecognized,
    ExpectedDateNotExtended,
    /// The expected date lies past the longest extension the governing rule allows.
    ExtensionExceedsMaximum,
    MissingNoticeSource,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StatutoryComplianceAnalysis {
    pub request_id: u64,
    /// The [`StatutoryClockRule::id`] the request was measured under.
    pub rule_id: String,
    pub jurisdiction: StatutoryJurisdiction,
    pub statute_citation: String,
    pub created_at: String,
    /// The recorded-offset date the clock started from: receipt, or the latest start the rule
    /// allows after it.
    pub clock_started_on: String,
    pub determination_at: Option<String>,
    pub determination_source: Option<String>,
    /// What `determination_at` actually is. A consumer that reports a violation without reading
    /// this is presenting a proxy, or an unfinished period, as a measured determination.
    pub determination_basis: DeterminationBasis,
    pub observed_at: String,
    pub initial_deadline_at: String,
    pub effective_deadline_at: String,
    pub calendar_days_elapsed: i64,
    pub allowed_calendar_days: i64,
    pub extension_status: StatutoryExtensionStatus,
    pub extension_reason: Option<String>,
    pub is_violation: bool,
    pub anomaly_markers: Vec<TimelineAnomalyMarker>,
    pub evidence_summary: String,
}

/// Everything recorded after receipt that may be, or bound, the agency's determination.
#[derive(Debug, Clone, Copy, Default)]
pub struct DeterminationEvidence<'a> {
    pub updated_at: Option<&'a str>,
    pub closed_at: Option<&'a str>,
    /// The earliest release event the source timeline records.
    pub first_release_at: Option<&'a str>,
    /// A determination the source records as such, with the notice that attests it.
    pub determination_at: Option<&'a str>,
    pub determination_source: Option<&'a str>,
}

pub fn calculate_operational_latency_for_status(
    request_id: u64,
    status: &str,
    created_at: &str,
    closed_at: Option<&str>,
) -> Result<StateTransitionLog, AppError> {
    calculate_operational_latency_internal(
        request_id,
        status,
        created_at,
        closed_at,
        Utc::now().fixed_offset(),
    )
}

pub fn detect_fast_drop_for_status(
    status: &str,
    created_at: &str,
    closed_at: &str,
) -> Result<Option<TimelineAnomalyMarker>, AppError> {
    if !status.trim().eq_ignore_ascii_case("closed") {
        return Ok(None);
    }
    let created = parse_timestamp(created_at, "created")?;
    let closed = parse_timestamp(closed_at, "closed")?;
    let lifetime = closed.signed_duration_since(created);
    Ok(is_suspiciously_fast_lifetime(lifetime)
        .then_some(TimelineAnomalyMarker::SuspiciouslyFastClosure))
}

/// Measures one request against `rule`. There is no default rule: a caller without one has no
/// statutory evaluation to make.
pub fn evaluate_statutory_clock(
    rule: &StatutoryClockRule,
    request_id: u64,
    created_at: &str,
    evidence: DeterminationEvidence<'_>,
    extension: Option<&StatutoryExtensionRecord>,
    observation_at: &str,
) -> Result<StatutoryComplianceAnalysis, AppError> {
    let created = parse_timestamp(created_at, "created")?;
    let observed = parse_timestamp(observation_at, "observation")?;
    if observed < created {
        return Err(AppError::Compliance(format!(
            "Request {request_id} was observed before its creation timestamp"
        )));
    }

    let sourced_determination = match evidence.determination_at {
        Some(value) => {
            let source = evidence
                .determination_source
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    AppError::Compliance(format!(
                        "Request {request_id} has a determination timestamp without a source"
                    ))
                })?;
            let timestamp = parse_timestamp(value, "determination")?;
            if timestamp < created {
                return Err(AppError::Compliance(format!(
                    "Request {request_id} determination predates receipt"
                )));
            }
            Some((timestamp, source.to_string()))
        }
        None => {
            if evidence
                .determination_source
                .is_some_and(|value| !value.trim().is_empty())
            {
                return Err(AppError::Compliance(format!(
                    "Request {request_id} has a determination source without a timestamp"
                )));
            }
            None
        }
    };
    let (determination, determination_basis, recorded_determination_source) =
        if let Some((timestamp, source)) = sourced_determination {
            (
                Some(timestamp),
                DeterminationBasis::SourcedDeterminationEvent,
                Some(source),
            )
        } else {
            let proxy = earliest_activity_after_receipt(created, &evidence)?;
            let basis = if proxy.is_some() {
                DeterminationBasis::EarliestRecordedActivityProxy
            } else {
                DeterminationBasis::ObservationWithoutRecordedActivity
            };
            (proxy, basis, None)
        };
    let calendar = rule.calendar;
    let created_date = created.date_naive();
    let clock_start_date = match rule.start.latest_after_receipt {
        Some(allowance) => calendar.period_end(created_date, allowance)?,
        None => created_date,
    };
    let initial_deadline_date = calendar.period_end(clock_start_date, rule.initial)?;
    let extension_evaluation = evaluate_extension(rule, created, initial_deadline_date, extension)?;
    let initial_deadline = on_date(created, initial_deadline_date)?;
    let effective_deadline = extension_evaluation
        .valid_deadline
        .unwrap_or(initial_deadline);
    // A recorded milestone is dated in its own recorded offset. The observation instant is the
    // evaluator's clock and carries no agency offset, so it is read in the receipt's: otherwise
    // an evening observation on the last legal day is already the next day in UTC.
    let comparison_date = match determination {
        Some(determination) => determination.date_naive(),
        None => observed.with_timezone(created.offset()).date_naive(),
    };
    let calendar_days_elapsed = comparison_date
        .signed_duration_since(created_date)
        .num_days();
    let allowed_calendar_days = effective_deadline
        .date_naive()
        .signed_duration_since(created_date)
        .num_days();
    let is_violation = calendar_days_elapsed > allowed_calendar_days;
    let mut anomaly_markers = Vec::with_capacity(2);
    if extension_evaluation.status != StatutoryExtensionStatus::NotInvoked
        && extension_evaluation.status != StatutoryExtensionStatus::Valid
    {
        anomaly_markers.push(TimelineAnomalyMarker::StatutoryExtensionDefective);
    }
    if is_violation {
        anomaly_markers.push(TimelineAnomalyMarker::StatutoryLatencyViolation);
    }

    let milestone_label = match determination_basis {
        DeterminationBasis::SourcedDeterminationEvent => "a recorded determination",
        DeterminationBasis::EarliestRecordedActivityProxy => {
            "the earliest recorded activity, a proxy for the determination that was not recorded"
        }
        DeterminationBasis::ObservationWithoutRecordedActivity => {
            "observation, with no determination or activity recorded"
        }
    };
    let determination_source_clause = recorded_determination_source
        .as_deref()
        .map(|source| format!(" ({source})"))
        .unwrap_or_default();
    let unit = rule.initial.kind.unit();
    let start_clause = if rule.start.latest_after_receipt.is_some() {
        format!(
            "; the clock is taken to start {clock_start_date}, the latest start {} allows",
            rule.start.citation.section
        )
    } else {
        String::new()
    };
    let evidence_summary = format!(
        "Request {request_id}: {calendar_days_elapsed} calendar days from receipt to {milestone_label}{determination_source_clause}; effective deadline was {} under {} ({} {unit}){start_clause}, where {}; extension status {:?}.",
        effective_deadline.to_rfc3339(),
        rule.initial_citation.section,
        rule.initial.days,
        calendar.basis(),
        extension_evaluation.status
    );

    Ok(StatutoryComplianceAnalysis {
        request_id,
        rule_id: rule.id.to_string(),
        jurisdiction: rule.jurisdiction,
        statute_citation: rule.initial_citation.section.to_string(),
        created_at: created.to_rfc3339(),
        clock_started_on: clock_start_date.to_string(),
        determination_at: determination.map(|value| value.to_rfc3339()),
        determination_source: recorded_determination_source,
        determination_basis,
        observed_at: observed.to_rfc3339(),
        initial_deadline_at: initial_deadline.to_rfc3339(),
        effective_deadline_at: effective_deadline.to_rfc3339(),
        calendar_days_elapsed,
        allowed_calendar_days,
        extension_status: extension_evaluation.status,
        extension_reason: extension.map(|value| value.reason.as_str().to_string()),
        is_violation,
        anomaly_markers,
        evidence_summary,
    })
}

struct ExtensionEvaluation {
    status: StatutoryExtensionStatus,
    valid_deadline: Option<DateTime<FixedOffset>>,
}

fn evaluate_extension(
    rule: &StatutoryClockRule,
    created: DateTime<FixedOffset>,
    initial_deadline_date: NaiveDate,
    extension: Option<&StatutoryExtensionRecord>,
) -> Result<ExtensionEvaluation, AppError> {
    let Some(extension) = extension else {
        return Ok(ExtensionEvaluation {
            status: StatutoryExtensionStatus::NotInvoked,
            valid_deadline: None,
        });
    };
    let notice = parse_timestamp(&extension.notice_at, "extension notice")?;
    let expected = parse_timestamp(
        &extension.expected_determination_at,
        "extension expected determination",
    )?;
    // The extension runs from the initial deadline as carried past any closed days, the reading
    // that grants the agency the most time.
    let latest_extended_date = rule
        .calendar
        .period_end(initial_deadline_date, rule.extension.maximum)?;
    let status = if extension.notice_source.trim().is_empty() {
        StatutoryExtensionStatus::MissingNoticeSource
    } else if notice < created {
        StatutoryExtensionStatus::NoticeBeforeReceipt
    } else if notice.date_naive() > initial_deadline_date {
        StatutoryExtensionStatus::NoticeAfterInitialDeadline
    } else if !rule.extension.grounds.contains(&extension.reason) {
        StatutoryExtensionStatus::GroundNotRecognized
    } else if expected.date_naive() <= initial_deadline_date {
        StatutoryExtensionStatus::ExpectedDateNotExtended
    } else if expected.date_naive() > latest_extended_date {
        StatutoryExtensionStatus::ExtensionExceedsMaximum
    } else {
        StatutoryExtensionStatus::Valid
    };
    let valid_deadline = if status == StatutoryExtensionStatus::Valid {
        Some(on_date(
            expected,
            rule.calendar.open_day_on_or_after(expected.date_naive())?,
        )?)
    } else {
        None
    };
    Ok(ExtensionEvaluation {
        status,
        valid_deadline,
    })
}

/// `anchor`'s time of day and offset, moved to `date`.
fn on_date(
    anchor: DateTime<FixedOffset>,
    date: NaiveDate,
) -> Result<DateTime<FixedOffset>, AppError> {
    let days = date.signed_duration_since(anchor.date_naive());
    anchor
        .checked_add_signed(days)
        .ok_or_else(|| AppError::Compliance("Statutory deadline overflowed".to_string()))
}

/// The earliest recorded activity strictly after receipt. A timestamp equal to receipt is the
/// receipt itself (a portal that reports no update repeats the creation instant), not activity.
fn earliest_activity_after_receipt(
    created: DateTime<FixedOffset>,
    evidence: &DeterminationEvidence<'_>,
) -> Result<Option<DateTime<FixedOffset>>, AppError> {
    let mut milestones = Vec::with_capacity(3);
    for (value, role) in [
        (evidence.updated_at, "updated"),
        (evidence.closed_at, "closed"),
        (evidence.first_release_at, "release"),
    ] {
        if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
            let parsed = parse_timestamp(value, role)?;
            if parsed > created {
                milestones.push(parsed);
            }
        }
    }
    Ok(milestones.into_iter().min())
}

fn calculate_operational_latency_internal(
    request_id: u64,
    status: &str,
    created_at: &str,
    closed_at: Option<&str>,
    observed: DateTime<FixedOffset>,
) -> Result<StateTransitionLog, AppError> {
    let created = parse_timestamp(created_at, "created")?;
    let normalized_status = normalize_status(status, closed_at.is_some());
    match closed_at {
        Some(value) => {
            let closed = parse_timestamp(value, "closed")?;
            let lifetime = closed.signed_duration_since(created);
            let anomaly_markers = if lifetime < TimeDelta::zero() {
                vec![TimelineAnomalyMarker::CloseBeforeCreation]
            } else if normalized_status == "CLOSED" && is_suspiciously_fast_lifetime(lifetime) {
                vec![TimelineAnomalyMarker::SuspiciouslyFastClosure]
            } else {
                Vec::new()
            };
            Ok(StateTransitionLog {
                request_id,
                transition_timestamp: closed.to_rfc3339(),
                previous_state: "OPEN".to_string(),
                next_state: normalized_status,
                time_spent_in_previous_state_seconds: lifetime.num_seconds(),
                anomaly_markers,
            })
        }
        None => {
            let lifetime = observed.signed_duration_since(created);
            let anomaly_markers = (lifetime < TimeDelta::zero())
                .then_some(TimelineAnomalyMarker::ObservationBeforeCreation)
                .into_iter()
                .collect();
            Ok(StateTransitionLog {
                request_id,
                transition_timestamp: observed.to_rfc3339(),
                previous_state: "OPEN".to_string(),
                next_state: "ACTIVE_STALL".to_string(),
                time_spent_in_previous_state_seconds: lifetime.num_seconds(),
                anomaly_markers,
            })
        }
    }
}

fn is_suspiciously_fast_lifetime(lifetime: TimeDelta) -> bool {
    lifetime >= TimeDelta::zero()
        && lifetime <= TimeDelta::seconds(SUSPICIOUS_CLOSURE_THRESHOLD_SECONDS)
}

fn normalize_status(status: &str, has_closed_timestamp: bool) -> String {
    let status = status.trim();
    if status.is_empty() {
        if has_closed_timestamp {
            "CLOSED".to_string()
        } else {
            "ACTIVE_STALL".to_string()
        }
    } else {
        status.to_uppercase()
    }
}

fn parse_timestamp(
    timestamp: &str,
    timestamp_role: &str,
) -> Result<DateTime<FixedOffset>, AppError> {
    DateTime::parse_from_rfc3339(timestamp).map_err(|error| {
        AppError::DataParsing(format!(
            "Invalid request {timestamp_role} timestamp '{timestamp}': {error}"
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::statute::{CPRA_DETERMINATION_RULE, FOIA_DETERMINATION_RULE};
    use crate::types::StatutoryExtensionReason;

    fn activity(updated_at: &str) -> DeterminationEvidence<'_> {
        DeterminationEvidence {
            updated_at: Some(updated_at),
            ..DeterminationEvidence::default()
        }
    }

    fn cpra(
        created_at: &str,
        evidence: DeterminationEvidence<'_>,
        extension: Option<&StatutoryExtensionRecord>,
        observation_at: &str,
    ) -> StatutoryComplianceAnalysis {
        evaluate_statutory_clock(
            &CPRA_DETERMINATION_RULE,
            1,
            created_at,
            evidence,
            extension,
            observation_at,
        )
        .unwrap()
    }

    fn foia(
        determined_at: &str,
        extension: Option<&StatutoryExtensionRecord>,
    ) -> StatutoryComplianceAnalysis {
        evaluate_statutory_clock(
            &FOIA_DETERMINATION_RULE,
            1,
            "2026-03-02T09:00:00-05:00",
            DeterminationEvidence {
                determination_at: Some(determined_at),
                determination_source: Some("Agency determination letter"),
                ..DeterminationEvidence::default()
            },
            extension,
            "2026-06-01T09:00:00-04:00",
        )
        .unwrap()
    }

    fn extension(
        notice_at: &str,
        expected: &str,
        reason: StatutoryExtensionReason,
    ) -> StatutoryExtensionRecord {
        StatutoryExtensionRecord {
            notice_at: notice_at.to_string(),
            expected_determination_at: expected.to_string(),
            reason,
            notice_source: "Agency extension letter".to_string(),
        }
    }

    #[test]
    fn fast_drop_boundary_preserves_subsecond_precision() {
        assert_eq!(
            detect_fast_drop_for_status("closed", "2024-05-01T12:00:00Z", "2024-05-01T12:05:00Z")
                .unwrap(),
            Some(TimelineAnomalyMarker::SuspiciouslyFastClosure)
        );
        assert_eq!(
            detect_fast_drop_for_status(
                "closed",
                "2024-05-01T12:00:00.000Z",
                "2024-05-01T12:05:00.001Z"
            )
            .unwrap(),
            None
        );
    }

    #[test]
    fn cpra_tenth_calendar_day_is_the_last_timely_day() {
        // Receipt Monday 2 March 2026; the tenth day, Thursday the 12th, is open.
        for (determined, violation) in [
            ("2026-03-11T16:00:00-07:00", false),
            ("2026-03-12T23:59:00-07:00", false),
            ("2026-03-13T09:00:00-07:00", true),
        ] {
            let result = cpra(
                "2026-03-02T00:01:00-08:00",
                activity(determined),
                None,
                determined,
            );
            assert_eq!(result.is_violation, violation, "{determined}");
        }
    }

    #[test]
    fn foia_twentieth_working_day_after_the_latest_start_is_the_last_timely_day() {
        // Receipt Monday 2 March 2026. The clock starts ten working days later (16 March), and
        // twenty working days after that is Monday 13 April.
        for (determined, violation) in [
            ("2026-04-10T16:00:00-04:00", false),
            ("2026-04-12T16:00:00-04:00", false),
            ("2026-04-13T16:00:00-04:00", false),
            ("2026-04-14T09:00:00-04:00", true),
        ] {
            let result = foia(determined, None);
            assert_eq!(result.is_violation, violation, "{determined}");
            assert_eq!(result.clock_started_on, "2026-03-16");
            assert_eq!(&result.initial_deadline_at[..10], "2026-04-13");
        }
    }

    #[test]
    fn foia_working_days_skip_federal_holidays_that_cpra_counts() {
        // Receipt Monday 11 May 2026: Memorial Day (25 May) delays the start to the 26th, and
        // Juneteenth (Friday 19 June) moves the twentieth working day to Wednesday 24 June.
        let result = evaluate_statutory_clock(
            &FOIA_DETERMINATION_RULE,
            1,
            "2026-05-11T09:00:00-04:00",
            DeterminationEvidence::default(),
            None,
            "2026-05-12T09:00:00-04:00",
        )
        .unwrap();
        assert_eq!(result.clock_started_on, "2026-05-26");
        assert_eq!(&result.initial_deadline_at[..10], "2026-06-24");
    }

    #[test]
    fn foia_extension_allows_ten_working_days_on_a_recognized_ground_only() {
        let notice = "2026-04-08T12:00:00-04:00";
        // Ten working days after Monday 13 April is Monday 27 April.
        let within = extension(
            notice,
            "2026-04-27T12:00:00-04:00",
            StatutoryExtensionReason::VoluminousRecords,
        );
        let result = foia("2026-04-27T15:00:00-04:00", Some(&within));
        assert_eq!(result.extension_status, StatutoryExtensionStatus::Valid);
        assert!(!result.is_violation);

        let beyond = extension(
            notice,
            "2026-04-28T12:00:00-04:00",
            StatutoryExtensionReason::VoluminousRecords,
        );
        let result = foia("2026-04-27T15:00:00-04:00", Some(&beyond));
        assert_eq!(
            result.extension_status,
            StatutoryExtensionStatus::ExtensionExceedsMaximum
        );
        assert!(result.is_violation);

        // Data compilation is a CPRA ground, not one of § 552(a)(6)(B)(iii)'s three.
        let cpra_only = extension(
            notice,
            "2026-04-27T12:00:00-04:00",
            StatutoryExtensionReason::DataCompilation,
        );
        let result = foia("2026-04-27T15:00:00-04:00", Some(&cpra_only));
        assert_eq!(
            result.extension_status,
            StatutoryExtensionStatus::GroundNotRecognized
        );
        assert!(result.is_violation);
    }

    #[test]
    fn cpra_extension_boundary_is_fourteen_days_past_the_carried_deadline() {
        // Receipt 1 January 2026: day ten is Sunday the 11th, carried to Monday the 12th; fourteen
        // days later is Monday the 26th.
        for (expected, status) in [
            ("2026-01-25T12:00:00-08:00", StatutoryExtensionStatus::Valid),
            ("2026-01-26T12:00:00-08:00", StatutoryExtensionStatus::Valid),
            (
                "2026-01-27T12:00:00-08:00",
                StatutoryExtensionStatus::ExtensionExceedsMaximum,
            ),
        ] {
            let record = extension(
                "2026-01-08T12:00:00-08:00",
                expected,
                StatutoryExtensionReason::DataCompilation,
            );
            let result = cpra(
                "2026-01-01T09:00:00-08:00",
                activity("2026-01-20T12:00:00-08:00"),
                Some(&record),
                "2026-01-20T12:00:00-08:00",
            );
            assert_eq!(result.extension_status, status, "{expected}");
            assert_eq!(
                result.is_violation,
                status != StatutoryExtensionStatus::Valid,
                "{expected}"
            );
        }
    }

    #[test]
    fn determination_basis_distinguishes_a_proxy_from_an_unfinished_period() {
        let with_activity = cpra(
            "2026-01-01T09:00:00-08:00",
            activity("2026-01-05T09:00:00-08:00"),
            None,
            "2026-01-20T09:00:00-08:00",
        );
        assert_eq!(
            with_activity.determination_basis,
            DeterminationBasis::EarliestRecordedActivityProxy
        );
        assert!(with_activity.determination_at.is_some());

        // Nothing recorded after receipt: the period is still running, so the finding is
        // provisional even though the deadline has demonstrably passed.
        let still_running = cpra(
            "2026-01-01T09:00:00-08:00",
            DeterminationEvidence::default(),
            None,
            "2026-01-20T09:00:00-08:00",
        );
        assert_eq!(
            still_running.determination_basis,
            DeterminationBasis::ObservationWithoutRecordedActivity
        );
        assert_eq!(still_running.determination_at, None);
        assert!(still_running.is_violation);

        // Neither an update recorded before receipt nor one at the receipt instant itself (a
        // portal reporting no update repeats the creation time) is activity after receipt.
        for updated in ["2025-12-30T09:00:00-08:00", "2026-01-01T09:00:00-08:00"] {
            let result = cpra(
                "2026-01-01T09:00:00-08:00",
                activity(updated),
                None,
                "2026-01-20T09:00:00-08:00",
            );
            assert_eq!(
                result.determination_basis,
                DeterminationBasis::ObservationWithoutRecordedActivity,
                "{updated}"
            );
            assert!(result.is_violation, "{updated}");
        }
    }

    #[test]
    fn a_release_bounds_the_determination_as_a_proxy_not_a_recorded_event() {
        // The first release on day 5 proves a determination by then, though the request was
        // closed on day 40.
        let result = cpra(
            "2026-01-01T09:00:00-08:00",
            DeterminationEvidence {
                closed_at: Some("2026-02-10T09:00:00-08:00"),
                first_release_at: Some("2026-01-06T09:00:00-08:00"),
                ..DeterminationEvidence::default()
            },
            None,
            "2026-03-01T09:00:00-08:00",
        );
        assert_eq!(
            result.determination_basis,
            DeterminationBasis::EarliestRecordedActivityProxy
        );
        assert_eq!(result.calendar_days_elapsed, 5);
        assert!(!result.is_violation);
        assert_eq!(result.determination_source, None);
    }

    #[test]
    fn sourced_determination_event_overrides_generic_activity_and_retains_provenance() {
        let analysis = cpra(
            "2026-01-01T09:00:00-08:00",
            DeterminationEvidence {
                updated_at: Some("2026-01-02T09:00:00-08:00"),
                closed_at: Some("2026-01-20T09:00:00-08:00"),
                first_release_at: Some("2026-01-03T09:00:00-08:00"),
                determination_at: Some("2026-01-09T15:30:00-08:00"),
                determination_source: Some("Agency determination notice dated January 9"),
            },
            None,
            "2026-01-20T09:00:00-08:00",
        );
        assert_eq!(
            analysis.determination_basis,
            DeterminationBasis::SourcedDeterminationEvent
        );
        assert_eq!(
            analysis.determination_at.as_deref(),
            Some("2026-01-09T15:30:00-08:00")
        );
        assert_eq!(
            analysis.determination_source.as_deref(),
            Some("Agency determination notice dated January 9")
        );
        assert!(!analysis.is_violation);

        assert!(evaluate_statutory_clock(
            &CPRA_DETERMINATION_RULE,
            17,
            "2026-01-01T09:00:00-08:00",
            DeterminationEvidence {
                determination_at: Some("2026-01-09T15:30:00-08:00"),
                ..DeterminationEvidence::default()
            },
            None,
            "2026-01-20T09:00:00-08:00",
        )
        .is_err());
    }

    #[test]
    fn an_evening_observation_on_the_last_legal_day_is_not_a_violation() {
        // 17:30 Pacific on the tenth day is already the eleventh in UTC. The evaluator's clock
        // carries no agency offset, so it is read in the receipt's, and the agency is still timely.
        let result = cpra(
            "2026-03-02T09:00:00-08:00",
            DeterminationEvidence::default(),
            None,
            "2026-03-13T00:30:00Z",
        );
        assert_eq!(result.calendar_days_elapsed, 10);
        assert!(!result.is_violation);
    }

    #[test]
    fn calendar_days_are_counted_in_the_recorded_offset() {
        // Receipt late on 2 March Pacific is 3 March in UTC. The statutory clock follows the
        // offset the agency recorded, so this is an eleventh-day determination and a violation.
        // Counting from a UTC-shifted date would report ten days and clear the agency.
        let result = cpra(
            "2026-03-02T23:00:00-08:00",
            activity("2026-03-13T10:00:00-07:00"),
            None,
            "2026-03-13T10:00:00-07:00",
        );
        assert_eq!(result.calendar_days_elapsed, 11);
        assert!(result.is_violation);
    }

    #[test]
    fn the_summary_names_the_rule_and_the_basis() {
        let result = foia("2026-04-14T09:00:00-04:00", None);
        assert_eq!(result.rule_id, "US_FOIA_DETERMINATION");
        assert!(result
            .evidence_summary
            .contains("5 U.S.C. § 552(a)(6)(A)(i) (20 working days)"));
        assert!(result.evidence_summary.contains("a recorded determination"));
        assert!(result
            .evidence_summary
            .contains("Agency determination letter"));
    }
}
