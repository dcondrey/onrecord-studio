//! Finding a stored quote again in a document's text.
//!
//! Reviewers select text in the masked view, so quotes and context are masked text. Positions are
//! stored against the unmasked stored text (the text the document's digest describes);
//! `TextView` maps between the two. Everything here is character-indexed and pure.

use crate::error::AppError;
use crate::spans::{reanchor, OrphanReason, Outcome, Selector};
use crate::storage::pii::mask_pii;
use crate::types::annotation::*;

/// A masked token standing in for source characters, in character units.
#[derive(Debug, Clone, Copy)]
struct Span {
    masked_start: usize,
    masked_length: usize,
    source_start: usize,
    source_length: usize,
}

/// A document's text as the viewer shows it (personal data masked) with the mapping back to the
/// stored text.
pub struct TextView {
    pub masked: String,
    spans: Vec<Span>,
}

impl TextView {
    pub fn new(source: &str) -> Result<Self, AppError> {
        let result = mask_pii(source)?;
        let mut spans = Vec::with_capacity(result.findings.len());
        let mut shift: isize = 0;
        for finding in &result.findings {
            let masked_length = finding.kind.replacement().chars().count();
            spans.push(Span {
                masked_start: (finding.character_start as isize + shift) as usize,
                masked_length,
                source_start: finding.character_start,
                source_length: finding.matched_character_count,
            });
            shift += masked_length as isize - finding.matched_character_count as isize;
        }
        Ok(Self {
            masked: result.masked_text,
            spans,
        })
    }

    /// A stored position as a masked-text position. Inside a mask it is the mask's start.
    pub fn to_masked(&self, source: usize) -> usize {
        let before = self
            .spans
            .partition_point(|span| span.source_start + span.source_length <= source);
        let mut shifted = source;
        for span in &self.spans[..before] {
            shifted = shifted + span.masked_length - span.source_length;
        }
        match self.spans.get(before) {
            Some(span) if span.source_start < source => span.masked_start,
            _ => shifted,
        }
    }

    /// A masked-text position as a stored position. `end` rounds a position inside a mask out to
    /// the end of what it hides; otherwise to its start.
    pub fn to_source(&self, masked: usize, end: bool) -> usize {
        let before = self
            .spans
            .partition_point(|span| span.masked_start + span.masked_length <= masked);
        let mut shifted = masked;
        for span in &self.spans[..before] {
            shifted = shifted + span.source_length - span.masked_length;
        }
        match self.spans.get(before) {
            Some(span) if span.masked_start < masked => {
                if end {
                    span.source_start + span.source_length
                } else {
                    span.source_start
                }
            }
            _ => shifted,
        }
    }

    fn context_around(&self, start: usize, length: usize) -> (String, String) {
        let characters: Vec<char> = self.masked.chars().collect();
        let end = (start + length).min(characters.len());
        let prefix_from = start.saturating_sub(CONTEXT_CHARACTERS);
        let suffix_to = (end + CONTEXT_CHARACTERS).min(characters.len());
        (
            characters[prefix_from..start].iter().collect(),
            characters[end..suffix_to].iter().collect(),
        )
    }

    /// Resolves a selection made in this view to where it will be stored: the position in the
    /// stored text and the context to keep. Anything not found exactly once is refused, so a
    /// repeated passage cannot be filed under the wrong occurrence.
    pub fn place_selection(
        &self,
        quote: &str,
        prefix: &str,
        suffix: &str,
    ) -> Result<Placement, AppError> {
        let selector = Selector {
            quote,
            prefix,
            suffix,
            offset: None,
        };
        let start = match reanchor(&self.masked, &selector) {
            Outcome::ReAnchored { start } | Outcome::Anchored { start } => start,
            Outcome::Orphaned(OrphanReason::Ambiguous) => {
                return Err(AppError::DataParsing(
                    "ANNOTATION_INVALID: that passage appears more than once in the document. \
                     Select a longer passage that includes something unique."
                        .to_string(),
                ))
            }
            Outcome::Orphaned(OrphanReason::Missing) => {
                return Err(AppError::DataParsing(
                    "ANNOTATION_INVALID: that passage was not found in the document text."
                        .to_string(),
                ))
            }
        };
        let length = quote.chars().count();
        let (prefix, suffix) = self.context_around(start, length);
        Ok(Placement {
            source_offset: self.to_source(start, false) as u64,
            prefix,
            suffix,
        })
    }

