//! Redaction-consistency detector.
//!
//! The same record is often released more than once: to different requesters, or again after a
//! reconsideration. When two public copies of the same file name in one portal differ in text,
//! this detector looks for a passage one copy redacts (a `[REDACTED]` label, a black block, or a
//! `withheld under exemption` note) that the other copy leaves readable, by matching the words
//! on either side of the redaction against the other copy.
//!
//! A hit is a pattern signal: two releases made different redaction decisions about one passage.
//! Which decision was right is not decided here. A later release may apply an exemption the
//! earlier one missed, a requester may have been entitled to more, or the copies may not be the
//! same version of the record at all; the finding says so.

use super::public_documents::{load_public_documents, PublicDocument};
use crate::{
    analysis::{
        cluster::{redaction_spans, word_tokens},
        finding::{
            finding_id, Finding, FindingCitation, FindingConfidence, FindingKind, FindingSeverity,
            FindingSubject,
        },
    },
    error::AppError,
    storage::DuckDbRepository,
};
use chrono::Utc;
use std::collections::BTreeMap;

pub const SLUG: &str = "redaction_inconsistency";

const MAX_DOCUMENTS: i64 = 2_000;
/// Copies of one file name compared per portal; more than this is a form, not a record.
const MAX_COPIES_PER_NAME: usize = 6;
const MAX_PAIRS: usize = 200;
const MAX_SPANS_PER_DOCUMENT: usize = 60;
const MAX_FINDINGS: usize = 50;
/// Words matched on each side of a redaction. Fewer and a common phrase would match anywhere.
const CONTEXT_WORDS: usize = 5;
const MIN_CONTEXT_WORDS: usize = 3;
/// The readable passage may be this many words at most; a longer gap is a different document.
const MAX_GAP_WORDS: usize = 40;

/// A passage redacted in one copy and readable in the other.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Divergence {
    /// Byte range of the redaction in the withholding copy.
    pub withheld_start: usize,
    pub withheld_end: usize,
    /// Byte range of the readable passage in the releasing copy.
    pub released_start: usize,
    pub released_end: usize,
}

/// Passages `withholding` redacts that `releasing` leaves readable.
pub fn divergences(withholding: &str, releasing: &str) -> Vec<Divergence> {
    let spans = redaction_spans(withholding);
    if spans.is_empty() {
        return Vec::new();
    }
    let withheld_words = word_tokens(withholding);
    let released_words = word_tokens(releasing);
    let released_spans = redaction_spans(releasing);
    let mut found = Vec::new();
    for span in spans.iter().take(MAX_SPANS_PER_DOCUMENT) {
        let before_end = withheld_words.partition_point(|w| w.end <= span.start);
        let after_start = withheld_words.partition_point(|w| w.start < span.end);
        let before: Vec<&str> = withheld_words
            [before_end.saturating_sub(CONTEXT_WORDS)..before_end]
            .iter()
            .map(|w| w.normalized.as_str())
            .collect();
        let after: Vec<&str> = withheld_words
            [after_start..(after_start + CONTEXT_WORDS).min(withheld_words.len())]
            .iter()
            .map(|w| w.normalized.as_str())
            .collect();
        if before.len() < MIN_CONTEXT_WORDS || after.len() < MIN_CONTEXT_WORDS {
            continue;
        }
        let Some(gap) = readable_gap(&released_words, &before, &after) else {
            continue;
        };
        let (released_start, released_end) = gap;
        // The other copy must actually read here: a redaction of its own in the gap is agreement.
        if released_spans
            .iter()
            .any(|s| s.start < released_end && s.end > released_start)
        {
            continue;
        }
        found.push(Divergence {
            withheld_start: span.start,
            withheld_end: span.end,
            released_start,
            released_end,
        });
    }
    found
}

