//! Cross-corpus orphan-topic detector.
//!
//! For a given portal (jurisdiction), when a topic — a distinctive token drawn from request
//! titles/descriptions — appears prominently in Agency A's records but is entirely absent from
//! Agency B's records despite Agency B having similar or overlapping scope, that asymmetry is
//! worth surfacing.
//!
//! **Prominent** means: ≥ 40% of A's requests reference the token in title/description AND
//! ≥ MIN_A_REQUESTS_FOR_TOPIC requests do so.
//!
//! **Overlapping scope** is approximated by "Agency B has ≥ MIN_B_REQUESTS_FOR_SCOPE requests
//! total in this portal." A tighter scope model would require a taxonomy the corpus doesn't
//! carry today; the count gate is a conservative substitute.
//!
//! **What this is not**: a legal-claim finding. Orphan topics may reflect legitimate scope
//! differences (one agency actually handles the topic and the other doesn't). The signal is
//! "worth checking why," not "worth citing in a filing."

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
use std::collections::{BTreeMap, HashMap};

pub const SLUG: &str = "cross_corpus_orphan_topic";

/// Minimum requests-with-topic in agency A before we consider the topic "prominent" there.
const MIN_A_REQUESTS_FOR_TOPIC: usize = 10;

/// Fraction of A's requests that must reference the token in title/description.
const TOPIC_PROMINENCE_FRACTION: f64 = 0.40;

/// Minimum total requests in a candidate absent agency (B) before we treat its absence as
/// meaningful.
const MIN_B_REQUESTS_FOR_SCOPE: usize = 30;

/// Cap distinct topics evaluated per portal so a chatty corpus doesn't blow up runtime.
const MAX_TOPICS_PER_PORTAL: usize = 50;

/// Cap findings per run.
const MAX_FINDINGS: usize = 50;

/// One row from the request scan.
#[derive(Debug, Clone)]
struct RequestRow {
    portal_key: String,
    agency_id: i64,
    agency_name: Option<String>,
    request_id: i64,
    title: Option<String>,
    description: Option<String>,
}

pub async fn run(repository: &DuckDbRepository) -> Result<Vec<Finding>, AppError> {
    let rows = load_requests(repository).await?;
    let mut findings = evaluate(&rows);
    findings.sort_by_key(|f| std::cmp::Reverse(f.confidence.rank_weight()));
    findings.truncate(MAX_FINDINGS);
    Ok(findings)
}

async fn load_requests(repository: &DuckDbRepository) -> Result<Vec<RequestRow>, AppError> {
    repository
        .query_connection("Orphan-topic request scan", move |conn| {
            let mut statement = conn
                .prepare(
                    "SELECT r.portal_subdomain, r.agency_id, a.name, r.id, r.title, r.description
                     FROM requests r
                     LEFT JOIN agencies a
                        ON a.id = r.agency_id
                       AND a.portal_subdomain = r.portal_subdomain
                     WHERE r.agency_id IS NOT NULL",
                )
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to prepare orphan-topic request scan: {error}"
                    ))
                })?;
            let rows = statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, Option<String>>(5)?,
                    ))
                })
                .map_err(|error| {
                    AppError::Storage(format!("Failed to run orphan-topic request scan: {error}"))
                })?;
            let mut out = Vec::new();
            for row in rows {
                let (portal_key, agency_id, agency_name, request_id, title, description) = row
                    .map_err(|error| {
                        AppError::Storage(format!("Failed to decode request row: {error}"))
                    })?;
                out.push(RequestRow {
                    portal_key,
                    agency_id,
                    agency_name,
                    request_id,
                    title,
                    description,
                });
            }
            Ok(out)
        })
        .await
}

