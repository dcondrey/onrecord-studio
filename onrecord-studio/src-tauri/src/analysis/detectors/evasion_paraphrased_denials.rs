//! Paraphrased-denial detector, the complement of `boilerplate_denial` within one agency.
//!
//! `boilerplate_denial` clusters denial passages that share their five-word shingles. This
//! detector reports the passages it would not cluster: those on requests to one agency, an agency
//! being a `(portal, id)` pair, that share most of their content words (`BOW_JACCARD_HIGH`) and
//! few five-word sequences (`SHINGLE_JACCARD_LOW`). Rewordings shared between agencies are
//! reported by `cross_corpus_shared_language`.
//!
//! Candidate pairs come from MinHash bands over the word sets (`BANDING_FROM_0_35`); exact
//! Jaccard decides.
//!
//! **Not a legal claim**, not evidence of coordination or of intent, and not a showing that a
//! request was not reviewed individually. A finding is emitted only at `MIN_CONFIDENCE_TO_FIRE`.

use super::text_shared::{
    agency_label, cite_passages, content_words, distinct_documents, distinct_titles, jaccard,
    linked_groups, load_passages, shingles, word_set, AgencyKey, BandIndex, PassageCandidate,
    PassageRules, BANDING_FROM_0_35, PATTERN_CAVEAT,
};
use crate::{
    analysis::finding::{
        finding_id, Finding, FindingConfidence, FindingKind, FindingSeverity, FindingSubject,
    },
    error::AppError,
    storage::DuckDbRepository,
};
use chrono::Utc;
use std::collections::{BTreeMap, HashSet};

pub const SLUG: &str = "evasion_paraphrased_denials";

const SHINGLE_WORDS: usize = 5;
/// A passage with fewer content words has too little vocabulary to compare.
const MIN_CONTENT_WORDS: usize = SHINGLE_WORDS * 2;
/// Content-word Jaccard at or above which two passages cover the same ground.
const BOW_JACCARD_HIGH: f64 = 0.35;
/// Content-word shingle Jaccard below which two passages are not the same wording.
const SHINGLE_JACCARD_LOW: f64 = 0.35;
/// Minimum distinct documents in a cluster.
const MIN_CLUSTER_SIZE: usize = 4;
const MIN_DISTINCT_TOPICS: usize = 3;
const MAX_FINDINGS: usize = 50;
const MIN_CONFIDENCE_TO_FIRE: u16 = 6500;
/// Members whose pairs are averaged for the reported overlaps.
const AVERAGED_MEMBERS: usize = 200;

static RULES: PassageRules = PassageRules {
    outcome: &[
        &["no", "responsive", "records"],
        &["no", "records", "exist"],
        &["no", "records", "were", "found"],
        &["no", "such", "records"],
    ],
    needs_addressee: &[
        &["unable", "to", "locate"],
        &["cannot", "locate"],
        &["does", "not", "maintain"],
        &["not", "readily", "available"],
        &["cannot", "be", "produced"],
        &["cannot", "fulfill"],
        &["not", "in", "our", "custody"],
    ],
    half_window: 200,
};

struct Sets {
    words: Vec<HashSet<u64>>,
    shingles: Vec<HashSet<u64>>,
}

pub async fn run(repository: &DuckDbRepository) -> Result<Vec<Finding>, AppError> {
    let mut candidates = Vec::new();
    let mut sets = Sets {
        words: Vec::new(),
        shingles: Vec::new(),
    };
    for candidate in load_passages(repository, "Paraphrased-denial passage query", &RULES).await? {
        let words = content_words(&candidate.excerpt);
        if candidate.agency.is_none() || words.len() < MIN_CONTENT_WORDS {
            continue;
        }
        sets.words.push(word_set(&words));
        sets.shingles.push(shingles(&words, SHINGLE_WORDS));
        candidates.push(candidate);
    }
    let index = BandIndex::build(&sets.words, BANDING_FROM_0_35);
    let clusters = paraphrase_clusters(&candidates, &sets, |member| index.later(member));
    let mut findings = build_findings(&candidates, clusters);
    findings.sort_by_key(|f| std::cmp::Reverse(f.confidence.rank_weight()));
    findings.truncate(MAX_FINDINGS);
    Ok(findings)
}