    pub fn resolve(&self, annotation: &DocumentAnnotation) -> Option<ResolvedAnchor> {
        if annotation.quote.is_empty() {
            return None;
        }
        let selector = Selector {
            quote: &annotation.quote,
            prefix: &annotation.prefix,
            suffix: &annotation.suffix,
            offset: annotation
                .character_offset
                .map(|offset| self.to_masked(offset as usize)),
        };
        let located = |state, start: usize| {
            let length = annotation.quote.chars().count();
            let source_start = self.to_source(start, false);
            let source_end = self.to_source(start + length, true);
            ResolvedAnchor {
                state,
                source_offset: Some(source_start as u64),
                source_length: Some((source_end - source_start) as u64),
                reason: None,
            }
        };
        Some(match reanchor(&self.masked, &selector) {
            Outcome::Anchored { start } => located(AnchorState::Anchored, start),
            Outcome::ReAnchored { start } => located(AnchorState::ReAnchored, start),
            Outcome::Orphaned(reason) => ResolvedAnchor {
                state: AnchorState::Orphaned,
                source_offset: None,
                source_length: None,
                reason: Some(
                    match reason {
                        OrphanReason::Missing => "The quoted text is no longer in the document.",
                        OrphanReason::Ambiguous => {
                            "The quoted text now appears more than once, so its place cannot be told."
                        }
                    }
                    .to_string(),
                ),
            },
        })
    }
}

pub struct Placement {
    pub source_offset: u64,
    pub prefix: String,
    pub suffix: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_selection_is_stored_against_the_unmasked_text_and_found_again() {
        let source = "Call 619-555-0100 now. The fee was waived. Done.";
        let view = TextView::new(source).unwrap();
        assert!(view.masked.contains("[MASKED_PHONE]"));
        let placement = view.place_selection("The fee was waived", "", "").unwrap();
        let expected = source.find("The fee").unwrap() as u64;
        assert_eq!(placement.source_offset, expected);

        let annotation = DocumentAnnotation {
            id: "a".into(),
            portal_key: "sd".into(),
            document_id: 1,
            kind: AnnotationKind::Highlight,
            character_offset: Some(placement.source_offset),
            quote: "The fee was waived".into(),
            prefix: placement.prefix,
            suffix: placement.suffix,
            text_sha256: None,
            context_unanchored: false,
            note: String::new(),
            tag: None,
            author: None,
            status: AnnotationStatus::Open,
            finding_id: None,
            page_region: None,
            imported_from_key: None,
            created_at: "2026-01-01T00:00:00Z".into(),
        };
        let anchor = view.resolve(&annotation).unwrap();
        assert_eq!(anchor.state, AnchorState::Anchored);
        assert_eq!(anchor.source_offset, Some(expected));
        assert_eq!(anchor.source_length, Some(18));

        // A quote that spans a mask covers all the source characters the mask hides.
        let spanning = view
            .place_selection("Call [MASKED_PHONE] now", "", "")
            .unwrap();
        assert_eq!(spanning.source_offset, 0);
        let mut across = annotation.clone();
        across.quote = "Call [MASKED_PHONE] now".into();
        across.character_offset = Some(0);
        across.prefix.clear();
        across.suffix.clear();
        assert_eq!(view.resolve(&across).unwrap().source_length, Some(21));
    }

    #[test]
    fn a_passage_that_appears_twice_is_refused_at_selection_time() {
        let view = TextView::new("rate is high. rate is high.").unwrap();
        assert!(view.place_selection("rate is high", "", "").is_err());
        assert!(view.place_selection("rate is high", "", ".").is_err());
        assert!(view.place_selection("absent", "", "").is_err());
    }
}