fn evaluate(rows: &[RequestRow]) -> Vec<Finding> {
    if rows.is_empty() {
        return Vec::new();
    }

    // Group by portal → agency_id → Vec<row>.
    let mut portals: BTreeMap<String, BTreeMap<i64, Vec<&RequestRow>>> = BTreeMap::new();
    for row in rows {
        portals
            .entry(row.portal_key.clone())
            .or_default()
            .entry(row.agency_id)
            .or_default()
            .push(row);
    }

    let mut findings = Vec::new();
    for (portal_key, agencies) in &portals {
        if agencies.len() < 2 {
            // Need at least two agencies in the same jurisdiction to compare.
            continue;
        }

        // Per agency, count distinct requests that reference each token.
        // token → (agency_id → count-of-requests-referencing-token)
        let mut token_counts: HashMap<String, HashMap<i64, usize>> = HashMap::new();
        let mut agency_totals: HashMap<i64, usize> = HashMap::new();
        for (agency_id, agency_rows) in agencies {
            agency_totals.insert(*agency_id, agency_rows.len());
            for row in agency_rows {
                let title = row.title.clone().unwrap_or_default();
                let description = row.description.clone().unwrap_or_default();
                let tokens = extract_topic_tokens(&title, &description);
                // De-duplicate per-row so one loud row can't inflate its own token counts.
                let mut seen_this_row = std::collections::HashSet::new();
                for token in tokens {
                    if seen_this_row.insert(token.clone()) {
                        *token_counts
                            .entry(token)
                            .or_default()
                            .entry(*agency_id)
                            .or_insert(0) += 1;
                    }
                }
            }
        }

        // For each token, find agencies where it's prominent and agencies where it's absent
        // despite scope.
        // Sort tokens by max count desc so the most prominent are evaluated first if we cap.
        let mut token_list: Vec<(&String, usize)> = token_counts
            .iter()
            .map(|(t, per)| (t, *per.values().max().unwrap_or(&0)))
            .collect();
        token_list.sort_by_key(|(_, count)| std::cmp::Reverse(*count));

        for (token, _) in token_list.into_iter().take(MAX_TOPICS_PER_PORTAL) {
            let per_agency = &token_counts[token];
            // Determine prominent A(s) for this token.
            for (a_id, a_count) in per_agency {
                if *a_count < MIN_A_REQUESTS_FOR_TOPIC {
                    continue;
                }
                let a_total = *agency_totals.get(a_id).unwrap_or(&0);
                if a_total == 0 {
                    continue;
                }
                let a_fraction = *a_count as f64 / a_total as f64;
                if a_fraction < TOPIC_PROMINENCE_FRACTION {
                    continue;
                }
                // Now find absent B(s).
                for (b_id, b_total) in &agency_totals {
                    if b_id == a_id {
                        continue;
                    }
                    if *b_total < MIN_B_REQUESTS_FOR_SCOPE {
                        continue;
                    }
                    if per_agency.get(b_id).copied().unwrap_or(0) > 0 {
                        continue;
                    }
                    // A prominent, B silent, B has scope → emit.
                    let a_rows = agencies.get(a_id).cloned().unwrap_or_default();
                    let a_name = a_rows
                        .first()
                        .and_then(|r| r.agency_name.clone())
                        .unwrap_or_else(|| format!("agency {a_id}"));
                    let b_rows = agencies.get(b_id).cloned().unwrap_or_default();
                    let b_name = b_rows
                        .first()
                        .and_then(|r| r.agency_name.clone())
                        .unwrap_or_else(|| format!("agency {b_id}"));

                    let finding = build_finding(
                        portal_key, token, *a_id, &a_name, *a_count, a_total, *b_id, &b_name,
                        *b_total, &a_rows,
                    );
                    findings.push(finding);
                }
            }
        }
    }

    findings
}

