//! Cross-corpus shared-language detector.
//!
//! Wave 2B's `boilerplate_denial` catches near-identical denial paragraphs within a single
//! agency. This detector catches the same paragraph appearing across MULTIPLE agencies — the
//! signature of a shared template, coordinated legal guidance, or a copy-paste circulating
//! between jurisdictions. That's a policy signal a single-agency detector cannot see.
//!
//! **Gate**: ≥ 3 distinct agencies + ≥ 5 total occurrences + Jaccard ≥ 0.85 on 5-word
//! shingles. The similarity threshold is intentionally higher than `boilerplate_denial`'s
//! 0.60: a cross-agency match at 0.60 could easily be two agencies independently reaching for
//! the same statutory boilerplate; requiring 0.85 raises the bar to "the same paragraph, not
//! just the same idea."
//!
//! **What this is not**: a legal-claim finding. Two agencies sharing wording is a pattern
//! signal about coordination or template reuse, not a violation of any stated rule.

use crate::{
    analysis::finding::{
        finding_id, Finding, FindingCitation, FindingConfidence, FindingKind, FindingSeverity,
        FindingSubject,
    },
    error::AppError,
    storage::DuckDbRepository,
};
use chrono::Utc;
use duckdb::params;
use std::collections::{BTreeMap, HashSet};

pub const SLUG: &str = "cross_corpus_shared_language";

/// Cap on candidate documents pulled per invocation. Bounded so a huge corpus keeps refresh
/// responsive; ordered newest-first so the freshest evidence surfaces even when the cap is hit.
const MAX_CANDIDATES: i64 = 2_000;

/// Minimum characters in a candidate excerpt. Below this the shingle overlap becomes spurious.
const MIN_EXCERPT_CHARS: usize = 120;

/// Shingle size in words. Five is standard for near-duplicate detection.
const SHINGLE_WORDS: usize = 5;

/// Jaccard threshold. Higher than the intra-agency detector — a cross-jurisdictional match at
/// this rung is very unlikely to be independent.
const JACCARD_THRESHOLD: f64 = 0.85;

/// Minimum distinct agencies for a cluster to fire.
const MIN_DISTINCT_AGENCIES: usize = 3;

/// Minimum total documents for a cluster to fire.
const MIN_TOTAL_OCCURRENCES: usize = 5;

/// Cap findings emitted per run.
const MAX_FINDINGS: usize = 50;

struct Candidate {
    document_id: i64,
    portal_key: String,
    agency_id: i64,
    request_title: Option<String>,
    file_name: Option<String>,
    excerpt: String,
    shingles: HashSet<u64>,
}

struct Cluster {
    members: Vec<usize>,
    agency_ids: HashSet<i64>,
    portal_keys: HashSet<String>,
    average_similarity: f64,
}

/// Language signals we look for. Same as boilerplate_denial's, plus a few "we hereby determine"
/// stock templates that show up in exemption-based responses.
const TEMPLATE_PHRASES: &[&str] = &[
    "no responsive records",
    "no records exist",
    "no records were located",
    "no records that respond",
    "no records available",
    "cannot locate any records",
    "unable to locate records",
    "does not maintain records",
    "not readily available",
    "records are exempt from disclosure",
    "does not possess records",
    "no documents responsive",
    "we have determined",
    "after diligent search",
    "the department has reviewed",
    "pursuant to government code",
];

pub async fn run(repository: &DuckDbRepository) -> Result<Vec<Finding>, AppError> {
    let candidates = load_candidates(repository).await?;
    if candidates.len() < MIN_TOTAL_OCCURRENCES {
        return Ok(Vec::new());
    }
    let clusters = cluster_candidates(&candidates);
    let mut findings = build_findings(&candidates, clusters);
    findings.sort_by_key(|f| std::cmp::Reverse(f.confidence.rank_weight()));
    findings.truncate(MAX_FINDINGS);
    Ok(findings)
}

