//! Reviewer annotations on extracted document text: one JSON file in the app data directory,
//! written atomically.
//!
//! Like the tracker, a per-device notebook outside DuckDB so it survives a corpus rebuild and is
//! deletable as one file. Nothing that produces findings reads it, and nothing in it is evidence
//! about the document: it records what the reviewer said, not what the record shows.

use crate::atomic_write::write_atomic;
use crate::error::AppError;
use crate::types::annotation::*;
use chrono::{SecondsFormat, Utc};
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use uuid::Uuid;

pub const ANNOTATION_FILE_NAME: &str = "annotations.json";
const LABEL: &str = "the annotations file";
const MAXIMUM_FILE_BYTES: u64 = 32 * 1024 * 1024;

fn invalid(message: impl Into<String>) -> AppError {
    AppError::DataParsing(format!("ANNOTATION_INVALID: {}", message.into()))
}

/// Trimmed text of at most `maximum` characters. Newlines and tabs survive; other control
/// characters do not. Over-long text is refused, never truncated.
fn clean_text(
    field: &str,
    value: &str,
    maximum: usize,
    multiline: bool,
) -> Result<String, AppError> {
    let value = value.trim();
    let allowed = |character: char| multiline && matches!(character, '\n' | '\t');
    if value
        .chars()
        .any(|character| character.is_control() && !allowed(character))
    {
        return Err(invalid(format!(
            "{field} must not contain control characters"
        )));
    }
    if value.chars().count() > maximum {
        return Err(invalid(format!(
            "{field} must be at most {maximum} characters"
        )));
    }
    Ok(value.to_string())
}

pub fn add_annotation(
    document: &mut AnnotationDocument,
    input: AddAnnotationInput,
) -> Result<DocumentAnnotation, AppError> {
    if document.annotations.len() >= MAXIMUM_ANNOTATIONS {
        return Err(invalid(format!(
            "at most {MAXIMUM_ANNOTATIONS} annotations are kept; delete one to add another"
        )));
    }
    let portal_key = clean_text(
        "portal key",
        &input.portal_key,
        MAXIMUM_PORTAL_KEY_CHARACTERS,
        false,
    )?;
    if portal_key.is_empty() {
        return Err(invalid("portal key is required"));
    }
    let quote = clean_text("quote", &input.quote, MAXIMUM_QUOTE_CHARACTERS, true)?;
    let note = clean_text("note", &input.note, MAXIMUM_NOTE_CHARACTERS, true)?;
    let tag = match input.tag.as_deref() {
        Some(tag) => Some(clean_text("tag", tag, MAXIMUM_TAG_CHARACTERS, false)?)
            .filter(|tag| !tag.is_empty()),
        None => None,
    };
    if input.kind == AnnotationKind::Note && note.is_empty() {
        return Err(invalid("a note needs text"));
    }
    if input.kind != AnnotationKind::Note && quote.is_empty() {
        return Err(invalid("select the passage to mark first"));
    }
    if input.kind == AnnotationKind::Issue && tag.is_none() {
        return Err(invalid("an issue code needs a label"));
    }
    let annotation = DocumentAnnotation {
        id: Uuid::new_v4().to_string(),
        portal_key,
        document_id: input.document_id,
        kind: input.kind,
        character_offset: input.character_offset,
        quote,
        note,
        tag,
        created_at: Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
    };
    document.annotations.push(annotation.clone());
    Ok(annotation)
}

pub fn remove_annotation(document: &mut AnnotationDocument, id: &str) -> Result<(), AppError> {
    let before = document.annotations.len();
    document
        .annotations
        .retain(|annotation| annotation.id != id);
    if document.annotations.len() == before {
        return Err(invalid("that annotation no longer exists"));
    }
    Ok(())
}

pub struct AnnotationStore {
    path: PathBuf,
    gate: Mutex<()>,
}

impl AnnotationStore {
    pub fn new(directory: &Path) -> Self {
        Self {
            path: directory.join(ANNOTATION_FILE_NAME),
            gate: Mutex::new(()),
        }
    }

    pub fn load(&self) -> Result<AnnotationDocument, AppError> {
        let _guard = self.lock()?;
        self.read()
    }