fn is_paraphrase(sets: &Sets, a: usize, b: usize) -> bool {
    jaccard(&sets.words[a], &sets.words[b]) >= BOW_JACCARD_HIGH
        && jaccard(&sets.shingles[a], &sets.shingles[b]) < SHINGLE_JACCARD_LOW
}

struct ParaphraseCluster {
    agency: AgencyKey,
    members: Vec<usize>,
    documents: usize,
    topics: usize,
    avg_bow_similarity: f64,
    avg_shingle_similarity: f64,
}

/// The largest group of mutually linked paraphrases for each agency, when it meets the size and
/// topic gates.
fn paraphrase_clusters(
    candidates: &[PassageCandidate],
    sets: &Sets,
    later: impl Fn(usize) -> Vec<usize>,
) -> Vec<ParaphraseCluster> {
    let groups = linked_groups(candidates.len(), later, |a, b| {
        candidates[a].agency == candidates[b].agency && is_paraphrase(sets, a, b)
    });
    let mut largest: BTreeMap<AgencyKey, (usize, Vec<usize>)> = BTreeMap::new();
    for group in groups {
        let Some(agency) = candidates[group[0]].agency.clone() else {
            continue;
        };
        let documents = distinct_documents(candidates, &group);
        if largest
            .get(&agency)
            .is_none_or(|(held, _)| documents > *held)
        {
            largest.insert(agency, (documents, group));
        }
    }
    largest
        .into_iter()
        .filter_map(|(agency, (documents, members))| {
            let topics = distinct_titles(candidates, &members);
            if documents < MIN_CLUSTER_SIZE || topics < MIN_DISTINCT_TOPICS {
                return None;
            }
            let sample = &members[..members.len().min(AVERAGED_MEMBERS)];
            let (mut bow, mut shingle, mut pairs) = (0.0, 0.0, 0.0);
            for (position, &a) in sample.iter().enumerate() {
                for &b in &sample[position + 1..] {
                    bow += jaccard(&sets.words[a], &sets.words[b]);
                    shingle += jaccard(&sets.shingles[a], &sets.shingles[b]);
                    pairs += 1.0;
                }
            }
            Some(ParaphraseCluster {
                agency,
                members,
                documents,
                topics,
                avg_bow_similarity: bow / pairs,
                avg_shingle_similarity: shingle / pairs,
            })
        })
        .collect()
}