async fn load_candidates(repository: &DuckDbRepository) -> Result<Vec<Candidate>, AppError> {
    repository
        .query_connection(
            "Cross-corpus shared-language candidate query",
            move |conn| {
                // Only public documents where agency identity is known; a cross-agency detector
                // cannot count agencies it can't identify.
                let mut statement = conn
                    .prepare(
                        "SELECT d.id, d.portal_subdomain, r.agency_id,
                            r.title, d.file_name, fts.extracted_text
                     FROM documents d
                     JOIN documents_fts fts
                        ON fts.document_id = d.id
                       AND fts.portal_subdomain = d.portal_subdomain
                     JOIN requests r
                        ON r.id = d.request_id
                       AND r.portal_subdomain = d.portal_subdomain
                     WHERE LOWER(TRIM(d.visibility)) = 'public'
                       AND fts.extracted_text IS NOT NULL
                       AND LENGTH(fts.extracted_text) >= ?1
                       AND r.agency_id IS NOT NULL
                     ORDER BY d.id DESC
                     LIMIT ?2",
                    )
                    .map_err(|error| {
                        AppError::Storage(format!(
                            "Failed to prepare cross-corpus shared-language query: {error}"
                        ))
                    })?;
                let rows = statement
                    .query_map(params![MIN_EXCERPT_CHARS as i64, MAX_CANDIDATES], |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, i64>(2)?,
                            row.get::<_, Option<String>>(3)?,
                            row.get::<_, Option<String>>(4)?,
                            row.get::<_, String>(5)?,
                        ))
                    })
                    .map_err(|error| {
                        AppError::Storage(format!(
                            "Failed to run cross-corpus shared-language query: {error}"
                        ))
                    })?;
                let mut out = Vec::new();
                for row in rows {
                    let (
                        document_id,
                        portal_key,
                        agency_id,
                        request_title,
                        file_name,
                        extracted_text,
                    ) = row.map_err(|error| {
                        AppError::Storage(format!("Failed to decode candidate row: {error}"))
                    })?;
                    let Some(excerpt) = extract_template_excerpt(&extracted_text) else {
                        continue;
                    };
                    let normalized = normalize(&excerpt);
                    if normalized.split_whitespace().count() < SHINGLE_WORDS {
                        continue;
                    }
                    let shingles = shingle(&normalized, SHINGLE_WORDS);
                    if shingles.is_empty() {
                        continue;
                    }
                    out.push(Candidate {
                        document_id,
                        portal_key,
                        agency_id,
                        request_title,
                        file_name,
                        excerpt,
                        shingles,
                    });
                }
                Ok(out)
            },
        )
        .await
}

/// Finds a bounded window around any of the template phrases. Same shape as
/// boilerplate_denial's excerpt extraction, deliberately consistent so both detectors observe
/// the same text units.
fn extract_template_excerpt(text: &str) -> Option<String> {
    let haystack = text.to_ascii_lowercase();
    let matched_at = TEMPLATE_PHRASES
        .iter()
        .filter_map(|phrase| haystack.find(phrase))
        .min()?;
    let start = matched_at.saturating_sub(300);
    let end = matched_at.saturating_add(300).min(text.len());
    let start = text[..start]
        .rfind(char::is_whitespace)
        .map_or(0, |i| i + 1);
    let end = text[end..]
        .find(char::is_whitespace)
        .map_or(text.len(), |i| end + i);
    let excerpt = text[start..end].trim().to_string();
    if excerpt.len() < MIN_EXCERPT_CHARS {
        return None;
    }
    Some(excerpt)
}

/// Word-normalize identically to boilerplate_denial so an excerpt appearing in both detectors'
/// candidate sets produces the same shingles.
fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut prev_was_space = true;
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            for lower in ch.to_lowercase() {
                out.push(lower);
            }
            prev_was_space = false;
        } else if (ch.is_whitespace() || ch.is_ascii_punctuation()) && !prev_was_space {
            out.push(' ');
            prev_was_space = true;
        }
    }
    out.trim().to_string()
}

fn shingle(normalized: &str, n: usize) -> HashSet<u64> {
    let words: Vec<&str> = normalized.split_whitespace().collect();
    if words.len() < n {
        return HashSet::new();
    }
    let mut set = HashSet::with_capacity(words.len().saturating_sub(n).saturating_add(1));
    for window in words.windows(n) {
        let mut hasher: u64 = 0xcbf2_9ce4_8422_2325;
        for word in window {
            for byte in word.as_bytes() {
                hasher ^= *byte as u64;
                hasher = hasher.wrapping_mul(0x0000_0100_0000_01B3);
            }
            hasher ^= 0x20;
            hasher = hasher.wrapping_mul(0x0000_0100_0000_01B3);
        }
        set.insert(hasher);
    }
    set
}

fn jaccard(a: &HashSet<u64>, b: &HashSet<u64>) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let intersection = a.intersection(b).count();
    let union = a.len() + b.len() - intersection;
    if union == 0 {
        return 0.0;
    }
    intersection as f64 / union as f64
}

