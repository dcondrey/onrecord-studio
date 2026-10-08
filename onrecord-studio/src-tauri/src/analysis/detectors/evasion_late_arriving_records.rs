//! "No records" denials followed by matching material from the same agency, dated after the
//! denial's determination.
//!
//! `no_records_contradicted` fires when a denial's topic is contradicted by material released
//! from OTHER requests. This detector operates temporally: an agency told requester A that no
//! records exist, and after that denial's determination_at, records matching the same topic
//! turned up in the agency's own release stream. The gap between denial and later-release matters:
//! a release in the same week is likely a records-management lag; a release months later, after
//! any plausible appeal window closed, is suggestive of held material surfaced only once the
//! requester could no longer challenge the original denial.
//!
//! **Not a legal claim** (`is_legal_claim = false`). Adversarial signal at ≥ 6500 bp only.
//! Confidence scales with gap size AND with the number of independently-matching later-releases.
//!
//! **Grace window**: releases within 14 days of the denial are treated as records-management
//! churn and do not fire. Between 14-180 days is `Notable`, 180-365 is `Serious`, > 365 is
//! `Critical` — appeal windows vary by jurisdiction but a year is universally beyond any of them.

use super::parse_utc;
use crate::{
    analysis::{
        assertion_index::{extract_denial, extract_topic_tokens},
        finding::{
            finding_id, Finding, FindingCitation, FindingConfidence, FindingKind, FindingSeverity,
            FindingSubject,
        },
    },
    error::AppError,
    storage::DuckDbRepository,
};
use chrono::{DateTime, Utc};
use duckdb::params;

pub const SLUG: &str = "evasion_late_arriving_records";

const MAX_DENIALS_INSPECTED: i64 = 500;
const MIN_TOPIC_TOKENS: usize = 2;
const MIN_MATCHING_LATER_DOCS: usize = 2;
const GRACE_PERIOD_DAYS: i64 = 14;
const SERIOUS_GAP_DAYS: i64 = 180;
const CRITICAL_GAP_DAYS: i64 = 365;
const MAX_CITATION_ROWS: usize = 4;
const MAX_FINDINGS: usize = 50;
const MIN_CONFIDENCE_TO_FIRE: u16 = 6500;

pub async fn run(repository: &DuckDbRepository) -> Result<Vec<Finding>, AppError> {
    let denials = load_denials(repository).await?;
    let mut findings = Vec::new();
    for denial in denials {
        let Some(fingerprint) = extract_denial(&denial.denial_text) else {
            continue;
        };
        if fingerprint.scope_narrow {
            continue;
        }
        let tokens = extract_topic_tokens(&denial.request_title, &denial.request_description);
        if tokens.len() < MIN_TOPIC_TOKENS {
            continue;
        }
        let later = search_later_matching(repository, &denial, &tokens).await?;
        if later.len() < MIN_MATCHING_LATER_DOCS {
            continue;
        }
        if let Some(finding) = build_finding(&denial, &fingerprint.matched_phrase, &tokens, &later)
        {
            findings.push(finding);
        }
    }
    findings.sort_by_key(|f| std::cmp::Reverse(f.confidence.rank_weight()));
    findings.truncate(MAX_FINDINGS);
    Ok(findings)
}

#[derive(Debug, Clone)]
struct DenialCandidate {
    portal_key: String,
    agency_id: i64,
    request_id: i64,
    denial_document_id: i64,
    denial_file_name: Option<String>,
    request_title: String,
    request_description: String,
    denial_text: String,
    determination_at: DateTime<Utc>,
}

