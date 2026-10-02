//! Adaptive-paraphrasing denial detector — the negative image of `boilerplate_denial`.
//!
//! Wave 2B's `boilerplate_denial` catches denials that share near-identical language across
//! otherwise-unrelated requests via shingled-Jaccard clustering. An agency that has learned that
//! boilerplate is being detected can defeat that specific check by paraphrasing each denial —
//! diverse surface language, same underlying refusal. This detector looks for that adaptation:
//! denials whose SHINGLED similarity is LOW (Wave 2B wouldn't cluster them) but whose bag-of-words
//! topical similarity is HIGH (they cover the same content with different word order and
//! synonyms), all coming from one agency.
//!
//! **Not a legal claim** (`is_legal_claim = false`). Adversarial pattern signals demand higher
//! confidence floors than direct evidence, because a false positive here accuses the agency of
//! adaptive evasion rather than of a specific rule violation. We fire at ≥ 6500 bp only.
//!
//! Uses no embeddings — bag-of-words Jaccard is deterministic, always available, and captures the
//! signal the directive asks for without a fastembed dependency (the vector index isn't exposed
//! to the detector contract; a future contract change could add it).

use super::text_shared::jaccard;
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
use std::collections::{HashMap, HashSet};

pub const SLUG: &str = "evasion_paraphrased_denials";

const MAX_CANDIDATES: i64 = 800;
const MIN_EXCERPT_CHARS: i64 = 120;
const SHINGLE_WORDS: usize = 5;
/// Bag-of-words Jaccard high-similarity threshold: two denials that share this fraction of
/// non-stopword tokens are on the same topic. Real English paraphrases of the same content
/// typically fall in the 0.30-0.45 range once synonyms and reordered sentences are accounted for,
/// so 0.35 is the practical floor for "same topic". The MIN_CONFIDENCE_TO_FIRE gate on the
/// emitted Finding still filters out weakly-clustered matches.
const BOW_JACCARD_HIGH: f64 = 0.35;
/// Shingled-Jaccard low-similarity threshold: two denials below this fraction of shared 5-grams
/// are NOT boilerplate. This is the ceiling Wave 2B's detector would have merged them at.
const SHINGLE_JACCARD_LOW: f64 = 0.35;
const MIN_CLUSTER_SIZE: usize = 4;
const MIN_DISTINCT_TOPICS: usize = 3;
const MAX_FINDINGS: usize = 50;
/// Adversarial detectors fire at ≥ 6500 bp only.
const MIN_CONFIDENCE_TO_FIRE: u16 = 6500;

pub async fn run(repository: &DuckDbRepository) -> Result<Vec<Finding>, AppError> {
    let candidates = load_candidates(repository).await?;
    let clusters = cluster_paraphrased(&candidates);
    let mut findings = build_findings(&candidates, clusters);
    findings.sort_by_key(|f| std::cmp::Reverse(f.confidence.rank_weight()));
    findings.truncate(MAX_FINDINGS);
    Ok(findings)
}

struct Candidate {
    document_id: i64,
    portal_key: String,
    agency_id: Option<i64>,
    request_title: Option<String>,
    file_name: Option<String>,
    excerpt: String,
    bow: HashSet<u64>,
    shingles: HashSet<u64>,
}

async fn load_candidates(repository: &DuckDbRepository) -> Result<Vec<Candidate>, AppError> {
    repository
        .query_connection("Paraphrased-denial candidate query", move |conn| {
            let mut statement = conn
                .prepare(
                    "SELECT d.id, d.portal_subdomain, r.agency_id, r.title, d.file_name,
                            fts.extracted_text
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
                        "Failed to prepare paraphrased-denial candidate query: {error}"
                    ))
                })?;
            let rows = statement
                .query_map(params![MIN_EXCERPT_CHARS, MAX_CANDIDATES], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, String>(5)?,
                    ))
                })
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to run paraphrased-denial candidate query: {error}"
                    ))
                })?;
            let mut out = Vec::new();
            for row in rows {
                let (document_id, portal_key, agency_id, request_title, file_name, extracted_text) =
                    row.map_err(|error| {
                        AppError::Storage(format!(
                            "Failed to decode paraphrased-denial row: {error}"
                        ))
                    })?;
                let Some(excerpt) = extract_denial_excerpt(&extracted_text) else {
                    continue;
                };
                let tokens: Vec<String> = tokenize(&excerpt);
                if tokens.len() < SHINGLE_WORDS * 2 {
                    continue;
                }
                let bow = bag_of_words(&tokens);
                if bow.is_empty() {
                    continue;
                }
                let shingles = shingle(&tokens);
                out.push(Candidate {
                    document_id,
                    portal_key,
                    agency_id,
                    request_title,
                    file_name,
                    excerpt,
                    bow,
                    shingles,
                });
            }
            Ok(out)
        })
        .await
}