/// Byte range of the words between the first place `before` is followed, within
/// `MAX_GAP_WORDS`, by `after`; `None` when the contexts do not both occur in that order.
fn readable_gap(
    words: &[crate::analysis::cluster::WordToken<'_>],
    before: &[&str],
    after: &[&str],
) -> Option<(usize, usize)> {
    let matches_at = |index: usize, phrase: &[&str]| {
        words
            .get(index..index + phrase.len())
            .is_some_and(|window| window.iter().zip(phrase).all(|(w, p)| w.normalized == *p))
    };
    (0..words.len()).find_map(|start| {
        if !matches_at(start, before) {
            return None;
        }
        let gap_start = start + before.len();
        (1..=MAX_GAP_WORDS).find_map(|gap| {
            let after_at = gap_start + gap;
            matches_at(after_at, after).then(|| (words[gap_start].start, words[after_at - 1].end))
        })
    })
}

fn char_offset(text: &str, byte: usize) -> i64 {
    text[..byte.min(text.len())].chars().count() as i64
}

fn normalized_name(document: &PublicDocument) -> Option<String> {
    let name = document.file_name.as_deref()?.trim().to_lowercase();
    (!name.is_empty()).then_some(name)
}

pub async fn run(repository: &DuckDbRepository) -> Result<Vec<Finding>, AppError> {
    let documents = load_public_documents(repository, MAX_DOCUMENTS).await?;
    let mut groups: BTreeMap<(String, String), Vec<&PublicDocument>> = BTreeMap::new();
    for document in &documents {
        if let Some(name) = normalized_name(document) {
            groups
                .entry((document.portal_key.clone(), name))
                .or_default()
                .push(document);
        }
    }
    let mut findings = Vec::new();
    let mut pairs = 0usize;
    'groups: for copies in groups.values() {
        if copies.len() < 2 {
            continue;
        }
        let copies = &copies[..copies.len().min(MAX_COPIES_PER_NAME)];
        for (index, withholding) in copies.iter().enumerate() {
            for releasing in copies
                .iter()
                .skip(index + 1)
                .chain(copies.iter().take(index))
            {
                if pairs >= MAX_PAIRS || findings.len() >= MAX_FINDINGS {
                    break 'groups;
                }
                if withholding.request_id == releasing.request_id
                    || withholding.text == releasing.text
                {
                    continue;
                }
                pairs += 1;
                let found = divergences(&withholding.text, &releasing.text);
                if found.is_empty() {
                    continue;
                }
                findings.push(to_finding(withholding, releasing, &found));
            }
        }
    }
    Ok(findings)
}