async fn load_denials(repository: &DuckDbRepository) -> Result<Vec<DenialCandidate>, AppError> {
    repository
        .query_connection("Late-arriving-records denial query", move |conn| {
            let mut statement = conn
                .prepare(
                    "SELECT d.portal_subdomain, r.agency_id, d.request_id, d.id, d.file_name,
                            COALESCE(r.title, ''), COALESCE(r.description, ''),
                            fts.extracted_text, r.determination_at
                     FROM documents d
                     JOIN documents_fts fts
                        ON fts.document_id = d.id
                       AND fts.portal_subdomain = d.portal_subdomain
                     JOIN requests r
                        ON r.id = d.request_id
                       AND r.portal_subdomain = d.portal_subdomain
                     WHERE r.agency_id IS NOT NULL
                       AND r.determination_at IS NOT NULL
                       AND LOWER(TRIM(d.visibility)) = 'public'
                       AND LENGTH(fts.extracted_text) > 0
                       AND (
                           LOWER(fts.extracted_text) LIKE '%no responsive records%' OR
                           LOWER(fts.extracted_text) LIKE '%no records exist%' OR
                           LOWER(fts.extracted_text) LIKE '%no records were found%' OR
                           LOWER(fts.extracted_text) LIKE '%unable to locate%' OR
                           LOWER(fts.extracted_text) LIKE '%cannot locate any records%'
                       )
                     ORDER BY r.determination_at DESC
                     LIMIT ?1",
                )
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to prepare late-arriving denial query: {error}"
                    ))
                })?;
            let rows = statement
                .query_map(params![MAX_DENIALS_INSPECTED], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, String>(7)?,
                        row.get::<_, String>(8)?,
                    ))
                })
                .map_err(|error| {
                    AppError::Storage(format!("Failed to run late-arriving denial query: {error}"))
                })?;
            let mut out = Vec::new();
            for row in rows {
                let (
                    portal_key,
                    agency_id,
                    request_id,
                    denial_document_id,
                    denial_file_name,
                    request_title,
                    request_description,
                    denial_text,
                    determination_at_str,
                ) = row.map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to decode late-arriving denial row: {error}"
                    ))
                })?;
                let Some(determination_at) = parse_utc(&determination_at_str) else {
                    continue;
                };
                out.push(DenialCandidate {
                    portal_key,
                    agency_id,
                    request_id,
                    denial_document_id,
                    denial_file_name,
                    request_title,
                    request_description,
                    denial_text,
                    determination_at,
                });
            }
            Ok(out)
        })
        .await
}

#[derive(Debug, Clone)]
struct LaterMatch {
    portal_key: String,
    document_id: i64,
    request_id: i64,
    file_name: Option<String>,
    days_later: i64,
    excerpt: String,
}

