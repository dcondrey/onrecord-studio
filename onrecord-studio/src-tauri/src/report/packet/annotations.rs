//! The optional "Reviewer annotations" section of an evidence packet.
//!
//! Annotations are the reviewer's reading of a document, never part of the record. They enter a
//! packet only when the reviewer asks, sit in their own section and type, and are covered by the
//! content hash like everything else in `PacketContent`. An excluded section is absent from the
//! JSON, so a packet without it is byte-identical to one built before this section existed.

use super::super::SourceDocuments;
use crate::annotation_anchor::TextView;
use crate::storage::pii::mask_pii;
use crate::types::annotation::{
    AnchorState, AnnotationDocument, AnnotationKind, AnnotationStatus, DocumentAnnotation,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const SECTION_TITLE: &str =
    "Reviewer annotations (the reviewer's reading, not part of the record)";
/// JSON key of the section; a read-only bundle refuses a packet that carries it.
pub const SECTION_KEY: &str = "reviewerAnnotations";

const NOTICE: &str = "These are one reviewer's marks and notes on the cited documents. They are \
    opinion, not a finding, and no detector read them. Anchor state is checked when the packet is \
    written, against the stored text.";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PacketAnnotation {
    pub id: String,
    pub kind: AnnotationKind,
    pub code: Option<String>,
    pub author: Option<String>,
    pub status: AnnotationStatus,
    /// `None` when there was nothing to check; `unchecked_reason` says why.
    pub anchor: Option<AnchorState>,
    pub unchecked_reason: Option<String>,
    pub portal_key: String,
    pub document_id: u64,
    /// Where the quote is in the stored text now. `None` when orphaned or unchecked.
    pub current_offset: Option<u64>,
    /// Where it was when marked.
    pub recorded_offset: Option<u64>,
    /// Digest of the stored text now. `None`: not recorded.
    pub document_text_sha256: Option<String>,
    /// Digest of the stored text when the mark was made. `None`: not recorded.
    pub marked_text_sha256: Option<String>,
    pub quote: String,
    pub note: String,
    pub imported_from_key: Option<String>,
    /// Present for a mark drawn on a page of the original file. It is located by page and
    /// rectangle, never by text, so `anchor` stays `None` for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_region: Option<crate::types::annotation::PageRegion>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReviewerAnnotations {
    pub notice: String,
    pub annotations: Vec<PacketAnnotation>,
    /// Marks on cited documents that are not public. They are counted, never written: a quote
    /// would publish text of a document the packet itself withholds.
    pub not_public_omitted: usize,
}

/// What a packet would carry. Computed from the section the export itself writes.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AnnotationPreview {
    pub included: usize,
    pub orphaned: usize,
    pub not_public_omitted: usize,
}

impl ReviewerAnnotations {
    pub fn preview(&self) -> AnnotationPreview {
        AnnotationPreview {
            included: self.annotations.len(),
            orphaned: self
                .annotations
                .iter()
                .filter(|mark| mark.anchor == Some(AnchorState::Orphaned))
                .count(),
            not_public_omitted: self.not_public_omitted,
        }
    }
}

fn masked(value: &str) -> Result<String, String> {
    mask_pii(value)
        .map(|result| result.masked_text)
        .map_err(|error| format!("Failed to screen an annotation: {error}"))
}

/// Marks on `cited` documents, with anchor state checked against the text in `documents`.
pub fn build_section(
    store: &AnnotationDocument,
    cited: &BTreeSet<(String, i64)>,
    documents: &SourceDocuments,
) -> Result<ReviewerAnnotations, String> {
    let mut annotations = Vec::new();
    let mut not_public_omitted = 0;
    for (portal_key, document_id) in cited {
        let Ok(unsigned_id) = u64::try_from(*document_id) else {
            continue;
        };
        let marks: Vec<&DocumentAnnotation> = store
            .annotations
            .iter()
            .filter(|mark| mark.portal_key == *portal_key && mark.document_id == unsigned_id)
            .collect();
        if marks.is_empty() {
            continue;
        }
        let Some(document) = documents
            .get(&(portal_key.clone(), *document_id))
            .filter(|document| document.is_public())
        else {
            not_public_omitted += marks.len();
            continue;
        };
        let any_quote = marks
            .iter()
            .any(|mark| mark.page_region.is_none() && !mark.quote.is_empty());
        let view = match (&document.text, any_quote) {
            (Some(text), true) => Some(
                TextView::new(text)
                    .map_err(|error| format!("Failed to read a document's text: {error}"))?,
            ),
            _ => None,
        };
        for mark in marks {
            let is_region = mark.page_region.is_some();
            let resolved = if is_region {
                None
            } else {
                view.as_ref().and_then(|view| view.resolve(mark))
            };
            let unchecked_reason = match (&resolved, is_region, mark.quote.is_empty()) {
                (Some(_), _, _) => None,
                (None, true, _) => Some(
                    "Drawn on a page of the original file: located by page and rectangle, not by text.",
                ),
                (None, false, true) => Some("A note on the whole document has no quote to anchor."),
                (None, false, false) => Some("The document's stored text was not available to check."),
            };
            annotations.push(PacketAnnotation {
                id: mark.id.clone(),
                kind: mark.kind,
                code: mark.tag.clone(),
                author: mark.author.clone(),
                status: mark.status,
                anchor: resolved.as_ref().map(|anchor| anchor.state),
                unchecked_reason: unchecked_reason.map(str::to_string),
                portal_key: portal_key.clone(),
                document_id: unsigned_id,
                current_offset: resolved.as_ref().and_then(|anchor| anchor.source_offset),
                recorded_offset: mark.character_offset,
                document_text_sha256: document.extracted_text_projection_sha256.clone(),
                marked_text_sha256: mark.text_sha256.clone(),
                quote: masked(&mark.quote)?,
                note: masked(&mark.note)?,
                imported_from_key: mark.imported_from_key.clone(),
                page_region: mark.page_region.clone(),
            });
        }
    }
    annotations.sort_by(|a, b| {
        let key = |mark: &PacketAnnotation| {
            (
                mark.portal_key.clone(),
                mark.document_id,
                mark.current_offset.or(mark.recorded_offset),
                mark.id.clone(),
            )
        };
        key(a).cmp(&key(b))
    });
    Ok(ReviewerAnnotations {
        notice: NOTICE.to_string(),
        annotations,
        not_public_omitted,
    })
}

/// True when a JSON value carries the section anywhere in its tree.
pub fn json_carries_section(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(map) => {
            map.contains_key(SECTION_KEY) || map.values().any(json_carries_section)
        }
        serde_json::Value::Array(items) => items.iter().any(json_carries_section),
        _ => false,
    }
}