    /// Reads, edits and writes under one lock. An error from `change` writes nothing.
    pub fn update<T>(
        &self,
        change: impl FnOnce(&mut AnnotationDocument) -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        let _guard = self.lock()?;
        let mut document = self.read()?;
        let result = change(&mut document)?;
        let bytes = serde_json::to_vec(&document)
            .map_err(|error| AppError::Storage(format!("Failed to encode {LABEL}: {error}")))?;
        if bytes.len() as u64 > MAXIMUM_FILE_BYTES {
            return Err(AppError::Storage(format!(
                "The annotations would exceed {MAXIMUM_FILE_BYTES} bytes. Nothing was saved."
            )));
        }
        write_atomic(&self.path, LABEL, &bytes)?;
        Ok(result)
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, ()>, AppError> {
        self.gate
            .lock()
            .map_err(|_| AppError::Internal("The annotation store lock was poisoned".to_string()))
    }

    /// A missing file is empty. A file that exists but cannot be read is an error, never "empty":
    /// showing it empty would overwrite it on the next save.
    fn read(&self) -> Result<AnnotationDocument, AppError> {
        let file = match File::open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(AnnotationDocument::default())
            }
            Err(error) => {
                return Err(AppError::Storage(format!(
                    "Failed to open {LABEL}: {error}"
                )))
            }
        };
        let mut bytes = Vec::new();
        file.take(MAXIMUM_FILE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| AppError::Storage(format!("Failed to read {LABEL}: {error}")))?;
        if bytes.len() as u64 > MAXIMUM_FILE_BYTES {
            return Err(AppError::Storage(format!(
                "The annotations file is larger than {MAXIMUM_FILE_BYTES} bytes and was not read. It was not modified."
            )));
        }
        let document: AnnotationDocument = serde_json::from_slice(&bytes).map_err(|error| {
            AppError::Storage(format!(
                "The annotations file is not valid ({error}). It was not modified."
            ))
        })?;
        if document.schema_version != ANNOTATION_SCHEMA_VERSION {
            return Err(AppError::Storage(format!(
                "The annotations file has schema version {}, this build reads {ANNOTATION_SCHEMA_VERSION}. It was not modified.",
                document.schema_version
            )));
        }
        Ok(document)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(kind: AnnotationKind) -> AddAnnotationInput {
        AddAnnotationInput {
            portal_key: "sandiego".into(),
            document_id: 7,
            kind,
            character_offset: 120,
            quote: "the contract was amended".into(),
            note: "Compare with the signed copy.".into(),
            tag: Some("amendment".into()),
        }
    }

    #[test]
    fn a_mark_without_what_it_marks_is_refused() {
        let mut document = AnnotationDocument::default();
        let mut highlight = input(AnnotationKind::Highlight);
        highlight.quote = "  ".into();
        assert!(add_annotation(&mut document, highlight).is_err());
        let mut issue = input(AnnotationKind::Issue);
        issue.tag = None;
        assert!(add_annotation(&mut document, issue).is_err());
        let mut note = input(AnnotationKind::Note);
        note.note = String::new();
        assert!(add_annotation(&mut document, note).is_err());
        assert!(document.annotations.is_empty());
    }

    #[test]
    fn limits_hold_at_the_boundary_and_never_truncate() {
        let mut document = AnnotationDocument::default();
        let mut at_limit = input(AnnotationKind::Note);
        at_limit.note = "n".repeat(MAXIMUM_NOTE_CHARACTERS);
        assert!(add_annotation(&mut document, at_limit).is_ok());
        let mut over = input(AnnotationKind::Note);
        over.note = "n".repeat(MAXIMUM_NOTE_CHARACTERS + 1);
        assert!(add_annotation(&mut document, over).is_err());
        assert_eq!(document.annotations.len(), 1);
        assert_eq!(document.annotations[0].note.len(), MAXIMUM_NOTE_CHARACTERS);
    }

    #[test]
    fn a_failed_update_writes_nothing_and_an_unreadable_file_is_not_empty() {
        let directory = tempfile::tempdir().unwrap();
        let store = AnnotationStore::new(directory.path());
        let mut bad = input(AnnotationKind::Note);
        bad.note = String::new();
        assert!(store
            .update(|document| add_annotation(document, bad))
            .is_err());
        assert!(!directory.path().join(ANNOTATION_FILE_NAME).exists());

        let added = store
            .update(|document| add_annotation(document, input(AnnotationKind::Issue)))
            .unwrap();
        assert_eq!(store.load().unwrap().annotations, vec![added.clone()]);
        store
            .update(|document| remove_annotation(document, &added.id))
            .unwrap();
        assert!(store.load().unwrap().annotations.is_empty());

        std::fs::write(directory.path().join(ANNOTATION_FILE_NAME), b"{ not json").unwrap();
        assert!(store.load().is_err());
        assert!(store
            .update(|document| add_annotation(document, input(AnnotationKind::Note)))
            .is_err());
        assert_eq!(
            std::fs::read(directory.path().join(ANNOTATION_FILE_NAME)).unwrap(),
            b"{ not json"
        );
    }
}
