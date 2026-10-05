//! How a document reached this application, and how a file the user added by hand relates to the
//! documents collected from sources.
//!
//! A match states that the user's file equals, or reads like, a file a source served. It is not a
//! statement that the user's file is authentic, and the absence of a match is not a statement that
//! it is fabricated. A comparison that could not be made is a different state from one that found
//! nothing, and the two never share a representation.

use super::portal::SourceKind;
use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DocumentOrigin {
    /// Put in by the user. Nothing was fetched from an agency source.
    AddedByUser,
    CollectedFromSource,
}

/// A document collected from a source, named well enough to find it.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HeldDocumentMatch {
    pub portal_key: String,
    pub source_name: String,
    pub request_id: i64,
    /// The source's own request reference, else the request title. `None`: neither is recorded.
    pub request_label: Option<String>,
    pub document_id: i64,
    pub file_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NearDuplicateMatch {
    pub document: HeldDocumentMatch,
    /// Differing bits between the two 64-bit text fingerprints; 0 is identical.
    pub simhash_distance: u32,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum NotComparedReason {
    /// No SHA-256 of the file's bytes is recorded for this document.
    FileDigestNotRecorded,
    /// No text has been extracted from this document, or the extraction produced none.
    TextNotExtracted,
    /// The similarity index holds no entry for this document's current text.
    NotInSimilarityIndex,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum HeldComparison<T> {
    NotCompared {
        reason: NotComparedReason,
    },
    /// The comparison ran. An empty `matches` means nothing matched among `compared_count`
    /// documents, which is all it means.
    #[serde(rename_all = "camelCase")]
    Compared {
        matches: Vec<T>,
        /// More matched than `matches` lists.
        truncated: bool,
        compared_count: u64,
    },
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HeldDocumentRelation {
    /// Collected documents whose file bytes have the same SHA-256.
    pub exact_copies: HeldComparison<HeldDocumentMatch>,
    /// Collected documents whose extracted text has the same digest and that are not exact copies.
    pub same_text: HeldComparison<HeldDocumentMatch>,
    /// Collected documents whose text fingerprint is within the similarity index's threshold.
    pub near_duplicates: HeldComparison<NearDuplicateMatch>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DocumentOriginReport {
    pub document_id: u64,
    pub portal_key: String,
    pub origin: DocumentOrigin,
    pub source_kind: SourceKind,
    /// `None` for a collected document: the comparison is defined for hand-added files only.
    pub held_relation: Option<HeldDocumentRelation>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum InternalMetadataFormat {
    Pdf,
    Ooxml,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum InternalMetadataUnreadReason {
    FileUnavailable,
    /// Neither a PDF nor an Office Open XML package; no reader exists for it.
    UnsupportedFormat,
    /// Named as a supported format but could not be parsed as one.
    Unreadable,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InternalMetadataField {
    pub name: String,
    /// `None`: the file records no value for this field.
    pub value: Option<String>,
}

/// What a file says about itself. Every value was written by whoever produced the file and can be
/// set to anything; none of it is verified.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DocumentInternalMetadata {
    NotRead {
        reason: InternalMetadataUnreadReason,
    },
    Read {
        format: InternalMetadataFormat,
        fields: Vec<InternalMetadataField>,
    },
}
