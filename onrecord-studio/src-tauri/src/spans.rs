//! Spans: durable references to a region of one document's extracted text, and the one resolver
//! that says where such a reference stands against the text as it is now.
//!
//! A span keeps the W3C Web Annotation selector envelope: a position (`start`/`end`, characters
//! into the extracted-text projection), a quote (`exact` plus up to [`SPAN_CONTEXT_CHARACTERS`] of
//! `prefix` and `suffix`), and the digest of the projection it was anchored against. Resolution
//! trusts the position and verifies it against the quote. The result is reported, never written
//! back: a moved passage is `Drifted` and a lost one is `Stale`, and neither silently re-points the
//! stored anchor at other words. Everything here is character-indexed and pure.

use serde::{Deserialize, Serialize};

/// Characters of context kept on each side of a span's exact text.
pub const SPAN_CONTEXT_CHARACTERS: usize = 48;

/// One stored span's selectors, as resolution reads them.
pub struct SpanSelectors<'a> {
    pub start: usize,
    pub exact: &'a str,
    pub prefix: &'a str,
    pub suffix: &'a str,
    /// `None` when the projection the span was anchored against was not recorded.
    pub text_projection_sha256: Option<&'a str>,
}

/// Whether the projection a span was anchored against is the one the document holds now.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProjectionComparison {
    Unchanged,
    Changed,
    /// One of the two digests was not recorded, so no comparison was made.
    NotCompared,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum StaleReason {
    /// The exact text is no longer in the document.
    Missing,
    /// The exact text now occurs more than once and its context does not tell the copies apart.
    Ambiguous,
    /// The document has no extracted text to check against.
    TextUnavailable,
}

/// Where a span stands. Offsets are characters into the current projection.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "state",
    rename_all = "SCREAMING_SNAKE_CASE",
    rename_all_fields = "camelCase"
)]
pub enum SpanResolution {
    /// The exact text is at the recorded position.
    Intact {
        start: u64,
        end: u64,
        projection: ProjectionComparison,
    },
    /// Not at the recorded position; found exactly once elsewhere.
    Drifted { start: u64, end: u64 },
    /// Not located. Never placed on a guess.
    Stale { reason: StaleReason },
}

/// Why a citation has no span to resolve.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum UnanchoredReason {
    /// The citation records no character range.
    NoRange,
    /// The range did not hold the quoted words when it was recorded, the document had no text,
    /// or (for a citation recorded before spans existed) no quote was recorded to check it by.
    NotAnchored,
}

/// One finding citation's standing, paired with the range it was read for so a caller holding
/// an older copy of the finding can tell the two apart.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FindingCitationAnchor {
    pub citation_index: usize,
    pub document_id: i64,
    pub character_start: Option<i64>,
    pub character_end: Option<i64>,
    pub anchor: CitationAnchorState,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "kind",
    rename_all = "SCREAMING_SNAKE_CASE",
    rename_all_fields = "camelCase"
)]
pub enum CitationAnchorState {
    Anchored {
        span_id: String,
        resolution: SpanResolution,
    },
    Unanchored {
        reason: UnanchoredReason,
    },
}