fn build_findings(
    candidates: &[PassageCandidate],
    clusters: Vec<ParaphraseCluster>,
) -> Vec<Finding> {
    let now = Utc::now();
    let mut out = Vec::new();
    for cluster in clusters {
        // Confidence rises with cluster size, word overlap, topic spread, and distance from the
        // shingle ceiling.
        let size_bp = ((cluster.documents as f64 - MIN_CLUSTER_SIZE as f64) * 400.0)
            .clamp(0.0, 2400.0) as u16;
        let bow_bp =
            ((cluster.avg_bow_similarity - BOW_JACCARD_HIGH) * 6000.0).clamp(0.0, 2200.0) as u16;
        let topic_bp = ((cluster.topics as f64 - MIN_DISTINCT_TOPICS as f64) * 250.0)
            .clamp(0.0, 1200.0) as u16;
        let anti_shingle_bp = ((SHINGLE_JACCARD_LOW - cluster.avg_shingle_similarity) * 6000.0)
            .clamp(0.0, 1500.0) as u16;
        let bp = (4000 + size_bp + bow_bp + topic_bp + anti_shingle_bp).min(10_000);
        if bp < MIN_CONFIDENCE_TO_FIRE {
            continue;
        }
        let subject = FindingSubject {
            agency_id: Some(cluster.agency.id),
            portal_key: Some(cluster.agency.portal.clone()),
            ..Default::default()
        };
        let salient = format!(
            "size={},bow_bin={}",
            cluster.documents,
            (cluster.avg_bow_similarity * 10.0).round() as i32
        );
        let id = finding_id(
            FindingKind::EvasionParaphrasedDenials,
            SLUG,
            &subject,
            &[&salient],
        );
        let agency = agency_label(
            &cluster.agency,
            candidates[cluster.members[0]].agency_name.as_deref(),
            false,
        );
        let headline = format!(
            "{n} denial passages on requests to {agency} share vocabulary in different wording",
            n = cluster.members.len(),
        );
        let explanation = format!(
            "{n} passages with denial wording, in {documents} public documents on requests to \
             {agency}, share an average of {bow:.0}% of their distinct content words across \
             {topics} request titles and an average of {sh:.0}% of their five-word sequences, \
             below the {threshold:.0}% at which they would count as the same wording. Each \
             citation gives the passage's position in its document. {PATTERN_CAVEAT}",
            n = cluster.members.len(),
            documents = cluster.documents,
            bow = cluster.avg_bow_similarity * 100.0,
            topics = cluster.topics,
            sh = cluster.avg_shingle_similarity * 100.0,
            threshold = SHINGLE_JACCARD_LOW * 100.0,
        );
        out.push(Finding {
            id,
            kind: FindingKind::EvasionParaphrasedDenials,
            severity: FindingSeverity::Serious,
            confidence: FindingConfidence::measured(bp),
            subject,
            headline,
            explanation,
            citations: cite_passages(candidates, &cluster.members),
            discovered_at: now,
            last_seen_at: now,
            dismissed_at: None,
            source_detector: SLUG.into(),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::detectors::text_shared::fixtures::{
        corpus, letter, overlapping, stored_slice, Letter, NO_RECORDS_LETTER, REWORDED_LETTERS,
    };

    fn reworded(count: usize) -> Vec<Letter> {
        REWORDED_LETTERS[..count]
            .iter()
            .enumerate()
            .map(|(index, text)| {
                letter("city-a", (7, "Housing Commission"), index as i64 + 1, text)
            })
            .collect()
    }

    #[tokio::test]
    async fn agencies_sharing_an_id_on_different_portals_are_not_pooled() {
        let letters = REWORDED_LETTERS
            .iter()
            .enumerate()
            .map(|(index, text)| {
                let portal = if index < 2 { "city-a" } else { "city-b" };
                letter(portal, (1, "Police"), index as i64 + 1, text)
            })
            .collect();
        let findings = run(&corpus(letters).await).await.unwrap();
        assert!(findings.is_empty(), "{findings:?}");
    }

    #[tokio::test]
    async fn four_rewordings_at_one_agency_are_reported_with_its_name_and_passages() {
        assert!(run(&corpus(reworded(3)).await).await.unwrap().is_empty());
        let repository = corpus(reworded(4)).await;
        let findings = run(&repository).await.unwrap();
        assert_eq!(findings.len(), 1, "{findings:?}");
        let finding = &findings[0];
        assert_eq!(finding.subject.agency_id, Some(7));
        assert_eq!(finding.subject.portal_key.as_deref(), Some("city-a"));
        assert!(
            finding.headline.contains("requests to Housing Commission"),
            "{}",
            finding.headline
        );
        assert!(finding.explanation.contains(PATTERN_CAVEAT));
        assert_eq!(finding.citations.len(), 4);
        for citation in &finding.citations {
            let passage = stored_slice(&repository, citation).await;
            assert!(REWORDED_LETTERS.contains(&passage.as_str()), "{passage}");
        }
        assert_eq!(finding.id, run(&repository).await.unwrap()[0].id);
    }

    #[tokio::test]
    async fn identical_denials_are_not_paraphrases() {
        let letters = (1..=4)
            .map(|id| letter("city-a", (7, "Housing Commission"), id, NO_RECORDS_LETTER))
            .collect();
        assert!(run(&corpus(letters).await).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn rewordings_under_one_request_title_are_not_reported() {
        let letters = reworded(4)
            .into_iter()
            .map(|letter| Letter {
                title: "Same title".into(),
                ..letter
            })
            .collect();
        assert!(run(&corpus(letters).await).await.unwrap().is_empty());
    }

    #[test]
    fn a_pair_needs_thirty_five_percent_of_words_and_less_of_sequences() {
        for (words, sequences, expected) in [
            (34, 0, false),
            (35, 0, true),
            (36, 0, true),
            (100, 34, true),
            (100, 35, false),
            (100, 36, false),
        ] {
            let (word_a, word_b) = overlapping(words, 100);
            let (shingle_a, shingle_b) = overlapping(sequences, 100);
            let sets = Sets {
                words: vec![word_a, word_b],
                shingles: vec![shingle_a, shingle_b],
            };
            assert_eq!(is_paraphrase(&sets, 0, 1), expected, "{words} {sequences}");
        }
    }
}