/// Greedy single-link clustering seeded on the tighter 0.85 threshold. O(n²) in worst case,
/// bounded by MAX_CANDIDATES.
fn cluster_candidates(candidates: &[Candidate]) -> Vec<Cluster> {
    let mut clusters = Vec::new();
    let mut assigned = vec![false; candidates.len()];
    for i in 0..candidates.len() {
        if assigned[i] {
            continue;
        }
        let mut members = vec![i];
        let mut similarity_total = 0.0;
        let mut pair_count = 0.0;
        assigned[i] = true;
        for j in (i + 1)..candidates.len() {
            if assigned[j] {
                continue;
            }
            let sim = jaccard(&candidates[i].shingles, &candidates[j].shingles);
            if sim >= JACCARD_THRESHOLD {
                assigned[j] = true;
                members.push(j);
                similarity_total += sim;
                pair_count += 1.0;
            }
        }
        if members.len() < MIN_TOTAL_OCCURRENCES {
            continue;
        }
        let mut agency_ids = HashSet::new();
        let mut portal_keys = HashSet::new();
        for &member in &members {
            agency_ids.insert(candidates[member].agency_id);
            portal_keys.insert(candidates[member].portal_key.clone());
        }
        if agency_ids.len() < MIN_DISTINCT_AGENCIES {
            continue;
        }
        let average = if pair_count > 0.0 {
            similarity_total / pair_count
        } else {
            1.0
        };
        clusters.push(Cluster {
            members,
            agency_ids,
            portal_keys,
            average_similarity: average,
        });
    }
    clusters
}

