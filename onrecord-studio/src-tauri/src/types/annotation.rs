use serde::{Deserialize, Serialize};

pub const ANNOTATION_SCHEMA_VERSION: u32 = 2;
pub const MAXIMUM_ANNOTATIONS: usize = 5_000;
pub const MAXIMUM_NOTE_CHARACTERS: usize = 4_000;
pub const MAXIMUM_QUOTE_CHARACTERS: usize = 2_000;
pub const MAXIMUM_TAG_CHARACTERS: usize = 60;
pub const MAXIMUM_PORTAL_KEY_CHARACTERS: usize = 120;
pub const MAXIMUM_AUTHOR_CHARACTERS: usize = 80;
pub const MAXIMUM_FINDING_ID_CHARACTERS: usize = 120;
/// Characters of surrounding text kept on each side of a quote.
pub const CONTEXT_CHARACTERS: usize = 32;
pub const MAXIMUM_CONTEXT_CHARACTERS: usize = 200;
pub const MAXIMUM_CODEBOOK_ENTRIES: usize = 500;
pub const MAXIMUM_CODE_LABEL_CHARACTERS: usize = 120;
pub const MAXIMUM_CODE_DEFINITION_CHARACTERS: usize = 2_000;
pub const DEFAULT_SEARCH_PAGE: usize = 50;
pub const MAXIMUM_SEARCH_PAGE: usize = 200;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AnnotationKind {
    Note,
    Highlight,
    /// A passage coded to an issue label (`tag`), for review and export.
    Issue,
}

/// The reviewer's own judgement of an annotation. Never evidence about the document.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AnnotationStatus {
    #[default]
    Open,
    Confirmed,
    Disputed,
}

/// A reviewer's mark on extracted text. `quote` with `prefix`/`suffix` is the anchor that can be
/// searched for again; `character_offset` (a position in the stored extracted text) is where it
/// was when `text_sha256` described that text. A note with no quote marks the whole document and
/// has no position.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DocumentAnnotation {
    pub id: String,
    pub portal_key: String,
    pub document_id: u64,
    pub kind: AnnotationKind,
    pub character_offset: Option<u64>,
    pub quote: String,
    #[serde(default)]
    pub prefix: String,
    #[serde(default)]
    pub suffix: String,
    /// Digest of the stored text when the mark was made. `None`: not recorded.
    #[serde(default)]
    pub text_sha256: Option<String>,
    /// True when no prefix/suffix was recorded (a mark made before context was kept), so the
    /// anchor is the quote and offset alone.
    #[serde(default)]
    pub context_unanchored: bool,
    pub note: String,
    pub tag: Option<String>,
    /// Reviewer label as typed by whoever made the mark. `None`: made before authors were kept.
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub status: AnnotationStatus,
    #[serde(default)]
    pub finding_id: Option<String>,
    /// Set only on a mark that arrived in a signed packet: the key that signed it.
    #[serde(default)]
    pub imported_from_key: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AddAnnotationInput {
    pub portal_key: String,
    pub document_id: u64,
    pub kind: AnnotationKind,
    pub quote: String,
    /// Text the viewer saw around the selection; used only to tell repeated passages apart.
    #[serde(default)]
    pub prefix: String,
    #[serde(default)]
    pub suffix: String,
    pub note: String,
    pub tag: Option<String>,
    #[serde(default)]
    pub finding_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodebookEntry {
    pub code: String,
    pub label: String,
    pub definition: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AnnotationDocument {
    pub schema_version: u32,
    pub annotations: Vec<DocumentAnnotation>,
    #[serde(default)]
    pub codebook: Vec<CodebookEntry>,
    /// The name this device's marks are attributed to.
    #[serde(default)]
    pub reviewer: Option<String>,
}

impl Default for AnnotationDocument {
    fn default() -> Self {
        Self {
            schema_version: ANNOTATION_SCHEMA_VERSION,
            annotations: Vec::new(),
            codebook: Vec::new(),
            reviewer: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AnnotationSettings {
    pub reviewer: Option<String>,
    pub codebook: Vec<CodebookEntry>,
}

/// How a stored quote relates to the document's text as it is now.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AnchorState {
    /// Found where it was recorded.
    Anchored,
    /// Not where it was recorded; found exactly once elsewhere.
    ReAnchored,
    /// Missing or ambiguous. Never moved on a guess.
    Orphaned,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedAnchor {
    pub state: AnchorState,
    /// Position in the stored text now. `None` when orphaned, or when the state came from an
    /// unchanged text digest without reading the text.
    pub source_offset: Option<u64>,
    pub source_length: Option<u64>,
    /// Why an orphan could not be placed.
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedAnnotation {
    #[serde(flatten)]
    pub annotation: DocumentAnnotation,
    /// `None` for a mark with no quote, or when the document's text was not available to check.
    pub anchor: Option<ResolvedAnchor>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DocumentAnnotationList {
    pub text_available: bool,
    pub current_text_sha256: Option<String>,
    pub annotations: Vec<ResolvedAnnotation>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AnnotationSearchQuery {
    pub code: Option<String>,
    pub author: Option<String>,
    pub status: Option<AnnotationStatus>,
    pub text: Option<String>,
    #[serde(default)]
    pub orphaned_only: bool,
    pub cursor: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AnnotationSearchRow {
    #[serde(flatten)]
    pub resolved: ResolvedAnnotation,
    pub request_id: Option<u64>,
    pub file_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AnnotationSearchPage {
    pub rows: Vec<AnnotationSearchRow>,
    /// Present exactly when more matching annotations follow this page.
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodebookConflict {
    pub code: String,
    pub local: CodebookEntry,
    pub incoming: CodebookEntry,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodebookImportPreview {
    pub added: Vec<CodebookEntry>,
    pub identical: usize,
    pub conflicts: Vec<CodebookConflict>,
}

/// What a signed packet of one document's annotations carries.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AnnotationPacket {
    pub format: String,
    pub exported_at: String,
    pub app_version: String,
    pub portal_key: String,
    pub document_id: u64,
    /// Digest of the document's stored text on the exporting device. `None`: not recorded.
    pub document_text_sha256: Option<String>,
    pub exporter: Option<String>,
    pub annotations: Vec<DocumentAnnotation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AnnotationImportConflict {
    pub local: DocumentAnnotation,
    pub incoming: DocumentAnnotation,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AnnotationImportPreview {
    pub signer_key_id: String,
    pub packet_sha256: String,
    pub exported_at: String,
    pub app_version: String,
    pub exporter: Option<String>,
    pub portal_key: String,
    pub document_id: u64,
    pub packet_text_sha256: Option<String>,
    pub local_text_sha256: Option<String>,
    pub to_add: Vec<DocumentAnnotation>,
    pub already_present: usize,
    pub conflicts: Vec<AnnotationImportConflict>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AnnotationImportResult {
    pub added: usize,
    pub already_present: usize,
    pub conflicts: usize,
}
