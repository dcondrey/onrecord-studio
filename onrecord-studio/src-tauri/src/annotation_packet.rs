//! A signed packet of one document's annotations, and how one is merged into this device's file.
//!
//! The signature proves the packet is unchanged since a key signed it; it does not make its
//! content valid. Every incoming mark is checked as strictly as one typed here, and a mark whose
//! id is already present is never overwritten: a differing one is reported and the local copy
//! stays.

use crate::annotations::{clean_author, clean_text, invalid};
use crate::error::AppError;
use crate::types::annotation::*;
use chrono::{DateTime, SecondsFormat, Utc};
use uuid::Uuid;

pub const PACKET_FORMAT: &str = "onrecord-annotations-v1";

pub fn build_packet(
    document: &AnnotationDocument,
    portal_key: &str,
    document_id: u64,
    document_text_sha256: Option<String>,
    app_version: &str,
) -> AnnotationPacket {
    AnnotationPacket {
        format: PACKET_FORMAT.to_string(),
        exported_at: Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
        app_version: app_version.to_string(),
        portal_key: portal_key.to_string(),
        document_id,
        document_text_sha256,
        exporter: document.reviewer.clone(),
        // Only marks made here: a colleague's marks are theirs to sign.
        annotations: document
            .annotations
            .iter()
            .filter(|annotation| {
                annotation.portal_key == portal_key
                    && annotation.document_id == document_id
                    && annotation.imported_from_key.is_none()
            })
            .cloned()
            .collect(),
    }
}

fn check_context(field: &str, value: &str) -> Result<(), AppError> {
    if value.chars().count() > MAXIMUM_CONTEXT_CHARACTERS
        || value
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\t'))
    {
        return Err(invalid(format!("{field} is not valid context text")));
    }
    Ok(())
}

fn validate_mark(
    packet: &AnnotationPacket,
    mark: &DocumentAnnotation,
    signer_key_id: &str,
) -> Result<DocumentAnnotation, AppError> {
    Uuid::parse_str(&mark.id).map_err(|_| invalid("a packet annotation has no valid id"))?;
    if mark.portal_key != packet.portal_key || mark.document_id != packet.document_id {
        return Err(invalid(
            "a packet annotation belongs to a different document than the packet",
        ));
    }
    DateTime::parse_from_rfc3339(&mark.created_at)
        .map_err(|_| invalid("a packet annotation has no valid creation time"))?;
    let quote = clean_text("quote", &mark.quote, MAXIMUM_QUOTE_CHARACTERS, true)?;
    let note = clean_text("note", &mark.note, MAXIMUM_NOTE_CHARACTERS, true)?;
    let tag = match mark.tag.as_deref() {
        Some(tag) => Some(clean_text("tag", tag, MAXIMUM_TAG_CHARACTERS, false)?)
            .filter(|tag| !tag.is_empty()),
        None => None,
    };
    let finding_id = match mark.finding_id.as_deref() {
        Some(id) => Some(clean_text(
            "finding id",
            id,
            MAXIMUM_FINDING_ID_CHARACTERS,
            false,
        )?)
        .filter(|id| !id.is_empty()),
        None => None,
    };
    let author = mark.author.as_deref().map(clean_author).transpose()?;
    check_context("prefix", &mark.prefix)?;
    check_context("suffix", &mark.suffix)?;
    if let Some(digest) = &mark.text_sha256 {
        if digest.len() != 64 || !digest.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(invalid("a packet annotation has an invalid text digest"));
        }
    }
    let has_passage = !quote.is_empty();
    if (mark.kind != AnnotationKind::Note && !has_passage)
        || (mark.kind == AnnotationKind::Issue && tag.is_none())
    {
        return Err(invalid("a packet annotation is internally inconsistent"));
    }
    Ok(DocumentAnnotation {
        quote,
        note,
        tag,
        finding_id,
        author,
        imported_from_key: Some(signer_key_id.to_string()),
        ..mark.clone()
    })
}

pub struct ImportPlan {
    pub to_add: Vec<DocumentAnnotation>,
    pub already_present: usize,
    pub conflicts: Vec<AnnotationImportConflict>,
}

pub fn plan_import(
    local: &AnnotationDocument,
    packet: &AnnotationPacket,
    signer_key_id: &str,
) -> Result<ImportPlan, AppError> {
    if packet.format != PACKET_FORMAT {
        return Err(invalid(format!("expected a {PACKET_FORMAT} packet")));
    }
    if packet.annotations.len() > MAXIMUM_ANNOTATIONS {
        return Err(invalid(format!(
            "a packet holds at most {MAXIMUM_ANNOTATIONS} annotations"
        )));
    }
    let mut plan = ImportPlan {
        to_add: Vec::new(),
        already_present: 0,
        conflicts: Vec::new(),
    };
    for mark in &packet.annotations {
        let incoming = validate_mark(packet, mark, signer_key_id)?;
        if plan.to_add.iter().any(|seen| seen.id == incoming.id) {
            return Err(invalid("a packet lists the same annotation twice"));
        }
        match local
            .annotations
            .iter()
            .find(|existing| existing.id == incoming.id)
        {
            None => plan.to_add.push(incoming),
            Some(existing) => {
                let same = DocumentAnnotation {
                    imported_from_key: existing.imported_from_key.clone(),
                    ..incoming.clone()
                } == *existing;
                if same {
                    plan.already_present += 1;
                } else {
                    plan.conflicts.push(AnnotationImportConflict {
                        local: existing.clone(),
                        incoming,
                    });
                }
            }
        }
    }
    Ok(plan)
}