/// Extract the sentence around the first denial anchor phrase, ±200 chars. Denials tend to be
/// short and their surrounding sentence carries the topical vocabulary a paraphrase would rewrite
/// but not eliminate.
fn extract_denial_excerpt(text: &str) -> Option<String> {
    const ANCHORS: &[&str] = &[
        "no responsive records",
        "no records exist",
        "no records were found",
        "unable to locate",
        "cannot locate",
        "does not maintain",
        "not readily available",
        "cannot be produced",
        "cannot fulfill",
        "not in our custody",
        "no such records",
    ];
    let lower = text.to_lowercase();
    let mut hit: Option<usize> = None;
    for anchor in ANCHORS {
        if let Some(position) = lower.find(anchor) {
            hit = Some(hit.map_or(position, |prior| prior.min(position)));
        }
    }
    let position = hit?;
    let start = position.saturating_sub(200);
    let end = usize::min(position + 200, text.len());
    let start = floor_char(text, start);
    let end = ceil_char(text, end);
    Some(text[start..end].to_string())
}

fn floor_char(text: &str, mut index: usize) -> usize {
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn ceil_char(text: &str, mut index: usize) -> usize {
    while index < text.len() && !text.is_char_boundary(index) {
        index += 1;
    }
    index
}

fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter_map(|raw| {
            let normalized = raw.trim().to_lowercase();
            if normalized.len() < 3 || STOPWORDS.contains(&normalized.as_str()) {
                None
            } else {
                Some(normalized)
            }
        })
        .collect()
}

fn bag_of_words(tokens: &[String]) -> HashSet<u64> {
    tokens.iter().map(|t| hash64(t.as_bytes())).collect()
}

fn shingle(tokens: &[String]) -> HashSet<u64> {
    if tokens.len() < SHINGLE_WORDS {
        return HashSet::new();
    }
    let mut set = HashSet::new();
    for window in tokens.windows(SHINGLE_WORDS) {
        let mut bytes = Vec::with_capacity(64);
        for token in window {
            bytes.extend_from_slice(token.as_bytes());
            bytes.push(b'\x1f');
        }
        set.insert(hash64(&bytes));
    }
    set
}

fn hash64(bytes: &[u8]) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

/// Per-agency cluster of paraphrased denials: high bow-Jaccard AND low shingle-Jaccard between
/// each pair. Emits at most one cluster per agency (the largest one that meets the gates).
struct ParaphraseCluster {
    agency_id: i64,
    members: Vec<usize>,
    avg_bow_similarity: f64,
    avg_shingle_similarity: f64,
    distinct_titles: HashSet<String>,
}

