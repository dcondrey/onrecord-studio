//! Cross-corpus shared-language detector.
//!
//! `boilerplate_denial` reports near-identical denial passages on any set of requests. This
//! detector reports one determination passage used by several agencies, an agency being a
//! `(portal, id)` pair, in two forms:
//!
//! - **Near-identical**: at least `MIN_DISTINCT_AGENCIES` agencies and `MIN_TOTAL_OCCURRENCES`
//!   documents whose passages share `JACCARD_THRESHOLD` of their five-word shingles. The bar is
//!   above `boilerplate_denial`'s 0.60 because two agencies reaching for the same statutory phrase
//!   can meet 0.60 without sharing a paragraph.
//! - **Reworded**: at least `MIN_DISTINCT_AGENCIES` agencies whose passages are linked by pairs
//!   sharing `REWORDED_WORD_OVERLAP` of their distinct content words while staying below
//!   `JACCARD_THRESHOLD` on shingles. The word-overlap floor is a chosen cut-off with no measured
//!   error rate, so these findings carry no confidence.
//!
//! Candidate pairs for both come from MinHash bands (`BANDING_FROM_0_60`); exact Jaccard decides.
//!
//! **What this is not**: a legal claim, evidence of coordination, or a showing that a request was
//! not reviewed individually.

use super::text_shared::{
    agencies_of, all_words, cite_passages, content_words, distinct_documents, exemplar, jaccard,
    linked_groups, load_passages, name_agencies, seed_clusters, shingles, word_set, BandIndex,
    PassageCandidate, PassageRules, SeedCluster, BANDING_FROM_0_60, PATTERN_CAVEAT,
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

pub const SLUG: &str = "cross_corpus_shared_language";

const SHINGLE_WORDS: usize = 5;

/// Shingle Jaccard at or above which two passages are the same paragraph.
const JACCARD_THRESHOLD: f64 = 0.85;

/// Content-word Jaccard at or above which two passages below `JACCARD_THRESHOLD` are rewordings
/// of each other.
const REWORDED_WORD_OVERLAP: f64 = 0.60;

const MIN_DISTINCT_AGENCIES: usize = 3;

/// Minimum distinct documents in a near-identical cluster.
const MIN_TOTAL_OCCURRENCES: usize = 5;

const MAX_FINDINGS: usize = 50;

static RULES: PassageRules = PassageRules {
    outcome: &[
        &["no", "responsive", "records"],
        &["no", "records", "exist"],
        &["no", "records", "were", "located"],
        &["no", "records", "that", "respond"],
        &["no", "records", "available"],
        &["cannot", "locate", "any", "records"],
        &["unable", "to", "locate", "records"],
        &["does", "not", "maintain", "records"],
        &["does", "not", "possess", "records"],
        &["no", "documents", "responsive"],
    ],
    needs_addressee: &[
        &["not", "readily", "available"],
        &["records", "are", "exempt", "from", "disclosure"],
        &["we", "have", "determined"],
        &["after", "diligent", "search"],
        &["after", "a", "diligent", "search"],
        &["the", "department", "has", "reviewed"],
        &["pursuant", "to", "government", "code"],
    ],
    half_window: 300,
};

/// The two views of each passage that the comparisons use.
struct Sets {
    shingles: Vec<HashSet<u64>>,
    words: Vec<HashSet<u64>>,
}

impl Sets {
    fn of(candidates: &[PassageCandidate]) -> Self {
        Self {
            shingles: candidates
                .iter()
                .map(|candidate| shingles(&all_words(&candidate.excerpt), SHINGLE_WORDS))
                .collect(),
            words: candidates
                .iter()
                .map(|candidate| word_set(&content_words(&candidate.excerpt)))
                .collect(),
        }
    }
}

pub async fn run(repository: &DuckDbRepository) -> Result<Vec<Finding>, AppError> {
    // A passage on a request with no recorded agency cannot be counted toward any agency.
    let candidates: Vec<PassageCandidate> = load_passages(
        repository,
        "Cross-corpus shared-language passage query",
        &RULES,
    )
    .await?
    .into_iter()
    .filter(|candidate| candidate.agency.is_some())
    .collect();
    let sets = Sets::of(&candidates);
    let shingle_index = BandIndex::build(&sets.shingles, BANDING_FROM_0_60);
    let word_index = BandIndex::build(&sets.words, BANDING_FROM_0_60);
    let identical = identical_clusters(&candidates, &sets, |seed| shingle_index.later(seed));
    let reworded = reworded_groups(&candidates, &sets, |index| word_index.later(index));
    let mut findings = identical_findings(&candidates, identical);
    findings.extend(reworded_findings(&candidates, reworded));
    findings.sort_by_key(|f| std::cmp::Reverse(f.confidence.rank_weight()));
    findings.truncate(MAX_FINDINGS);
    Ok(findings)
}

fn identical_clusters(
    candidates: &[PassageCandidate],
    sets: &Sets,
    later: impl Fn(usize) -> Vec<usize>,
) -> Vec<SeedCluster> {
    seed_clusters(&sets.shingles, JACCARD_THRESHOLD, later)
        .into_iter()
        .filter(|cluster| {
            distinct_documents(candidates, &cluster.members) >= MIN_TOTAL_OCCURRENCES
                && agencies_of(candidates, &cluster.members).len() >= MIN_DISTINCT_AGENCIES
        })
        .collect()
}

fn is_reworded(sets: &Sets, a: usize, b: usize) -> bool {
    jaccard(&sets.words[a], &sets.words[b]) >= REWORDED_WORD_OVERLAP
        && jaccard(&sets.shingles[a], &sets.shingles[b]) < JACCARD_THRESHOLD
}

fn reworded_groups(
    candidates: &[PassageCandidate],
    sets: &Sets,
    later: impl Fn(usize) -> Vec<usize>,
) -> Vec<Vec<usize>> {
    linked_groups(candidates.len(), later, |a, b| is_reworded(sets, a, b))
        .into_iter()
        .filter(|group| agencies_of(candidates, group).len() >= MIN_DISTINCT_AGENCIES)
        .collect()
}

struct Shape<'a> {
    exemplar: &'a PassageCandidate,
    documents: usize,
    agencies: usize,
    portals: usize,
    named: String,
}

