//! Release events a portal's request timeline records ("Document(s) Released"). Each row says
//! that the portal recorded an upload of documents to the request at that time; it is not the
//! agency's determination and never feeds `determination_at`.

use super::connection::execute_transaction;
use super::*;

/// A release visible to anyone with access to the request.
pub const RELEASED: &str = "Document(s) Released";
/// A release shown only to the requester and staff.
pub const RELEASED_TO_REQUESTER: &str = "Document(s) Released to Requester";

/// One release event as the timeline recorded it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseEvent {
    /// The portal's own identifier for the event, when it gave one.
    pub timeline_event_id: Option<i64>,
    /// [`RELEASED`] or [`RELEASED_TO_REQUESTER`].
    pub event_name: String,
    /// The byline's wall-clock time with the offset it is recorded at (RFC 3339).
    pub released_at: String,
    /// The file names the event lists. Empty when it lists none, which is not a count of zero.
    pub listed_file_names: Vec<String>,
    /// The byline exactly as the portal gave it ("September  7, 2022,  7:14am by ...").
    pub source_byline: String,
}

/// One stored release of a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredRelease {
    pub event_name: String,
    pub released_at: String,
    /// `None` when the event listed no file names.
    pub listed_document_count: Option<u32>,
}

/// A request's stored releases, oldest first, with what identifies the request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestReleaseHistory {
    pub portal_key: String,
    pub request_id: i64,
    pub rpid: Option<String>,
    pub agency_id: Option<i64>,
    pub releases: Vec<StoredRelease>,
}

fn validate(event: &ReleaseEvent) -> Result<(), AppError> {
    if event.event_name != RELEASED && event.event_name != RELEASED_TO_REQUESTER {
        return Err(AppError::DataParsing(format!(
            "'{}' is not a release event",
            event.event_name
        )));
    }
    if chrono::DateTime::parse_from_rfc3339(&event.released_at).is_err() {
        return Err(AppError::DataParsing(format!(
            "Release time '{}' is not RFC 3339",
            event.released_at
        )));
    }
    if event
        .listed_file_names
        .iter()
        .any(|name| name.contains('\n'))
    {
        return Err(AppError::DataParsing(
            "A listed file name spans lines".to_string(),
        ));
    }
    Ok(())
}

impl DuckDbRepository {
    /// Replaces a request's release events with what its timeline now records. The request row
    /// must already exist: the relation is logical, so this write path is what keeps it whole.
    pub async fn replace_release_events(
        &self,
        portal_key: &str,
        request_id: u64,
        events: Vec<ReleaseEvent>,
    ) -> Result<(), AppError> {
        for event in &events {
            validate(event)?;
        }
        let request_id = i64::try_from(request_id).map_err(|_| {
            AppError::DataParsing("Request identifier exceeds capacity".to_string())
        })?;
        let portal_key = portal_key.to_string();
        self.query_connection("Replace release events", move |conn| {
            execute_transaction(conn, "release events", |tx| {
                let exists: i64 = tx
                    .query_row(
                        "SELECT COUNT(*) FROM requests WHERE id = ?1 AND portal_subdomain = ?2",
                        params![request_id, portal_key],
                        |row| row.get(0),
                    )
                    .map_err(|error| {
                        AppError::Storage(format!("Failed to find the released request: {error}"))
                    })?;
                if exists == 0 {
                    return Err(AppError::Storage(format!(
                        "Release events name request {request_id}, which is not stored"
                    )));
                }
                tx.execute(
                    "DELETE FROM request_release_events
                     WHERE portal_subdomain = ?1 AND request_id = ?2",
                    params![portal_key, request_id],
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to clear release events: {error}"))
                })?;
                for (ordinal, event) in events.iter().enumerate() {
                    let listed = (!event.listed_file_names.is_empty())
                        .then(|| event.listed_file_names.join("\n"));
                    let count = (!event.listed_file_names.is_empty())
                        .then(|| i64::try_from(event.listed_file_names.len()))
                        .transpose()
                        .map_err(|_| {
                            AppError::DataParsing("Listed file count exceeds capacity".to_string())
                        })?;
                    tx.execute(
                        "INSERT INTO request_release_events (
                            portal_subdomain, request_id, event_ordinal, timeline_event_id,
                            event_name, released_at, listed_document_count, listed_file_names,
                            source_byline
                         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                        params![
                            portal_key,
                            request_id,
                            ordinal as i64,
                            event.timeline_event_id,
                            event.event_name,
                            event.released_at,
                            count,
                            listed,
                            event.source_byline,
                        ],
                    )
                    .map_err(|error| {
                        AppError::Storage(format!("Failed to record a release event: {error}"))
                    })?;
                }
                Ok(())
            })
        })
        .await
    }

    /// Release histories of requests with at least `minimum_events` stored releases, up to
    /// `limit` requests.
    pub async fn request_release_histories(
        &self,
        minimum_events: usize,
        limit: usize,
    ) -> Result<Vec<RequestReleaseHistory>, AppError> {
        let minimum_events = i64::try_from(minimum_events)
            .map_err(|_| AppError::DataParsing("Event minimum exceeds capacity".to_string()))?;
        let limit = i64::try_from(limit)
            .map_err(|_| AppError::DataParsing("Request limit exceeds capacity".to_string()))?;
        self.query_connection("Read release histories", move |conn| {
            let mut statement = conn
                .prepare(
                    "WITH candidates AS (
                        SELECT portal_subdomain, request_id FROM request_release_events
                        GROUP BY portal_subdomain, request_id
                        HAVING COUNT(*) >= ?1
                        ORDER BY portal_subdomain, request_id
                        LIMIT ?2
                     )
                     SELECT events.portal_subdomain, events.request_id, requests.rpid,
                        requests.agency_id, events.event_name, events.released_at,
                        events.listed_document_count
                     FROM candidates
                     JOIN request_release_events events
                       ON events.portal_subdomain = candidates.portal_subdomain
                      AND events.request_id = candidates.request_id
                     JOIN requests
                       ON requests.id = events.request_id
                      AND requests.portal_subdomain = events.portal_subdomain
                     ORDER BY events.portal_subdomain, events.request_id, events.event_ordinal",
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to prepare release histories: {error}"))
                })?;
            let rows = statement
                .query_map(params![minimum_events, limit], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<i64>>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, Option<i64>>(6)?,
                    ))
                })
                .map_err(|error| {
                    AppError::Storage(format!("Failed to read release histories: {error}"))
                })?;
            let mut histories: Vec<RequestReleaseHistory> = Vec::new();
            for row in rows {
                let (portal_key, request_id, rpid, agency_id, event_name, released_at, count) = row
                    .map_err(|error| {
                        AppError::Storage(format!("Failed to decode a release event: {error}"))
                    })?;
                let listed_document_count = count.map(u32::try_from).transpose().map_err(|_| {
                    AppError::Storage("A release count is out of range".to_string())
                })?;
                let release = StoredRelease {
                    event_name,
                    released_at,
                    listed_document_count,
                };
                match histories.last_mut() {
                    Some(history)
                        if history.portal_key == portal_key && history.request_id == request_id =>
                    {
                        history.releases.push(release);
                    }
                    _ => histories.push(RequestReleaseHistory {
                        portal_key,
                        request_id,
                        rpid,
                        agency_id,
                        releases: vec![release],
                    }),
                }
            }
            Ok(histories)
        })
        .await
    }
}
