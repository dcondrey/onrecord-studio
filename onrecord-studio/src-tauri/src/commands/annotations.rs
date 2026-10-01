//! IPC commands for reviewer annotations. Local to this device; nothing here is sent anywhere or
//! read by a detector. The only thing that leaves is a packet the reviewer exports.

use crate::{
    annotation_anchor::TextView,
    annotation_packet,
    annotations::{self, clean_text, invalid, AnnotationStore, NewMark},
    blocking, codebook,
    error::AppError,
    integrity::{
        check_revocation, load_or_create_signing_key, load_revocations, sign_package,
        verify_package_detailed, SignedPackage, REVOCATION_STATUS_RETIRED,
    },
    storage::sqlite::DuckDbRepository,
    types::annotation::*,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::State;

/// A signed packet carries the marks twice (canonical bytes and the readable tree), so it runs
/// larger than the marks themselves. Checked before an untrusted payload is parsed.
const MAXIMUM_SIGNED_PACKET_BYTES: usize = 16 * 1024 * 1024;

async fn on_store<T, F>(
    store: &Arc<AnnotationStore>,
    task: &'static str,
    operation: F,
) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce(&AnnotationStore) -> Result<T, AppError> + Send + 'static,
{
    let store = Arc::clone(store);
    blocking::spawn(task, move || operation(&store)).await?
}

struct DocumentFacts {
    request_id: Option<u64>,
    file_name: Option<String>,
    /// Digest of the stored text as recorded; `None` when the document predates the column.
    text_sha256: Option<String>,
}

fn portal_key(value: &str) -> Result<String, AppError> {
    let key = clean_text("portal key", value, MAXIMUM_PORTAL_KEY_CHARACTERS, false)?;
    if key.is_empty() {
        return Err(invalid("portal key is required"));
    }
    Ok(key)
}

/// Facts about a public document, or `None` when it is absent or not public.
async fn document_facts(
    repository: &DuckDbRepository,
    portal: &str,
    document_id: u64,
) -> Result<Option<DocumentFacts>, AppError> {
    let portal = portal.to_string();
    let id = i64::try_from(document_id)
        .map_err(|_| AppError::DataParsing("Document id exceeds capacity".to_string()))?;
    repository
        .query_connection("Annotation document facts", move |conn| {
            let row = conn.query_row(
                "SELECT request_id, file_name, extracted_text_projection_sha256
                 FROM documents
                 WHERE id = ?1 AND portal_subdomain = ?2 AND LOWER(TRIM(visibility)) = 'public'",
                duckdb::params![id, portal],
                |row| {
                    Ok((
                        row.get::<_, Option<i64>>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            );
            match row {
                Ok((request_id, file_name, digest)) => Ok(Some(DocumentFacts {
                    request_id: request_id.and_then(|value| u64::try_from(value).ok()),
                    file_name,
                    text_sha256: digest
                        .map(|value| value.trim().to_ascii_lowercase())
                        .filter(|value| !value.is_empty()),
                })),
                Err(duckdb::Error::QueryReturnedNoRows) => Ok(None),
                Err(error) => Err(AppError::Storage(format!(
                    "Failed to read document facts: {error}"
                ))),
            }
        })
        .await
}

/// The stored extracted text of a public document, or `None` when there is none to read.
async fn document_text(
    repository: &DuckDbRepository,
    portal: &str,
    document_id: u64,
) -> Result<Option<String>, AppError> {
    let portal = portal.to_string();
    let id = i64::try_from(document_id)
        .map_err(|_| AppError::DataParsing("Document id exceeds capacity".to_string()))?;
    repository
        .query_connection("Annotation document text", move |conn| {
            let row = conn.query_row(
                "SELECT f.extracted_text
                 FROM documents d
                 JOIN documents_fts f
                   ON f.document_id = d.id AND f.portal_subdomain = d.portal_subdomain
                 WHERE d.id = ?1 AND d.portal_subdomain = ?2
                   AND LOWER(TRIM(d.visibility)) = 'public'
                 LIMIT 1",
                duckdb::params![id, portal],
                |row| row.get::<_, Option<String>>(0),
            );
            match row {
                Ok(text) => Ok(text),
                Err(duckdb::Error::QueryReturnedNoRows) => Ok(None),
                Err(error) => Err(AppError::Storage(format!(
                    "Failed to read document text: {error}"
                ))),
            }
        })
        .await
}

async fn text_view(
    repository: &DuckDbRepository,
    portal: &str,
    document_id: u64,
) -> Result<Option<Arc<TextView>>, AppError> {
    let Some(text) = document_text(repository, portal, document_id).await? else {
        return Ok(None);
    };
    let view = blocking::spawn("Annotation text view", move || TextView::new(&text)).await??;
    Ok(Some(Arc::new(view)))
}

fn resolve(view: Option<&TextView>, annotation: DocumentAnnotation) -> ResolvedAnnotation {
    let anchor = view.and_then(|view| view.resolve(&annotation));
    ResolvedAnnotation { annotation, anchor }
}

async fn list_annotations(
    portal_key: String,
    document_id: u64,
    store: &Arc<AnnotationStore>,
    repository: &DuckDbRepository,
) -> Result<DocumentAnnotationList, AppError> {
    let portal = self::portal_key(&portal_key)?;
    let matching: Vec<DocumentAnnotation> = {
        let portal = portal.clone();
        on_store(store, "Annotation read", move |store| {
            Ok(store
                .load()?
                .annotations
                .into_iter()
                .filter(|annotation| {
                    annotation.portal_key == portal && annotation.document_id == document_id
                })
                .collect())
        })
        .await?
    };
    let facts = document_facts(repository, &portal, document_id).await?;
    let view = if matching
        .iter()
        .any(|annotation| !annotation.quote.is_empty())
    {
        text_view(repository, &portal, document_id).await?
    } else {
        None
    };
    let mut annotations: Vec<ResolvedAnnotation> = matching
        .into_iter()
        .map(|annotation| resolve(view.as_deref(), annotation))
        .collect();
    annotations.sort_by_key(|resolved| {
        let position = resolved
            .anchor
            .as_ref()
            .and_then(|anchor| anchor.source_offset)
            .or(resolved.annotation.character_offset)
            .unwrap_or(0);
        (position, resolved.annotation.created_at.clone())
    });
    Ok(DocumentAnnotationList {
        text_available: view.is_some(),
        current_text_sha256: facts.and_then(|facts| facts.text_sha256),
        annotations,
    })
}

async fn add_annotation(
    input: AddAnnotationInput,
    store: &Arc<AnnotationStore>,
    repository: &DuckDbRepository,
) -> Result<DocumentAnnotation, AppError> {
    let portal = self::portal_key(&input.portal_key)?;
    let reviewer = on_store(store, "Annotation read", |store| Ok(store.load()?.reviewer))
        .await?
        .ok_or_else(|| invalid("enter your reviewer name before adding annotations"))?;
    let quote = clean_text("quote", &input.quote, MAXIMUM_QUOTE_CHARACTERS, true)?;
    let text_sha256 = document_facts(repository, &portal, input.document_id)
        .await?
        .and_then(|facts| facts.text_sha256);
    let placement = if quote.is_empty() {
        None
    } else {
        let view = text_view(repository, &portal, input.document_id)
            .await?
            .ok_or_else(|| invalid("this document's text is not available to anchor to"))?;
        Some(view.place_selection(&quote, &input.prefix, &input.suffix)?)
    };
    on_store(store, "Annotation write", move |store| {
        store.update(|document| {
            annotations::add_annotation(
                document,
                input,
                NewMark {
                    author: reviewer,
                    placement,
                    text_sha256,
                },
            )
        })
    })
    .await
}

#[tauri::command]
pub async fn remove_document_annotation(
    id: String,
    store: State<'_, Arc<AnnotationStore>>,
) -> Result<(), AppError> {
    on_store(&store, "Annotation removal", move |store| {
        store.update(|document| annotations::remove_annotation(document, &id))
    })
    .await
}

#[tauri::command]
pub async fn set_annotation_status(
    id: String,
    status: AnnotationStatus,
    store: State<'_, Arc<AnnotationStore>>,
) -> Result<(), AppError> {
    on_store(&store, "Annotation status", move |store| {
        store.update(|document| annotations::set_status(document, &id, status))
    })
    .await
}

fn settings(document: AnnotationDocument) -> AnnotationSettings {
    AnnotationSettings {
        reviewer: document.reviewer,
        codebook: document.codebook,
    }
}

#[tauri::command]
pub async fn get_annotation_settings(
    store: State<'_, Arc<AnnotationStore>>,
) -> Result<AnnotationSettings, AppError> {
    on_store(&store, "Annotation settings read", |store| {
        Ok(settings(store.load()?))
    })
    .await
}

#[tauri::command]
pub async fn set_annotation_reviewer(
    reviewer: String,
    store: State<'_, Arc<AnnotationStore>>,
) -> Result<AnnotationSettings, AppError> {
    on_store(&store, "Annotation reviewer", move |store| {
        store.update(|document| {
            annotations::set_reviewer(document, &reviewer)?;
            Ok(settings(document.clone()))
        })
    })
    .await
}

#[tauri::command]
pub async fn save_codebook_entry(
    entry: CodebookEntry,
    store: State<'_, Arc<AnnotationStore>>,
) -> Result<AnnotationSettings, AppError> {
    on_store(&store, "Codebook write", move |store| {
        store.update(|document| {
            codebook::upsert(document, &entry)?;
            Ok(settings(document.clone()))
        })
    })
    .await
}

#[tauri::command]
pub async fn delete_codebook_entry(
    code: String,
    store: State<'_, Arc<AnnotationStore>>,
) -> Result<AnnotationSettings, AppError> {
    on_store(&store, "Codebook removal", move |store| {
        store.update(|document| {
            codebook::remove(document, &code)?;
            Ok(settings(document.clone()))
        })
    })
    .await
}

#[tauri::command]
pub async fn export_codebook(store: State<'_, Arc<AnnotationStore>>) -> Result<String, AppError> {
    on_store(&store, "Codebook export", |store| {
        codebook::export(&store.load()?.codebook)
    })
    .await
}

#[tauri::command]
pub async fn preview_codebook_import(
    payload: String,
    store: State<'_, Arc<AnnotationStore>>,
) -> Result<CodebookImportPreview, AppError> {
    on_store(&store, "Codebook preview", move |store| {
        codebook::preview_import(&store.load()?.codebook, &payload)
    })
    .await
}

#[tauri::command]
pub async fn apply_codebook_import(
    payload: String,
    overwrite_codes: Vec<String>,
    store: State<'_, Arc<AnnotationStore>>,
) -> Result<AnnotationSettings, AppError> {
    on_store(&store, "Codebook import", move |store| {
        store.update(|document| {
            codebook::apply_import(document, &payload, &overwrite_codes)?;
            Ok(settings(document.clone()))
        })
    })
    .await
}

/// Orders annotations for paging: by document, then position, then id, so a cursor names a place
/// in the sequence that adding or removing other marks cannot move.
fn sort_key(annotation: &DocumentAnnotation) -> (String, u64, u64, String) {
    (
        annotation.portal_key.clone(),
        annotation.document_id,
        annotation.character_offset.unwrap_or(0),
        annotation.id.clone(),
    )
}

fn parse_cursor(cursor: &str) -> Result<(String, u64, u64, String), AppError> {
    serde_json::from_str(cursor).map_err(|_| invalid("the page cursor is not valid"))
}

fn matches_filters(annotation: &DocumentAnnotation, query: &AnnotationSearchQuery) -> bool {
    if let Some(code) = query.code.as_deref().filter(|code| !code.is_empty()) {
        if annotation.tag.as_deref() != Some(code) {
            return false;
        }
    }
    if let Some(author) = query.author.as_deref().filter(|author| !author.is_empty()) {
        if annotation.author.as_deref() != Some(author) {
            return false;
        }
    }
    if let Some(status) = query.status {
        if annotation.status != status {
            return false;
        }
    }
    if let Some(text) = query
        .text
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
    {
        let needle = text.to_lowercase();
        let haystack_hits = [
            Some(annotation.quote.as_str()),
            Some(annotation.note.as_str()),
            annotation.tag.as_deref(),
        ]
        .into_iter()
        .flatten()
        .any(|field| field.to_lowercase().contains(&needle));
        if !haystack_hits {
            return false;
        }
    }
    true
}

struct DocumentContext {
    facts: Option<DocumentFacts>,
    view: Option<Option<Arc<TextView>>>,
}

/// Lists annotations across every document. The text is read only for a document whose recorded
/// text digest differs from the one the mark was made against (or where either is unknown): an
/// unchanged digest means the text, and so the position, is the one recorded.
async fn search_annotations(
    query: AnnotationSearchQuery,
    store: &Arc<AnnotationStore>,
    repository: &DuckDbRepository,
) -> Result<AnnotationSearchPage, AppError> {
    let limit = query.limit.unwrap_or(DEFAULT_SEARCH_PAGE);
    if !(1..=MAXIMUM_SEARCH_PAGE).contains(&limit) {
        return Err(invalid(format!(
            "a page is between 1 and {MAXIMUM_SEARCH_PAGE} annotations"
        )));
    }
    let after = query.cursor.as_deref().map(parse_cursor).transpose()?;
    let candidates = {
        let query = query.clone();
        on_store(store, "Annotation search", move |store| {
            let mut kept: Vec<DocumentAnnotation> = store
                .load()?
                .annotations
                .into_iter()
                .filter(|annotation| matches_filters(annotation, &query))
                .collect();
            kept.sort_by_key(sort_key);
            Ok(kept)
        })
        .await?
    };
    let mut contexts: HashMap<(String, u64), DocumentContext> = HashMap::new();
    let mut rows: Vec<AnnotationSearchRow> = Vec::new();
    let mut next_cursor = None;
    for annotation in candidates {
        if after
            .as_ref()
            .is_some_and(|after| sort_key(&annotation) <= *after)
        {
            continue;
        }
        let key = (annotation.portal_key.clone(), annotation.document_id);
        if !contexts.contains_key(&key) {
            let facts = document_facts(repository, &key.0, key.1).await?;
            contexts.insert(key.clone(), DocumentContext { facts, view: None });
        }
        let unchanged = {
            let context = &contexts[&key];
            matches!(
                (&annotation.text_sha256, context.facts.as_ref().and_then(|f| f.text_sha256.as_ref())),
                (Some(recorded), Some(current)) if recorded.eq_ignore_ascii_case(current)
            )
        };
        let needs_text = !annotation.quote.is_empty() && !unchanged;
        if needs_text && contexts[&key].view.is_none() {
            let view = if contexts[&key].facts.is_some() {
                text_view(repository, &key.0, key.1).await?
            } else {
                None
            };
            if let Some(context) = contexts.get_mut(&key) {
                context.view = Some(view);
            }
        }
        let context = &contexts[&key];
        let resolved = if annotation.quote.is_empty() {
            ResolvedAnnotation {
                annotation,
                anchor: None,
            }
        } else if unchanged {
            let offset = annotation.character_offset;
            ResolvedAnnotation {
                anchor: Some(ResolvedAnchor {
                    state: AnchorState::Anchored,
                    source_offset: offset,
                    source_length: None,
                    reason: None,
                }),
                annotation,
            }
        } else {
            resolve(
                context.view.as_ref().and_then(|view| view.as_deref()),
                annotation,
            )
        };
        let orphaned = resolved
            .anchor
            .as_ref()
            .is_some_and(|anchor| anchor.state == AnchorState::Orphaned);
        if query.orphaned_only && !orphaned {
            continue;
        }
        if rows.len() == limit {
            next_cursor = rows.last().map(|row| {
                serde_json::to_string(&sort_key(&row.resolved.annotation)).unwrap_or_default()
            });
            break;
        }
        rows.push(AnnotationSearchRow {
            request_id: context.facts.as_ref().and_then(|facts| facts.request_id),
            file_name: context
                .facts
                .as_ref()
                .and_then(|facts| facts.file_name.clone()),
            resolved,
        });
    }
    Ok(AnnotationSearchPage { rows, next_cursor })
}

#[tauri::command]
pub async fn export_document_annotations(
    portal_key: String,
    document_id: u64,
    store: State<'_, Arc<AnnotationStore>>,
    repository: State<'_, DuckDbRepository>,
    key_directory: State<'_, crate::integrity::SigningKeyDirectory>,
) -> Result<String, AppError> {
    let portal = self::portal_key(&portal_key)?;
    let text_sha256 = document_facts(&repository, &portal, document_id)
        .await?
        .and_then(|facts| facts.text_sha256);
    let packet = {
        let portal = portal.clone();
        on_store(&store, "Annotation export", move |store| {
            Ok(annotation_packet::build_packet(
                &store.load()?,
                &portal,
                document_id,
                text_sha256,
                env!("CARGO_PKG_VERSION"),
            ))
        })
        .await?
    };
    if packet.annotations.is_empty() {
        return Err(invalid(
            "this document has no annotations of your own to export",
        ));
    }
    let signing_key = load_or_create_signing_key(&key_directory.0).await?;
    let signed = sign_package(packet, &signing_key).map_err(AppError::Export)?;
    let envelope = serde_json::to_string_pretty(&signed).map_err(|error| {
        AppError::DataParsing(format!("Failed to encode signed annotations: {error}"))
    })?;
    if envelope.len() > MAXIMUM_SIGNED_PACKET_BYTES {
        return Err(AppError::DataParsing(format!(
            "The signed export exceeds the {MAXIMUM_SIGNED_PACKET_BYTES}-byte transport limit."
        )));
    }
    Ok(envelope)
}

struct VerifiedPacket {
    packet: AnnotationPacket,
    signer_key_id: String,
    packet_sha256: String,
}

/// Checks the signature and this installation's revocation list before anything in the packet is
/// read as annotations. Only an exact RETIRED status is informational; anything else fails
/// closed.
async fn verify_packet(
    payload: &str,
    report_directory: &std::path::Path,
) -> Result<VerifiedPacket, AppError> {
    if payload.len() > MAXIMUM_SIGNED_PACKET_BYTES {
        return Err(AppError::DataParsing(format!(
            "This packet exceeds the {MAXIMUM_SIGNED_PACKET_BYTES}-byte transport limit."
        )));
    }
    let package: SignedPackage<AnnotationPacket> =
        serde_json::from_str(payload).map_err(|error| {
            AppError::DataParsing(format!("This is not a signed annotation packet ({error})."))
        })?;
    let checks = verify_package_detailed(&package);
    if !checks.all_passed() {
        return Err(AppError::DataParsing(format!(
            "This packet's signature does not verify; the bytes have changed since it was signed. \
             It was not previewed or imported. {}",
            checks.detail.unwrap_or_default()
        )));
    }
    let signer_key_id = package.signature.signer_key_id.clone();
    let revocations = load_revocations(report_directory).await?;
    if let Some(revoked) = check_revocation(&signer_key_id, &revocations) {
        if revoked.status != REVOCATION_STATUS_RETIRED {
            return Err(AppError::DataParsing(format!(
                "This packet was signed by local export key {signer_key_id}, which this \
                 installation revoked as compromised on {} ({}). It was not previewed or \
                 imported.",
                revoked.revoked_at, revoked.reason
            )));
        }
    }
    Ok(VerifiedPacket {
        packet: package.manifest,
        signer_key_id,
        packet_sha256: package.manifest_sha256,
    })
}

#[tauri::command]
pub async fn preview_annotation_import(
    payload: String,
    store: State<'_, Arc<AnnotationStore>>,
    repository: State<'_, DuckDbRepository>,
    report_directory: State<'_, PathBuf>,
) -> Result<AnnotationImportPreview, AppError> {
    let verified = verify_packet(&payload, &report_directory).await?;
    let packet = verified.packet.clone();
    let signer = verified.signer_key_id.clone();
    let plan = on_store(&store, "Annotation import preview", move |store| {
        annotation_packet::plan_import(&store.load()?, &packet, &signer)
    })
    .await?;
    let local_text_sha256 = document_facts(
        &repository,
        &verified.packet.portal_key,
        verified.packet.document_id,
    )
    .await?
    .and_then(|facts| facts.text_sha256);
    Ok(AnnotationImportPreview {
        signer_key_id: verified.signer_key_id,
        packet_sha256: verified.packet_sha256,
        exported_at: verified.packet.exported_at,
        app_version: verified.packet.app_version,
        exporter: verified.packet.exporter,
        portal_key: verified.packet.portal_key,
        document_id: verified.packet.document_id,
        packet_text_sha256: verified.packet.document_text_sha256,
        local_text_sha256,
        to_add: plan.to_add,
        already_present: plan.already_present,
        conflicts: plan.conflicts,
    })
}

#[tauri::command]
pub async fn import_annotation_packet(
    payload: String,
    packet_sha256: String,
    store: State<'_, Arc<AnnotationStore>>,
    report_directory: State<'_, PathBuf>,
) -> Result<AnnotationImportResult, AppError> {
    let verified = verify_packet(&payload, &report_directory).await?;
    if verified.packet_sha256 != packet_sha256 {
        return Err(invalid(
            "this packet is not the one that was previewed; preview it again",
        ));
    }
    on_store(&store, "Annotation import", move |store| {
        store.update(|document| {
            annotation_packet::apply_import(document, &verified.packet, &verified.signer_key_id)
        })
    })
    .await
}

#[tauri::command]
pub async fn list_document_annotations(
    portal_key: String,
    document_id: u64,
    store: State<'_, Arc<AnnotationStore>>,
    repository: State<'_, DuckDbRepository>,
) -> Result<DocumentAnnotationList, AppError> {
    list_annotations(portal_key, document_id, &store, &repository).await
}

#[tauri::command]
pub async fn add_document_annotation(
    input: AddAnnotationInput,
    store: State<'_, Arc<AnnotationStore>>,
    repository: State<'_, DuckDbRepository>,
) -> Result<DocumentAnnotation, AppError> {
    add_annotation(input, &store, &repository).await
}

#[tauri::command]
pub async fn search_document_annotations(
    query: AnnotationSearchQuery,
    store: State<'_, Arc<AnnotationStore>>,
    repository: State<'_, DuckDbRepository>,
) -> Result<AnnotationSearchPage, AppError> {
    search_annotations(query, &store, &repository).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integrity::SignedPackage;
    use ed25519_dalek::SigningKey;

    const ORIGINAL: &str = "The fee was waived. The contract was amended in May. Done.";

    async fn repository_with(text: &str) -> DuckDbRepository {
        let repository = DuckDbRepository::new(":memory:").unwrap();
        repository
            .run_migrations(include_str!("../../migrations/0001_initial_schema.sql"))
            .await
            .unwrap();
        let conn = repository.analytics_connection().await.unwrap();
        conn.execute("INSERT INTO portals(subdomain,display_name,base_url,lifecycle_state) VALUES ('city','city','https://www.sandiego.gov','ACTIVE')", []).unwrap();
        conn.execute("INSERT INTO requests(id,portal_subdomain,title,status,created_at,updated_at) VALUES (42,'city','Fixture','closed','2026-09-26','2026-09-26')", []).unwrap();
        conn.execute("INSERT INTO documents(id,request_id,portal_subdomain,file_name,download_url,visibility,created_at,updated_at,raw_file_sha256,extracted_text_sha256,extracted_text_projection_sha256,extraction_status) VALUES (1,42,'city','contract.pdf','https://www.sandiego.gov/c.pdf','public','2026-09-26','2026-09-26',?1,?1,?2,'SEARCHABLE')", duckdb::params!["a".repeat(64), crate::integrity::sha256_hex(text.as_bytes())]).unwrap();
        conn.execute("INSERT INTO documents_fts(document_id,portal_subdomain,file_name,extracted_text) VALUES (1,'city','contract.pdf',?1)", duckdb::params![text]).unwrap();
        repository
    }

    async fn replace_text(repository: &DuckDbRepository, text: &str) {
        let conn = repository.analytics_connection().await.unwrap();
        conn.execute(
            "UPDATE documents_fts SET extracted_text = ?1 WHERE document_id = 1",
            duckdb::params![text],
        )
        .unwrap();
        conn.execute(
            "UPDATE documents SET extracted_text_projection_sha256 = ?1 WHERE id = 1",
            duckdb::params![crate::integrity::sha256_hex(text.as_bytes())],
        )
        .unwrap();
    }

    fn highlight(quote: &str, prefix: &str, suffix: &str) -> AddAnnotationInput {
        AddAnnotationInput {
            portal_key: "city".into(),
            document_id: 1,
            kind: AnnotationKind::Highlight,
            quote: quote.into(),
            prefix: prefix.into(),
            suffix: suffix.into(),
            note: String::new(),
            tag: None,
            finding_id: None,
        }
    }

    #[tokio::test]
    async fn marks_follow_edited_text_and_orphans_are_found_across_documents_in_pages() {
        let repository = repository_with(ORIGINAL).await;
        let directory = tempfile::tempdir().unwrap();
        let store = Arc::new(AnnotationStore::new(directory.path()));
        assert!(
            add_annotation(highlight("fee", "", ""), &store, &repository)
                .await
                .is_err()
        );
        store
            .update(|document| annotations::set_reviewer(document, "Maria"))
            .unwrap();
        for (quote, prefix, suffix) in [
            ("contract was amended", "The ", " in May"),
            ("fee was waived", "", ""),
            ("Done", "", ""),
        ] {
            add_annotation(highlight(quote, prefix, suffix), &store, &repository)
                .await
                .unwrap();
        }
        // "e" appears many times: refused rather than filed under one of them.
        assert!(add_annotation(highlight("e", "", ""), &store, &repository)
            .await
            .is_err());

        let state = |list: &DocumentAnnotationList| -> Vec<(String, AnchorState, Option<u64>)> {
            list.annotations
                .iter()
                .map(|item| {
                    let anchor = item.anchor.clone().unwrap();
                    (
                        item.annotation.quote.clone(),
                        anchor.state,
                        anchor.source_offset,
                    )
                })
                .collect()
        };
        let list = list_annotations("city".into(), 1, &store, &repository)
            .await
            .unwrap();
        assert!(list.current_text_sha256.is_some());
        assert_eq!(
            state(&list)[1],
            (
                "contract was amended".to_string(),
                AnchorState::Anchored,
                Some(ORIGINAL.find("contract").unwrap() as u64)
            )
        );
        assert!(state(&list)
            .iter()
            .all(|(_, s, _)| *s == AnchorState::Anchored));

        // Text inserted before the marks moves two of them and removes one.
        let edited = ORIGINAL.replace("Done", "Finished");
        replace_text(&repository, &format!("Draft. {edited}")).await;
        let list = list_annotations("city".into(), 1, &store, &repository)
            .await
            .unwrap();
        let after = state(&list);
        let moved = |quote: &str| after.iter().find(|(q, _, _)| q == quote).unwrap().clone();
        assert_eq!(
            moved("contract was amended"),
            (
                "contract was amended".into(),
                AnchorState::ReAnchored,
                Some(ORIGINAL.find("contract").unwrap() as u64 + 7)
            )
        );
        assert_eq!(moved("Done").1, AnchorState::Orphaned);
        assert_eq!(moved("Done").2, None);

        // Across documents: every match once, in pages, with no silent cap.
        let mut seen = Vec::new();
        let mut cursor = None;
        loop {
            let page = search_annotations(
                AnnotationSearchQuery {
                    limit: Some(2),
                    cursor: cursor.clone(),
                    ..Default::default()
                },
                &store,
                &repository,
            )
            .await
            .unwrap();
            seen.extend(
                page.rows
                    .iter()
                    .map(|row| row.resolved.annotation.quote.clone()),
            );
            assert_eq!(page.rows[0].file_name.as_deref(), Some("contract.pdf"));
            assert_eq!(page.rows[0].request_id, Some(42));
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
            assert_eq!(page.rows.len(), 2);
        }
        seen.sort();
        assert_eq!(seen, ["Done", "contract was amended", "fee was waived"]);

        let orphans = search_annotations(
            AnnotationSearchQuery {
                orphaned_only: true,
                ..Default::default()
            },
            &store,
            &repository,
        )
        .await
        .unwrap();
        let quotes: Vec<_> = orphans
            .rows
            .iter()
            .map(|r| r.resolved.annotation.quote.as_str())
            .collect();
        assert_eq!(quotes, ["Done"]);
        assert_eq!(orphans.next_cursor, None);
    }

    fn packet() -> AnnotationPacket {
        annotation_packet::build_packet(&AnnotationDocument::default(), "sd", 7, None, "1.0.0")
    }

    #[tokio::test]
    async fn a_changed_or_unsigned_packet_is_rejected_before_it_is_read() {
        let directory = tempfile::tempdir().unwrap();
        let signed =
            crate::integrity::sign_package(packet(), &SigningKey::from_bytes(&[7_u8; 32])).unwrap();
        let good = serde_json::to_string(&signed).unwrap();
        let verified = verify_packet(&good, directory.path()).await.unwrap();
        assert_eq!(verified.packet.document_id, 7);

        let mut tampered: SignedPackage<AnnotationPacket> = signed;
        tampered.manifest.document_id = 8;
        let tampered = serde_json::to_string(&tampered).unwrap();
        assert!(verify_packet(&tampered, directory.path()).await.is_err());

        let unsigned = serde_json::to_string(&packet()).unwrap();
        assert!(verify_packet(&unsigned, directory.path()).await.is_err());
    }
}