fn shape<'a>(candidates: &'a [PassageCandidate], members: &[usize]) -> Shape<'a> {
    let agencies = agencies_of(candidates, members);
    Shape {
        exemplar: exemplar(candidates, members),
        documents: distinct_documents(candidates, members),
        agencies: agencies.len(),
        portals: agencies
            .keys()
            .map(|key| key.portal.as_str())
            .collect::<HashSet<_>>()
            .len(),
        named: name_agencies(&agencies),
    }
}

/// A document id means nothing without its portal, so the subject always carries both.
fn subject(shape: &Shape) -> FindingSubject {
    FindingSubject {
        portal_key: Some(shape.exemplar.portal_key.clone()),
        document_id: Some(shape.exemplar.document_id),
        ..Default::default()
    }
}

fn id(form: &str, shape: &Shape, subject: &FindingSubject) -> String {
    // The passage offset keeps two reused paragraphs of one letter apart.
    let salient = [
        format!("form={form}"),
        format!("size_bucket={}", shape.documents / 5),
        format!("agencies_bucket={}", shape.agencies),
        format!("portals_bucket={}", shape.portals),
        format!("passage_start={}", shape.exemplar.character_start),
    ];
    let salient: Vec<&str> = salient.iter().map(String::as_str).collect();
    finding_id(
        FindingKind::CrossCorpusSharedLanguage,
        SLUG,
        subject,
        &salient,
    )
}

fn jurisdictions(shape: &Shape) -> String {
    if shape.portals > 1 {
        format!(" in {} jurisdictions", shape.portals)
    } else {
        String::new()
    }
}

fn identical_findings(candidates: &[PassageCandidate], clusters: Vec<SeedCluster>) -> Vec<Finding> {
    let now = Utc::now();
    let mut by_id = BTreeMap::new();
    for cluster in clusters {
        let shape = shape(candidates, &cluster.members);
        let severity = if shape.portals > 1 {
            FindingSeverity::Serious
        } else {
            FindingSeverity::Notable
        };
        let similarity_component = ((cluster.average_similarity - JACCARD_THRESHOLD)
            / (1.0 - JACCARD_THRESHOLD))
            .clamp(0.0, 1.0)
            * 4_000.0;
        let agency_component = ((shape.agencies as f64 / 8.0).min(1.0)) * 3_000.0;
        let size_component = ((shape.documents as f64 / 20.0).min(1.0)) * 3_000.0;
        let bp = (similarity_component + agency_component + size_component).round() as u16;
        let subject = subject(&shape);
        let similarity_pct = (cluster.average_similarity * 100.0).round() as u32;
        let finding = Finding {
            id: id("identical", &shape, &subject),
            kind: FindingKind::CrossCorpusSharedLanguage,
            severity,
            confidence: FindingConfidence::measured(bp),
            subject,
            headline: format!(
                "Near-identical wording appears across {} agencies{}",
                shape.agencies,
                jurisdictions(&shape)
            ),
            explanation: format!(
                "{passages} passages in {documents} public documents share near-identical wording \
                 (average {similarity_pct}% of five-word sequences in common with the first) \
                 across {agencies} agencies{jurisdictions}. Agencies: {named}. Each citation \
                 gives the passage's position in its document. {PATTERN_CAVEAT}",
                passages = cluster.members.len(),
                documents = shape.documents,
                agencies = shape.agencies,
                jurisdictions = jurisdictions(&shape),
                named = shape.named,
            ),
            citations: cite_passages(candidates, &cluster.members),
            discovered_at: now,
            last_seen_at: now,
            dismissed_at: None,
            source_detector: SLUG.into(),
        };
        by_id.entry(finding.id.clone()).or_insert(finding);
    }
    by_id.into_values().collect()
}

