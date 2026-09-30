//! Cross-corpus reused-denial-grounds detector.
//!
//! Wave 2B's `extension_abuse` catches an agency reaching for the same statutory extension
//! reason on the majority of its extended requests. This detector catches the parallel pattern
//! for the extension-reasons across the corpus's *statutory_extensions* table, gated on
//! *topical diversity* of the extended requests: an agency invoking a single reason on 90% of
//! its extensions is only surprising if those extensions cover a wide range of request topics.
//! When they do, the pattern is consistent with the ground being used as a catch-all rather
//! than earned per-request.
//!
//! **Difference vs. extension_abuse**: extension_abuse fires on concentration alone. This
//! detector adds the topical-entropy gate. A high-entropy set of topics + one dominant ground
//! is a stronger signal for pretext than concentration alone.
//!
//! **What this is not**: a legal-claim finding. A single reason used across diverse topics is
//! a pattern that reads as pretext; whether any specific extension was unlawful requires
//! per-request statutory analysis.

use crate::{
    analysis::assertion_index::extract_topic_tokens,
    analysis::finding::{
        finding_id, Finding, FindingCitation, FindingConfidence, FindingKind, FindingSeverity,
        FindingSubject,
    },
    error::AppError,
    storage::DuckDbRepository,
};
use chrono::Utc;
use duckdb::params;
use std::collections::{HashMap, HashSet};

pub const SLUG: &str = "cross_corpus_reused_denial_grounds";

/// Minimum extensions for a (agency, reason) pair to be considered.
const MIN_EXTENSIONS: i64 = 20;

/// Minimum share of an agency's extensions on one reason for the pattern to fire.
const REASON_SHARE_FLOOR: f64 = 0.60;

/// Minimum topical-entropy (Shannon, base 2) across the request titles/descriptions of the
/// extended requests. Zero = single topic; log2(N) = perfectly diverse. This bar rejects
/// specialization (one program area that legitimately hits the same ground repeatedly) and
/// keeps the catch-all pattern.
const MIN_TOPIC_ENTROPY_BITS: f64 = 1.5;

/// Minimum distinct topic tokens across the extended requests, as a floor beneath the entropy
/// gate. Even a high entropy on a 3-topic set is not sufficient by itself.
const MIN_DISTINCT_TOPIC_TOKENS: usize = 5;

/// Findings cap per run.
const MAX_FINDINGS: usize = 50;

/// Citations per finding.
const CITATIONS_PER_FINDING: i64 = 5;

#[derive(Debug, Clone)]
struct ExtensionSummary {
    portal_key: String,
    agency_id: i64,
    agency_name: Option<String>,
    reason: String,
    reason_count: i64,
    agency_total: i64,
}

pub async fn run(repository: &DuckDbRepository) -> Result<Vec<Finding>, AppError> {
    let summaries = load_summaries(repository).await?;
    let mut findings = evaluate(repository, &summaries).await?;
    findings.sort_by_key(|f| std::cmp::Reverse(f.confidence.rank_weight()));
    findings.truncate(MAX_FINDINGS);
    Ok(findings)
}

