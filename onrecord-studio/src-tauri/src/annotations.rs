//! Reviewer annotations on extracted document text: one JSON file in the app data directory,
//! written atomically.
//!
//! Like the tracker, a per-device notebook outside DuckDB so it survives a corpus rebuild and is
//! deletable as one file. Nothing that produces findings reads it, and nothing in it is evidence
//! about the document: it records what the reviewer said, not what the record shows.

use crate::annotation_anchor::Placement;
use crate::atomic_write::{read_bounded, write_atomic};
use crate::error::AppError;
use crate::types::annotation::*;
use chrono::{SecondsFormat, Utc};
use std::io;
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
    /// SHA-256 of the original file a page region is drawn on, computed by the caller from the
    /// file itself. Required exactly when the input carries a region.
    pub original_sha256: Option<String>,
}

/// A rectangle whose far edge is the page edge can sum to a hair over 1 after the viewer rounds
/// its coordinates; a billionth of a page is far below anything drawn.
const EDGE_TOLERANCE: f64 = 1e-9;

/// A rectangle fit to store: finite, inside the page, with area, on a page index of sane size.
/// Values are never clamped; a rectangle that is not valid is the caller's to redraw.
pub fn validate_rectangle(rectangle: &PageRectangle) -> Result<(), AppError> {
    let PageRectangle {
        page_index,
        x,
        y,
        width,
        height,
    } = *rectangle;
    if page_index > MAXIMUM_PAGE_INDEX {
        return Err(invalid(format!(
            "page index must be at most {MAXIMUM_PAGE_INDEX}"
        )));
    }
    if ![x, y, width, height].iter().all(|value| value.is_finite()) {
        return Err(invalid("the region has a coordinate that is not a number"));
    }
    if x < 0.0
        || y < 0.0
        || x > 1.0
        || y > 1.0
        || x + width > 1.0 + EDGE_TOLERANCE
        || y + height > 1.0 + EDGE_TOLERANCE
    {
        return Err(invalid("the region must lie inside the page"));
    }
    if width <= 0.0 || height <= 0.0 {
        return Err(invalid("the region must have a width and a height"));
    }
    Ok(())
}

pub fn validate_original_digest(value: &str) -> Result<(), AppError> {
    if crate::integrity::is_sha256_hex(value) {
        Ok(())
    } else {
        Err(invalid("the original file digest is not a SHA-256"))
    }
}

