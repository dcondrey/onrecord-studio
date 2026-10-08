//! Held-back-response detector.
//!
//! Surfaces documents whose PDF `ModDate` (and/or XMP `xmp:ModifyDate`) predates their release by
//! more than 24 hours, when the release itself landed within 48 hours of the statutory deadline.
//! That combination — "edit happened days or weeks ago, release happened right before the clock
//! ran out" — is evidence the response was sat on rather than genuinely worked until the wire.
//!
//! **What this is not**: a legal-claim finding. The deadline was met. This is a
//! behavioral/pattern signal about intent, priced accordingly (`FindingKind::HeldBackResponse`,
//! `is_legal_claim = false`).
//!
//! **Bounds**: reads at most `MAX_CANDIDATES` documents per invocation; caller runs the sweep on
//! ingest completion and on explicit refresh, no background heartbeat.

use super::release_timing::{portal_upload_time, ReleaseBasis, PORTAL_LISTED_FILE};
use crate::{
    analysis::finding::{
        finding_id, Finding, FindingCitation, FindingConfidence, FindingKind, FindingSeverity,
        FindingSubject,
    },
    error::AppError,
    parsers::pdf_metadata_signals::read_pdf_metadata_signals,
    storage::DuckDbRepository,
};
#[cfg(test)]
use chrono::TimeZone;
use chrono::{DateTime, Duration, Utc};
use duckdb::params;
use std::path::Path;

pub const SLUG: &str = "held_back_response";

/// Max documents inspected per invocation. Bounded so a huge corpus doesn't stall the refresh.
/// Ordered newest-first so the most recent evidence surfaces even when the cap is hit.
const MAX_CANDIDATES: i64 = 500;

/// Minimum ModDate-before-release window that flags a document. Under 24h is normal editorial
/// churn on a same-day release.
const MIN_HOLDBACK_HOURS: i64 = 24;

/// A release lands "at the wire" if it's within this many hours of the deadline.
const AT_THE_WIRE_HOURS: i64 = 48;

/// Threshold for Critical severity: the doc was sitting for more than two weeks.
const CRITICAL_HOLDBACK_DAYS: i64 = 14;

/// Threshold for Serious severity: the doc was sitting for more than three days.
const SERIOUS_HOLDBACK_DAYS: i64 = 3;

struct Candidate {
    document_id: i64,
    portal_key: String,
    request_id: i64,
    file_name: Option<String>,
    local_path: String,
    release_at: DateTime<Utc>,
    release_basis: ReleaseBasis,
    deadline_at: DateTime<Utc>,
}

pub async fn run(repository: &DuckDbRepository) -> Result<Vec<Finding>, AppError> {
    let candidates = load_candidates(repository).await?;
    let mut findings = Vec::new();
    for candidate in candidates {
        // An unreadable or oversized PDF is not a detector failure; the residual-evidence audit
        // records it through its own channel.
        let Ok(signals) = read_pdf_metadata_signals(Path::new(&candidate.local_path)) else {
            continue;
        };
        let dates = PdfMetadataDates {
            mod_date: signals.info_mod_date,
            xmp_modify_date: signals.xmp_modify_date,
        };
        if let Some(finding) = evaluate(&candidate, dates) {
            findings.push(finding);
        }
    }
    Ok(findings)
}