async fn search_later_matching(
    repository: &DuckDbRepository,
    denial: &DenialCandidate,
    tokens: &[String],
) -> Result<Vec<LaterMatch>, AppError> {
    // Build a LIKE-based OR query across tokens. Bounded token count keeps this SQL manageable.
    // We compare release timestamp to the denial's determination_at using documents.updated_at as
    // a proxy for release time — updated_at is set on release ingestion.
    let portal_key = denial.portal_key.clone();
    let agency_id = denial.agency_id;
    let denial_request_id = denial.request_id;
    let determination_at_iso = denial.determination_at.to_rfc3339();
    let tokens: Vec<String> = tokens.iter().take(6).cloned().collect();
    let grace_days = GRACE_PERIOD_DAYS;
    repository
        .query_connection("Late-arriving matching-release query", move |conn| {
            let mut like_clauses = Vec::new();
            let mut params_dyn: Vec<duckdb::types::Value> = vec![
                portal_key.clone().into(),
                agency_id.into(),
                denial_request_id.into(),
                determination_at_iso.clone().into(),
                grace_days.into(),
            ];
            for token in &tokens {
                like_clauses.push("LOWER(fts.extracted_text) LIKE '%' || ? || '%'".to_string());
                params_dyn.push(token.clone().into());
            }
            let token_or = if like_clauses.is_empty() {
                "1=0".to_string()
            } else {
                like_clauses.join(" OR ")
            };
            let sql = format!(
                "SELECT d.portal_subdomain, d.id, d.request_id, d.file_name,
                        SUBSTR(fts.extracted_text, 1, 240) AS excerpt,
                        d.updated_at
                 FROM documents d
                 JOIN documents_fts fts
                    ON fts.document_id = d.id
                   AND fts.portal_subdomain = d.portal_subdomain
                 JOIN requests r
                    ON r.id = d.request_id
                   AND r.portal_subdomain = d.portal_subdomain
                 WHERE d.portal_subdomain = ?
                   AND r.agency_id = ?
                   AND d.request_id != ?
                   AND LOWER(TRIM(d.visibility)) = 'public'
                   AND LENGTH(fts.extracted_text) > 0
                   AND d.updated_at > ?
                   AND date_diff('day', ?, d.updated_at) > ?
                   AND ({token_or})
                 ORDER BY d.updated_at ASC
                 LIMIT 30"
            );
            // Duplicate determination_at for the date_diff argument.
            let mut adjusted: Vec<duckdb::types::Value> = vec![
                portal_key.clone().into(),
                agency_id.into(),
                denial_request_id.into(),
                determination_at_iso.clone().into(),
                determination_at_iso.clone().into(),
                grace_days.into(),
            ];
            for token in &tokens {
                adjusted.push(token.clone().into());
            }
            let mut statement = conn.prepare(&sql).map_err(|error| {
                AppError::Storage(format!(
                    "Failed to prepare late-arriving match query: {error}"
                ))
            })?;
            let param_refs: Vec<&dyn duckdb::ToSql> =
                adjusted.iter().map(|v| v as &dyn duckdb::ToSql).collect();
            let rows = statement
                .query_map(param_refs.as_slice(), |row| {
                    let portal_key: String = row.get(0)?;
                    let document_id: i64 = row.get(1)?;
                    let request_id: i64 = row.get(2)?;
                    let file_name: Option<String> = row.get(3)?;
                    let excerpt: String = row.get(4)?;
                    let updated_at_str: String = row.get(5)?;
                    Ok((
                        portal_key,
                        document_id,
                        request_id,
                        file_name,
                        excerpt,
                        updated_at_str,
                    ))
                })
                .map_err(|error| {
                    AppError::Storage(format!("Failed to run late-arriving match query: {error}"))
                })?;
            let determination_at = parse_utc(&determination_at_iso).ok_or_else(|| {
                AppError::Storage("Failed to reparse determination_at".to_string())
            })?;
            let mut out = Vec::new();
            for row in rows {
                let (portal_key, document_id, request_id, file_name, excerpt, updated_at_str) = row
                    .map_err(|error| {
                        AppError::Storage(format!(
                            "Failed to decode late-arriving match row: {error}"
                        ))
                    })?;
                let Some(updated_at) = parse_utc(&updated_at_str) else {
                    continue;
                };
                let days_later = (updated_at - determination_at).num_days();
                out.push(LaterMatch {
                    portal_key,
                    document_id,
                    request_id,
                    file_name,
                    days_later,
                    excerpt,
                });
            }
            Ok(out)
        })
        .await
}

