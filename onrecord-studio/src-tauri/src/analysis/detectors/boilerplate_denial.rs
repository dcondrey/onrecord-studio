//! Boilerplate-denial detector.
//!
//! Surfaces clusters of passages with denial wording that is near-identical across requests on
//! different topics, and names the agencies the requests were made to.
//!
//! **Passages**: nothing stored labels a document as a denial letter, so candidates are the
//! paragraphs around the phrases in `RULES`, up to `MAX_PASSAGES_PER_DOCUMENT` per document, each
//! compared on its own.
//!
//! **Similarity**: Jaccard over hashed five-word shingles, deterministic and offline. Candidate
//! pairs come from MinHash bands (`BANDING_FROM_0_60`); exact Jaccard decides.
//!
//! **What this is not**: a legal finding, evidence of coordination, or a showing that a request
//! was not reviewed individually. `FindingKind::BoilerplateDenial` is a pattern signal.

use super::text_shared::{
    agencies_of, agency_label, all_words, cite_passages, distinct_documents, distinct_titles,
    exemplar, load_passages, name_agencies, seed_clusters, shingles, BandIndex, PassageCandidate,
    PassageRules, SeedCluster, BANDING_FROM_0_60, PATTERN_CAVEAT,
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

pub const SLUG: &str = "boilerplate_denial";

const SHINGLE_WORDS: usize = 5;

/// Jaccard at or above which two passages count as the same wording.
const JACCARD_THRESHOLD: f64 = 0.60;

/// Minimum distinct documents in a cluster.
const MIN_CLUSTER_SIZE: usize = 5;

/// Minimum distinct request titles across a cluster. Identical denials on one topic are weaker
/// than identical denials on several.
const MIN_DISTINCT_TOPICS: usize = 3;

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
    ],
    half_window: 300,
};

pub async fn run(repository: &DuckDbRepository) -> Result<Vec<Finding>, AppError> {
    let candidates = load_passages(repository, "Boilerplate-denial passage query", &RULES).await?;
    let sets = shingle_sets(&candidates);
    let index = BandIndex::build(&sets, BANDING_FROM_0_60);
    let clusters = qualifying(
        &candidates,
        seed_clusters(&sets, JACCARD_THRESHOLD, |seed| index.later(seed)),
    );
    let mut findings = build_findings(&candidates, clusters);
    findings.sort_by_key(|f| std::cmp::Reverse(f.confidence.rank_weight()));
    findings.truncate(MAX_FINDINGS);
    Ok(findings)
}

fn shingle_sets(candidates: &[PassageCandidate]) -> Vec<HashSet<u64>> {
    candidates
        .iter()
        .map(|candidate| shingles(&all_words(&candidate.excerpt), SHINGLE_WORDS))
        .collect()
}

fn qualifying(candidates: &[PassageCandidate], clusters: Vec<SeedCluster>) -> Vec<SeedCluster> {
    clusters
        .into_iter()
        .filter(|cluster| {
            distinct_documents(candidates, &cluster.members) >= MIN_CLUSTER_SIZE
                && distinct_titles(candidates, &cluster.members) >= MIN_DISTINCT_TOPICS
        })
        .collect()
}