fn build_findings(candidates: &[Candidate], clusters: Vec<Cluster>) -> Vec<Finding> {
    let mut findings = Vec::new();
    let now = Utc::now();
    for cluster in clusters {
        // Stable exemplar: lowest document_id in the cluster.
        let exemplar_idx = cluster
            .members
            .iter()
            .copied()
            .min_by_key(|&idx| candidates[idx].document_id)
            .expect("cluster has ≥ MIN_TOTAL_OCCURRENCES members");
        let exemplar = &candidates[exemplar_idx];
        let cluster_size = cluster.members.len();
        let distinct_agencies = cluster.agency_ids.len();
        let distinct_portals = cluster.portal_keys.len();

        // Severity: crossing portal boundaries (different jurisdictions) is a stronger signal
        // than crossing only agency boundaries within one portal.
        let severity = if distinct_portals > 1 {
            FindingSeverity::Serious
        } else {
            FindingSeverity::Notable
        };

        // Confidence: scaled by similarity above the (tighter) threshold, agency count, and
        // cluster size. Weights sum to 10000.
        let similarity_component = ((cluster.average_similarity - JACCARD_THRESHOLD)
            / (1.0 - JACCARD_THRESHOLD))
            .clamp(0.0, 1.0)
            * 4_000.0;
        let agency_component = ((distinct_agencies as f64 / 8.0).min(1.0)) * 3_000.0;
        let size_component = ((cluster_size as f64 / 20.0).min(1.0)) * 3_000.0;
        let bp = (similarity_component + agency_component + size_component).round() as u16;
        let confidence = FindingConfidence::measured(bp);

        // Subject anchor: portal-only when multi-portal, else portal + document.
        let subject = if distinct_portals > 1 {
            FindingSubject {
                document_id: Some(exemplar.document_id),
                ..Default::default()
            }
        } else {
            FindingSubject {
                portal_key: Some(exemplar.portal_key.clone()),
                document_id: Some(exemplar.document_id),
                ..Default::default()
            }
        };

        let size_bucket = format!("size_bucket={}", cluster_size / 5);
        let agency_bucket = format!("agencies_bucket={}", distinct_agencies);
        let portal_bucket = format!("portals_bucket={}", distinct_portals);
        let id = finding_id(
            FindingKind::CrossCorpusSharedLanguage,
            SLUG,
            &subject,
            &[&size_bucket, &agency_bucket, &portal_bucket],
        );

        let headline = if distinct_portals > 1 {
            format!(
                "Identical wording appears across {distinct_agencies} agencies in {distinct_portals} jurisdictions",
            )
        } else {
            format!("Identical wording appears across {distinct_agencies} agencies")
        };

        let similarity_pct = (cluster.average_similarity * 100.0).round() as u32;
        let explanation = format!(
            "{cluster_size} response passages share near-identical wording (average \
             {similarity_pct}% shingle overlap) across {distinct_agencies} distinct agencies\
             {portal_hint}. Independent per-agency drafting would be expected to produce distinct \
             language; this cluster is consistent with a shared template, coordinated legal \
             guidance, or a copy-paste circulating between offices.",
            portal_hint = if distinct_portals > 1 {
                format!(" and {distinct_portals} jurisdictions")
            } else {
                String::new()
            },
        );

        let mut citations = Vec::new();
        for &member in cluster.members.iter().take(5) {
            let candidate = &candidates[member];
            let label = candidate
                .file_name
                .clone()
                .or_else(|| candidate.request_title.clone())
                .unwrap_or_else(|| format!("document {}", candidate.document_id));
            let quote_preview = candidate
                .excerpt
                .split_whitespace()
                .take(24)
                .collect::<Vec<_>>()
                .join(" ");
            citations.push(FindingCitation {
                portal_key: candidate.portal_key.clone(),
                document_id: candidate.document_id,
                character_start: None,
                character_end: None,
                quote: Some(quote_preview),
                label: Some(label),
            });
        }
        debug_assert!(
            !citations.is_empty(),
            "cluster guaranteed to have ≥ 1 member"
        );

        findings.push(Finding {
            id,
            kind: FindingKind::CrossCorpusSharedLanguage,
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
        });
    }
    // Deterministic order; dedupe on id via a BTreeMap.
    let mut seen = BTreeMap::new();
    for finding in findings {
        seen.entry(finding.id.clone()).or_insert(finding);
    }
    seen.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(id: i64, portal: &str, agency: i64, title: &str, excerpt: &str) -> Candidate {
        let normalized = normalize(excerpt);
        let shingles = shingle(&normalized, SHINGLE_WORDS);
        Candidate {
            document_id: id,
            portal_key: portal.into(),
            agency_id: agency,
            request_title: Some(title.into()),
            file_name: Some(format!("doc-{id}.pdf")),
            excerpt: excerpt.into(),
            shingles,
        }
    }

    fn stock() -> String {
        "The department has reviewed the request and has determined that no responsive records \
         exist within its custody at this time pursuant to government code section 6250"
            .into()
    }

    #[test]
    fn fires_when_three_agencies_share_language_at_high_similarity() {
        // 6 documents, 3 distinct agencies (2, 3, 4) across two portals (city-a, city-b).
        let candidates: Vec<Candidate> = (0..6)
            .map(|i| {
                cand(
                    i,
                    if i < 3 { "city-a" } else { "city-b" },
                    2 + (i % 3),
                    &format!("topic-{i}"),
                    &stock(),
                )
            })
            .collect();
        let clusters = cluster_candidates(&candidates);
        assert_eq!(clusters.len(), 1);
        let findings = build_findings(&candidates, clusters);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].kind, FindingKind::CrossCorpusSharedLanguage);
        // Multi-portal => Serious.
        assert_eq!(findings[0].severity, FindingSeverity::Serious);
    }

    #[test]
    fn stays_silent_below_agency_threshold() {
        // 5 documents but only 2 distinct agencies. Below MIN_DISTINCT_AGENCIES.
        let candidates: Vec<Candidate> = (0..5)
            .map(|i| {
                cand(
                    i,
                    "city-a",
                    if i < 2 { 1 } else { 2 },
                    &format!("topic-{i}"),
                    &stock(),
                )
            })
            .collect();
        let clusters = cluster_candidates(&candidates);
        assert!(
            clusters.is_empty(),
            "cannot fire cross-agency below MIN_DISTINCT_AGENCIES"
        );
    }

    #[test]
    fn stays_silent_below_count_threshold() {
        // 3 agencies but only 4 documents. Below MIN_TOTAL_OCCURRENCES.
        let candidates: Vec<Candidate> = (0..4)
            .map(|i| cand(i, "city-a", i + 1, &format!("topic-{i}"), &stock()))
            .collect();
        let clusters = cluster_candidates(&candidates);
        assert!(clusters.is_empty());
    }

    #[test]
    fn does_not_conflate_dissimilar_wording() {
        // Two agencies with the boilerplate, three agencies with very different sentences.
        let mut candidates: Vec<Candidate> = (0..2)
            .map(|i| cand(i, "city-a", i + 1, "same", &stock()))
            .collect();
        candidates.push(cand(2, "city-a", 3, "diff", "The public records at issue involve extensive personnel matters and financial disclosures relevant to municipal governance."));
        candidates.push(cand(3, "city-b", 4, "diff", "Please review the attached response letter for details on responsive records provided under separate cover."));
        candidates.push(cand(4, "city-c", 5, "diff", "This response addresses the specific questions raised regarding fiscal-year budget reporting for the requesting party."));
        let clusters = cluster_candidates(&candidates);
        assert!(
            clusters.is_empty(),
            "no cluster should form when only 2 documents share language"
        );
    }

    #[test]
    fn multi_portal_earns_serious_severity_and_stable_id() {
        let candidates: Vec<Candidate> = (0..5)
            .map(|i| {
                cand(
                    i,
                    if i < 2 {
                        "city-a"
                    } else if i < 4 {
                        "city-b"
                    } else {
                        "city-c"
                    },
                    i + 1,
                    &format!("t-{i}"),
                    &stock(),
                )
            })
            .collect();
        let findings_a = {
            let c = cluster_candidates(&candidates);
            build_findings(&candidates, c)
        };
        let findings_b = {
            let c = cluster_candidates(&candidates);
            build_findings(&candidates, c)
        };
        assert_eq!(findings_a.len(), 1);
        assert_eq!(findings_a[0].severity, FindingSeverity::Serious);
        assert_eq!(findings_a[0].id, findings_b[0].id, "id must be stable");
    }
}
