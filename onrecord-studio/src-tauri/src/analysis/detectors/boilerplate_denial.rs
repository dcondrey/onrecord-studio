//! Boilerplate-denial detector.
//!
//! Surfaces clusters of denial responses whose language is near-identical across otherwise
//! unrelated requests. An agency running 500 distinct requests through the same refusal
//! paragraph is asserting a policy, not conducting individualized review — and that fact is
//! worth surfacing without an analyst having to notice.
//!
//! **Similarity approach**: word-shingled Jaccard over normalized denial excerpts. The Wave 1
//! detector-contract exposes `run(&DuckDbRepository)` — it does not surface the `VectorIndex`,
//! and forcing a fastembed load inside a bounded refresh call would either stall the refresh
//! (first-run model download) or require plumbing the vector handle through the whole detector
//! layer just for this one detector. Shingling gives the same "these paragraphs are copy-paste"
//! answer that a cosine ≥ 0.90 in bge-small space would give, deterministically, offline, and
//! without new dependencies. If a later wave wants embedding-based recall (semantic similarity
//! across differently-worded denials that mean the same thing), it will need a new contract that
//! passes the vector handle in.
//!
//! **What this is not**: a legal finding. `FindingKind::BoilerplateDenial` is a pattern signal.
//! `is_legal_claim = false`.

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

pub const SLUG: &str = "boilerplate_denial";

/// Cap on candidate denial documents pulled per invocation. Bounded so a huge corpus keeps
/// refresh responsive.
const MAX_CANDIDATES: i64 = 2_000;

/// Minimum characters in the extracted denial excerpt. Below this, similarity scores become
/// spurious — "no records" alone will Jaccard-match itself trivially and doesn't warrant an
/// alert.
const MIN_EXCERPT_CHARS: usize = 120;

/// Shingle size in words. Five is standard for near-duplicate paragraph detection and captures
/// the kind of stock language ("The [department] has reviewed the request and has determined
/// that no responsive records exist within its custody") that this detector is built to find.
const SHINGLE_WORDS: usize = 5;

/// Jaccard-similarity threshold to consider two excerpts near-duplicates. 0.60 on 5-shingles
/// corresponds roughly to bge-small cosine ≥ 0.90 in practice for paragraphs of this length.
const JACCARD_THRESHOLD: f64 = 0.60;

/// Minimum cluster size (distinct requests) to fire a finding. Below this it's coincidence, not
/// a pattern.
const MIN_CLUSTER_SIZE: usize = 5;

/// Minimum distinct request titles across a cluster. Two identical denials on the same topic
/// are far weaker evidence than five identical denials on five distinct topics.
const MIN_DISTINCT_TOPICS: usize = 3;

/// Cap the number of findings this detector can emit per run.
const MAX_FINDINGS: usize = 50;

/// A denial excerpt pulled from documents_fts alongside its request identity.
struct DenialCandidate {
    document_id: i64,
    portal_key: String,
    agency_id: Option<i64>,
    request_title: Option<String>,
    file_name: Option<String>,
    excerpt: String,
    shingles: HashSet<u64>,
}

/// One cluster of near-identical denials with a picked exemplar.
struct DenialCluster {
    members: Vec<usize>,
    agency_ids: HashSet<i64>,
    distinct_titles: HashSet<String>,
    average_similarity: f64,
}

/// Phrases whose presence in a document's text is a strong signal that the passage is denial
/// language. Kept short and conservative — we would rather miss denials than tag a released
/// document as one.
const DENIAL_PHRASES: &[&str] = &[
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
];

pub async fn run(repository: &DuckDbRepository) -> Result<Vec<Finding>, AppError> {
    let candidates = load_candidates(repository).await?;
    if candidates.len() < MIN_CLUSTER_SIZE {
        return Ok(Vec::new());
    }
    let clusters = cluster_candidates(&candidates);
    let mut findings = build_findings(&candidates, clusters);
    // Rank clusters by (avg_similarity desc, cluster_size desc) then cap.
    findings.sort_by_key(|f| std::cmp::Reverse(f.confidence.rank_weight()));
    findings.truncate(MAX_FINDINGS);
    Ok(findings)
}

