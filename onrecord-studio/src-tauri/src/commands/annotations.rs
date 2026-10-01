//! IPC commands for reviewer annotations. Local to this device; nothing here is sent anywhere or
//! read by a detector.

use crate::{
    annotations::{self, AnnotationStore},
    blocking,
    error::AppError,
    types::annotation::*,
};
use std::sync::Arc;
use tauri::State;

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

#[tauri::command]
pub async fn list_document_annotations(
    portal_key: String,
    document_id: u64,
    store: State<'_, Arc<AnnotationStore>>,
) -> Result<Vec<DocumentAnnotation>, AppError> {
    on_store(&store, "Annotation read", move |store| {
        let mut matching: Vec<DocumentAnnotation> = store
            .load()?
            .annotations
            .into_iter()
            .filter(|annotation| {
                annotation.portal_key == portal_key && annotation.document_id == document_id
            })
            .collect();
        matching.sort_by_key(|annotation| annotation.character_offset);
        Ok(matching)
    })
    .await
}

#[tauri::command]
pub async fn add_document_annotation(
    input: AddAnnotationInput,
    store: State<'_, Arc<AnnotationStore>>,
) -> Result<DocumentAnnotation, AppError> {
    on_store(&store, "Annotation write", move |store| {
        store.update(|document| annotations::add_annotation(document, input))
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