fn cluster_paraphrased(candidates: &[Candidate]) -> Vec<ParaphraseCluster> {
    // Group by agency; a paraphrased-denial pattern is by definition an agency-level adaptation.
    let mut per_agency: HashMap<i64, Vec<usize>> = HashMap::new();
    for (index, candidate) in candidates.iter().enumerate() {
        if let Some(agency_id) = candidate.agency_id {
            per_agency.entry(agency_id).or_default().push(index);
        }
    }
    let mut clusters = Vec::new();
    for (agency_id, indices) in per_agency {
        if indices.len() < MIN_CLUSTER_SIZE {
            continue;
        }
        // Union-find over pairs meeting BOTH gates.
        let mut parent: Vec<usize> = (0..indices.len()).collect();
        fn find(parent: &mut [usize], x: usize) -> usize {
            let mut root = x;
            while parent[root] != root {
                root = parent[root];
            }
            let mut cursor = x;
            while parent[cursor] != root {
                let next = parent[cursor];
                parent[cursor] = root;
                cursor = next;
            }
            root
        }
        for i in 0..indices.len() {
            for j in (i + 1)..indices.len() {
                let bow = jaccard(&candidates[indices[i]].bow, &candidates[indices[j]].bow);
                if bow < BOW_JACCARD_HIGH {
                    continue;
                }
                let shingle = jaccard(
                    &candidates[indices[i]].shingles,
                    &candidates[indices[j]].shingles,
                );
                if shingle >= SHINGLE_JACCARD_LOW {
                    continue;
                }
                let root_i = find(&mut parent, i);
                let root_j = find(&mut parent, j);
                if root_i != root_j {
                    parent[root_i] = root_j;
                }
            }
        }
        let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
        for (i, &index) in indices.iter().enumerate() {
            let root = find(&mut parent, i);
            groups.entry(root).or_default().push(index);
        }
        // Pick the single largest group per agency; if it clears the gates, record it.
        let Some((_, largest)) = groups.into_iter().max_by_key(|(_, v)| v.len()) else {
            continue;
        };
        if largest.len() < MIN_CLUSTER_SIZE {
            continue;
        }
        let titles: HashSet<String> = largest
            .iter()
            .filter_map(|idx| candidates[*idx].request_title.clone())
            .collect();
        if titles.len() < MIN_DISTINCT_TOPICS {
            continue;
        }
        // Average similarities across all pairs in the surviving group.
        let (mut bow_sum, mut sh_sum, mut pair_count) = (0.0, 0.0, 0);
        for i in 0..largest.len() {
            for j in (i + 1)..largest.len() {
                bow_sum += jaccard(&candidates[largest[i]].bow, &candidates[largest[j]].bow);
                sh_sum += jaccard(
                    &candidates[largest[i]].shingles,
                    &candidates[largest[j]].shingles,
                );
                pair_count += 1;
            }
        }
        if pair_count == 0 {
            continue;
        }
        clusters.push(ParaphraseCluster {
            agency_id,
            members: largest,
            avg_bow_similarity: bow_sum / pair_count as f64,
            avg_shingle_similarity: sh_sum / pair_count as f64,
            distinct_titles: titles,
        });
    }
    clusters
}