async fn load_candidates(repository: &DuckDbRepository) -> Result<Vec<DenialCandidate>, AppError> {
    repository
        .query_connection("Boilerplate-denial candidate query", move |conn| {
            let mut statement = conn
                .prepare(
                    "SELECT d.id, d.portal_subdomain, d.request_id, r.agency_id,
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
                     ORDER BY d.id DESC
                     LIMIT ?2",
                )
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to prepare boilerplate-denial candidate query: {error}"
                    ))
                })?;
            let rows = statement
                .query_map(params![MIN_EXCERPT_CHARS as i64, MAX_CANDIDATES], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, Option<i64>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, String>(6)?,
                    ))
                })
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to run boilerplate-denial candidate query: {error}"
                    ))
                })?;
            let mut out = Vec::new();
            for row in rows {
                let (
                    document_id,
                    portal_key,
                    request_id,
                    agency_id,
                    request_title,
                    file_name,
                    extracted_text,
                ) = row.map_err(|error| {
                    AppError::Storage(format!("Failed to decode denial candidate row: {error}"))
                })?;
                let Some(excerpt) = extract_denial_excerpt(&extracted_text) else {
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
                let _ = request_id; // reserved for future intra-request dedup
                out.push(DenialCandidate {
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
        })
        .await
}

/// Finds the passage surrounding a denial phrase inside a full extracted-text body. Returns
/// `None` when no denial phrase appears. The window is roughly the paragraph the phrase sits in,
/// bounded to keep shingling costs stable.
fn extract_denial_excerpt(text: &str) -> Option<String> {
    let haystack = text.to_ascii_lowercase();
    let matched_at = DENIAL_PHRASES
        .iter()
        .filter_map(|phrase| haystack.find(phrase))
        .min()?;
    // Bounded window: ~600 characters around the matched phrase, clamped to the string. That
    // captures the typical boilerplate paragraph without dragging in whole documents.
    let start = matched_at.saturating_sub(300);
    let end = matched_at.saturating_add(300).min(text.len());
    // Snap start/end to whitespace so we don't split a shingle mid-word.
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

/// Lower-cases, collapses whitespace, strips punctuation. Any two paragraphs that differ only in
/// letter case, spacing, or trailing periods hash to the same tokens.
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

/// Emits the set of contiguous `n`-word shingles as 64-bit FNV-1a hashes. Hash-set membership
/// then serves as the input to Jaccard similarity in O(|A|+|B|).
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
            hasher ^= 0x20; // shingle-word separator
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

/// Greedy single-link clustering. For each unseen candidate, expand to any other candidate whose
/// Jaccard vs. the seed exceeds the threshold. O(n²) worst case, bounded by MAX_CANDIDATES.
fn cluster_candidates(candidates: &[DenialCandidate]) -> Vec<DenialCluster> {
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
        if members.len() < MIN_CLUSTER_SIZE {
            continue;
        }
        let mut agency_ids = HashSet::new();
        let mut distinct_titles = HashSet::new();
        for &member in &members {
            if let Some(id) = candidates[member].agency_id {
                agency_ids.insert(id);
            }
            if let Some(title) = &candidates[member].request_title {
                let t = title.trim().to_ascii_lowercase();
                if !t.is_empty() {
                    distinct_titles.insert(t);
                }
            }
        }
        if distinct_titles.len() < MIN_DISTINCT_TOPICS {
            continue;
        }
        let average = if pair_count > 0.0 {
            similarity_total / pair_count
        } else {
            // Single-member clusters were rejected above, so `pair_count == 0` cannot happen
            // for surviving clusters. Kept defensively.
            1.0
        };
        clusters.push(DenialCluster {
            members,
            agency_ids,
            distinct_titles,
            average_similarity: average,
        });
    }
    clusters
}

fn build_findings(candidates: &[DenialCandidate], clusters: Vec<DenialCluster>) -> Vec<Finding> {
    let mut findings = Vec::new();
    let now = Utc::now();
    for cluster in clusters {
        // Prefer stable representative: lowest document_id in the cluster.
        let exemplar_idx = cluster
            .members
            .iter()
            .copied()
            .min_by_key(|&idx| candidates[idx].document_id)
            .expect("cluster has ≥ MIN_CLUSTER_SIZE members");
        let exemplar = &candidates[exemplar_idx];
        let cluster_size = cluster.members.len();
        let distinct_topics = cluster.distinct_titles.len();
        let cross_agency = cluster.agency_ids.len() > 1;

        // Severity: crossing agency lines is a stronger signal than within-agency reuse.
        let severity = if cross_agency {
            FindingSeverity::Serious
        } else {
            FindingSeverity::Notable
        };

        // Confidence: scaled by (cluster_size, similarity, distinct_topics).
        let size_component = ((cluster_size as f64 / 20.0).min(1.0)) * 3_000.0;
        let similarity_component = ((cluster.average_similarity - JACCARD_THRESHOLD)
            / (1.0 - JACCARD_THRESHOLD))
            .clamp(0.0, 1.0)
            * 4_000.0;
        let topic_component = ((distinct_topics as f64 / 10.0).min(1.0)) * 3_000.0;
        let bp = (size_component + similarity_component + topic_component).round() as u16;
        let confidence = FindingConfidence::measured(bp);

        // Subject anchor: agency when within-agency cluster; portal only when cross-agency.
        let subject = if cross_agency {
            FindingSubject {
                portal_key: Some(exemplar.portal_key.clone()),
                document_id: Some(exemplar.document_id),
                ..Default::default()
            }
        } else {
            FindingSubject {
                portal_key: Some(exemplar.portal_key.clone()),
                agency_id: exemplar.agency_id,
                document_id: Some(exemplar.document_id),
                ..Default::default()
            }
        };

        // Salient bits determine id stability. Cluster size and topic count are bucketed so that
        // adding one more copy of the same denial upserts the same row until it crosses a
        // decade boundary; that keeps the inbox stable without erasing legitimate growth.
        let size_bucket = format!("size_bucket={}", cluster_size / 10);
        let topics_bucket = format!("topics_bucket={}", distinct_topics / 5);
        let cross_agency_bit = format!(
            "cross_agency={}",
            if cross_agency { "true" } else { "false" }
        );
        let id = finding_id(
            FindingKind::BoilerplateDenial,
            SLUG,
            &subject,
            &[&size_bucket, &topics_bucket, &cross_agency_bit],
        );

        let headline = if cross_agency {
            format!(
                "Same denial language appears across {cluster_size} requests from {agencies} agencies",
                agencies = cluster.agency_ids.len()
            )
        } else {
            format!(
                "Agency's denial language repeats near-verbatim across {cluster_size} distinct requests"
            )
        };

        let similarity_pct = (cluster.average_similarity * 100.0).round() as u32;
        let explanation = format!(
            "{cluster_size} denial passages share near-identical wording (average {similarity_pct}% \
             shingle overlap) across {distinct_topics} distinct request topics. \
             Individualized review would be expected to produce distinct language per request; \
             this cluster is consistent with a stock refusal being applied uniformly."
        );

        // Cite up to 5 representative documents from the cluster.
        let mut cited = 0usize;
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
            cited += 1;
        }
        debug_assert!(cited >= 1, "cluster guaranteed to have ≥ 1 member");

        findings.push(Finding {
            id,
            kind: FindingKind::BoilerplateDenial,
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
    // Deterministic order for callers that inspect the vec directly (tests, dumps).
    findings.sort_by(|a, b| {
        b.confidence
            .rank_weight()
            .cmp(&a.confidence.rank_weight())
            .then_with(|| a.id.cmp(&b.id))
    });
    // Deduplicate on id in case two subject-equivalent clusters produced the same anchor; the
    // storage layer's upsert would resolve this anyway, but returning duplicates would confuse
    // any caller that trusts the vec verbatim.
    let mut seen = BTreeMap::new();
    for finding in findings {
        seen.entry(finding.id.clone()).or_insert(finding);
    }
    seen.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(id: i64, agency: Option<i64>, title: &str, excerpt: &str) -> DenialCandidate {
        let normalized = normalize(excerpt);
        let shingles = shingle(&normalized, SHINGLE_WORDS);
        DenialCandidate {
            document_id: id,
            portal_key: "test".into(),
            agency_id: agency,
            request_title: Some(title.into()),
            file_name: Some(format!("doc-{id}.pdf")),
            excerpt: excerpt.into(),
            shingles,
        }
    }

    fn stock_denial() -> String {
        // 40+ tokens so shingling produces enough windows to be meaningful.
        "The Department has carefully reviewed the request and determined that no responsive \
         records exist within its custody, care, or control at this time and therefore respectfully \
         declines the request pursuant to applicable exemptions."
            .into()
    }

    fn stock_denial_lightly_edited() -> String {
        // Same passage with punctuation swaps and a comma added — Jaccard should stay above the
        // threshold because word content is unchanged.
        "The Department has carefully reviewed the request and determined that, no responsive \
         records exist within its custody care or control at this time; and therefore respectfully \
         declines the request pursuant to applicable exemptions."
            .into()
    }

    fn distinct_denial() -> String {
        "After conducting a diligent search across multiple databases, the department was unable \
         to locate any responsive materials matching the terms described in the request, and \
         accordingly no documents will be produced under this cover."
            .into()
    }

    #[test]
    fn normalize_lowercases_and_strips_punctuation() {
        assert_eq!(
            normalize("  The Department, has REVIEWED the request. "),
            "the department has reviewed the request"
        );
    }

    #[test]
    fn shingle_produces_deterministic_hashes_and_agrees_across_calls() {
        let a = shingle(&normalize(&stock_denial()), SHINGLE_WORDS);
        let b = shingle(&normalize(&stock_denial()), SHINGLE_WORDS);
        assert_eq!(a, b);
        assert!(!a.is_empty(), "long normalized text produces shingles");
    }

    #[test]
    fn jaccard_is_high_for_lightly_edited_reuse_and_low_for_distinct_prose() {
        let base = shingle(&normalize(&stock_denial()), SHINGLE_WORDS);
        let edited = shingle(&normalize(&stock_denial_lightly_edited()), SHINGLE_WORDS);
        let different = shingle(&normalize(&distinct_denial()), SHINGLE_WORDS);
        assert!(
            jaccard(&base, &edited) >= JACCARD_THRESHOLD,
            "punctuation edits must not drop below threshold"
        );
        assert!(
            jaccard(&base, &different) < JACCARD_THRESHOLD,
            "genuinely different denials must not cluster"
        );
    }

    #[test]
    fn extract_denial_excerpt_finds_the_paragraph_containing_a_phrase() {
        let body = format!(
            "Cover letter irrelevant intro paragraph. {}\n\nSincerely,\nRecords Officer",
            stock_denial()
        );
        let excerpt = extract_denial_excerpt(&body).expect("must extract when phrase present");
        assert!(excerpt.contains("no responsive records"));
    }

    #[test]
    fn extract_denial_excerpt_none_when_no_phrase_present() {
        assert!(extract_denial_excerpt(
            "Cover letter about the enclosed records, which are attached in full."
        )
        .is_none());
    }

    #[test]
    fn extract_denial_excerpt_none_when_matched_passage_too_short() {
        // Match present but the surrounding text is too short to meet MIN_EXCERPT_CHARS.
        assert!(extract_denial_excerpt("no records exist here.").is_none());
    }

    #[test]
    fn cluster_forms_when_five_agency_denials_are_near_identical_across_distinct_topics() {
        let mut candidates = Vec::new();
        for (i, topic) in ["salaries", "vendors", "meetings", "budgets", "training"]
            .iter()
            .enumerate()
        {
            candidates.push(cand(i as i64 + 1, Some(7), topic, &stock_denial()));
        }
        let clusters = cluster_candidates(&candidates);
        assert_eq!(clusters.len(), 1, "one cluster of five reused denials");
        assert_eq!(clusters[0].members.len(), 5);
        assert_eq!(clusters[0].distinct_titles.len(), 5);
        let findings = build_findings(&candidates, clusters);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].kind, FindingKind::BoilerplateDenial);
        assert_eq!(findings[0].severity, FindingSeverity::Notable);
        assert!(findings[0].headline.contains("repeats near-verbatim"));
    }

    #[test]
    fn cross_agency_cluster_is_scored_more_severely() {
        let mut candidates = Vec::new();
        for i in 0..5 {
            candidates.push(cand(
                i,
                Some(i % 2 + 1), // alternating agencies 1 and 2
                &format!("topic-{i}"),
                &stock_denial(),
            ));
        }
        let clusters = cluster_candidates(&candidates);
        assert_eq!(clusters.len(), 1);
        let findings = build_findings(&candidates, clusters);
        assert_eq!(findings[0].severity, FindingSeverity::Serious);
        assert!(findings[0].headline.contains("across"));
    }

    #[test]
    fn no_cluster_below_minimum_size() {
        // Four members — below MIN_CLUSTER_SIZE (5).
        let candidates: Vec<_> = ["a", "b", "c", "d"]
            .iter()
            .enumerate()
            .map(|(i, t)| cand(i as i64, Some(1), t, &stock_denial()))
            .collect();
        let clusters = cluster_candidates(&candidates);
        assert!(
            clusters.is_empty(),
            "four near-duplicate denials must not fire"
        );
    }

    #[test]
    fn no_cluster_when_topics_are_not_diverse() {
        // Five denials with same topic title — coincidence, not a pattern.
        let candidates: Vec<_> = (0..5)
            .map(|i| cand(i, Some(1), "same-topic", &stock_denial()))
            .collect();
        let clusters = cluster_candidates(&candidates);
        assert!(
            clusters.is_empty(),
            "same-topic cluster must not fire; topic diversity is required"
        );
    }

    #[test]
    fn genuinely_distinct_denials_do_not_cluster() {
        let candidates = vec![
            cand(1, Some(1), "one", &stock_denial()),
            cand(2, Some(1), "two", &distinct_denial()),
            cand(
                3,
                Some(1),
                "three",
                "The requested information cannot be released because releasing it \
                 would compromise ongoing investigations that involve sensitive personnel matters \
                 currently under active administrative review by the department.",
            ),
        ];
        let clusters = cluster_candidates(&candidates);
        assert!(clusters.is_empty(), "no cluster from distinct prose");
    }

    #[test]
    fn finding_id_is_stable_across_reruns_for_the_same_cluster() {
        let candidates: Vec<_> = ["a", "b", "c", "d", "e"]
            .iter()
            .enumerate()
            .map(|(i, t)| cand(i as i64 + 1, Some(1), t, &stock_denial()))
            .collect();
        let first = build_findings(&candidates, cluster_candidates(&candidates));
        let second = build_findings(&candidates, cluster_candidates(&candidates));
        assert_eq!(first.len(), 1);
        assert_eq!(second.len(), 1);
        assert_eq!(first[0].id, second[0].id);
    }
}