async fn load_candidates(repository: &DuckDbRepository) -> Result<Vec<Candidate>, AppError> {
    repository
        .query_connection("Held-back-response candidate query", move |conn| {
            // Response documents = public documents attached to a request whose determination_at
            // and due_date are both known. A file's release time is the portal's own upload time
            // for it when one is recorded, and the request's determination_at otherwise;
            // due_date is the statutory deadline the request was working against.
            let mut statement = conn
                .prepare(&format!(
                    "SELECT d.id, d.portal_subdomain, d.request_id, d.file_name, d.local_path,
                        r.determination_at, r.due_date,
                        CASE WHEN {PORTAL_LISTED_FILE} THEN d.created_at END, r.created_at
                     FROM documents d
                     JOIN requests r
                        ON r.id = d.request_id
                       AND r.portal_subdomain = d.portal_subdomain
                     LEFT JOIN portals p ON p.subdomain = d.portal_subdomain
                     WHERE d.local_path IS NOT NULL
                       AND LOWER(TRIM(d.visibility)) = 'public'
                       AND LOWER(COALESCE(d.content_type, '')) LIKE 'application/pdf%'
                       AND r.determination_at IS NOT NULL
                       AND r.due_date IS NOT NULL
                     ORDER BY r.determination_at DESC
                     LIMIT ?1"
                ))
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to prepare held-back candidate query: {error}"
                    ))
                })?;
            let rows = statement
                .query_map(params![MAX_CANDIDATES], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, Option<String>>(7)?,
                        row.get::<_, String>(8)?,
                    ))
                })
                .map_err(|error| {
                    AppError::Storage(format!("Failed to run held-back candidate query: {error}"))
                })?;
            let mut candidates = Vec::new();
            for row in rows {
                let (
                    document_id,
                    portal_key,
                    request_id,
                    file_name,
                    local_path,
                    determination_at,
                    due_date,
                    portal_file_time,
                    request_created_at,
                ) = row.map_err(|error| {
                    AppError::Storage(format!("Failed to decode candidate row: {error}"))
                })?;
                let uploaded = portal_file_time
                    .and_then(|value| portal_upload_time(&value, &request_created_at));
                let (release_at, release_basis) = match uploaded {
                    Some(uploaded) => (uploaded.with_timezone(&Utc), ReleaseBasis::DocumentUpload),
                    None => {
                        let Ok(determined) = parse_iso(&determination_at) else {
                            continue;
                        };
                        (determined, ReleaseBasis::RequestDetermination)
                    }
                };
                let Ok(deadline_at) = parse_iso(&due_date) else {
                    continue;
                };
                candidates.push(Candidate {
                    document_id,
                    portal_key,
                    request_id,
                    file_name,
                    local_path,
                    release_at,
                    release_basis,
                    deadline_at,
                });
            }
            Ok(candidates)
        })
        .await
}

/// The dates a PDF reports about itself.
#[derive(Debug, Clone, Copy)]
struct PdfMetadataDates {
    mod_date: Option<DateTime<Utc>>,
    xmp_modify_date: Option<DateTime<Utc>>,
}

impl PdfMetadataDates {
    /// The effective modification instant, if any signal was found. Prefer ModDate when both are
    /// present; XMP fallback when only that exists.
    fn effective_modification(self) -> Option<DateTime<Utc>> {
        self.mod_date.or(self.xmp_modify_date)
    }

    /// True when both signals were present and they agreed (within 60 minutes to tolerate
    /// timezone bookkeeping quirks between the two clocks).
    fn both_signals_agree(self) -> bool {
        match (self.mod_date, self.xmp_modify_date) {
            (Some(a), Some(b)) => (a - b).num_minutes().abs() < 60,
            _ => false,
        }
    }
}

fn parse_iso(value: &str) -> Result<DateTime<Utc>, AppError> {
    DateTime::parse_from_rfc3339(value.trim())
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|error| AppError::DataParsing(format!("Invalid ISO date '{value}': {error}")))
}