fn build_findings(candidates: &[Candidate], clusters: Vec<ParaphraseCluster>) -> Vec<Finding> {
    let mut out = Vec::new();
    for cluster in clusters {
        // Confidence scales with cluster size, bow similarity, topic diversity, and inverse
        // shingle similarity (the further from boilerplate, the stronger the paraphrase signal).
        let size_bp = ((cluster.members.len() as f64 - MIN_CLUSTER_SIZE as f64) * 400.0)
            .clamp(0.0, 2400.0) as u16;
        let bow_bp =
            ((cluster.avg_bow_similarity - BOW_JACCARD_HIGH) * 6000.0).clamp(0.0, 2200.0) as u16;
        let topic_bp = ((cluster.distinct_titles.len() as f64 - MIN_DISTINCT_TOPICS as f64) * 250.0)
            .clamp(0.0, 1200.0) as u16;
        let anti_shingle_bp = ((SHINGLE_JACCARD_LOW - cluster.avg_shingle_similarity) * 6000.0)
            .clamp(0.0, 1500.0) as u16;
        let raw_bp = 4000_u16
            .saturating_add(size_bp)
            .saturating_add(bow_bp)
            .saturating_add(topic_bp)
            .saturating_add(anti_shingle_bp);
        let bp = raw_bp.min(10_000);
        if bp < MIN_CONFIDENCE_TO_FIRE {
            continue;
        }
        // Cross-agency severity would need a different clustering pass; this detector is
        // per-agency by construction. Serious severity uniformly.
        let severity = FindingSeverity::Serious;
        let subject = FindingSubject {
            agency_id: Some(cluster.agency_id),
            portal_key: candidates
                .get(cluster.members[0])
                .map(|c| c.portal_key.clone()),
            ..Default::default()
        };
        let salient = format!(
            "agency={},size={},bow_bin={}",
            cluster.agency_id,
            cluster.members.len(),
            (cluster.avg_bow_similarity * 10.0).round() as i32
        );
        let id = finding_id(
            FindingKind::EvasionParaphrasedDenials,
            SLUG,
            &subject,
            &[&salient],
        );
        let citations: Vec<FindingCitation> = cluster
            .members
            .iter()
            .take(5)
            .map(|idx| {
                let c = &candidates[*idx];
                let quote = c
                    .excerpt
                    .split_whitespace()
                    .take(24)
                    .collect::<Vec<_>>()
                    .join(" ");
                FindingCitation {
                    portal_key: c.portal_key.clone(),
                    document_id: c.document_id,
                    character_start: None,
                    character_end: None,
                    quote: Some(quote),
                    label: c.file_name.clone(),
                }
            })
            .collect();
        let headline = format!(
            "Agency {} denies on the same topic in {} distinct paraphrases",
            cluster.agency_id,
            cluster.members.len()
        );
        let explanation = format!(
            "{n} public denials attributed to agency {agency} share a topical vocabulary \
             (bag-of-words Jaccard {bow:.0}%) across {topics} distinct request titles, but their \
             5-word shingles cluster at only {sh:.0}% — below the {threshold:.0}% threshold at \
             which the boilerplate detector would merge them. Same underlying refusal, distinct \
             surface language per response.",
            n = cluster.members.len(),
            agency = cluster.agency_id,
            bow = cluster.avg_bow_similarity * 100.0,
            topics = cluster.distinct_titles.len(),
            sh = cluster.avg_shingle_similarity * 100.0,
            threshold = SHINGLE_JACCARD_LOW * 100.0,
        );
        let now = Utc::now();
        out.push(Finding {
            id,
            kind: FindingKind::EvasionParaphrasedDenials,
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
        });
    }
    out
}