pub fn apply_import(
    document: &mut AnnotationDocument,
    packet: &AnnotationPacket,
    signer_key_id: &str,
) -> Result<AnnotationImportResult, AppError> {
    let plan = plan_import(document, packet, signer_key_id)?;
    if document.annotations.len() + plan.to_add.len() > MAXIMUM_ANNOTATIONS {
        return Err(invalid(format!(
            "importing would exceed the {MAXIMUM_ANNOTATIONS} annotations kept; delete some first"
        )));
    }
    let added = plan.to_add.len();
    document.annotations.extend(plan.to_add);
    Ok(AnnotationImportResult {
        added,
        already_present: plan.already_present,
        conflicts: plan.conflicts.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mark(id: &str, note: &str) -> DocumentAnnotation {
        DocumentAnnotation {
            id: id.into(),
            portal_key: "sd".into(),
            document_id: 7,
            kind: AnnotationKind::Issue,
            character_offset: Some(10),
            quote: "was amended".into(),
            prefix: "contract ".into(),
            suffix: " in May".into(),
            text_sha256: Some("ab".repeat(32)),
            context_unanchored: false,
            note: note.into(),
            tag: Some("AMEND".into()),
            author: Some("Maria".into()),
            status: AnnotationStatus::Confirmed,
            finding_id: None,
            imported_from_key: None,
            created_at: "2026-01-01T00:00:00Z".into(),
        }
    }

    fn packet(marks: Vec<DocumentAnnotation>) -> AnnotationPacket {
        AnnotationPacket {
            format: PACKET_FORMAT.into(),
            exported_at: "2026-01-02T00:00:00Z".into(),
            app_version: "1.0.0".into(),
            portal_key: "sd".into(),
            document_id: 7,
            document_text_sha256: None,
            exporter: Some("Maria".into()),
            annotations: marks,
        }
    }

    const A: &str = "11111111-1111-4111-8111-111111111111";
    const B: &str = "22222222-2222-4222-8222-222222222222";

    #[test]
    fn an_import_never_overwrites_and_reports_a_differing_copy() {
        let mut local = AnnotationDocument::default();
        let mut mine = mark(A, "my reading");
        mine.status = AnnotationStatus::Disputed;
        local.annotations.push(mine.clone());

        let incoming = packet(vec![mark(A, "their reading"), mark(B, "new one")]);
        let result = apply_import(&mut local, &incoming, "signer").unwrap();
        assert_eq!(
            (result.added, result.already_present, result.conflicts),
            (1, 0, 1)
        );
        assert_eq!(local.annotations[0], mine);
        let added = local.annotations.iter().find(|a| a.id == B).unwrap();
        assert_eq!(added.imported_from_key.as_deref(), Some("signer"));

        // The same packet again changes nothing.
        let again = apply_import(&mut local, &incoming, "signer").unwrap();
        assert_eq!(
            (again.added, again.already_present, again.conflicts),
            (0, 1, 1)
        );
        assert_eq!(local.annotations.len(), 2);
    }

    #[test]
    fn an_invalid_mark_rejects_the_whole_packet() {
        let mut local = AnnotationDocument::default();
        let mut elsewhere = mark(B, "x");
        elsewhere.document_id = 8;
        let mut long = mark(B, "x");
        long.note = "n".repeat(MAXIMUM_NOTE_CHARACTERS + 1);
        let mut bad_digest = mark(B, "x");
        bad_digest.text_sha256 = Some("zz".into());
        let mut no_passage = mark(B, "x");
        no_passage.quote.clear();
        for bad in [
            vec![mark(A, "ok"), elsewhere],
            vec![mark(A, "ok"), long],
            vec![mark(A, "ok"), bad_digest],
            vec![mark(A, "ok"), no_passage],
            vec![mark("not-a-uuid", "x")],
            vec![mark(A, "x"), mark(A, "y")],
        ] {
            assert!(apply_import(&mut local, &packet(bad), "signer").is_err());
        }
        assert!(local.annotations.is_empty());
    }

    #[test]
    fn export_carries_only_this_documents_own_marks() {
        let mut document = AnnotationDocument {
            reviewer: Some("Maria".into()),
            ..Default::default()
        };
        let mut theirs = mark(B, "theirs");
        theirs.imported_from_key = Some("other".into());
        let mut elsewhere = mark("33333333-3333-4333-8333-333333333333", "else");
        elsewhere.document_id = 9;
        document.annotations = vec![mark(A, "mine"), theirs, elsewhere];
        let built = build_packet(&document, "sd", 7, None, "1.0.0");
        assert_eq!(built.annotations.len(), 1);
        assert_eq!(built.annotations[0].id, A);
        assert_eq!(built.exporter.as_deref(), Some("Maria"));
    }
}