async fn load_summaries(repository: &DuckDbRepository) -> Result<Vec<ExtensionSummary>, AppError> {
    repository
        .query_connection(
            "Cross-corpus reused-denial-grounds aggregation",
            move |conn| {
                // Same shape as extension_abuse but collects the per-reason request texts so the
                // topic-entropy gate has something to score.
                let mut statement = conn
                    .prepare(
                        "WITH per_reason AS (
                             SELECT e.portal_subdomain,
                                    r.agency_id,
                                    e.reason,
                                    COUNT(*) AS reason_count,
                                    SUM(COUNT(*)) OVER (
                                        PARTITION BY e.portal_subdomain, r.agency_id
                                    ) AS agency_total
                             FROM statutory_extensions e
                             JOIN requests r
                                ON r.id = e.request_id
                               AND r.portal_subdomain = e.portal_subdomain
                             WHERE r.agency_id IS NOT NULL
                             GROUP BY e.portal_subdomain, r.agency_id, e.reason
                         )
                         SELECT p.portal_subdomain, p.agency_id, a.name,
                                p.reason, p.reason_count, p.agency_total
                         FROM per_reason p
                         LEFT JOIN agencies a
                            ON a.id = p.agency_id
                           AND a.portal_subdomain = p.portal_subdomain",
                    )
                    .map_err(|error| {
                        AppError::Storage(format!(
                            "Failed to prepare reused-denial-grounds aggregation: {error}"
                        ))
                    })?;
                let rows = statement
                    .query_map([], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, Option<String>>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, i64>(4)?,
                            row.get::<_, i64>(5)?,
                        ))
                    })
                    .map_err(|error| {
                        AppError::Storage(format!(
                            "Failed to run reused-denial-grounds aggregation: {error}"
                        ))
                    })?;
                let mut out = Vec::new();
                for row in rows {
                    let (portal_key, agency_id, agency_name, reason, reason_count, agency_total) =
                        row.map_err(|error| {
                            AppError::Storage(format!(
                                "Failed to decode reused-denial-grounds row: {error}"
                            ))
                        })?;
                    out.push(ExtensionSummary {
                        portal_key,
                        agency_id,
                        agency_name,
                        reason,
                        reason_count,
                        agency_total,
                    });
                }
                Ok(out)
            },
        )
        .await
}

async fn evaluate(
    repository: &DuckDbRepository,
    summaries: &[ExtensionSummary],
) -> Result<Vec<Finding>, AppError> {
    let mut findings = Vec::new();

    // Bucket to walk each (agency, reason) with numeric gate first, THEN pull the extension's
    // request texts for the topic-entropy scan.
    for summary in summaries {
        if summary.reason_count < MIN_EXTENSIONS {
            continue;
        }
        if summary.agency_total <= 0 {
            continue;
        }
        let share = summary.reason_count as f64 / summary.agency_total as f64;
        if share < REASON_SHARE_FLOOR {
            continue;
        }
        // Pull per-extension request title + description for topic-entropy computation.
        let contexts = load_request_contexts(repository, summary).await?;
        if contexts.is_empty() {
            continue;
        }

        // Extract topic tokens per request, aggregate to a token-frequency map.
        let mut token_counts: HashMap<String, usize> = HashMap::new();
        for ctx in &contexts {
            let tokens = extract_topic_tokens(&ctx.title, &ctx.description);
            let mut seen = HashSet::new();
            for token in tokens {
                if seen.insert(token.clone()) {
                    *token_counts.entry(token).or_insert(0) += 1;
                }
            }
        }
        if token_counts.len() < MIN_DISTINCT_TOPIC_TOKENS {
            continue;
        }
        let entropy = shannon_entropy_bits(&token_counts);
        if entropy < MIN_TOPIC_ENTROPY_BITS {
            continue;
        }

        // Passed all gates. Build finding.
        findings.push(build_finding(summary, contexts, entropy, share));
    }

    Ok(findings)
}

#[derive(Debug, Clone)]
struct RequestContext {
    request_id: i64,
    title: String,
    description: String,
    notice_at: String,
    document_id: Option<i64>,
}