fn build_finding(
    denial: &DenialCandidate,
    denial_phrase: &str,
    tokens: &[String],
    later: &[LaterMatch],
) -> Option<Finding> {
    // Defense in depth: the caller's loop already gates on this, but build_finding is called
    // directly from tests, so enforce the minimum here too.
    if later.len() < MIN_MATCHING_LATER_DOCS {
        return None;
    }
    // Gap = the SHORTEST time between denial and its matching later release. The tightest match
    // is the strongest signal; wider gaps are lower priority even if there are many of them.
    let min_gap = later.iter().map(|m| m.days_later).min()?;
    let max_gap = later.iter().map(|m| m.days_later).max()?;
    let severity = if max_gap >= CRITICAL_GAP_DAYS {
        FindingSeverity::Critical
    } else if max_gap >= SERIOUS_GAP_DAYS {
        FindingSeverity::Serious
    } else {
        FindingSeverity::Notable
    };
    // Confidence: base + scale by (n, min gap widening, max gap widening), floor at MIN_CONFIDENCE.
    let n_bp =
        ((later.len() as f64 - MIN_MATCHING_LATER_DOCS as f64) * 350.0).clamp(0.0, 1400.0) as u16;
    let min_gap_bp =
        ((min_gap.saturating_sub(GRACE_PERIOD_DAYS)) as f64 * 4.0).clamp(0.0, 1400.0) as u16;
    let max_gap_bp =
        ((max_gap.saturating_sub(SERIOUS_GAP_DAYS)) as f64 * 3.0).clamp(0.0, 1400.0) as u16;
    // Base 6500 = MIN_CONFIDENCE_TO_FIRE floor. The 2+ matches gate at line 60 is the
    // sufficiency check; confidence scales UP from that floor with better evidence.
    let raw_bp = 6500_u16
        .saturating_add(n_bp)
        .saturating_add(min_gap_bp)
        .saturating_add(max_gap_bp);
    let bp = raw_bp.min(10_000);
    if bp < MIN_CONFIDENCE_TO_FIRE {
        return None;
    }
    let subject = FindingSubject {
        portal_key: Some(denial.portal_key.clone()),
        agency_id: Some(denial.agency_id),
        request_id: Some(denial.request_id),
        document_id: Some(denial.denial_document_id),
        ..Default::default()
    };
    let salient = format!(
        "denial_doc={},later_n={},max_gap_bucket={}",
        denial.denial_document_id,
        later.len(),
        (max_gap / 30).min(120)
    );
    let id = finding_id(
        FindingKind::EvasionLateArrivingRecords,
        SLUG,
        &subject,
        &[&salient],
    );
    let denial_name = denial
        .denial_file_name
        .clone()
        .unwrap_or_else(|| format!("document {}", denial.denial_document_id));
    let title = denial
        .request_title
        .chars()
        .take(60)
        .collect::<String>()
        .trim_end()
        .to_string();
    let headline = format!(
        "Denial \"{title}\" preceded {n} matching public release{plural} from the same agency",
        n = later.len(),
        plural = if later.len() == 1 { "" } else { "s" }
    );
    let explanation = format!(
        "\"{denial_name}\" (request {denial_request}) told the requester \"{phrase}\" on {when}. \
         Between that determination and the current corpus refresh, {n} public documents released \
         from other requests to the same agency (id {agency_id}) mention terms from this \
         request's topic vocabulary ({tokens}). Time from denial to first matching release: \
         {min_gap} days; to the last matching release: {max_gap} days. \
         Records matching the denied topic surfaced elsewhere after the denial, potentially past \
         any appeal window this jurisdiction imposes.",
        denial_request = denial.request_id,
        agency_id = denial.agency_id,
        phrase = denial_phrase,
        when = denial.determination_at.format("%Y-%m-%d"),
        tokens = tokens.join(", "),
        n = later.len(),
        min_gap = min_gap,
        max_gap = max_gap,
    );
    let mut citations = Vec::with_capacity(1 + MAX_CITATION_ROWS);
    citations.push(FindingCitation {
        portal_key: denial.portal_key.clone(),
        document_id: denial.denial_document_id,
        character_start: None,
        character_end: None,
        quote: Some(denial_phrase.to_string()),
        label: Some(format!("denial: {denial_name}")),
    });
    for (index, matched) in later.iter().take(MAX_CITATION_ROWS).enumerate() {
        let label = matched
            .file_name
            .clone()
            .or_else(|| Some(format!("later release #{}", index + 1)));
        citations.push(FindingCitation {
            portal_key: matched.portal_key.clone(),
            document_id: matched.document_id,
            character_start: None,
            character_end: None,
            quote: Some(
                matched
                    .excerpt
                    .split_whitespace()
                    .take(30)
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
            label: label.map(|l| {
                format!(
                    "{l} (request {}, {} days later)",
                    matched.request_id, matched.days_later
                )
            }),
        });
    }
    let now = Utc::now();
    Some(Finding {
        id,
        kind: FindingKind::EvasionLateArrivingRecords,
        severity,
        confidence: FindingConfidence::measured(bp),
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
    use chrono::TimeZone;

    fn iso(y: i32, m: u32, d: u32) -> DateTime<Utc> {
        chrono::Utc
            .with_ymd_and_hms(y, m, d, 12, 0, 0)
            .single()
            .unwrap()
    }

    fn denial(request_id: i64, determination: DateTime<Utc>) -> DenialCandidate {
        DenialCandidate {
            portal_key: "test".into(),
            agency_id: 7,
            request_id,
            denial_document_id: request_id * 10,
            denial_file_name: Some("denial.pdf".into()),
            request_title: "Downtown Housing Contract 2024".into(),
            request_description: "Records regarding downtown housing contracts.".into(),
            denial_text: "After thorough search, no responsive records exist.".into(),
            determination_at: determination,
        }
    }

    fn later(request_id: i64, days: i64) -> LaterMatch {
        LaterMatch {
            portal_key: "test".into(),
            document_id: request_id * 100,
            request_id,
            file_name: Some(format!("late-{request_id}.pdf")),
            days_later: days,
            excerpt: "downtown housing contract materials responsive to a different request".into(),
        }
    }

    #[test]
    fn fires_when_many_later_matches_across_a_serious_gap() {
        let d = denial(1, iso(2025, 1, 1));
        let tokens = vec!["downtown".into(), "housing".into(), "contract".into()];
        let matches = vec![later(2, 90), later(3, 120), later(4, 200)];
        let finding =
            build_finding(&d, "no responsive records exist", &tokens, &matches).expect("must fire");
        assert!(matches!(
            finding.severity,
            FindingSeverity::Serious | FindingSeverity::Critical
        ));
        assert!(finding.headline.contains("preceded"));
    }

    #[test]
    fn suppressed_when_matches_below_minimum() {
        let d = denial(1, iso(2025, 1, 1));
        let tokens = vec!["downtown".into(), "housing".into()];
        let matches = vec![later(2, 60)];
        assert!(build_finding(&d, "no responsive records exist", &tokens, &matches).is_none());
    }

    #[test]
    fn severity_scales_with_max_gap() {
        let d = denial(1, iso(2025, 1, 1));
        let tokens = vec!["downtown".into(), "housing".into()];
        let mid = build_finding(
            &d,
            "no responsive records exist",
            &tokens,
            &[later(2, 30), later(3, 40)],
        )
        .expect("mid fires");
        assert_eq!(mid.severity, FindingSeverity::Notable);
        let hi = build_finding(
            &d,
            "no responsive records exist",
            &tokens,
            &[later(2, 200), later(3, 250)],
        )
        .expect("serious fires");
        assert_eq!(hi.severity, FindingSeverity::Serious);
        let cr = build_finding(
            &d,
            "no responsive records exist",
            &tokens,
            &[later(2, 30), later(3, 400)],
        )
        .expect("critical fires");
        assert_eq!(cr.severity, FindingSeverity::Critical);
    }

    #[test]
    fn confidence_scales_with_match_count_and_gap_width() {
        let d = denial(1, iso(2025, 1, 1));
        let tokens = vec!["downtown".into(), "housing".into()];
        let small = build_finding(
            &d,
            "no responsive records exist",
            &tokens,
            &[later(2, 30), later(3, 40)],
        )
        .expect("small fires");
        let large = build_finding(
            &d,
            "no responsive records exist",
            &tokens,
            &[later(2, 300), later(3, 350), later(4, 400), later(5, 450)],
        )
        .expect("large fires");
        assert!(large.confidence.rank_weight() > small.confidence.rank_weight());
    }

    #[test]
    fn id_stable_for_same_evidence() {
        let d = denial(1, iso(2025, 1, 1));
        let tokens = vec!["downtown".into(), "housing".into()];
        let matches = vec![later(2, 90), later(3, 120)];
        let a = build_finding(&d, "no responsive records exist", &tokens, &matches).unwrap();
        let b = build_finding(&d, "no responsive records exist", &tokens, &matches).unwrap();
        assert_eq!(a.id, b.id);
    }
}
