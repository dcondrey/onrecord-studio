//! Regenerated-response detector.
//!
//! Fires when a released document's non-C2PA metadata says the file was created or last edited
//! *weeks or months* before the request's response date, with the metadata rungs internally
//! self-consistent. Interpretation: the record was, on its face, prepared long before the
//! release was issued — a form letter mailed as a response, a boilerplate PDF regenerated with
//! a fresh cover sheet, a canned Excel export mailed to every requester.
//!
//! **Distinct from `held_back_response`**: that detector fires when the metadata predates the
//! release AND the release lands near the deadline (an intent-signal about wire-hitting). This
//! detector has no proximity-to-deadline requirement — it fires whenever the metadata gap alone
//! is large enough that the release cannot plausibly be a same-cycle response.
//!
//! Both may fire on the same document. That is expected: a boilerplate response held to the
//! wire is *both* regenerated and held back.
//!
//! **Confidence gate**: only fires when the non-C2PA ladder rung is `SelfConsistentMetadata` or
//! `CorroboratedAcrossFormats` — a `MetadataPresentUncorroborated` document could carry a
//! spurious ancient date, and firing on that is not worth the false-positive cost.
//!
//! **Bounded**: reads at most `MAX_CANDIDATES` documents per invocation, ordered newest-first.

use chrono::{DateTime, Duration, Utc};
use duckdb::params;
use std::path::PathBuf;

use crate::analysis::finding::{
    finding_id, Finding, FindingCitation, FindingConfidence, FindingKind, FindingSeverity,
    FindingSubject,
};
use crate::analysis::non_c2pa_provenance::{
    classify_ooxml, classify_pdf, NonC2paProvenanceSignal, NonC2paRung,
};
use crate::error::AppError;
use crate::parsers::ooxml_docprops::read_ooxml_docprops;
use crate::parsers::pdf_metadata_signals::read_pdf_metadata_signals;
use crate::storage::DuckDbRepository;

use super::release_timing::{portal_upload_time, ReleaseBasis, PORTAL_LISTED_FILE};

pub const SLUG: &str = "regenerated_response";

const MAX_CANDIDATES: i64 = 500;

/// Minimum backdating window that trips a finding. 7 days is the smallest window that reliably
/// separates a same-week response cycle from a preassembled document.
const MIN_BACKDATE_DAYS: i64 = 7;

/// Serious rung threshold in days. Backdating of 30+ days is not consistent with any legitimate
/// same-cycle drafting.
const SERIOUS_BACKDATE_DAYS: i64 = 30;

/// Critical rung threshold in days. Backdating measured in quarters or years is not "the response
/// was prepared ahead of time" — it's "the response was fetched from an inventory".
const CRITICAL_BACKDATE_DAYS: i64 = 90;

struct Candidate {
    document_id: i64,
    portal_key: String,
    request_id: i64,
    file_name: Option<String>,
    local_path: String,
    content_type: Option<String>,
    release_at: DateTime<Utc>,
    release_basis: ReleaseBasis,
}

pub async fn run(repository: &DuckDbRepository) -> Result<Vec<Finding>, AppError> {
    let candidates = load_candidates(repository).await?;
    let mut findings = Vec::new();
    for candidate in candidates {
        let inspection = inspect(&candidate);
        if let Some(finding) = evaluate(&candidate, &inspection) {
            findings.push(finding);
        }
    }
    Ok(findings)
}

