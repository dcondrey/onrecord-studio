//! The issue-code codebook kept beside the annotations, and its plain-JSON exchange format.
//!
//! A code is a short key; the label and definition say what it means. Importing a colleague's
//! codebook adds codes this device lacks and never replaces a differing definition unless the
//! caller names that code, so two people using one code for different things see the difference
//! before it is lost.

use crate::annotations::{clean_text, invalid};
use crate::error::AppError;
use crate::types::annotation::*;
use serde::{Deserialize, Serialize};

pub const CODEBOOK_FORMAT: &str = "onrecord-codebook";
const CODEBOOK_VERSION: u32 = 1;
const MAXIMUM_IMPORT_BYTES: usize = 1024 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodebookFile {
    format: String,
    version: u32,
    codes: Vec<CodebookEntry>,
}

pub fn clean_entry(entry: &CodebookEntry) -> Result<CodebookEntry, AppError> {
    let code = clean_text("code", &entry.code, MAXIMUM_TAG_CHARACTERS, false)?;
    let label = clean_text("label", &entry.label, MAXIMUM_CODE_LABEL_CHARACTERS, false)?;
    let definition = clean_text(
        "definition",
        &entry.definition,
        MAXIMUM_CODE_DEFINITION_CHARACTERS,
        true,
    )?;
    if code.is_empty() || label.is_empty() {
        return Err(invalid("a code needs both a code and a label"));
    }
    Ok(CodebookEntry {
        code,
        label,
        definition,
    })
}

pub fn upsert(document: &mut AnnotationDocument, entry: &CodebookEntry) -> Result<(), AppError> {
    let entry = clean_entry(entry)?;
    match document
        .codebook
        .iter_mut()
        .find(|existing| existing.code == entry.code)
    {
        Some(existing) => *existing = entry,
        None => {
            if document.codebook.len() >= MAXIMUM_CODEBOOK_ENTRIES {
                return Err(invalid(format!(
                    "at most {MAXIMUM_CODEBOOK_ENTRIES} codes are kept"
                )));
            }
            document.codebook.push(entry);
        }
    }
    Ok(())
}

pub fn remove(document: &mut AnnotationDocument, code: &str) -> Result<(), AppError> {
    let before = document.codebook.len();
    document.codebook.retain(|entry| entry.code != code);
    if document.codebook.len() == before {
        return Err(invalid("that code no longer exists"));
    }
    Ok(())
}

pub fn export(codebook: &[CodebookEntry]) -> Result<String, AppError> {
    serde_json::to_string_pretty(&CodebookFile {
        format: CODEBOOK_FORMAT.to_string(),
        version: CODEBOOK_VERSION,
        codes: codebook.to_vec(),
    })
    .map_err(|error| AppError::Export(format!("Failed to encode the codebook: {error}")))
}

fn parse(payload: &str) -> Result<Vec<CodebookEntry>, AppError> {
    if payload.len() > MAXIMUM_IMPORT_BYTES {
        return Err(invalid(format!(
            "a codebook file is at most {MAXIMUM_IMPORT_BYTES} bytes"
        )));
    }
    let file: CodebookFile = serde_json::from_str(payload)
        .map_err(|error| invalid(format!("this is not a codebook file ({error})")))?;
    if file.format != CODEBOOK_FORMAT || file.version != CODEBOOK_VERSION {
        return Err(invalid(format!(
            "expected a {CODEBOOK_FORMAT} file, version {CODEBOOK_VERSION}"
        )));
    }
    if file.codes.len() > MAXIMUM_CODEBOOK_ENTRIES {
        return Err(invalid(format!(
            "a codebook holds at most {MAXIMUM_CODEBOOK_ENTRIES} codes"
        )));
    }
    let mut entries: Vec<CodebookEntry> = Vec::with_capacity(file.codes.len());
    for entry in &file.codes {
        let entry = clean_entry(entry)?;
        if entries.iter().any(|seen| seen.code == entry.code) {
            return Err(invalid(format!(
                "the code \"{}\" appears twice",
                entry.code
            )));
        }
        entries.push(entry);
    }
    Ok(entries)
}