fn evaluate(candidate: &Candidate, dates: PdfMetadataDates) -> Option<Finding> {
    let effective_mod = dates.effective_modification()?;
    let holdback = candidate.release_at - effective_mod;
    if holdback < Duration::hours(MIN_HOLDBACK_HOURS) {
        return None;
    }
    let time_to_deadline = candidate.deadline_at - candidate.release_at;
    if time_to_deadline.abs() > Duration::hours(AT_THE_WIRE_HOURS) {
        // Released comfortably early or well past the deadline: this pattern is about the
        // wire-hitting release specifically, not late responses (those are the statutory
        // detector's territory) or genuinely-early ones.
        return None;
    }
    // Sanity: ModDate later than release_at is a metadata artifact, not evidence of holdback.
    if holdback <= Duration::zero() {
        return None;
    }
    let holdback_days = holdback.num_days().max(0);
    let severity = if holdback_days >= CRITICAL_HOLDBACK_DAYS {
        FindingSeverity::Critical
    } else if holdback_days >= SERIOUS_HOLDBACK_DAYS {
        FindingSeverity::Serious
    } else {
        FindingSeverity::Notable
    };
    // Two agreeing signals earn full confidence; one signal alone earns half. This mirrors the
    // project's provenance-confidence pattern (independently-corroborated evidence outranks a
    // lone recorded claim).
    let confidence = if dates.both_signals_agree() {
        FindingConfidence::measured(9000)
    } else {
        FindingConfidence::measured(5000)
    };
    let subject = FindingSubject {
        portal_key: Some(candidate.portal_key.clone()),
        request_id: Some(candidate.request_id),
        document_id: Some(candidate.document_id),
        ..Default::default()
    };
    let holdback_bucket = format!("holdback_days={holdback_days}");
    let id = finding_id(
        FindingKind::HeldBackResponse,
        SLUG,
        &subject,
        &[&holdback_bucket],
    );
    let file_name = candidate
        .file_name
        .clone()
        .unwrap_or_else(|| format!("document {}", candidate.document_id));
    let headline = format!(
        "Response held {holdback_days} day{plural} after last edit",
        plural = if holdback_days == 1 { "" } else { "s" }
    );
    let signal_source = if dates.both_signals_agree() {
        "PDF ModDate and xmp:ModifyDate agree"
    } else if dates.mod_date.is_some() {
        "PDF ModDate"
    } else {
        "xmp:ModifyDate"
    };
    let explanation = format!(
        "\"{file_name}\" (request {request_id}) was released on {release} ({basis}) \
         but its {signal_source} shows it was last modified on {edited} \
         — {holdback_days} day{plural} earlier. The release landed within {wire_hours}h of \
         the statutory deadline ({deadline}), consistent with a document that was ready \
         earlier and held back until the wire.",
        request_id = candidate.request_id,
        release = candidate.release_at.format("%Y-%m-%d"),
        basis = candidate.release_basis.describe(),
        edited = effective_mod.format("%Y-%m-%d"),
        plural = if holdback_days == 1 { "" } else { "s" },
        wire_hours = AT_THE_WIRE_HOURS,
        deadline = candidate.deadline_at.format("%Y-%m-%d"),
    );
    let citations = vec![FindingCitation {
        portal_key: candidate.portal_key.clone(),
        document_id: candidate.document_id,
        character_start: None,
        character_end: None,
        quote: None,
        label: Some(format!("{signal_source} on {file_name}")),
    }];
    let now = Utc::now();
    Some(Finding {
        id,
        kind: FindingKind::HeldBackResponse,
        severity,
        confidence,
        subject,
        headline,
        explanation,
        citations,
        discovered_at: now,
        last_seen_at: now,
        dismissed_at: None,
        source_detector: SLUG.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(release_at: DateTime<Utc>, deadline_at: DateTime<Utc>) -> Candidate {
        Candidate {
            document_id: 42,
            portal_key: "test".into(),
            request_id: 7,
            file_name: Some("response.pdf".into()),
            local_path: "/tmp/response.pdf".into(),
            release_at,
            release_basis: ReleaseBasis::RequestDetermination,
            deadline_at,
        }
    }

    fn ts(day: u32, hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, day, hour, 0, 0).unwrap()
    }

    #[test]
    fn both_signals_agree_tolerates_up_to_59_minutes() {
        let a = Utc.with_ymd_and_hms(2026, 3, 15, 10, 0, 0).unwrap();
        let close = Utc.with_ymd_and_hms(2026, 3, 15, 10, 45, 0).unwrap();
        let far = Utc.with_ymd_and_hms(2026, 3, 15, 12, 0, 0).unwrap();
        assert!(PdfMetadataDates {
            mod_date: Some(a),
            xmp_modify_date: Some(close),
        }
        .both_signals_agree());
        assert!(!PdfMetadataDates {
            mod_date: Some(a),
            xmp_modify_date: Some(far),
        }
        .both_signals_agree());
        // One signal alone never counts as agreement.
        assert!(!PdfMetadataDates {
            mod_date: Some(a),
            xmp_modify_date: None,
        }
        .both_signals_agree());
    }

    #[test]
    fn no_finding_when_holdback_under_the_minimum() {
        // Same-day churn — edit at 06:00, release at 15:00 = 9h holdback. Below MIN_HOLDBACK_HOURS.
        let candidate = candidate(ts(15, 15), ts(15, 23));
        let dates = PdfMetadataDates {
            mod_date: Some(ts(15, 6)),
            xmp_modify_date: None,
        };
        assert!(evaluate(&candidate, dates).is_none());
    }

    #[test]
    fn no_finding_when_release_is_comfortably_early() {
        // Released 5 days ahead of deadline: this pattern is about wire-hitting releases only.
        let candidate = candidate(ts(1, 12), ts(10, 12));
        let dates = PdfMetadataDates {
            mod_date: Some(Utc.with_ymd_and_hms(2025, 12, 25, 12, 0, 0).unwrap()),
            xmp_modify_date: None,
        };
        assert!(evaluate(&candidate, dates).is_none());
    }

    #[test]
    fn no_finding_when_moddate_is_after_release() {
        // ModDate later than release is a metadata artifact, not holdback evidence.
        let candidate = candidate(ts(1, 12), ts(2, 12));
        let dates = PdfMetadataDates {
            mod_date: Some(ts(5, 12)),
            xmp_modify_date: None,
        };
        assert!(evaluate(&candidate, dates).is_none());
    }

    #[test]
    fn serious_finding_for_a_five_day_holdback_at_the_wire() {
        // Deadline day 10 12:00, released day 10 08:00 (4h early, "at the wire" is 48h).
        // ModDate day 5 12:00 => 4 days 20h ago => rounds to 4 days holdback.
        let candidate = candidate(ts(10, 8), ts(10, 12));
        let dates = PdfMetadataDates {
            mod_date: Some(ts(5, 12)),
            xmp_modify_date: None,
        };
        let finding = evaluate(&candidate, dates).expect("must fire");
        assert_eq!(finding.severity, FindingSeverity::Serious);
        // One signal only => half confidence.
        assert_eq!(finding.confidence, FindingConfidence::measured(5000));
        assert_eq!(finding.subject.document_id, Some(42));
        assert!(finding.headline.contains("held"));
        assert!(finding.explanation.contains("PDF ModDate"));
    }

    #[test]
    fn critical_severity_and_full_confidence_when_both_signals_agree_over_two_weeks() {
        let deadline = Utc.with_ymd_and_hms(2026, 2, 10, 12, 0, 0).unwrap();
        let release = Utc.with_ymd_and_hms(2026, 2, 10, 10, 0, 0).unwrap();
        let edit = Utc.with_ymd_and_hms(2026, 1, 20, 10, 0, 0).unwrap();
        let candidate = Candidate {
            release_at: release,
            deadline_at: deadline,
            ..candidate(release, deadline)
        };
        let dates = PdfMetadataDates {
            mod_date: Some(edit),
            xmp_modify_date: Some(edit + Duration::minutes(3)),
        };
        let finding = evaluate(&candidate, dates).expect("must fire");
        assert_eq!(finding.severity, FindingSeverity::Critical);
        assert_eq!(finding.confidence, FindingConfidence::measured(9000));
    }

    #[test]
    fn finding_id_stable_across_re_runs_for_same_holdback_bucket() {
        let candidate = candidate(ts(10, 8), ts(10, 12));
        let dates = PdfMetadataDates {
            mod_date: Some(ts(5, 12)),
            xmp_modify_date: None,
        };
        let a = evaluate(&candidate, dates).unwrap();
        let b = evaluate(&candidate, dates).unwrap();
        assert_eq!(a.id, b.id, "same evidence must produce same id for upsert");
    }

    #[tokio::test]
    async fn a_file_is_timed_by_its_own_upload_and_by_the_determination_only_without_one() {
        let repository = super::super::release_timing::repository_with_one_dated_upload().await;
        let candidates = load_candidates(&repository).await.expect("load");
        let timed = |id: i64| {
            let candidate = candidates
                .iter()
                .find(|candidate| candidate.document_id == id)
                .expect("candidate");
            (candidate.release_at, candidate.release_basis)
        };
        let uploaded = Utc.with_ymd_and_hms(2026, 3, 4, 23, 30, 0).unwrap();
        let determined = Utc.with_ymd_and_hms(2026, 3, 5, 20, 0, 0).unwrap();
        assert_eq!(timed(1), (uploaded, ReleaseBasis::DocumentUpload));
        assert_eq!(timed(2), (determined, ReleaseBasis::RequestDetermination));
        assert_eq!(timed(3), (determined, ReleaseBasis::RequestDetermination));
    }
}