async fn load_candidates(repository: &DuckDbRepository) -> Result<Vec<Candidate>, AppError> {
    repository
        .query_connection("Regenerated-response candidate query", move |conn| {
            let mut statement = conn
                .prepare(&format!(
                    "SELECT d.id, d.portal_subdomain, d.request_id, d.file_name, d.local_path,
                        d.content_type, r.determination_at,
                        CASE WHEN {PORTAL_LISTED_FILE} THEN d.created_at END, r.created_at
                     FROM documents d
                     JOIN requests r
                        ON r.id = d.request_id
                       AND r.portal_subdomain = d.portal_subdomain
                     LEFT JOIN portals p ON p.subdomain = d.portal_subdomain
                     WHERE d.local_path IS NOT NULL
                       AND LOWER(TRIM(d.visibility)) = 'public'
                       AND r.determination_at IS NOT NULL
                     ORDER BY r.determination_at DESC
                     LIMIT ?1"
                ))
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to prepare regenerated-response query: {error}"
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
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, Option<String>>(7)?,
                        row.get::<_, String>(8)?,
                    ))
                })
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to run regenerated-response candidate query: {error}"
                    ))
                })?;
            let mut candidates = Vec::new();
            for row in rows {
                let (
                    document_id,
                    portal_key,
                    request_id,
                    file_name,
                    local_path,
                    content_type,
                    determination_at,
                    portal_file_time,
                    request_created_at,
                ) = row.map_err(|error| {
                    AppError::Storage(format!("Failed to decode candidate row: {error}"))
                })?;
                // The file's own upload time when the portal recorded one; the request's
                // determination time otherwise.
                let uploaded = portal_file_time
                    .and_then(|value| portal_upload_time(&value, &request_created_at));
                let (release_at, release_basis) = match uploaded {
                    Some(uploaded) => (uploaded.with_timezone(&Utc), ReleaseBasis::DocumentUpload),
                    None => {
                        let Ok(determined) = DateTime::parse_from_rfc3339(&determination_at) else {
                            continue;
                        };
                        (
                            determined.with_timezone(&Utc),
                            ReleaseBasis::RequestDetermination,
                        )
                    }
                };
                candidates.push(Candidate {
                    document_id,
                    portal_key,
                    request_id,
                    file_name,
                    local_path,
                    content_type,
                    release_at,
                    release_basis,
                });
            }
            Ok(candidates)
        })
        .await
}

struct Inspection {
    signal: NonC2paProvenanceSignal,
    effective_edit: Option<DateTime<Utc>>,
    signal_source: &'static str,
}

fn inspect(candidate: &Candidate) -> Inspection {
    let content_type_lower = candidate
        .content_type
        .as_deref()
        .unwrap_or("")
        .to_ascii_lowercase();
    if content_type_lower.starts_with("application/pdf") {
        let signals =
            read_pdf_metadata_signals(&PathBuf::from(&candidate.local_path)).unwrap_or_default();
        let effective = signals.effective_modification();
        return Inspection {
            signal: classify_pdf(&signals),
            effective_edit: effective,
            signal_source: "PDF metadata",
        };
    }
    // OOXML-adjacent MIME types.
    if is_ooxml_mime(&content_type_lower) {
        let props = read_ooxml_docprops(&PathBuf::from(&candidate.local_path)).unwrap_or_default();
        let effective = props.latest_touch();
        return Inspection {
            signal: classify_ooxml(&props),
            effective_edit: effective,
            signal_source: "OOXML docProps",
        };
    }
    // Attempt PDF then OOXML on unknown content types — some ingest paths lose the MIME.
    if let Ok(pdf_signals) = read_pdf_metadata_signals(&PathBuf::from(&candidate.local_path)) {
        if !pdf_signals.is_empty() {
            let effective = pdf_signals.effective_modification();
            return Inspection {
                signal: classify_pdf(&pdf_signals),
                effective_edit: effective,
                signal_source: "PDF metadata",
            };
        }
    }
    let ooxml_props =
        read_ooxml_docprops(&PathBuf::from(&candidate.local_path)).unwrap_or_default();
    let effective = ooxml_props.latest_touch();
    Inspection {
        signal: classify_ooxml(&ooxml_props),
        effective_edit: effective,
        signal_source: "OOXML docProps",
    }
}

fn is_ooxml_mime(mime: &str) -> bool {
    mime.starts_with("application/vnd.openxmlformats-officedocument")
        || mime.starts_with("application/vnd.oasis.opendocument")
        || mime.starts_with("application/vnd.ms-excel")
        || mime.starts_with("application/vnd.ms-powerpoint")
        || mime.starts_with("application/msword")
}