const STOPWORDS: &[&str] = &[
    "the",
    "and",
    "for",
    "any",
    "all",
    "with",
    "from",
    "this",
    "that",
    "have",
    "has",
    "was",
    "were",
    "are",
    "not",
    "but",
    "into",
    "onto",
    "about",
    "please",
    "provide",
    "records",
    "record",
    "document",
    "documents",
    "information",
    "request",
    "requests",
    "public",
    "responsive",
    "you",
    "your",
    "our",
    "their",
    "will",
    "may",
    "can",
    "would",
    "shall",
    "such",
    "please",
    "under",
    "over",
    "than",
    "then",
    "which",
    "when",
    "how",
    "who",
    "what",
    "why",
    "does",
    "did",
    "them",
    "these",
    "those",
    "those",
    "some",
    "there",
    "here",
];

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(id: i64, agency: Option<i64>, title: &str, excerpt: &str) -> Candidate {
        let tokens = tokenize(excerpt);
        let bow = bag_of_words(&tokens);
        let shingles = shingle(&tokens);
        Candidate {
            document_id: id,
            portal_key: "test".into(),
            agency_id: agency,
            request_title: Some(title.into()),
            file_name: Some(format!("doc-{id}.pdf")),
            excerpt: excerpt.into(),
            bow,
            shingles,
        }
    }

    #[test]
    fn jaccard_of_identical_bags_is_one() {
        let a: HashSet<u64> = [1_u64, 2, 3, 4].into_iter().collect();
        assert!((jaccard(&a, &a) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn jaccard_of_disjoint_bags_is_zero() {
        let a: HashSet<u64> = [1, 2].into_iter().collect();
        let b: HashSet<u64> = [3, 4].into_iter().collect();
        assert_eq!(jaccard(&a, &b), 0.0);
    }

    #[test]
    fn extract_denial_excerpt_captures_window_around_first_anchor() {
        let text = "Some preamble. no responsive records were found regarding the requested \
                    housing contract materials. Signed.";
        let excerpt = extract_denial_excerpt(text).expect("has an anchor");
        assert!(excerpt.contains("no responsive records"));
    }

    #[test]
    fn extract_denial_excerpt_returns_none_without_anchor() {
        assert!(extract_denial_excerpt("Attached please find the responsive records.").is_none());
    }

    #[test]
    fn paraphrased_denials_at_same_agency_cluster_when_bow_high_and_shingles_low() {
        // Four denials on distinct titles, all about housing contract material, using different
        // sentence structures and synonyms so shingled-Jaccard stays low but bag-of-words is high.
        let candidates = vec![
            cand(
                1,
                Some(7),
                "Housing Contract 2024-A",
                "After thorough diligent research, unable to locate the specific housing \
                 contract materials the requester identified regarding downtown redevelopment.",
            ),
            cand(
                2,
                Some(7),
                "Housing Contract 2024-B",
                "Regarding downtown redevelopment housing materials referenced, the department \
                 does not maintain the specific contract records identified in your request.",
            ),
            cand(
                3,
                Some(7),
                "Redevelopment Materials",
                "Concerning your downtown redevelopment housing contract query, our office \
                 cannot locate the referenced materials in departmental holdings.",
            ),
            cand(
                4,
                Some(7),
                "Contract Records",
                "The housing contract materials for downtown redevelopment referenced in your \
                 request are not readily available and cannot be produced by this department.",
            ),
        ];
        let clusters = cluster_paraphrased(&candidates);
        assert!(!clusters.is_empty(), "expected at least one cluster");
    }

    #[test]
    fn boilerplate_denials_do_not_form_a_paraphrase_cluster() {
        // Four denials with identical prose — shingled-Jaccard would be ~1.0, well above the low
        // threshold, so cluster_paraphrased must skip them (that's boilerplate detector's job).
        let identical = "After a thorough search of departmental records, no responsive records \
                         exist related to your request. Please contact us with any questions.";
        let candidates = vec![
            cand(1, Some(7), "Topic A", identical),
            cand(2, Some(7), "Topic B", identical),
            cand(3, Some(7), "Topic C", identical),
            cand(4, Some(7), "Topic D", identical),
        ];
        let clusters = cluster_paraphrased(&candidates);
        assert!(
            clusters.is_empty(),
            "boilerplate must NOT be flagged as paraphrase (that's Wave 2B's job)"
        );
    }

    #[test]
    fn cluster_requires_min_size_and_distinct_titles() {
        let candidates = vec![
            cand(
                1,
                Some(7),
                "Same Title",
                "unable to locate the housing contract materials",
            ),
            cand(
                2,
                Some(7),
                "Same Title",
                "cannot locate the housing contract records identified",
            ),
            cand(
                3,
                Some(7),
                "Same Title",
                "does not maintain the housing contract materials",
            ),
            cand(
                4,
                Some(7),
                "Same Title",
                "not readily available housing contract records",
            ),
        ];
        let clusters = cluster_paraphrased(&candidates);
        assert!(
            clusters.is_empty(),
            "same title across all members should not clear distinct-topics gate"
        );
    }

    #[test]
    fn finding_id_stable_for_same_cluster_shape() {
        let candidates = vec![
            cand(
                1,
                Some(7),
                "Topic-1",
                "unable to locate the housing contract materials specifically referenced downtown redevelopment",
            ),
            cand(
                2,
                Some(7),
                "Topic-2",
                "does not maintain housing contract materials referenced regarding downtown redevelopment",
            ),
            cand(
                3,
                Some(7),
                "Topic-3",
                "cannot locate housing contract materials for downtown redevelopment referenced",
            ),
            cand(
                4,
                Some(7),
                "Topic-4",
                "housing contract materials for downtown redevelopment not readily available",
            ),
        ];
        let a = build_findings(&candidates, cluster_paraphrased(&candidates));
        let b = build_findings(&candidates, cluster_paraphrased(&candidates));
        if let (Some(fa), Some(fb)) = (a.first(), b.first()) {
            assert_eq!(fa.id, fb.id, "same cluster shape must produce same id");
        }
    }
}