async fn load_request_contexts(
    repository: &DuckDbRepository,
    summary: &ExtensionSummary,
) -> Result<Vec<RequestContext>, AppError> {
    let portal = summary.portal_key.clone();
    let agency = summary.agency_id;
    let reason = summary.reason.clone();
    repository
        .query_connection("Reused-denial-grounds request contexts", move |conn| {
            let mut statement = conn
                .prepare(
                    "SELECT e.request_id, r.title, r.description, e.notice_at,
                            (SELECT d.id FROM documents d
                              WHERE d.request_id = e.request_id
                                AND d.portal_subdomain = e.portal_subdomain
                                AND LOWER(TRIM(d.visibility)) = 'public'
                              ORDER BY d.id ASC LIMIT 1) AS doc_id
                     FROM statutory_extensions e
                     JOIN requests r
                       ON r.id = e.request_id
                      AND r.portal_subdomain = e.portal_subdomain
                     WHERE e.portal_subdomain = ?1
                       AND r.agency_id = ?2
                       AND e.reason = ?3
                     ORDER BY e.notice_at DESC",
                )
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to prepare reused-denial-grounds context query: {error}"
                    ))
                })?;
            let rows = statement
                .query_map(params![portal, agency, reason], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, Option<i64>>(4)?,
                    ))
                })
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to run reused-denial-grounds context query: {error}"
                    ))
                })?;
            let mut out = Vec::new();
            for row in rows {
                let (request_id, title, description, notice_at, doc_id) = row.map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to decode reused-denial-grounds context row: {error}"
                    ))
                })?;
                out.push(RequestContext {
                    request_id,
                    title: title.unwrap_or_default(),
                    description: description.unwrap_or_default(),
                    notice_at,
                    document_id: doc_id,
                });
            }
            Ok(out)
        })
        .await
}

fn shannon_entropy_bits(counts: &HashMap<String, usize>) -> f64 {
    let total: usize = counts.values().sum();
    if total == 0 {
        return 0.0;
    }
    let total_f = total as f64;
    let mut h = 0.0;
    for &count in counts.values() {
        if count == 0 {
            continue;
        }
        let p = count as f64 / total_f;
        h -= p * p.log2();
    }
    h
}