fn evaluate(candidate: &Candidate, inspection: &Inspection) -> Option<Finding> {
    // Confidence gate: only fire when the metadata is at least self-consistent. An
    // uncorroborated one-value metadata reading is too weak a basis for a regeneration claim.
    if !matches!(
        inspection.signal.rung,
        NonC2paRung::SelfConsistentMetadata | NonC2paRung::CorroboratedAcrossFormats
    ) {
        return None;
    }
    let edit = inspection.effective_edit?;
    let backdate = candidate.release_at - edit;
    if backdate < Duration::days(MIN_BACKDATE_DAYS) {
        return None;
    }
    // Sanity: a metadata timestamp *after* the release is not backdating — it's a metadata
    // artifact from a system that rewrites the file after export. Silently ignore.
    if backdate <= Duration::zero() {
        return None;
    }
    let backdate_days = backdate.num_days();
    let severity = if backdate_days >= CRITICAL_BACKDATE_DAYS {
        FindingSeverity::Critical
    } else if backdate_days >= SERIOUS_BACKDATE_DAYS {
        FindingSeverity::Serious
    } else {
        FindingSeverity::Notable
    };
    // Confidence tracks the underlying ladder rung: a corroborated-across-formats signal is
    // fuller basis for the claim than a merely self-consistent one.
    let confidence = FindingConfidence::measured(inspection.signal.confidence_basis_points);
    let subject = FindingSubject {
        portal_key: Some(candidate.portal_key.clone()),
        request_id: Some(candidate.request_id),
        document_id: Some(candidate.document_id),
        ..Default::default()
    };
    let bucket = format!(
        "backdate_days={};rung={}",
        backdate_days,
        inspection.signal.rung.as_str()
    );
    let id = finding_id(FindingKind::RegeneratedResponse, SLUG, &subject, &[&bucket]);
    let file_name = candidate
        .file_name
        .clone()
        .unwrap_or_else(|| format!("document {}", candidate.document_id));
    let headline = format!(
        "Response predates its release by {backdate_days} day{plural}",
        plural = if backdate_days == 1 { "" } else { "s" }
    );
    let explanation = format!(
        "\"{file_name}\" (request {request_id}) was released on {release} ({basis}) but its {source} \
         reports its most recent edit as {edit} — {backdate_days} day{plural} earlier. \
         The metadata is {rung_label} (basis {basis_points} bp), so the gap between edit and \
         release cannot be explained by a same-cycle drafting pass. Consistent with a canned \
         document dropped in as the response.",
        request_id = candidate.request_id,
        release = format_short(candidate.release_at),
        basis = candidate.release_basis.describe(),
        source = inspection.signal_source,
        edit = format_short(edit),
        plural = if backdate_days == 1 { "" } else { "s" },
        rung_label = describe_rung(inspection.signal.rung),
        basis_points = inspection.signal.confidence_basis_points,
    );
    let citations = vec![FindingCitation {
        portal_key: candidate.portal_key.clone(),
        document_id: candidate.document_id,
        character_start: None,
        character_end: None,
        quote: None,
        label: Some(format!(
            "{source} on {file_name} — {rationale}",
            source = inspection.signal_source,
            rationale = inspection.signal.rationale,
        )),
    }];
    let now = Utc::now();
    Some(Finding {
        id,
        kind: FindingKind::RegeneratedResponse,
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

fn describe_rung(rung: NonC2paRung) -> &'static str {
    match rung {
        NonC2paRung::NoSignals => "carrying no signals",
        NonC2paRung::MetadataPresentUncorroborated => "present but uncorroborated",
        NonC2paRung::SelfConsistentMetadata => "internally self-consistent",
        NonC2paRung::CorroboratedAcrossFormats => "corroborated across independent sources",
    }
}

fn format_short(instant: DateTime<Utc>) -> String {
    instant.format("%Y-%m-%d").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parsers::pdf_metadata_signals::PdfMetadataSignals;

    fn candidate(release_at: DateTime<Utc>) -> Candidate {
        Candidate {
            document_id: 1,
            portal_key: "test".into(),
            request_id: 42,
            file_name: Some("response.pdf".into()),
            local_path: "/tmp/x.pdf".into(),
            content_type: Some("application/pdf".into()),
            release_at,
            release_basis: ReleaseBasis::RequestDetermination,
        }
    }

    fn dt(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    fn self_consistent(edit_at: DateTime<Utc>) -> Inspection {
        let signals = PdfMetadataSignals {
            info_creation_date: Some(edit_at - Duration::days(1)),
            info_mod_date: Some(edit_at),
            ..Default::default()
        };
        Inspection {
            signal: classify_pdf(&signals),
            effective_edit: signals.effective_modification(),
            signal_source: "PDF metadata",
        }
    }

    #[test]
    fn does_not_fire_when_edit_is_within_the_minimum_backdating_window() {
        let candidate = candidate(dt("2026-05-01T10:00:00Z"));
        let inspection = self_consistent(dt("2026-04-28T10:00:00Z"));
        assert!(evaluate(&candidate, &inspection).is_none());
    }

    #[test]
    fn does_not_fire_on_uncorroborated_metadata_even_if_dates_look_ancient() {
        let candidate = candidate(dt("2026-05-01T10:00:00Z"));
        let inspection = Inspection {
            signal: NonC2paProvenanceSignal {
                rung: NonC2paRung::MetadataPresentUncorroborated,
                confidence_basis_points: 2500,
                rationale: "producer only".into(),
            },
            effective_edit: Some(dt("2020-01-01T10:00:00Z")),
            signal_source: "PDF metadata",
        };
        assert!(
            evaluate(&candidate, &inspection).is_none(),
            "uncorroborated metadata is too weak a basis for a regeneration claim"
        );
    }

    #[test]
    fn notable_severity_for_a_two_week_backdate_with_self_consistent_metadata() {
        let candidate = candidate(dt("2026-05-01T10:00:00Z"));
        let inspection = self_consistent(dt("2026-04-15T10:00:00Z"));
        let finding = evaluate(&candidate, &inspection).expect("must fire");
        assert_eq!(finding.severity, FindingSeverity::Notable);
        assert_eq!(
            finding.confidence,
            FindingConfidence::measured(
                NonC2paRung::SelfConsistentMetadata.confidence_basis_points()
            )
        );
    }

    #[test]
    fn serious_severity_for_a_forty_five_day_backdate() {
        let candidate = candidate(dt("2026-05-01T10:00:00Z"));
        let inspection = self_consistent(dt("2026-03-15T10:00:00Z"));
        let finding = evaluate(&candidate, &inspection).expect("must fire");
        assert_eq!(finding.severity, FindingSeverity::Serious);
    }

    #[test]
    fn critical_severity_for_a_multi_month_backdate() {
        let candidate = candidate(dt("2026-05-01T10:00:00Z"));
        let inspection = self_consistent(dt("2025-11-01T10:00:00Z"));
        let finding = evaluate(&candidate, &inspection).expect("must fire");
        assert_eq!(finding.severity, FindingSeverity::Critical);
    }

    #[test]
    fn moddate_after_release_does_not_fire() {
        let candidate = candidate(dt("2026-05-01T10:00:00Z"));
        let inspection = self_consistent(dt("2026-05-05T10:00:00Z"));
        assert!(evaluate(&candidate, &inspection).is_none());
    }

    #[test]
    fn corroborated_metadata_yields_higher_confidence_than_self_consistent() {
        let candidate = candidate(dt("2026-05-01T10:00:00Z"));
        let self_consistent = self_consistent(dt("2026-03-01T10:00:00Z"));
        let corroborated = {
            let signals = PdfMetadataSignals {
                info_mod_date: Some(dt("2026-03-01T10:00:00Z")),
                xmp_modify_date: Some(dt("2026-03-01T10:15:00Z")),
                ..Default::default()
            };
            Inspection {
                signal: classify_pdf(&signals),
                effective_edit: signals.effective_modification(),
                signal_source: "PDF metadata",
            }
        };
        let a = evaluate(&candidate, &self_consistent).unwrap();
        let b = evaluate(&candidate, &corroborated).unwrap();
        match (a.confidence, b.confidence) {
            (
                FindingConfidence::Measured { basis_points: bp_a },
                FindingConfidence::Measured { basis_points: bp_b },
            ) => assert!(bp_b > bp_a),
            _ => panic!("both must be measured"),
        }
    }

    #[test]
    fn finding_id_stable_across_re_runs_for_same_bucket() {
        let candidate = candidate(dt("2026-05-01T10:00:00Z"));
        let inspection = self_consistent(dt("2026-04-15T10:00:00Z"));
        let a = evaluate(&candidate, &inspection).unwrap();
        let b = evaluate(&candidate, &inspection).unwrap();
        assert_eq!(a.id, b.id);
    }

    #[test]
    fn is_ooxml_mime_recognizes_the_common_office_families() {
        assert!(is_ooxml_mime(
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        ));
        assert!(is_ooxml_mime("application/vnd.ms-excel"));
        assert!(is_ooxml_mime("application/msword"));
        assert!(!is_ooxml_mime("application/pdf"));
        assert!(!is_ooxml_mime(""));
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
        let determined = dt("2026-03-05T20:00:00Z");
        assert_eq!(
            timed(1),
            (dt("2026-03-04T23:30:00Z"), ReleaseBasis::DocumentUpload)
        );
        assert_eq!(timed(2), (determined, ReleaseBasis::RequestDetermination));
        assert_eq!(timed(3), (determined, ReleaseBasis::RequestDetermination));
        let stated = evaluate(
            &Candidate {
                release_basis: ReleaseBasis::DocumentUpload,
                ..candidate(dt("2026-05-01T10:00:00Z"))
            },
            &self_consistent(dt("2026-04-15T10:00:00Z")),
        )
        .expect("fires");
        assert!(stated
            .explanation
            .contains("(the portal's recorded upload time of this file)"));
    }
}