fn to_finding(
    withholding: &PublicDocument,
    releasing: &PublicDocument,
    found: &[Divergence],
) -> Finding {
    let subject = FindingSubject {
        portal_key: Some(withholding.portal_key.clone()),
        agency_id: None,
        request_id: Some(withholding.request_id),
        document_id: Some(withholding.document_id),
        entity_id: None,
    };
    let salient = format!("{}:{}", releasing.document_id, found.len());
    let id = finding_id(
        FindingKind::RedactionInconsistentAcrossReleases,
        SLUG,
        &subject,
        &[&salient],
    );
    let name = withholding
        .file_name
        .clone()
        .unwrap_or_else(|| format!("document {}", withholding.document_id));
    let count = found.len();
    let headline = format!(
        "'{name}' redacts {count} passage{} that another release of the same file leaves readable",
        if count == 1 { "" } else { "s" }
    );
    let explanation = format!(
        "Two public copies of '{name}' in this portal, released under request {} and request {}, \
         differ in what they redact: {count} passage{} marked as redacted or withheld in the \
         first copy read as ordinary text in the second, matched by the words on either side. \
         This records that two releases made different redaction decisions about the same \
         passage. It does not say which decision was right: a later release may apply an \
         exemption the earlier one did not, a requester may have been entitled to more, or the \
         two copies may be different versions of the record. Compare the copies before drawing \
         any further inference.",
        withholding.request_id,
        releasing.request_id,
        if count == 1 { "" } else { "s" },
    );
    let mut citations = Vec::new();
    for divergence in found.iter().take(3) {
        citations.push(FindingCitation {
            portal_key: withholding.portal_key.clone(),
            document_id: withholding.document_id,
            character_start: Some(char_offset(&withholding.text, divergence.withheld_start)),
            character_end: Some(char_offset(&withholding.text, divergence.withheld_end)),
            quote: Some(
                withholding.text[divergence.withheld_start..divergence.withheld_end]
                    .chars()
                    .take(120)
                    .collect(),
            ),
            label: Some(format!(
                "{name} (request {}), redacted",
                withholding.request_id
            )),
        });
        citations.push(FindingCitation {
            portal_key: releasing.portal_key.clone(),
            document_id: releasing.document_id,
            character_start: Some(char_offset(&releasing.text, divergence.released_start)),
            character_end: Some(char_offset(&releasing.text, divergence.released_end)),
            quote: Some(
                releasing.text[divergence.released_start..divergence.released_end]
                    .chars()
                    .take(120)
                    .collect(),
            ),
            label: Some(format!(
                "{} (request {}), readable",
                releasing
                    .file_name
                    .clone()
                    .unwrap_or_else(|| format!("document {}", releasing.document_id)),
                releasing.request_id
            )),
        });
    }
    let severity = if count >= 3 {
        FindingSeverity::Serious
    } else {
        FindingSeverity::Notable
    };
    // Derived from what was observed: how many passages diverged, each matched on both sides.
    let confidence = FindingConfidence::measured(5000 + (count.min(6) as u16) * 500);
    let now = Utc::now();
    Finding {
        id,
        kind: FindingKind::RedactionInconsistentAcrossReleases,
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

    const RELEASED: &str = "Memorandum to the City Manager. The vendor selected for the contract was Harbor Services Inc and the award amount was set at the meeting. Staff recommend approval.";
    const WITHHELD: &str = "Memorandum to the City Manager. The vendor selected for the contract was [REDACTED] and the award amount was set at the meeting. Staff recommend approval.";

    #[test]
    fn a_passage_redacted_in_one_copy_and_readable_in_the_other_is_found() {
        let found = divergences(WITHHELD, RELEASED);
        assert_eq!(found.len(), 1, "{found:?}");
        let released = &RELEASED[found[0].released_start..found[0].released_end];
        assert_eq!(released, "Harbor Services Inc");
        assert_eq!(
            &WITHHELD[found[0].withheld_start..found[0].withheld_end],
            "[REDACTED]"
        );
    }

    #[test]
    fn agreement_and_unrelated_text_yield_nothing() {
        // Both copies redact the same passage.
        assert!(divergences(WITHHELD, WITHHELD).is_empty());
        // The other copy does not carry the surrounding words at all.
        assert!(divergences(
            WITHHELD,
            "An unrelated notice about street sweeping schedules for the spring season."
        )
        .is_empty());
        // The withholding copy is the readable one: nothing is redacted in it.
        assert!(divergences(RELEASED, WITHHELD).is_empty());
        // The gap is too long to be the same passage.
        let long_gap = format!(
            "Memorandum to the City Manager. The vendor selected for the contract was {} and the award amount was set at the meeting.",
            "word ".repeat(60)
        );
        assert!(divergences(WITHHELD, &long_gap).is_empty());
    }

    async fn seeded() -> DuckDbRepository {
        let repository = DuckDbRepository::new(":memory:").expect("open");
        repository
            .run_migrations(include_str!("../../../migrations/0001_initial_schema.sql"))
            .await
            .expect("migrate");
        let mut sql = String::from(
            "INSERT INTO portals (subdomain, display_name, base_url) VALUES ('sd', 'SD', 'https://sd.example');
             INSERT INTO requests (id, portal_subdomain, title, status, created_at, updated_at)
               VALUES (1, 'sd', 'First', 'closed', '2026-01-01T00:00:00Z', '2026-01-02T00:00:00Z');
             INSERT INTO requests (id, portal_subdomain, title, status, created_at, updated_at)
               VALUES (2, 'sd', 'Second', 'closed', '2026-02-01T00:00:00Z', '2026-02-02T00:00:00Z');",
        );
        for (id, request, text) in [(1, 1, WITHHELD), (2, 2, RELEASED)] {
            sql.push_str(&format!(
                "INSERT INTO documents (id, request_id, portal_subdomain, file_name, visibility, created_at, updated_at, extracted_text_sha256)
                   VALUES ({id}, {request}, 'sd', 'Memo.pdf', 'public', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', 'h{id}');
                 INSERT INTO documents_fts (document_id, portal_subdomain, file_name, extracted_text)
                   VALUES ({id}, 'sd', 'Memo.pdf', '{text}');"
            ));
        }
        repository
            .query_connection("redaction fixture", move |connection| {
                connection
                    .execute_batch(&sql)
                    .map_err(|error| AppError::Storage(format!("fixture: {error}")))
            })
            .await
            .expect("seed");
        repository
    }

    #[tokio::test]
    async fn two_releases_of_one_file_name_are_compared_in_both_directions() {
        let repository = seeded().await;
        let findings = run(&repository).await.unwrap();
        assert_eq!(findings.len(), 1, "{findings:?}");
        let finding = &findings[0];
        assert_eq!(
            finding.kind,
            FindingKind::RedactionInconsistentAcrossReleases
        );
        assert_eq!(finding.subject.document_id, Some(1));
        assert_eq!(finding.citations.len(), 2);
        assert_eq!(
            finding.citations[1].quote.as_deref(),
            Some("Harbor Services Inc")
        );
        assert!(finding.headline.contains("1 passage that another release"));
        assert!(!finding.explanation.to_lowercase().contains("unlawful"));
    }
}