/// What the original file looks like now against what a region recorded. Pure so the rule is
/// tested without a file: a page that no longer exists outranks a changed file, because there is
/// nothing left to show the region on.
pub fn resolve_page_region(
    region: &PageRegion,
    current_sha256: &str,
    page_count: u32,
) -> PageRegionState {
    if region.rectangle.page_index >= page_count {
        PageRegionState::Orphaned
    } else if !region.original_sha256.eq_ignore_ascii_case(current_sha256) {
        PageRegionState::OriginalChanged
    } else {
        PageRegionState::Anchored
    }
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
    let page_region = match (input.page_region, mark.original_sha256) {
        (Some(rectangle), Some(original_sha256)) => {
            validate_rectangle(&rectangle)?;
            validate_original_digest(&original_sha256)?;
            let kept = document
                .annotations
                .iter()
                .filter(|existing| {
                    existing.page_region.is_some()
                        && existing.portal_key == portal_key
                        && existing.document_id == input.document_id
                })
                .count();
            if kept >= MAXIMUM_PAGE_REGIONS_PER_DOCUMENT {
                return Err(invalid(format!(
                    "at most {MAXIMUM_PAGE_REGIONS_PER_DOCUMENT} page regions are kept per document; delete one to add another"
                )));
            }
            Some(PageRegion {
                rectangle,
                original_sha256,
            })
        }
        (Some(_), None) => return Err(invalid("the original file's digest is not available")),
        (None, _) => None,
    };
    if input.kind != AnnotationKind::Note && quote.is_empty() && page_region.is_none() {
        return Err(invalid("select the passage to mark first"));
    }
    if input.kind == AnnotationKind::Issue && tag.is_none() {
        return Err(invalid("an issue code needs a label"));
    }
    let (character_offset, prefix, suffix) = match (quote.is_empty(), mark.placement) {
        _ if page_region.is_some() => (None, String::new(), String::new()),
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
        text_sha256: if page_region.is_some() {
            None
        } else {
            mark.text_sha256
        },
        context_unanchored: false,
        note,
        tag,
        author: Some(clean_author(&mark.author)?),
        status: AnnotationStatus::Open,
        finding_id,
        imported_from_key: None,
        created_at: Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
        page_region,
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
        let bytes = match read_bounded(&self.path, MAXIMUM_FILE_BYTES) {
            Ok(Some(bytes)) => bytes,
            Ok(None) => return Ok(AnnotationDocument::default()),
            Err(error) if error.kind() == io::ErrorKind::FileTooLarge => {
                return Err(AppError::Storage(format!(
                    "The annotations file is larger than {MAXIMUM_FILE_BYTES} bytes and was not read. It was not modified."
                )))
            }
            Err(error) => {
                return Err(AppError::Storage(format!("Failed to read {LABEL}: {error}")))
            }
        };
        let mut document: AnnotationDocument = serde_json::from_slice(&bytes).map_err(|error| {
            AppError::Storage(format!(
                "The annotations file is not valid ({error}). It was not modified."
            ))
        })?;
        match document.schema_version {
            ANNOTATION_SCHEMA_VERSION => {}
            // Version 3 only adds `pageRegion`, which no earlier mark has; nothing is rewritten.
            2 => document.schema_version = ANNOTATION_SCHEMA_VERSION,
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
            page_region: None,
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
            original_sha256: None,
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
            .contains("\"schemaVersion\":3"));
    }

    fn rectangle(x: f64, y: f64, width: f64, height: f64) -> PageRectangle {
        PageRectangle {
            page_index: 2,
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn a_rectangle_is_stored_only_when_finite_inside_the_page_and_not_empty() {
        for good in [
            rectangle(0.0, 0.0, 1.0, 1.0),
            rectangle(0.3, 0.3, 0.7 + 5e-10, 0.7),
            rectangle(0.25, 0.5, 0.75, 0.5),
            rectangle(0.999, 0.999, 0.001, 0.001),
        ] {
            assert!(validate_rectangle(&good).is_ok(), "{good:?}");
        }
        for bad in [
            rectangle(f64::NAN, 0.0, 0.5, 0.5),
            rectangle(0.0, f64::INFINITY, 0.5, 0.5),
            rectangle(0.0, 0.0, 0.0, 0.5),
            rectangle(0.0, 0.0, 0.5, 0.0),
            rectangle(0.0, 0.0, -0.1, 0.5),
            rectangle(-0.001, 0.0, 0.5, 0.5),
            rectangle(0.0, 0.0, 1.001, 0.5),
            rectangle(0.6, 0.0, 0.5, 0.5),
            rectangle(0.0, 0.0, 1.0 + 2e-9, 0.5),
            rectangle(1.001, 0.0, 0.0001, 0.5),
            PageRectangle {
                page_index: MAXIMUM_PAGE_INDEX + 1,
                ..rectangle(0.0, 0.0, 0.5, 0.5)
            },
        ] {
            assert!(validate_rectangle(&bad).is_err(), "{bad:?}");
        }
        assert!(validate_rectangle(&PageRectangle {
            page_index: MAXIMUM_PAGE_INDEX,
            ..rectangle(0.0, 0.0, 0.5, 0.5)
        })
        .is_ok());
    }

    fn region_mark() -> NewMark {
        NewMark {
            placement: None,
            original_sha256: Some("cd".repeat(32)),
            ..mark()
        }
    }

    #[test]
    fn a_region_mark_records_the_original_digest_not_the_text_digest_and_needs_no_text() {
        let mut document = AnnotationDocument::default();
        let mut drawn = input(AnnotationKind::Highlight);
        drawn.quote = String::new();
        drawn.page_region = Some(rectangle(0.1, 0.2, 0.3, 0.4));
        let added = add_annotation(&mut document, drawn.clone(), region_mark()).unwrap();
        let region = added.page_region.clone().unwrap();
        assert_eq!(region.rectangle.page_index, 2);
        assert_eq!(region.original_sha256, "cd".repeat(32));
        assert_eq!(added.text_sha256, None);
        assert_eq!(added.character_offset, None);
        assert!(added.quote.is_empty());

        let no_digest = NewMark {
            original_sha256: None,
            ..region_mark()
        };
        assert!(add_annotation(&mut document, drawn.clone(), no_digest).is_err());
        let mut outside = drawn.clone();
        outside.page_region = Some(rectangle(0.9, 0.9, 0.2, 0.2));
        assert!(add_annotation(&mut document, outside, region_mark()).is_err());
        assert_eq!(document.annotations.len(), 1);

        document
            .annotations
            .resize(MAXIMUM_PAGE_REGIONS_PER_DOCUMENT, added.clone());
        assert!(add_annotation(&mut document, drawn, region_mark()).is_err());
    }

    #[test]
    fn a_region_is_orphaned_past_the_last_page_and_flagged_when_the_original_changed() {
        let region = PageRegion {
            rectangle: rectangle(0.1, 0.1, 0.2, 0.2),
            original_sha256: "ab".repeat(32),
        };
        let same = "AB".repeat(32);
        let other = "cd".repeat(32);
        assert_eq!(
            resolve_page_region(&region, &same, 3),
            PageRegionState::Anchored
        );
        assert_eq!(
            resolve_page_region(&region, &other, 3),
            PageRegionState::OriginalChanged
        );
        assert_eq!(
            resolve_page_region(&region, &same, 2),
            PageRegionState::Orphaned
        );
        assert_eq!(
            resolve_page_region(&region, &other, 2),
            PageRegionState::Orphaned
        );
    }

    /// A file exactly as the previous build wrote it.
    const VERSION_TWO: &str = r#"{"schemaVersion":2,"annotations":[
        {"id":"two-1","portalKey":"sandiego","documentId":7,"kind":"ISSUE",
         "characterOffset":120,"quote":"the contract was amended","prefix":"that ","suffix":" in May",
         "textSha256":null,"contextUnanchored":false,"note":"check","tag":"amendment",
         "author":"Maria","status":"CONFIRMED","createdAt":"2026-01-01T00:00:00Z"}]}"#;

    #[test]
    fn a_version_two_file_keeps_every_mark_unchanged_and_gains_no_regions() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(ANNOTATION_FILE_NAME);
        std::fs::write(&path, VERSION_TWO).unwrap();
        let store = AnnotationStore::new(directory.path());
        let loaded = store.load().unwrap();
        assert_eq!(loaded.schema_version, ANNOTATION_SCHEMA_VERSION);
        let kept = &loaded.annotations[0];
        assert_eq!(kept.id, "two-1");
        assert_eq!(kept.prefix, "that ");
        assert_eq!(kept.status, AnnotationStatus::Confirmed);
        assert!(!kept.context_unanchored);
        assert_eq!(kept.page_region, None);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), VERSION_TWO);
        store
            .update(|document| set_reviewer(document, "Maria"))
            .unwrap();
        let rewritten = std::fs::read_to_string(&path).unwrap();
        assert!(rewritten.contains("\"schemaVersion\":3"));
        assert!(!rewritten.contains("pageRegion"));
    }

    #[test]
    fn a_newer_schema_is_refused_without_touching_the_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(ANNOTATION_FILE_NAME);
        let future = br#"{"schemaVersion":4,"annotations":[]}"#;
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
