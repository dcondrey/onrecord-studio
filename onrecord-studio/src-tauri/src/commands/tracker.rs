//! IPC commands for "My requests".
//!
//! The tracker holds what the user typed and dropped in. No portal credential is stored or used,
//! nothing is fetched, and nothing here is an input to any finding or detector.

use crate::{
    blocking,
    error::AppError,
    storage::sqlite::DuckDbRepository,
    tracker::{self, deadline, events, TrackerStore},
    types::tracker::*,
};
use chrono::{SecondsFormat, Utc};
use std::sync::Arc;
use tauri::State;

fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

async fn registered_portal_keys(repository: &DuckDbRepository) -> Result<Vec<String>, AppError> {
    Ok(repository
        .list_portal_registrations()
        .await?
        .into_iter()
        .map(|saved| saved.registration.portal_key)
        .collect())
}

#[tauri::command]
pub async fn list_claimed_requests(
    store: State<'_, Arc<TrackerStore>>,
) -> Result<Vec<ClaimedRequest>, AppError> {
    blocking::on_shared(&store, "Tracker read", |store| Ok(store.load()?.claims)).await
}

#[tauri::command]
pub async fn claim_request(
    input: ClaimRequestInput,
    store: State<'_, Arc<TrackerStore>>,
    repository: State<'_, DuckDbRepository>,
) -> Result<ClaimedRequest, AppError> {
    let portals = registered_portal_keys(&repository).await?;
    blocking::on_shared(&store, "Tracker claim", move |store| {
        store.update(|document| tracker::claim_request(document, input, &portals, &now()))
    })
    .await
}

#[tauri::command]
pub async fn update_claimed_request(
    input: UpdateClaimedRequestInput,
    store: State<'_, Arc<TrackerStore>>,
    repository: State<'_, DuckDbRepository>,
) -> Result<ClaimedRequest, AppError> {
    let portals = registered_portal_keys(&repository).await?;
    blocking::on_shared(&store, "Tracker update", move |store| {
        store.update(|document| tracker::update_claim(document, input, &portals))
    })
    .await
}

#[tauri::command]
pub async fn remove_claimed_request(
    claim_id: String,
    store: State<'_, Arc<TrackerStore>>,
) -> Result<(), AppError> {
    blocking::on_shared(&store, "Tracker removal", move |store| {
        store.update(|document| tracker::remove_claim(document, &claim_id))
    })
    .await
}

#[tauri::command]
pub async fn add_timeline_event(
    input: AddTimelineEventInput,
    store: State<'_, Arc<TrackerStore>>,
) -> Result<TimelineEvent, AppError> {
    blocking::on_shared(&store, "Tracker event", move |store| {
        store.update(|document| tracker::add_event(document, input, &now()))
    })
    .await
}

#[tauri::command]
pub async fn remove_timeline_event(
    input: RemoveTimelineEventInput,
    store: State<'_, Arc<TrackerStore>>,
) -> Result<(), AppError> {
    blocking::on_shared(&store, "Tracker event removal", move |store| {
        store.update(|document| tracker::remove_event(document, &input.claim_id, &input.event_id))
    })
    .await
}

#[tauri::command]
pub async fn confirm_suggested_event(
    input: ConfirmSuggestedEventInput,
    store: State<'_, Arc<TrackerStore>>,
) -> Result<TimelineEvent, AppError> {
    blocking::on_shared(&store, "Tracker confirmation", move |store| {
        store.update(|document| tracker::confirm_suggestion(document, input, &now()))
    })
    .await
}

#[tauri::command]
pub async fn reject_suggested_event(
    input: RejectSuggestedEventInput,
    store: State<'_, Arc<TrackerStore>>,
) -> Result<(), AppError> {
    blocking::on_shared(&store, "Tracker rejection", move |store| {
        store.update(|document| {
            tracker::reject_suggestion(document, &input.claim_id, &input.suggestion_id)
        })
    })
    .await
}

/// Reads each chosen file (never following a link, never opening an attachment), then records all
/// of them in one write. A file that cannot be read is reported and skipped; it does not fail the
/// others.
#[tauri::command]
pub async fn import_request_emails(
    input: ImportRequestEmailsInput,
    store: State<'_, Arc<TrackerStore>>,
) -> Result<EmailImportReport, AppError> {
    if input.paths.is_empty() || input.paths.len() > MAXIMUM_IMPORT_PATHS {
        return Err(AppError::DataParsing(format!(
            "TRACKER_INVALID: choose between 1 and {MAXIMUM_IMPORT_PATHS} email files at a time"
        )));
    }
    let mut read = Vec::with_capacity(input.paths.len());
    for path in &input.paths {
        read.push(events::read_email_file(path).await);
    }
    let claim_id = input.claim_id;
    blocking::on_shared(&store, "Tracker email import", move |store| {
        store.update(|document| {
            let stamp = now();
            let mut files = Vec::with_capacity(read.len());
            for (path, outcome) in input.paths.iter().zip(read) {
                match outcome {
                    Ok(email) => {
                        files.push(tracker::import_email(document, &claim_id, &email, &stamp)?)
                    }
                    Err(error) => files.push(EmailImportFileResult {
                        file_name: std::path::Path::new(path)
                            .file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_default(),
                        outcome: EmailImportOutcome::Rejected,
                        suggestions_added: 0,
                        detail: Some(error.to_string()),
                    }),
                }
            }
            let claim = tracker::claim_mut(document, &claim_id)?.clone();
            Ok(EmailImportReport { claim, files })
        })
    })
    .await
}

#[tauri::command]
pub async fn get_request_deadline(
    claim_id: String,
    store: State<'_, Arc<TrackerStore>>,
) -> Result<RequestDeadline, AppError> {
    blocking::on_shared(&store, "Tracker deadline", move |store| {
        let document = store.load()?;
        let claim = document
            .claims
            .iter()
            .find(|claim| claim.id == claim_id)
            .ok_or_else(|| {
                AppError::DataParsing("TRACKER_INVALID: no such tracked request".to_string())
            })?;
        Ok(deadline::request_deadline(claim, Utc::now()))
    })
    .await
}