fn plan(local: &[CodebookEntry], incoming: Vec<CodebookEntry>) -> CodebookImportPreview {
    let mut preview = CodebookImportPreview {
        added: Vec::new(),
        identical: 0,
        conflicts: Vec::new(),
    };
    for entry in incoming {
        match local.iter().find(|existing| existing.code == entry.code) {
            None => preview.added.push(entry),
            Some(existing) if *existing == entry => preview.identical += 1,
            Some(existing) => preview.conflicts.push(CodebookConflict {
                code: entry.code.clone(),
                local: existing.clone(),
                incoming: entry,
            }),
        }
    }
    preview
}

pub fn preview_import(
    local: &[CodebookEntry],
    payload: &str,
) -> Result<CodebookImportPreview, AppError> {
    Ok(plan(local, parse(payload)?))
}

/// Adds the new codes and replaces a local definition only for a code in `overwrite`. Returns
/// the plan it acted on, so the caller can show which conflicts were left as they were.
pub fn apply_import(
    document: &mut AnnotationDocument,
    payload: &str,
    overwrite: &[String],
) -> Result<CodebookImportPreview, AppError> {
    let preview = preview_import(&document.codebook, payload)?;
    if document.codebook.len() + preview.added.len() > MAXIMUM_CODEBOOK_ENTRIES {
        return Err(invalid(format!(
            "importing would exceed {MAXIMUM_CODEBOOK_ENTRIES} codes"
        )));
    }
    document.codebook.extend(preview.added.iter().cloned());
    for conflict in &preview.conflicts {
        if overwrite.contains(&conflict.code) {
            if let Some(existing) = document
                .codebook
                .iter_mut()
                .find(|existing| existing.code == conflict.code)
            {
                *existing = conflict.incoming.clone();
            }
        }
    }
    Ok(preview)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(code: &str, definition: &str) -> CodebookEntry {
        CodebookEntry {
            code: code.into(),
            label: format!("{code} label"),
            definition: definition.into(),
        }
    }

    #[test]
    fn importing_never_replaces_a_differing_definition_unless_named() {
        let mut document = AnnotationDocument::default();
        upsert(
            &mut document,
            &entry("DELAY", "Response later than the statute allows."),
        )
        .unwrap();
        upsert(&mut document, &entry("SAME", "unchanged")).unwrap();
        let payload = export(&[
            entry("DELAY", "Any slow response."),
            entry("SAME", "unchanged"),
            entry("NEW", "added"),
        ])
        .unwrap();

        let preview = preview_import(&document.codebook, &payload).unwrap();
        assert_eq!(preview.added, vec![entry("NEW", "added")]);
        assert_eq!(preview.identical, 1);
        assert_eq!(preview.conflicts.len(), 1);

        apply_import(&mut document, &payload, &[]).unwrap();
        let delay = document
            .codebook
            .iter()
            .find(|e| e.code == "DELAY")
            .unwrap();
        assert_eq!(delay.definition, "Response later than the statute allows.");
        assert_eq!(document.codebook.len(), 3);

        apply_import(&mut document, &payload, &["DELAY".to_string()]).unwrap();
        let delay = document
            .codebook
            .iter()
            .find(|e| e.code == "DELAY")
            .unwrap();
        assert_eq!(delay.definition, "Any slow response.");
    }

    #[test]
    fn a_malformed_or_oversized_codebook_changes_nothing() {
        let mut document = AnnotationDocument::default();
        upsert(&mut document, &entry("A", "kept")).unwrap();
        let before = document.clone();
        for payload in [
            "not json".to_string(),
            r#"{"format":"other","version":1,"codes":[]}"#.to_string(),
            export(&[entry("DUP", "x"), entry("DUP", "y")]).unwrap(),
            export(&[CodebookEntry {
                code: "TOO LONG".repeat(20),
                label: "l".into(),
                definition: String::new(),
            }])
            .unwrap(),
        ] {
            assert!(apply_import(&mut document, &payload, &[]).is_err());
        }
        assert_eq!(document, before);
    }
}