fn reworded_findings(candidates: &[PassageCandidate], groups: Vec<Vec<usize>>) -> Vec<Finding> {
    let now = Utc::now();
    let mut by_id = BTreeMap::new();
    for members in groups {
        let shape = shape(candidates, &members);
        let subject = subject(&shape);
        let finding = Finding {
            id: id("reworded", &shape, &subject),
            kind: FindingKind::CrossCorpusSharedLanguage,
            severity: FindingSeverity::Notable,
            confidence: FindingConfidence::Unassessed,
            subject,
            headline: format!(
                "Reworded versions of one passage appear across {} agencies{}",
                shape.agencies,
                jurisdictions(&shape)
            ),
            explanation: format!(
                "{passages} passages in {documents} public documents from {agencies} \
                 agencies{jurisdictions} use largely the same words in different phrasing. Each \
                 shares at least {words:.0}% of its distinct content words with another passage \
                 of the group and less than {sequences:.0}% of its five-word sequences with it, \
                 so these are rewordings, not copies. Agencies: {named}. The {words:.0}% floor is \
                 a chosen cut-off with no measured error rate, so no confidence is given. Each \
                 citation gives the passage's position in its document. {PATTERN_CAVEAT}",
                passages = members.len(),
                documents = shape.documents,
                agencies = shape.agencies,
                jurisdictions = jurisdictions(&shape),
                words = REWORDED_WORD_OVERLAP * 100.0,
                sequences = JACCARD_THRESHOLD * 100.0,
                named = shape.named,
            ),
            citations: cite_passages(candidates, &members),
            discovered_at: now,
            last_seen_at: now,
            dismissed_at: None,
            source_detector: SLUG.into(),
        };
        by_id.entry(finding.id.clone()).or_insert(finding);
    }
    by_id.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::detectors::text_shared::fixtures::{
        corpus, letter, overlapping, stored_slice, Letter, NO_RECORDS_LETTER, REWORDED_LETTERS,
    };

    const AGENCIES: [(i64, &str); 4] = [(1, "Police"), (2, "Clerk"), (3, "Parks"), (4, "Water")];

    /// `count` copies of one letter spread over the first `agencies` agencies of `city-a`.
    fn spread(count: usize, agencies: usize, text: &str) -> Vec<Letter> {
        (0..count)
            .map(|index| letter("city-a", AGENCIES[index % agencies], index as i64 + 1, text))
            .collect()
    }

    #[tokio::test]
    async fn agencies_sharing_an_id_on_different_portals_are_counted_separately() {
        let mut letters = Vec::new();
        for (index, portal) in ["city-a", "city-b"].into_iter().enumerate() {
            for (offset, agency) in [(1, "Police"), (2, "Clerk"), (1, "Police")]
                .into_iter()
                .enumerate()
            {
                let id = (index * 3 + offset) as i64 + 1;
                letters.push(letter(portal, agency, id, NO_RECORDS_LETTER));
            }
        }
        let findings = run(&corpus(letters).await).await.unwrap();
        assert_eq!(findings.len(), 1, "{findings:?}");
        let finding = &findings[0];
        assert!(
            finding.headline.ends_with("4 agencies in 2 jurisdictions"),
            "{}",
            finding.headline
        );
        assert_eq!(finding.severity, FindingSeverity::Serious);
        assert_eq!(finding.subject.portal_key.as_deref(), Some("city-a"));
        assert!(
            finding.explanation.contains(
                "Agencies: Police (city-a); Clerk (city-a); Police (city-b); Clerk (city-b)."
            ),
            "{}",
            finding.explanation
        );
        assert!(finding.explanation.contains(PATTERN_CAVEAT));
        let cited: HashSet<(&str, &str)> = finding
            .citations
            .iter()
            .map(|citation| {
                let label = citation.label.as_deref().unwrap();
                (
                    citation.portal_key.as_str(),
                    label.split(':').next().unwrap(),
                )
            })
            .collect();
        assert_eq!(cited.len(), 4, "every agency is cited: {cited:?}");
    }

    #[tokio::test]
    async fn fires_at_three_agencies_and_five_documents_and_not_below_either() {
        for (count, agencies, expected) in [(6, 2, 0), (6, 3, 1), (6, 4, 1), (4, 3, 0), (5, 3, 1)] {
            let findings = run(&corpus(spread(count, agencies, NO_RECORDS_LETTER)).await)
                .await
                .unwrap();
            assert_eq!(
                findings.len(),
                expected,
                "{count} letters, {agencies} agencies"
            );
        }
    }

    #[tokio::test]
    async fn each_citation_range_slices_the_stored_text_to_the_passage() {
        let body = format!(
            "Señor Núñez — re: café records\n\n{NO_RECORDS_LETTER}\n\nSincerely,\nRecords Officer"
        );
        let repository = corpus(spread(6, 3, &body)).await;
        let findings = run(&repository).await.unwrap();
        assert_eq!(findings.len(), 1, "{findings:?}");
        for citation in &findings[0].citations {
            assert_eq!(stored_slice(&repository, citation).await, NO_RECORDS_LETTER);
        }
    }

    #[tokio::test]
    async fn reworded_reuse_is_reported_at_three_agencies_and_not_at_two() {
        for (agencies, expected) in [(2, 0), (3, 1), (4, 1)] {
            let letters = REWORDED_LETTERS
                .iter()
                .enumerate()
                .map(|(index, text)| {
                    letter("city-a", AGENCIES[index % agencies], index as i64 + 1, text)
                })
                .collect();
            let repository = corpus(letters).await;
            let findings = run(&repository).await.unwrap();
            assert_eq!(
                findings.len(),
                expected,
                "{agencies} agencies: {findings:?}"
            );
            let Some(finding) = findings.first() else {
                continue;
            };
            assert!(finding.headline.starts_with("Reworded versions"));
            assert_eq!(finding.confidence, FindingConfidence::Unassessed);
            assert!(finding
                .explanation
                .contains("Agencies: Police; Clerk; Parks"));
            assert!(finding.explanation.contains(PATTERN_CAVEAT));
            for citation in &finding.citations {
                let passage = stored_slice(&repository, citation).await;
                assert!(REWORDED_LETTERS.contains(&passage.as_str()), "{passage}");
            }
        }
    }

    #[test]
    fn rewording_needs_sixty_percent_of_words_and_under_eighty_five_of_sequences() {
        for (words, sequences, expected) in [
            (59, 0, false),
            (60, 0, true),
            (61, 0, true),
            (100, 84, true),
            (100, 85, false),
            (100, 86, false),
        ] {
            let (word_a, word_b) = overlapping(words, 100);
            let (shingle_a, shingle_b) = overlapping(sequences, 100);
            let sets = Sets {
                words: vec![word_a, word_b],
                shingles: vec![shingle_a, shingle_b],
            };
            assert_eq!(is_reworded(&sets, 0, 1), expected, "{words} {sequences}");
        }
    }

    #[test]
    fn the_identical_threshold_is_inclusive() {
        for (shared, clustered) in [(84, false), (85, true), (86, true)] {
            let (a, b) = overlapping(shared, 100);
            let sets = [a, b];
            let clusters = seed_clusters(&sets, JACCARD_THRESHOLD, |seed| (seed + 1..2).collect());
            assert_eq!(clusters.len() == 1, clustered, "{shared}/100");
        }
    }

    #[tokio::test]
    async fn a_released_record_quoting_the_statute_is_not_reported() {
        let quoted = "Staff report, item 12. Pursuant to Government Code section 54957 the \
            Council may meet in closed session, and under the Public Records Act personnel \
            records are exempt from disclosure where release would be an unwarranted invasion of \
            personal privacy. The department has reviewed the attached budget tables.";
        assert!(run(&corpus(spread(6, 3, quoted)).await)
            .await
            .unwrap()
            .is_empty());
        let addressed = format!("This letter answers your public records request. {quoted}");
        assert_eq!(
            run(&corpus(spread(6, 3, &addressed)).await)
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn finding_id_is_stable_across_reruns() {
        let repository = corpus(spread(6, 3, NO_RECORDS_LETTER)).await;
        let first = run(&repository).await.unwrap();
        let second = run(&repository).await.unwrap();
        assert_eq!(first[0].id, second[0].id);
    }
}
