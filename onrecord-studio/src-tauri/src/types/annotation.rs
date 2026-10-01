use serde::{Deserialize, Serialize};

pub const ANNOTATION_SCHEMA_VERSION: u32 = 1;
pub const MAXIMUM_ANNOTATIONS: usize = 5_000;
pub const MAXIMUM_NOTE_CHARACTERS: usize = 4_000;
pub const MAXIMUM_QUOTE_CHARACTERS: usize = 2_000;
pub const MAXIMUM_TAG_CHARACTERS: usize = 60;
pub const MAXIMUM_PORTAL_KEY_CHARACTERS: usize = 120;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AnnotationKind {
    Note,
    Highlight,
    /// A passage coded to an issue label (`tag`), for review and export.
    Issue,
}

/// A reviewer's mark on extracted text. `quote` is what was selected when the mark was made; the
/// offsets locate it in the extracted text, which is derived from the original and not itself
/// the record.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DocumentAnnotation {
    pub id: String,
    pub portal_key: String,
    pub document_id: u64,
    pub kind: AnnotationKind,
    pub character_offset: u64,
    pub quote: String,
    pub note: String,
    pub tag: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AddAnnotationInput {
    pub portal_key: String,
    pub document_id: u64,
    pub kind: AnnotationKind,
    pub character_offset: u64,
    pub quote: String,
    pub note: String,
    pub tag: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AnnotationDocument {
    pub schema_version: u32,
    pub annotations: Vec<DocumentAnnotation>,
}

impl Default for AnnotationDocument {
    fn default() -> Self {
        Self {
            schema_version: ANNOTATION_SCHEMA_VERSION,
            annotations: Vec::new(),
        }
    }
}