fn build_finding(
    summary: &ExtensionSummary,
    contexts: Vec<RequestContext>,
    entropy: f64,
    share: f64,
) -> Finding {
    let share_pct = (share * 100.0).round() as u32;
    let severity = if share >= 0.80 && entropy >= 2.5 {
        FindingSeverity::Serious
    } else {
        FindingSeverity::Notable
    };

    // Confidence: (share above floor) + (extension count) + (topic entropy).
    let share_component =
        ((share - REASON_SHARE_FLOOR) / (1.0 - REASON_SHARE_FLOOR)).clamp(0.0, 1.0) * 4_000.0;
    let count_component = ((summary.reason_count as f64 / 100.0).min(1.0)) * 3_000.0;
    let entropy_component = (entropy / 4.0).min(1.0) * 3_000.0;
    let bp = (share_component + count_component + entropy_component).round() as u16;
    let confidence = FindingConfidence::measured(bp);

    let subject = FindingSubject {
        portal_key: Some(summary.portal_key.clone()),
        agency_id: Some(summary.agency_id),
        ..Default::default()
    };

    let salient_reason = format!("reason={}", summary.reason);
    let salient_share = format!("share_bucket={}", share_pct / 10);
    let id = finding_id(
        FindingKind::CrossCorpusReusedDenialGrounds,
        SLUG,
        &subject,
        &[&salient_reason, &salient_share],
    );

    let agency_name = summary
        .agency_name
        .clone()
        .unwrap_or_else(|| format!("agency {}", summary.agency_id));

    let headline = format!(
        "{share_pct}% of {agency_name}'s extensions cite \"{reason}\" across {entropy:.1}-bit topical entropy",
        reason = summary.reason,
    );
    let explanation = format!(
        "{count} of {agency_name}'s {agency_total} statutory extensions cite \"{reason}\" \
         as the ground for the delay. The extended requests span {entropy:.2} bits of \
         topical entropy across {token_variety}+ distinct topic tokens — a wide range of \
         subject matter. A single ground appearing across topically diverse requests reads as \
         a catch-all rather than a per-request statutory analysis.",
        count = summary.reason_count,
        agency_total = summary.agency_total,
        reason = summary.reason,
        token_variety = MIN_DISTINCT_TOPIC_TOKENS,
    );

    let mut citations = Vec::new();
    for ctx in contexts
        .into_iter()
        .take(CITATIONS_PER_FINDING.max(0) as usize)
    {
        let short_notice = ctx
            .notice_at
            .split_once('T')
            .map(|(day, _)| day.to_string())
            .unwrap_or_else(|| ctx.notice_at.clone());
        let title_or_fallback = if ctx.title.is_empty() {
            format!("request #{}", ctx.request_id)
        } else {
            ctx.title.clone()
        };
        citations.push(FindingCitation {
            portal_key: summary.portal_key.clone(),
            document_id: ctx.document_id.unwrap_or(0),
            character_start: None,
            character_end: None,
            quote: Some(title_or_fallback.clone()),
            label: Some(format!(
                "request #{} extended on {}",
                ctx.request_id, short_notice
            )),
        });
    }

    let now = Utc::now();
    Finding {
        id,
        kind: FindingKind::CrossCorpusReusedDenialGrounds,
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(id: i64, title: &str, notice: &str) -> RequestContext {
        RequestContext {
            request_id: id,
            title: title.into(),
            description: String::new(),
            notice_at: notice.into(),
            document_id: Some(id),
        }
    }

    #[test]
    fn shannon_entropy_is_zero_for_single_topic() {
        let mut counts = HashMap::new();
        counts.insert("housing".into(), 30);
        assert_eq!(shannon_entropy_bits(&counts), 0.0);
    }

    #[test]
    fn shannon_entropy_rises_with_diverse_topics() {
        let mut counts = HashMap::new();
        for token in [
            "housing",
            "budget",
            "elections",
            "policing",
            "contracts",
            "planning",
            "personnel",
            "grants",
        ] {
            counts.insert(token.to_string(), 10);
        }
        let h = shannon_entropy_bits(&counts);
        assert!(h > 2.5, "8 balanced tokens should give ~3 bits, got {h}");
    }

    #[test]
    fn build_finding_marks_serious_on_high_share_and_high_entropy() {
        let summary = ExtensionSummary {
            portal_key: "city-a".into(),
            agency_id: 1,
            agency_name: Some("Records Bureau".into()),
            reason: "VoluminousRecords".into(),
            reason_count: 90,
            agency_total: 100,
        };
        let contexts = vec![
            ctx(1, "Housing complaints", "2026-01-01T00:00:00Z"),
            ctx(2, "Budget overruns", "2026-01-02T00:00:00Z"),
            ctx(3, "Election ballots", "2026-01-03T00:00:00Z"),
            ctx(4, "Policing incident logs", "2026-01-04T00:00:00Z"),
            ctx(5, "Contract awards", "2026-01-05T00:00:00Z"),
        ];
        let finding = build_finding(&summary, contexts, 3.0, 0.90);
        assert_eq!(finding.kind, FindingKind::CrossCorpusReusedDenialGrounds);
        assert_eq!(finding.severity, FindingSeverity::Serious);
        assert!(finding.headline.contains("VoluminousRecords"));
    }

    #[test]
    fn build_finding_is_notable_when_share_or_entropy_is_moderate() {
        let summary = ExtensionSummary {
            portal_key: "city-a".into(),
            agency_id: 2,
            agency_name: Some("Public Works".into()),
            reason: "ConsultOtherAgency".into(),
            reason_count: 40,
            agency_total: 60,
        };
        let contexts = vec![
            ctx(10, "Traffic study", "2026-01-01T00:00:00Z"),
            ctx(11, "Water infrastructure", "2026-01-02T00:00:00Z"),
        ];
        let finding = build_finding(&summary, contexts, 1.8, 0.67);
        assert_eq!(finding.severity, FindingSeverity::Notable);
    }

    #[test]
    fn id_is_stable_across_reruns() {
        let summary = ExtensionSummary {
            portal_key: "city-a".into(),
            agency_id: 3,
            agency_name: None,
            reason: "OtherOffices".into(),
            reason_count: 50,
            agency_total: 60,
        };
        let contexts = vec![ctx(1, "topic", "2026-01-01T00:00:00Z")];
        let a = build_finding(&summary, contexts.clone(), 2.0, 0.83);
        let b = build_finding(&summary, contexts, 2.0, 0.83);
        assert_eq!(a.id, b.id);
    }
}