/// Resolves a span against a document's current text and the digest recorded for it.
pub fn resolve_span(
    span: &SpanSelectors,
    current_projection_sha256: Option<&str>,
    text: Option<&str>,
) -> SpanResolution {
    let Some(text) = text else {
        return SpanResolution::Stale {
            reason: StaleReason::TextUnavailable,
        };
    };
    let length = span.exact.chars().count() as u64;
    if let Some(byte) = byte_of(text, span.start) {
        if !span.exact.is_empty() && text[byte..].starts_with(span.exact) {
            let projection = match (span.text_projection_sha256, current_projection_sha256) {
                (Some(anchored), Some(current)) if anchored.eq_ignore_ascii_case(current) => {
                    ProjectionComparison::Unchanged
                }
                (Some(_), Some(_)) => ProjectionComparison::Changed,
                _ => ProjectionComparison::NotCompared,
            };
            return SpanResolution::Intact {
                start: span.start as u64,
                end: span.start as u64 + length,
                projection,
            };
        }
    }
    let selector = Selector {
        quote: span.exact,
        prefix: span.prefix,
        suffix: span.suffix,
        offset: None,
    };
    match reanchor(text, &selector) {
        Outcome::Anchored { start } | Outcome::ReAnchored { start } => SpanResolution::Drifted {
            start: start as u64,
            end: start as u64 + length,
        },
        Outcome::Orphaned(OrphanReason::Missing) => SpanResolution::Stale {
            reason: StaleReason::Missing,
        },
        Outcome::Orphaned(OrphanReason::Ambiguous) => SpanResolution::Stale {
            reason: StaleReason::Ambiguous,
        },
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrphanReason {
    Missing,
    Ambiguous,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Anchored { start: usize },
    ReAnchored { start: usize },
    Orphaned(OrphanReason),
}

pub struct Selector<'a> {
    pub quote: &'a str,
    pub prefix: &'a str,
    pub suffix: &'a str,
    /// Where the quote was when marked, in this text's character positions.
    pub offset: Option<usize>,
}

fn byte_of(text: &str, character: usize) -> Option<usize> {
    if character == 0 {
        return Some(0);
    }
    let mut seen = 0;
    for (byte, _) in text.char_indices() {
        if seen == character {
            return Some(byte);
        }
        seen += 1;
    }
    (seen == character).then_some(text.len())
}

fn character_of(text: &str, byte: usize) -> usize {
    text[..byte].chars().count()
}

/// Byte offsets of up to `limit` occurrences, overlapping ones included: "aa" occurs twice in
/// "aaa", and calling that unique would pick a position arbitrarily.
fn occurrences(text: &str, needle: &str, limit: usize) -> Vec<usize> {
    let mut found = Vec::new();
    let mut from = 0;
    while found.len() < limit {
        let Some(relative) = text[from..].find(needle) else {
            break;
        };
        let at = from + relative;
        found.push(at);
        from = at + text[at..].chars().next().map_or(1, char::len_utf8);
    }
    found
}

fn context_matches(text: &str, byte: usize, selector: &Selector) -> bool {
    text[..byte].ends_with(selector.prefix)
        && text[byte + selector.quote.len()..].starts_with(selector.suffix)
}

/// The one place a stored quote is relocated. In order: the recorded position (quote and context
/// still there), the quote with its context found exactly once, the quote alone found exactly
/// once. Several candidates or none is `Orphaned`; a position is never chosen among equals.
pub fn reanchor(text: &str, selector: &Selector) -> Outcome {
    if selector.quote.is_empty() {
        return Outcome::Orphaned(OrphanReason::Missing);
    }
    if let Some(offset) = selector.offset {
        if let Some(byte) = byte_of(text, offset) {
            if text[byte..].starts_with(selector.quote) && context_matches(text, byte, selector) {
                return Outcome::Anchored { start: offset };
            }
        }
    }
    if !selector.prefix.is_empty() || !selector.suffix.is_empty() {
        let with_context = format!("{}{}{}", selector.prefix, selector.quote, selector.suffix);
        let hits = occurrences(text, &with_context, 2);
        match hits.as_slice() {
            [only] => {
                let start = character_of(text, *only) + selector.prefix.chars().count();
                return Outcome::ReAnchored { start };
            }
            [] => {}
            _ => return Outcome::Orphaned(OrphanReason::Ambiguous),
        }
    }
    let hits = occurrences(text, selector.quote, 2);
    match hits.as_slice() {
        [only] => Outcome::ReAnchored {
            start: character_of(text, *only),
        },
        [] => Outcome::Orphaned(OrphanReason::Missing),
        _ => Outcome::Orphaned(OrphanReason::Ambiguous),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(text: &str, quote: &str, prefix: &str, suffix: &str, offset: Option<usize>) -> Outcome {
        reanchor(
            text,
            &Selector {
                quote,
                prefix,
                suffix,
                offset,
            },
        )
    }

    #[test]
    fn each_outcome_at_its_boundary() {
        let text = "The fee was waived. The contract was amended in May.";
        // Where it was recorded.
        assert_eq!(
            find(
                text,
                "contract was amended",
                "waived. The ",
                " in May",
                Some(24)
            ),
            Outcome::Anchored { start: 24 }
        );
        // Text inserted before it: found once by its context.
        let edited = format!("Draft. {text}");
        assert_eq!(
            find(
                &edited,
                "contract was amended",
                "waived. The ",
                " in May",
                Some(24)
            ),
            Outcome::ReAnchored { start: 31 }
        );
        // Offset past the end is a search, not an error.
        assert_eq!(
            find(
                text,
                "contract was amended",
                "waived. The ",
                " in May",
                Some(9_999)
            ),
            Outcome::ReAnchored { start: 24 }
        );
        // Quote removed.
        assert_eq!(
            find("Nothing here.", "contract was amended", "", "", Some(3)),
            Outcome::Orphaned(OrphanReason::Missing)
        );
        // Quote repeated and context no longer separates them: never picked.
        let repeated = "see fee. see fee.";
        assert_eq!(
            find(repeated, "see fee", "", "", Some(40)),
            Outcome::Orphaned(OrphanReason::Ambiguous)
        );
    }

    #[test]
    fn overlapping_repeats_are_ambiguous_and_context_separates_them() {
        assert_eq!(
            find("aaa", "aa", "", "", None),
            Outcome::Orphaned(OrphanReason::Ambiguous)
        );
        assert_eq!(
            find("x one y one z", "one", "x ", " y", None),
            Outcome::ReAnchored { start: 2 }
        );
        // Context changed around the one remaining copy: falls back to the quote alone.
        assert_eq!(
            find("q one r", "one", "x ", " y", None),
            Outcome::ReAnchored { start: 2 }
        );
    }

    #[test]
    fn positions_count_characters_not_bytes() {
        let text = "\u{e9}\u{e9}\u{e9} target \u{1f600} end";
        assert_eq!(
            find(text, "target", "", "", Some(4)),
            Outcome::Anchored { start: 4 }
        );
        assert_eq!(
            find(text, "end", "", "", None),
            Outcome::ReAnchored { start: 13 }
        );
    }

    fn span<'a>(
        start: usize,
        exact: &'a str,
        prefix: &'a str,
        suffix: &'a str,
    ) -> SpanSelectors<'a> {
        SpanSelectors {
            start,
            exact,
            prefix,
            suffix,
            text_projection_sha256: Some(ANCHORED_DIGEST),
        }
    }

    const ANCHORED_DIGEST: &str =
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const OTHER_DIGEST: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const TEXT: &str = "The fee was waived. The contract was amended in May.";

    #[test]
    fn a_span_at_its_position_is_intact_and_says_whether_the_projection_changed() {
        let cited = span(24, "contract was amended", "waived. The ", " in May");
        assert_eq!(
            resolve_span(&cited, Some(ANCHORED_DIGEST), Some(TEXT)),
            SpanResolution::Intact {
                start: 24,
                end: 44,
                projection: ProjectionComparison::Unchanged
            }
        );
        // Re-extracted text that left the passage where it was: still intact, and said so.
        let changed_after = format!("{TEXT} Appendix follows.");
        assert_eq!(
            resolve_span(&cited, Some(OTHER_DIGEST), Some(&changed_after)),
            SpanResolution::Intact {
                start: 24,
                end: 44,
                projection: ProjectionComparison::Changed
            }
        );
        let unrecorded = SpanSelectors {
            text_projection_sha256: None,
            ..cited
        };
        assert_eq!(
            resolve_span(&unrecorded, Some(OTHER_DIGEST), Some(TEXT)),
            SpanResolution::Intact {
                start: 24,
                end: 44,
                projection: ProjectionComparison::NotCompared
            }
        );
    }

    #[test]
    fn a_moved_passage_found_once_is_drifted_and_a_repeated_one_is_stale() {
        let cited = span(24, "contract was amended", "waived. The ", " in May");
        let shifted = format!("Draft. {TEXT}");
        assert_eq!(
            resolve_span(&cited, Some(OTHER_DIGEST), Some(&shifted)),
            SpanResolution::Drifted { start: 31, end: 51 }
        );
        // Two copies whose context no longer separates them: never placed on either.
        let repeated = "contract was amended. contract was amended.";
        assert_eq!(
            resolve_span(&cited, Some(OTHER_DIGEST), Some(repeated)),
            SpanResolution::Stale {
                reason: StaleReason::Ambiguous
            }
        );
        // The same two copies, told apart by the recorded context: drifted to the right one.
        let separated = "x contract was amended y. waived. The contract was amended in May.";
        assert_eq!(
            resolve_span(&cited, Some(OTHER_DIGEST), Some(separated)),
            SpanResolution::Drifted { start: 38, end: 58 }
        );
    }

    #[test]
    fn removed_text_and_absent_text_are_stale_with_their_reasons() {
        let cited = span(24, "contract was amended", "waived. The ", " in May");
        assert_eq!(
            resolve_span(&cited, Some(OTHER_DIGEST), Some("The fee was waived.")),
            SpanResolution::Stale {
                reason: StaleReason::Missing
            }
        );
        // Other words now occupy the recorded offsets: not intact, and not pointed at them.
        let replaced = "The fee was waived. The ordinance was repealed in May.";
        assert_eq!(
            resolve_span(&cited, Some(OTHER_DIGEST), Some(replaced)),
            SpanResolution::Stale {
                reason: StaleReason::Missing
            }
        );
        assert_eq!(
            resolve_span(&cited, Some(ANCHORED_DIGEST), None),
            SpanResolution::Stale {
                reason: StaleReason::TextUnavailable
            }
        );
    }

    #[test]
    fn span_offsets_count_characters_across_multibyte_text() {
        let text = "\u{e9}t\u{e9} \u{1f600} fee waived \u{4e2d}\u{6587} end";
        let cited = span(6, "fee waived", "", "");
        assert_eq!(
            resolve_span(&cited, None, Some(text)),
            SpanResolution::Intact {
                start: 6,
                end: 16,
                projection: ProjectionComparison::NotCompared
            }
        );
        let drifted = format!("\u{1f600}{text}");
        assert_eq!(
            resolve_span(&cited, None, Some(&drifted)),
            SpanResolution::Drifted { start: 7, end: 17 }
        );
        // A start past the end, or one landing inside the text, never panics on a char boundary.
        let beyond = span(9_999, "fee waived", "", "");
        assert_eq!(
            resolve_span(&beyond, None, Some(text)),
            SpanResolution::Drifted { start: 6, end: 16 }
        );
        let multibyte_exact = span(17, "\u{4e2d}\u{6587}", "", "");
        assert_eq!(
            resolve_span(&multibyte_exact, None, Some(text)),
            SpanResolution::Intact {
                start: 17,
                end: 19,
                projection: ProjectionComparison::NotCompared
            }
        );
    }
}