#[allow(clippy::too_many_arguments)]
fn build_finding(
    portal_key: &str,
    token: &str,
    a_id: i64,
    a_name: &str,
    a_count: usize,
    a_total: usize,
    b_id: i64,
    b_name: &str,
    b_total: usize,
    a_rows: &[&RequestRow],
) -> Finding {
    let a_fraction_pct = ((a_count as f64 / a_total as f64) * 100.0).round() as u32;
    let severity = if a_fraction_pct >= 60 && b_total >= 60 {
        FindingSeverity::Serious
    } else {
        FindingSeverity::Notable
    };

    // Confidence: (fraction above threshold) + (sample size in A) + (sample size in B). Higher
    // when the asymmetry is sharper and both agencies are well-populated.
    let fraction_component = ((a_fraction_pct as f64 - 40.0).max(0.0) / 60.0) * 4_000.0;
    let a_size_component = ((a_count as f64 / 50.0).min(1.0)) * 3_000.0;
    let b_size_component = ((b_total as f64 / 100.0).min(1.0)) * 3_000.0;
    let bp = (fraction_component + a_size_component + b_size_component).round() as u16;
    let confidence = FindingConfidence::measured(bp);

    let subject = FindingSubject {
        portal_key: Some(portal_key.to_string()),
        agency_id: Some(a_id),
        ..Default::default()
    };

    let salient_topic = format!("topic={token}");
    let salient_b = format!("silent_agency={b_id}");
    let id = finding_id(
        FindingKind::CrossCorpusOrphanTopic,
        SLUG,
        &subject,
        &[&salient_topic, &salient_b],
    );

    let headline = format!(
        "\"{token}\" surfaces in {a_fraction_pct}% of {a_name}'s requests but zero of {b_name}'s"
    );
    let explanation = format!(
        "In this jurisdiction, {a_count} of {a_name}'s {a_total} requests reference \
         \"{token}\" in the request title or description. {b_name} has {b_total} requests in \
         this portal but none mention this term. If both agencies share overlap in scope, the \
         asymmetry warrants investigation — whether it reflects a topic that only reaches one \
         office, a routing decision at intake, or records held by one that the other silently \
         does not."
    );

    // Cite up to 5 representative A-requests.
    let mut citations = Vec::new();
    for row in a_rows.iter().take(5) {
        citations.push(FindingCitation {
            portal_key: portal_key.to_string(),
            document_id: 0,
            character_start: None,
            character_end: None,
            quote: row.title.clone(),
            label: Some(format!("request #{}", row.request_id)),
        });
    }

    let now = Utc::now();
    Finding {
        id,
        kind: FindingKind::CrossCorpusOrphanTopic,
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

    fn row(portal: &str, agency: i64, id: i64, title: &str) -> RequestRow {
        RequestRow {
            portal_key: portal.into(),
            agency_id: agency,
            agency_name: Some(format!("Agency-{agency}")),
            request_id: id,
            title: Some(title.into()),
            description: None,
        }
    }

    #[test]
    fn fires_when_topic_prominent_in_a_absent_in_b_with_scope() {
        // Agency 1: 25 requests, 13 mention "housing" — 52%, above 40% threshold and 10 count.
        // Agency 2: 40 requests, 0 mention "housing" — has scope.
        let mut rows = Vec::new();
        for i in 0..13 {
            rows.push(row("city-a", 1, i + 1, "Housing complaint records"));
        }
        for i in 0..12 {
            rows.push(row("city-a", 1, 13 + i + 1, "Other topic entirely"));
        }
        for i in 0..40 {
            rows.push(row("city-a", 2, 100 + i, "Traffic incident report"));
        }
        let findings = evaluate(&rows);
        assert!(!findings.is_empty(), "must fire cross-agency orphan topic");
        let f = findings
            .iter()
            .find(|f| f.headline.to_lowercase().contains("housing"))
            .expect("housing finding present");
        assert_eq!(f.kind, FindingKind::CrossCorpusOrphanTopic);
    }

    #[test]
    fn does_not_fire_when_b_lacks_scope() {
        // Agency 2 has only 20 total requests, below MIN_B_REQUESTS_FOR_SCOPE=30.
        let mut rows = Vec::new();
        for i in 0..15 {
            rows.push(row("city-a", 1, i + 1, "Housing complaint records"));
        }
        for i in 0..20 {
            rows.push(row("city-a", 2, 100 + i, "Nothing about housing"));
        }
        let findings = evaluate(&rows);
        assert!(
            findings
                .iter()
                .all(|f| !f.headline.to_lowercase().contains("housing")),
            "must not fire when B lacks scope"
        );
    }

    #[test]
    fn does_not_fire_below_prominence_threshold() {
        // A has 100 requests, only 20 mention "housing" — 20%, below 40% threshold. Filler
        // titles are entirely unique per row (using i-embedded tokens ≥ 3 chars) so no single
        // filler token itself reaches prominence anywhere.
        let mut rows = Vec::new();
        for i in 0..20 {
            rows.push(row("city-a", 1, i + 1, "Housing complaint records"));
        }
        for i in 0..80 {
            let filler = format!("q{i:03}a q{i:03}b q{i:03}c");
            rows.push(row("city-a", 1, 20 + i + 1, &filler));
        }
        for i in 0..40 {
            let filler = format!("r{i:03}a r{i:03}b r{i:03}c");
            rows.push(row("city-a", 2, 200 + i, &filler));
        }
        let findings = evaluate(&rows);
        assert!(findings.is_empty());
    }

    #[test]
    fn skips_portals_with_only_one_agency() {
        let mut rows = Vec::new();
        for i in 0..25 {
            rows.push(row("city-solo", 1, i + 1, "Housing complaint records"));
        }
        let findings = evaluate(&rows);
        assert!(
            findings.is_empty(),
            "single-agency portal cannot yield a comparison"
        );
    }
}
