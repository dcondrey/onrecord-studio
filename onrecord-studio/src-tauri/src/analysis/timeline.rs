use crate::analysis::court_calendar::{self, COURT_CALENDAR_BASIS};
use crate::analysis::statute::DeterminationBasis;
use crate::error::AppError;
use crate::types::{StateTransitionLog, StatutoryExtensionRecord, TimelineAnomalyMarker};
use chrono::{DateTime, Days, FixedOffset, NaiveDate, TimeDelta, Utc};
use serde::{Deserialize, Serialize};

// Thresholds and the citation live in `analysis::statute` because the vectorized aggregate in
// `analysis::anomaly` renders its SQL from the same values. They are re-exported here so every
// existing `analysis::timeline` import path keeps working.
pub use crate::analysis::statute::{
    CPRA_INITIAL_DETERMINATION_DAYS, CPRA_MAXIMUM_EXTENSION_DAYS, CPRA_STATUTE_CITATION, SUSPICIOUS_CLOSURE_THRESHOLD_SECONDS,
};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum StatutoryExtensionStatus {
    NotInvoked,
    Valid,
    NoticeBeforeReceipt,
    NoticeAfterInitialDeadline,
    ExpectedDateNotExtended,
    ExtensionExceedsFourteenDays,
    MissingNoticeSource,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StatutoryComplianceAnalysis {
    pub request_id: u64,
    pub statute_citation: String,
    pub created_at: String,
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

pub fn evaluate_statutory_compliance_at(
    request_id: u64,
    created_at: &str,
    updated_at: Option<&str>,
    closed_at: Option<&str>,
    extension: Option<&StatutoryExtensionRecord>,
    observation_at: &str,
) -> Result<StatutoryComplianceAnalysis, AppError> {
    evaluate_statutory_compliance_with_determination_at(
        request_id,
        created_at,
        updated_at,
        closed_at,
        None,
        None,
        extension,
        observation_at,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn evaluate_statutory_compliance_with_determination_at(
    request_id: u64,
    created_at: &str,
    updated_at: Option<&str>,
    closed_at: Option<&str>,
    determination_at: Option<&str>,
    determination_source: Option<&str>,
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

    let sourced_determination = match determination_at {
        Some(value) => {
            let source = determination_source
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
            if determination_source.is_some_and(|value| !value.trim().is_empty()) {
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
            let proxy = earliest_milestone(created, updated_at, closed_at)?;
            let basis = if proxy.is_some() {
                DeterminationBasis::EarliestRecordedActivityProxy
            } else {
                DeterminationBasis::ObservationWithoutRecordedActivity
            };
            (proxy, basis, None)
        };
    let created_date = created.date_naive();
    let initial_deadline_date = court_calendar::court_day_on_or_after(
        created_date
            .checked_add_days(Days::new(CPRA_INITIAL_DETERMINATION_DAYS))
            .ok_or_else(|| {
                AppError::Compliance(format!(
                    "Request {request_id} initial statutory deadline overflowed"
                ))
            })?,
    )?;
    let extension_evaluation = evaluate_extension(created, initial_deadline_date, extension)?;
    let initial_deadline = on_date(created, initial_deadline_date)?;
    let effective_deadline = extension_evaluation
        .valid_deadline
        .unwrap_or(initial_deadline);
    let comparison_timestamp = determination.unwrap_or(observed);
    let calendar_days_elapsed = comparison_timestamp
        .date_naive()
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

    let milestone_label = if determination_basis.is_sourced_determination() {
        "sourced determination event"
    } else if determination_basis.is_recorded_activity() {
        "recorded determination milestone"
    } else {
        "observation without a recorded determination milestone"
    };
    let determination_source_clause = recorded_determination_source
        .as_deref()
        .map(|source| format!("; determination source: {source}"))
        .unwrap_or_default();
    let evidence_summary = format!(
        "Request {request_id}: {calendar_days_elapsed} calendar days from receipt to {milestone_label}{determination_source_clause}; effective deadline was {} under {CPRA_STATUTE_CITATION}, where {COURT_CALENDAR_BASIS}; extension status {:?}.",
        effective_deadline.to_rfc3339(),
        extension_evaluation.status
    );

    Ok(StatutoryComplianceAnalysis {
        request_id,
        statute_citation: CPRA_STATUTE_CITATION.to_string(),
        created_at: created.to_rfc3339(),
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
    // The 14 days run from the initial deadline as carried past any closed days, the reading
    // that grants the agency the most time.
    let latest_extended_date = court_calendar::court_day_on_or_after(
        initial_deadline_date
            .checked_add_days(Days::new(CPRA_MAXIMUM_EXTENSION_DAYS))
            .ok_or_else(|| AppError::Compliance("Extended deadline overflowed".to_string()))?,
    )?;
    let status = if extension.notice_source.trim().is_empty() {
        StatutoryExtensionStatus::MissingNoticeSource
    } else if notice < created {
        StatutoryExtensionStatus::NoticeBeforeReceipt
    } else if notice.date_naive() > initial_deadline_date {
        StatutoryExtensionStatus::NoticeAfterInitialDeadline
    } else if expected.date_naive() <= initial_deadline_date {
        StatutoryExtensionStatus::ExpectedDateNotExtended
    } else if expected.date_naive() > latest_extended_date {
        StatutoryExtensionStatus::ExtensionExceedsFourteenDays
    } else {
        StatutoryExtensionStatus::Valid
    };
    let valid_deadline = if status == StatutoryExtensionStatus::Valid {
        Some(on_date(
            expected,
            court_calendar::court_day_on_or_after(expected.date_naive())?,
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

fn earliest_milestone(
    created: DateTime<FixedOffset>,
    updated_at: Option<&str>,
    closed_at: Option<&str>,
) -> Result<Option<DateTime<FixedOffset>>, AppError> {
    let mut milestones = Vec::with_capacity(2);
    if let Some(value) = updated_at.filter(|value| !value.trim().is_empty()) {
        let parsed = parse_timestamp(value, "updated")?;
        if parsed >= created {
            milestones.push(parsed);
        }
    }
    if let Some(value) = closed_at.filter(|value| !value.trim().is_empty()) {
        let parsed = parse_timestamp(value, "closed")?;
        if parsed >= created {
            milestones.push(parsed);
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
    use crate::types::StatutoryExtensionReason;

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
    fn eleven_calendar_days_without_extension_is_a_violation() {
        let boundary = evaluate_statutory_compliance_at(
            1,
            "2026-03-02T00:01:00-08:00",
            Some("2026-03-12T23:59:00-07:00"),
            None,
            None,
            "2026-03-12T23:59:00-07:00",
        )
        .unwrap();
        assert!(!boundary.is_violation);
        let result = evaluate_statutory_compliance_at(
            1,
            "2026-03-02T09:00:00-08:00",
            Some("2026-03-13T09:00:00-07:00"),
            None,
            None,
            "2026-03-13T09:00:00-07:00",
        )
        .unwrap();
        assert!(result.is_violation);
        assert_eq!(result.calendar_days_elapsed, 11);
    }

    #[test]
    fn determination_basis_distinguishes_a_proxy_from_an_unfinished_period() {
        let with_activity = evaluate_statutory_compliance_at(
            1,
            "2026-01-01T09:00:00-08:00",
            Some("2026-01-05T09:00:00-08:00"),
            None,
            None,
            "2026-01-20T09:00:00-08:00",
        )
        .unwrap();
        assert_eq!(
            with_activity.determination_basis,
            DeterminationBasis::EarliestRecordedActivityProxy
        );
        assert!(with_activity.determination_at.is_some());

        // Nothing recorded after receipt: the period is still running, so the finding is
        // provisional even though the deadline has demonstrably passed.
        let still_running = evaluate_statutory_compliance_at(
            2,
            "2026-01-01T09:00:00-08:00",
            None,
            None,
            None,
            "2026-01-20T09:00:00-08:00",
        )
        .unwrap();
        assert_eq!(
            still_running.determination_basis,
            DeterminationBasis::ObservationWithoutRecordedActivity
        );
        assert_eq!(still_running.determination_at, None);
        assert!(still_running.is_violation);

        // An update recorded before receipt is not activity after receipt.
        let backdated = evaluate_statutory_compliance_at(
            3,
            "2026-01-01T09:00:00-08:00",
            Some("2025-12-30T09:00:00-08:00"),
            None,
            None,
            "2026-01-03T09:00:00-08:00",
        )
        .unwrap();
        assert_eq!(
            backdated.determination_basis,
            DeterminationBasis::ObservationWithoutRecordedActivity
        );
    }

    #[test]
    fn sourced_determination_event_overrides_generic_activity_and_retains_provenance() {
        let analysis = evaluate_statutory_compliance_with_determination_at(
            17,
            "2026-01-01T09:00:00-08:00",
            Some("2026-01-02T09:00:00-08:00"),
            Some("2026-01-20T09:00:00-08:00"),
            Some("2026-01-09T15:30:00-08:00"),
            Some("Agency determination notice dated January 9"),
            None,
            "2026-01-20T09:00:00-08:00",
        )
        .expect("a sourced determination must evaluate");
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

        assert!(evaluate_statutory_compliance_with_determination_at(
            17,
            "2026-01-01T09:00:00-08:00",
            None,
            None,
            Some("2026-01-09T15:30:00-08:00"),
            None,
            None,
            "2026-01-20T09:00:00-08:00",
        )
        .is_err());
    }

    #[test]
    fn calendar_days_are_counted_in_the_recorded_offset() {
        // Receipt late on 2 March Pacific is 3 March in UTC. The statutory clock follows the
        // offset the agency recorded, so this is an eleventh-day determination and a violation.
        // Counting from a UTC-shifted date would report ten days and clear the agency.
        let result = evaluate_statutory_compliance_at(
            4,
            "2026-03-02T23:00:00-08:00",
            Some("2026-03-13T10:00:00-07:00"),
            None,
            None,
            "2026-03-13T10:00:00-07:00",
        )
        .unwrap();
        assert_eq!(result.calendar_days_elapsed, 11);
        assert!(result.is_violation);
    }

    #[test]
    fn timely_written_extension_moves_the_effective_deadline() {
        let extension = StatutoryExtensionRecord {
            notice_at: "2026-01-08T12:00:00-08:00".to_string(),
            expected_determination_at: "2026-01-20T12:00:00-08:00".to_string(),
            reason: StatutoryExtensionReason::VoluminousRecords,
            notice_source: "Portal message 17".to_string(),
        };
        let result = evaluate_statutory_compliance_at(
            2,
            "2026-01-01T09:00:00-08:00",
            Some("2026-01-20T12:00:00-08:00"),
            None,
            Some(&extension),
            "2026-01-20T12:00:00-08:00",
        )
        .unwrap();
        assert!(!result.is_violation);
        assert_eq!(result.extension_status, StatutoryExtensionStatus::Valid);
    }

    #[test]
    fn extension_beyond_fourteen_days_is_defective_and_does_not_extend_deadline() {
        let extension = StatutoryExtensionRecord {
            notice_at: "2026-01-08T12:00:00-08:00".to_string(),
            expected_determination_at: "2026-01-27T12:00:00-08:00".to_string(),
            reason: StatutoryExtensionReason::DataCompilation,
            notice_source: "Portal notice".to_string(),
        };
        let result = evaluate_statutory_compliance_at(
            3,
            "2026-01-01T09:00:00-08:00",
            Some("2026-01-20T12:00:00-08:00"),
            None,
            Some(&extension),
            "2026-01-20T12:00:00-08:00",
        )
        .unwrap();
        assert!(result.is_violation);
        assert_eq!(
            result.extension_status,
            StatutoryExtensionStatus::ExtensionExceedsFourteenDays
        );
    }
}
