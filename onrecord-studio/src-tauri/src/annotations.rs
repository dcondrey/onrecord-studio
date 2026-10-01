//! Reviewer annotations on extracted document text: one JSON file in the app data directory,
//! written atomically.
//!
//! Like the tracker, a per-device notebook outside DuckDB so it survives a corpus rebuild and is
//! deletable as one file. Nothing that produces findings reads it, and nothing in it is evidence
//! about the document: it records what the reviewer said, not what the record shows.

use crate::annotation_anchor::Placement;
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

pub fn invalid(message: impl Into<String>) -> AppError {
    AppError::DataParsing(format!("ANNOTATION_INVALID: {}", message.into()))
}

/// Trimmed text of at most `maximum` characters. Newlines and tabs survive; other control
/// characters do not. Over-long text is refused, never truncated.
pub fn clean_text(
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

pub fn clean_author(value: &str) -> Result<String, AppError> {
    let author = clean_text("reviewer name", value, MAXIMUM_AUTHOR_CHARACTERS, false)?;
    if author.is_empty() {
        return Err(invalid("a reviewer name is required"));
    }
    Ok(author)
}

pub fn set_reviewer(document: &mut AnnotationDocument, reviewer: &str) -> Result<(), AppError> {
    document.reviewer = Some(clean_author(reviewer)?);
    Ok(())
}

/// What a new mark takes from outside the reviewer's input: who is making it, and where and
/// against which text it was placed. A mark with no quote has no placement.
pub struct NewMark {
    pub author: String,
    pub placement: Option<Placement>,
    pub text_sha256: Option<String>,
}

pub fn add_annotation(
    document: &mut AnnotationDocument,
    input: AddAnnotationInput,
    mark: NewMark,
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
    let finding_id = match input.finding_id.as_deref() {
        Some(id) => Some(clean_text(
            "finding id",
            id,
            MAXIMUM_FINDING_ID_CHARACTERS,
            false,
        )?)
        .filter(|id| !id.is_empty()),
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
    let (character_offset, prefix, suffix) = match (quote.is_empty(), mark.placement) {
        (false, Some(placement)) => (
            Some(placement.source_offset),
            placement.prefix,
            placement.suffix,
        ),
        (false, None) => return Err(invalid("the passage was not placed in the document text")),
        (true, _) => (None, String::new(), String::new()),
    };
    let annotation = DocumentAnnotation {
        id: Uuid::new_v4().to_string(),
        portal_key,
        document_id: input.document_id,
        kind: input.kind,
        character_offset,
        quote,
        prefix,
        suffix,
        text_sha256: mark.text_sha256,
        context_unanchored: false,
        note,
        tag,
        author: Some(clean_author(&mark.author)?),
        status: AnnotationStatus::Open,
        finding_id,
        imported_from_key: None,
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

/// A status is its author's judgement of their own mark. One that arrived in a packet is a
/// colleague's, and answering it means adding a mark of one's own.
pub fn set_status(
    document: &mut AnnotationDocument,
    id: &str,
    status: AnnotationStatus,
) -> Result<(), AppError> {
    let annotation = document
        .annotations
        .iter_mut()
        .find(|annotation| annotation.id == id)
        .ok_or_else(|| invalid("that annotation no longer exists"))?;
    if annotation.imported_from_key.is_some() {
        return Err(invalid(
            "this annotation came from a colleague; add your own to agree or disagree with it",
        ));
    }
    annotation.status = status;
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
        let mut document: AnnotationDocument = serde_json::from_slice(&bytes).map_err(|error| {
            AppError::Storage(format!(
                "The annotations file is not valid ({error}). It was not modified."
            ))
        })?;
        match document.schema_version {
            ANNOTATION_SCHEMA_VERSION => {}
            // Marks made before anchors kept context keep every field they had. What they lack
            // stays absent and is flagged, not invented.
            1 => {
                for annotation in &mut document.annotations {
                    annotation.context_unanchored = true;
                }
                document.schema_version = ANNOTATION_SCHEMA_VERSION;
            }
            other => {
                return Err(AppError::Storage(format!(
                    "The annotations file has schema version {other}, this build reads up to {ANNOTATION_SCHEMA_VERSION}. It was not modified."
                )))
            }
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
            quote: "the contract was amended".into(),
            prefix: String::new(),
            suffix: String::new(),
            note: "Compare with the signed copy.".into(),
            tag: Some("amendment".into()),
            finding_id: None,
        }
    }

    fn mark() -> NewMark {
        NewMark {
            author: "Maria".into(),
            placement: Some(Placement {
                source_offset: 120,
                prefix: "that ".into(),
                suffix: " in May".into(),
            }),
            text_sha256: Some("ab".repeat(32)),
        }
    }

    #[test]
    fn a_mark_without_what_it_marks_is_refused() {
        let mut document = AnnotationDocument::default();
        let mut highlight = input(AnnotationKind::Highlight);
        highlight.quote = "  ".into();
        assert!(add_annotation(&mut document, highlight, mark()).is_err());
        let mut issue = input(AnnotationKind::Issue);
        issue.tag = None;
        assert!(add_annotation(&mut document, issue, mark()).is_err());
        let mut note = input(AnnotationKind::Note);
        note.note = String::new();
        assert!(add_annotation(&mut document, note, mark()).is_err());
        let unplaced = NewMark {
            placement: None,
            ..mark()
        };
        assert!(add_annotation(&mut document, input(AnnotationKind::Highlight), unplaced).is_err());
        assert!(document.annotations.is_empty());
    }

    #[test]
    fn limits_hold_at_the_boundary_and_never_truncate() {
        let mut document = AnnotationDocument::default();
        let mut at_limit = input(AnnotationKind::Note);
        at_limit.quote = String::new();
        at_limit.note = "n".repeat(MAXIMUM_NOTE_CHARACTERS);
        let added = add_annotation(&mut document, at_limit, mark()).unwrap();
        assert_eq!(added.character_offset, None);
        let mut over = input(AnnotationKind::Note);
        over.quote = String::new();
        over.note = "n".repeat(MAXIMUM_NOTE_CHARACTERS + 1);
        assert!(add_annotation(&mut document, over, mark()).is_err());
        assert_eq!(document.annotations.len(), 1);
        assert_eq!(document.annotations[0].note.len(), MAXIMUM_NOTE_CHARACTERS);
    }

    #[test]
    fn a_failed_update_writes_nothing_and_an_unreadable_file_is_not_empty() {
        let directory = tempfile::tempdir().unwrap();
        let store = AnnotationStore::new(directory.path());
        let mut bad = input(AnnotationKind::Note);
        bad.quote = String::new();
        bad.note = String::new();
        assert!(store
            .update(|document| add_annotation(document, bad, mark()))
            .is_err());
        assert!(!directory.path().join(ANNOTATION_FILE_NAME).exists());

        let added = store
            .update(|document| add_annotation(document, input(AnnotationKind::Issue), mark()))
            .unwrap();
        assert_eq!(store.load().unwrap().annotations, vec![added.clone()]);
        store
            .update(|document| remove_annotation(document, &added.id))
            .unwrap();
        assert!(store.load().unwrap().annotations.is_empty());

        std::fs::write(directory.path().join(ANNOTATION_FILE_NAME), b"{ not json").unwrap();
        assert!(store.load().is_err());
        assert!(store
            .update(|document| add_annotation(document, input(AnnotationKind::Highlight), mark()))
            .is_err());
        assert_eq!(
            std::fs::read(directory.path().join(ANNOTATION_FILE_NAME)).unwrap(),
            b"{ not json"
        );
    }

    /// A file exactly as the previous build wrote it.
    const VERSION_ONE: &str = r#"{"schemaVersion":1,"annotations":[
        {"id":"old-1","portalKey":"sandiego","documentId":7,"kind":"ISSUE",
         "characterOffset":120,"quote":"the contract was amended","note":"check",
         "tag":"amendment","createdAt":"2026-01-01T00:00:00Z"},
        {"id":"old-2","portalKey":"sandiego","documentId":7,"kind":"NOTE",
         "characterOffset":0,"quote":"","note":"whole file","tag":null,
         "createdAt":"2026-01-02T00:00:00Z"}]}"#;

    #[test]
    fn a_version_one_file_keeps_every_mark_and_flags_the_missing_context() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(ANNOTATION_FILE_NAME);
        std::fs::write(&path, VERSION_ONE).unwrap();
        let store = AnnotationStore::new(directory.path());

        let loaded = store.load().unwrap();
        assert_eq!(loaded.schema_version, ANNOTATION_SCHEMA_VERSION);
        let ids: Vec<_> = loaded.annotations.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["old-1", "old-2"]);
        let first = &loaded.annotations[0];
        assert_eq!(first.quote, "the contract was amended");
        assert_eq!(first.character_offset, Some(120));
        assert_eq!(first.tag.as_deref(), Some("amendment"));
        assert!(first.context_unanchored);
        assert!(first.prefix.is_empty() && first.suffix.is_empty());
        assert_eq!(first.text_sha256, None);
        assert_eq!(first.author, None);
        assert_eq!(first.status, AnnotationStatus::Open);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), VERSION_ONE);

        // The first save writes version 2 and the old marks survive it; a new mark is not flagged.
        store
            .update(|document| add_annotation(document, input(AnnotationKind::Highlight), mark()))
            .unwrap();
        let saved = store.load().unwrap();
        assert_eq!(saved.annotations.len(), 3);
        assert!(saved.annotations[0].context_unanchored);
        assert!(!saved.annotations[2].context_unanchored);
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("\"schemaVersion\":2"));
    }

    #[test]
    fn a_newer_schema_is_refused_without_touching_the_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(ANNOTATION_FILE_NAME);
        let future = br#"{"schemaVersion":3,"annotations":[]}"#;
        std::fs::write(&path, future).unwrap();
        let store = AnnotationStore::new(directory.path());
        assert!(store.load().is_err());
        assert!(store
            .update(|document| set_reviewer(document, "Maria"))
            .is_err());
        assert_eq!(std::fs::read(&path).unwrap(), future);
    }

    #[test]
    fn a_colleagues_status_cannot_be_changed_but_ones_own_can() {
        let mut document = AnnotationDocument::default();
        let own = add_annotation(&mut document, input(AnnotationKind::Issue), mark()).unwrap();
        let mut theirs = own.clone();
        theirs.id = "theirs".into();
        theirs.imported_from_key = Some("key".into());
        document.annotations.push(theirs);
        set_status(&mut document, &own.id, AnnotationStatus::Confirmed).unwrap();
        assert!(set_status(&mut document, "theirs", AnnotationStatus::Disputed).is_err());
        assert_eq!(document.annotations[0].status, AnnotationStatus::Confirmed);
        assert_eq!(document.annotations[1].status, AnnotationStatus::Open);
    }
}