fn build_findings(candidates: &[PassageCandidate], clusters: Vec<SeedCluster>) -> Vec<Finding> {
    let now = Utc::now();
    let mut by_id = BTreeMap::new();
    for cluster in clusters {
        let exemplar = exemplar(candidates, &cluster.members);
        let cluster_size = distinct_documents(candidates, &cluster.members);
        let distinct_topics = distinct_titles(candidates, &cluster.members);
        let agencies = agencies_of(candidates, &cluster.members);
        let unattributed = cluster
            .members
            .iter()
            .filter(|&&member| candidates[member].agency.is_none())
            .count();
        // One agency only when every member is attributed to it; a member with no recorded agency
        // may belong to any.
        let single_agency = (agencies.len() == 1 && unattributed == 0)
            .then(|| agencies.iter().next())
            .flatten();

        let severity = if agencies.len() > 1 {
            FindingSeverity::Serious
        } else {
            FindingSeverity::Notable
        };

        let size_component = ((cluster_size as f64 / 20.0).min(1.0)) * 3_000.0;
        let similarity_component = ((cluster.average_similarity - JACCARD_THRESHOLD)
            / (1.0 - JACCARD_THRESHOLD))
            .clamp(0.0, 1.0)
            * 4_000.0;
        let topic_component = ((distinct_topics as f64 / 10.0).min(1.0)) * 3_000.0;
        let bp = (size_component + similarity_component + topic_component).round() as u16;

        let subject = FindingSubject {
            portal_key: Some(exemplar.portal_key.clone()),
            agency_id: single_agency.map(|(key, _)| key.id),
            document_id: Some(exemplar.document_id),
            ..Default::default()
        };

        // Size and topic counts are bucketed so one more copy of the same denial upserts the same
        // row. The passage offset keeps two reused paragraphs of one letter apart.
        let salient = [
            format!("size_bucket={}", cluster_size / 10),
            format!("topics_bucket={}", distinct_topics / 5),
            format!("agencies={}", agencies.len()),
            format!("passage_start={}", exemplar.character_start),
        ];
        let salient: Vec<&str> = salient.iter().map(String::as_str).collect();
        let id = finding_id(FindingKind::BoilerplateDenial, SLUG, &subject, &salient);

        let (headline, who) = match single_agency {
            Some((key, name)) => {
                let agency = agency_label(key, name.as_deref(), false);
                (
                    format!(
                        "Denial wording repeats near-verbatim across {cluster_size} requests to \
                         {agency}"
                    ),
                    format!("All were made to {agency}."),
                )
            }
            None if agencies.is_empty() => (
                format!(
                    "Denial wording repeats near-verbatim across {cluster_size} requests with no \
                     recorded agency"
                ),
                "No agency is recorded for these requests.".to_string(),
            ),
            None => {
                let unattributed_note = if unattributed > 0 {
                    format!(" {unattributed} passages are on requests with no recorded agency.")
                } else {
                    String::new()
                };
                (
                    format!(
                        "Same denial wording appears across {cluster_size} requests to {} agencies",
                        agencies.len()
                    ),
                    format!("Agencies: {}.{unattributed_note}", name_agencies(&agencies)),
                )
            }
        };

        let similarity_pct = (cluster.average_similarity * 100.0).round() as u32;
        let explanation = format!(
            "{passages} passages in {cluster_size} public documents share near-identical wording \
             (average {similarity_pct}% of five-word sequences in common with the first) across \
             {distinct_topics} distinct request titles. {who} Each citation gives the passage's \
             position in its document. {PATTERN_CAVEAT}",
            passages = cluster.members.len(),
        );

        let finding = Finding {
            id,
            kind: FindingKind::BoilerplateDenial,
            severity,
            confidence: FindingConfidence::measured(bp),
            subject,
            headline,
            explanation,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::detectors::text_shared::fixtures::{
        corpus, letter, overlapping, stored_slice, Letter, NO_RECORDS_LETTER,
    };

    const DISTINCT_DENIAL: &str = "After conducting a search across multiple databases, the \
        office has no responsive records matching the terms described, and accordingly nothing \
        will be produced under this cover for the period named in the correspondence.";

    fn letters(count: i64, text: &str) -> Vec<Letter> {
        (1..=count)
            .map(|id| letter("city-a", (7, "Police Department"), id, text))
            .collect()
    }

    #[tokio::test]
    async fn agencies_sharing_an_id_on_different_portals_are_counted_separately() {
        let letters = (1..=6)
            .map(|id| {
                let portal = if id % 2 == 0 { "city-a" } else { "city-b" };
                letter(portal, (1, "Police"), id, NO_RECORDS_LETTER)
            })
            .collect();
        let findings = run(&corpus(letters).await).await.unwrap();
        assert_eq!(findings.len(), 1, "{findings:?}");
        let finding = &findings[0];
        assert!(
            finding.headline.contains("2 agencies"),
            "{}",
            finding.headline
        );
        assert_eq!(finding.severity, FindingSeverity::Serious);
        assert_eq!(finding.subject.agency_id, None);
        assert!(
            finding
                .explanation
                .contains("Agencies: Police (city-a); Police (city-b)."),
            "{}",
            finding.explanation
        );
    }

    #[tokio::test]
    async fn one_agency_is_named_and_the_finding_states_what_it_does_not_show() {
        let findings = run(&corpus(letters(5, NO_RECORDS_LETTER)).await)
            .await
            .unwrap();
        assert_eq!(findings.len(), 1, "{findings:?}");
        let finding = &findings[0];
        assert_eq!(finding.kind, FindingKind::BoilerplateDenial);
        assert_eq!(finding.severity, FindingSeverity::Notable);
        assert_eq!(finding.subject.agency_id, Some(7));
        assert!(finding
            .headline
            .ends_with("5 requests to Police Department"));
        assert!(finding.explanation.contains(PATTERN_CAVEAT));
    }

    #[tokio::test]
    async fn a_blank_agency_name_is_reported_as_missing_not_as_a_name() {
        let letters = (1..=5)
            .map(|id| letter("city-a", (7, " "), id, NO_RECORDS_LETTER))
            .collect();
        let findings = run(&corpus(letters).await).await.unwrap();
        assert!(
            findings[0]
                .headline
                .ends_with("an agency with no recorded name (id 7 on city-a)"),
            "{}",
            findings[0].headline
        );
    }

    #[tokio::test]
    async fn requests_without_an_agency_are_not_attributed_to_one() {
        let mut letters = letters(3, NO_RECORDS_LETTER);
        for id in 4..=5 {
            letters.push(Letter {
                agency: None,
                ..letter("city-a", (0, ""), id, NO_RECORDS_LETTER)
            });
        }
        let findings = run(&corpus(letters).await).await.unwrap();
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].subject.agency_id, None);
        assert!(findings[0]
            .explanation
            .contains("2 passages are on requests with no recorded agency"));
    }

    #[tokio::test]
    async fn each_citation_range_slices_the_stored_text_to_the_passage() {
        let body = format!(
            "Señor Núñez — re: café records, naïve résumé file\n\n{NO_RECORDS_LETTER}\n\n\
             Sincerely,\nRecords Officer"
        );
        let repository = corpus(letters(5, &body)).await;
        let findings = run(&repository).await.unwrap();
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].citations.len(), 5);
        for citation in &findings[0].citations {
            let sliced = stored_slice(&repository, citation).await;
            assert_eq!(sliced, NO_RECORDS_LETTER);
            assert!(citation
                .label
                .as_deref()
                .unwrap()
                .starts_with("Police Department: "));
        }
    }

    #[tokio::test]
    async fn two_reused_paragraphs_in_one_letter_are_two_clusters() {
        let second = "The remaining portion of your request seeks personnel evaluations. Those \
            records are exempt from disclosure because releasing them would be an unwarranted \
            invasion of personal privacy, and they are withheld in full on that ground.";
        let letters = (1..=5)
            .map(|id| {
                let text = format!(
                    "{NO_RECORDS_LETTER}\n\nReference number {id} concerns a matter unlike any \
                     other in this set, described here in words chosen for letter {id} only.\n\n\
                     {second}"
                );
                letter("city-a", (7, "Police Department"), id, &text)
            })
            .collect();
        let repository = corpus(letters).await;
        let findings = run(&repository).await.unwrap();
        assert_eq!(findings.len(), 2, "{findings:?}");
        let mut passages = Vec::new();
        for finding in &findings {
            passages.push(stored_slice(&repository, &finding.citations[0]).await);
        }
        passages.sort();
        assert_eq!(passages, [second, NO_RECORDS_LETTER]);
    }

    #[tokio::test]
    async fn a_released_record_quoting_the_statute_is_not_denial_wording() {
        let quoted = "Section 4.2 of the records manual restates the statute: personnel, medical \
            and similar records are exempt from disclosure where release would be an unwarranted \
            invasion of personal privacy, and such files are not readily available to the public \
            counter.";
        assert!(run(&corpus(letters(5, quoted)).await)
            .await
            .unwrap()
            .is_empty());
        let addressed = format!("We write in response to your request of last month. {quoted}");
        assert_eq!(
            run(&corpus(letters(5, &addressed)).await)
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn fires_at_five_documents_on_three_topics_and_not_below_either() {
        let titled = |count: i64, topics: i64| -> Vec<Letter> {
            (1..=count)
                .map(|id| Letter {
                    title: format!("Topic {}", id % topics),
                    ..letter("city-a", (7, "Police Department"), id, NO_RECORDS_LETTER)
                })
                .collect()
        };
        for (count, topics, expected) in [(4, 4, 0), (5, 5, 1), (6, 6, 1), (6, 2, 0), (6, 3, 1)] {
            let findings = run(&corpus(titled(count, topics)).await).await.unwrap();
            assert_eq!(findings.len(), expected, "{count} letters, {topics} topics");
        }
    }

    #[tokio::test]
    async fn differently_worded_denials_do_not_cluster() {
        let mut letters = letters(3, NO_RECORDS_LETTER);
        for id in 4..=6 {
            letters.push(letter(
                "city-a",
                (7, "Police Department"),
                id,
                DISTINCT_DENIAL,
            ));
        }
        assert!(run(&corpus(letters).await).await.unwrap().is_empty());
    }

    #[test]
    fn the_similarity_threshold_is_inclusive() {
        for (shared, clustered) in [(59, false), (60, true), (61, true)] {
            let (a, b) = overlapping(shared, 100);
            let sets = [a, b];
            let clusters = seed_clusters(&sets, JACCARD_THRESHOLD, |seed| (seed + 1..2).collect());
            assert_eq!(clusters.len() == 1, clustered, "{shared}/100");
        }
    }

    #[tokio::test]
    async fn finding_id_is_stable_across_reruns() {
        let repository = corpus(letters(5, NO_RECORDS_LETTER)).await;
        let first = run(&repository).await.unwrap();
        let second = run(&repository).await.unwrap();
        assert_eq!(first[0].id, second[0].id);
    }
}
