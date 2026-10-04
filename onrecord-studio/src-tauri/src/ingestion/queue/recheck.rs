//! Re-fetching collected documents to observe whether their source still serves the same bytes,
//! on the operator's command or on the schedule the operator opted into for a source.
//!
//! Different bytes are kept in the content-addressed store, referenced by the check that received
//! them, so purge releases them by the same reference count as collected files. Same bytes are
//! already stored as the document's own object and are discarded.

use super::objects::{commit_staged_object, release_object_link};
use super::*;
use crate::ingestion::client::SourceRecheck;
use crate::storage::sqlite::{SourceCheckObservation, SourceRecheckTarget};
use crate::types::{SavedSource, SourceCheckOutcome, SourceLifecycleState, SourceRecheckSummary};
use chrono::DateTime;

const MAXIMUM_RECHECK_DOCUMENTS: usize = 500;
/// Bytes one run may download before it stops, so a source of large exports cannot turn a check
/// into an unbounded re-download.
const MAXIMUM_RECHECK_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// Documents one scheduled run checks; the same cap the toolbar's Check source uses.
pub(crate) const SCHEDULED_RECHECK_DOCUMENTS: usize = 100;
/// How long after the background service starts before any scheduled check may begin, so opening
/// the app never starts a burst of fetches on its own.
const SCHEDULE_LAUNCH_GRACE: chrono::Duration = chrono::Duration::minutes(10);

/// Sources with a check running, from any caller. A second run of one source would race the
/// first through the same least-recently-checked targets and double every fetch.
static RECHECKS_IN_FLIGHT: std::sync::LazyLock<StdMutex<HashSet<String>>> =
    std::sync::LazyLock::new(|| StdMutex::new(HashSet::new()));

struct RecheckClaim(String);

impl RecheckClaim {
    fn acquire(portal_key: &str) -> Option<Self> {
        let mut running = RECHECKS_IN_FLIGHT
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        running
            .insert(portal_key.to_string())
            .then(|| Self(portal_key.to_string()))
    }
}

impl Drop for RecheckClaim {
    fn drop(&mut self) {
        RECHECKS_IN_FLIGHT
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.0);
    }
}

fn recheck_running(portal_key: &str) -> bool {
    RECHECKS_IN_FLIGHT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .contains(portal_key)
}

impl QueueManager {
    /// Re-fetches up to `max_documents` collected documents of one active source, least recently
    /// checked first, through the same governed client and per-source limiter as a sync.
    pub async fn recheck_source(
        &self,
        portal_key: &str,
        max_documents: usize,
    ) -> Result<SourceRecheckSummary, AppError> {
        if !(1..=MAXIMUM_RECHECK_DOCUMENTS).contains(&max_documents) {
            return Err(AppError::DataParsing(format!(
                "A source check covers between 1 and {MAXIMUM_RECHECK_DOCUMENTS} documents"
            )));
        }
        let _claim = RecheckClaim::acquire(portal_key).ok_or_else(|| {
            AppError::DataParsing(format!("Source '{portal_key}' is already being checked"))
        })?;
        let source = self
            .repository
            .list_portal_registrations()
            .await?
            .into_iter()
            .find(|saved: &SavedSource| saved.registration.portal_key == portal_key)
            .ok_or_else(|| {
                AppError::DataParsing(format!("No saved source is registered as '{portal_key}'"))
            })?;
        // A paused or removed source is one the operator stopped contacting; checking it anyway
        // would also fill the history with refusals that say nothing about its records.
        if source.lifecycle_state != SourceLifecycleState::Active {
            return Err(AppError::DataParsing(format!(
                "Source '{portal_key}' is not active; resume it before checking it"
            )));
        }
        // Compact retention is the operator choosing not to keep originals; a check honours it.
        let keep_different_bytes = self
            .repository
            .maintenance_status()
            .await?
            .policy
            .retention_mode
            != "REFERENCE_ONLY";
        let (targets, eligible) = self
            .repository
            .source_recheck_targets(portal_key, max_documents)
            .await?;
        let mut summary = SourceRecheckSummary {
            run_id: Uuid::new_v4().to_string(),
            portal_key: portal_key.to_string(),
            same_bytes: 0,
            different_bytes: 0,
            gone: 0,
            unreachable: 0,
            not_checked: eligible,
            stopped_early: None,
        };
        let mut received_bytes = 0_u64;
        for target in &targets {
            if received_bytes >= MAXIMUM_RECHECK_BYTES {
                summary.stopped_early = Some(format!(
                    "Stopped after downloading {received_bytes} bytes, the per-check limit"
                ));
                break;
            }
            if self.repository.source_lifecycle_state(portal_key).await?
                != Some(SourceLifecycleState::Active)
            {
                summary.stopped_early =
                    Some("Stopped because the source was paused or removed".to_string());
                break;
            }
            if let Err(error) = self
                .repository
                .check_retained_file_budget(&self.download_root)
                .await
            {
                summary.stopped_early = Some(error.to_string());
                break;
            }
            self.limiter.acquire(portal_key).await?;
            let observation = self
                .observe_source(&source, target, keep_different_bytes)
                .await?;
            let kept = match &observation {
                SourceCheckObservation::Received {
                    byte_count,
                    object_path,
                    ..
                } => {
                    received_bytes = received_bytes.saturating_add(*byte_count);
                    object_path.clone()
                }
                _ => None,
            };
            let recorded = self
                .repository
                .record_source_check(&summary.run_id, portal_key, target, observation)
                .await;
            if let Some(path) = &kept {
                release_object_link(Path::new(path)).await;
                if !matches!(recorded, Ok(Some(_))) {
                    self.discard_unreferenced_object(path).await?;
                }
            }
            match recorded? {
                Some(SourceCheckOutcome::SameBytes) => summary.same_bytes += 1,
                Some(SourceCheckOutcome::DifferentBytes) => summary.different_bytes += 1,
                Some(SourceCheckOutcome::Gone) => summary.gone += 1,
                Some(SourceCheckOutcome::Unreachable) => summary.unreachable += 1,
                // Purged mid-fetch: no longer a document of this source to check.
                None => {}
            }
            summary.not_checked -= 1;
        }
        Ok(summary)
    }

    /// Unlinks an object this run committed and could not record, unless some row references it.
    async fn discard_unreferenced_object(&self, path: &str) -> Result<(), AppError> {
        if !self.repository.evidence_object_referenced(path).await? {
            remove_released_evidence_files(&[path.to_string()]).await;
        }
        Ok(())
    }

    /// A local failure (staging directory, file removal) is an error for the whole run; anything
    /// the network or the source policy did is an observation about this one document.
    async fn observe_source(
        &self,
        source: &SavedSource,
        target: &SourceRecheckTarget,
        keep_different_bytes: bool,
    ) -> Result<SourceCheckObservation, AppError> {
        let staging_directory = self.download_root.join(".staging");
        tokio::fs::create_dir_all(&staging_directory)
            .await
            .map_err(|error| {
                AppError::Storage(format!(
                    "Failed to create recheck staging directory '{}': {error}",
                    staging_directory.display()
                ))
            })?;
        let staging_path = staging_directory.join(format!(
            ".recheck.{}.{}.partial",
            target.document_id,
            Uuid::new_v4()
        ));
        let fetched = self
            .client
            .recheck_document_to_path(&source.registration, &target.download_url, &staging_path)
            .await;
        let object_path = match &fetched {
            Ok(SourceRecheck::Retrieved(payload))
                if keep_different_bytes
                    && !payload
                        .sha256
                        .eq_ignore_ascii_case(&target.collected_sha256) =>
            {
                // Takes the staged file, so there is nothing left to discard.
                let committed = commit_staged_object(
                    &self.download_root,
                    &staging_path,
                    &payload.sha256.to_ascii_lowercase(),
                )
                .await?;
                Some(committed.to_string_lossy().into_owned())
            }
            _ => None,
        };
        if object_path.is_none() && tokio::fs::try_exists(&staging_path).await.unwrap_or(false) {
            tokio::fs::remove_file(&staging_path)
                .await
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to discard recheck download '{}': {error}",
                        staging_path.display()
                    ))
                })?;
        }
        Ok(match fetched {
            Ok(SourceRecheck::Retrieved(payload)) => SourceCheckObservation::Received {
                sha256: payload.sha256,
                byte_count: payload.byte_count,
                acquisition: payload.acquisition,
                object_path,
            },
            Ok(SourceRecheck::Gone(acquisition)) => SourceCheckObservation::Gone { acquisition },
            Err(error) => SourceCheckObservation::Unreachable {
                detail: error.to_string(),
            },
        })
    }
}

/// Whether a source's opt-in schedule starts a check now.
///
/// Never within [`SCHEDULE_LAUNCH_GRACE`] of the service starting, never for a source that is not
/// active (a paused source stays uncontacted), never while a check of it is running, and otherwise
/// once its interval has passed since it was last checked or a scheduled check was last started.
pub(crate) fn scheduled_recheck_due(
    source: &SavedSource,
    now: DateTime<Utc>,
    service_started_at: DateTime<Utc>,
    last_ran_at: Option<DateTime<Utc>>,
    running: bool,
) -> bool {
    let Some(interval_days) = source.recheck_interval_days else {
        return false;
    };
    if interval_days == 0
        || running
        || now < service_started_at + SCHEDULE_LAUNCH_GRACE
        || source.lifecycle_state != SourceLifecycleState::Active
        || matches!(
            source.registration.source_kind,
            SourceKind::LocalFile | SourceKind::PastedContent
        )
    {
        return false;
    }
    last_ran_at.is_none_or(|last| now - last >= chrono::Duration::days(i64::from(interval_days)))
}

/// The opt-in check schedule, driven by the background service's tick while the app is open.
pub struct RecheckSchedule {
    started_at: DateTime<Utc>,
    /// When this process last started a scheduled check of each source. A run that recorded no
    /// check (nothing eligible, or a failure) still waits a full interval before the next.
    started_runs: HashMap<String, DateTime<Utc>>,
}

impl RecheckSchedule {
    pub fn new(started_at: DateTime<Utc>) -> Self {
        Self {
            started_at,
            started_runs: HashMap::new(),
        }
    }

    /// Starts a background check of every source that is due, and returns their keys. After each
    /// run the watches are checked, so a Source copies change appears without the operator.
    pub async fn start_due(
        &mut self,
        manager: &Arc<QueueManager>,
        now: DateTime<Utc>,
    ) -> Result<Vec<String>, AppError> {
        let repository = manager.repository.clone();
        let mut started = Vec::new();
        for source in repository.list_portal_registrations().await? {
            let key = source.registration.portal_key.clone();
            if source.recheck_interval_days.is_none() {
                continue;
            }
            let last_check = repository
                .latest_source_check_at(&key)
                .await?
                .and_then(|stamp| DateTime::parse_from_rfc3339(&stamp).ok())
                .map(|stamp| stamp.with_timezone(&Utc));
            let last_ran_at = last_check.max(self.started_runs.get(&key).copied());
            if !scheduled_recheck_due(
                &source,
                now,
                self.started_at,
                last_ran_at,
                recheck_running(&key),
            ) {
                continue;
            }
            self.started_runs.insert(key.clone(), now);
            let manager = Arc::clone(manager);
            let task_key = key.clone();
            tokio::spawn(async move {
                let repository = manager.repository.clone();
                let outcome = match manager
                    .recheck_source(&task_key, SCHEDULED_RECHECK_DOCUMENTS)
                    .await
                {
                    Ok(_) => repository.check_watches(None).await.map(|_| ()),
                    Err(error) => Err(error),
                };
                if let Err(error) = outcome {
                    let _ = repository
                        .research_phase(
                            "Scheduled source check needs attention",
                            Some(format!("{task_key}: {error}")),
                        )
                        .await;
                }
            });
            started.push(key);
        }
        Ok(started)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingestion::client::{ApiClient, HttpClientConfig, SourceRecheck};
    use crate::types::{PortalApiLayout, PortalRegistration, SourceKind};

    fn scheduled(days: Option<u32>, lifecycle: SourceLifecycleState) -> SavedSource {
        SavedSource {
            registration: PortalRegistration {
                portal_key: "scheduled".to_string(),
                display_name: "Scheduled".to_string(),
                canonical_url: "https://records.example.gov/".to_string(),
                origin_url: "https://records.example.gov".to_string(),
                api_layout: PortalApiLayout {
                    requests_endpoint: "https://records.example.gov/".to_string(),
                    requests_pointer: "/requests".to_string(),
                    metadata_pointer: None,
                    page_parameter: "page".to_string(),
                    page_size_parameter: "per_page".to_string(),
                },
                verified_at: "2026-10-01T00:00:00Z".to_string(),
                source_kind: SourceKind::WebCrawl,
            },
            lifecycle_state: lifecycle,
            health_status: "verified".to_string(),
            last_error: None,
            recheck_interval_days: days,
        }
    }

    /// Opening the app never starts a check, even of a source long overdue; the first scheduled
    /// check waits out the grace, and then runs once per interval.
    #[test]
    fn the_schedule_never_fires_at_launch_and_then_once_per_interval() {
        let started = DateTime::parse_from_rfc3339("2026-10-03T09:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let overdue = Some(started - chrono::Duration::days(30));
        let source = scheduled(Some(7), SourceLifecycleState::Active);
        let at = |minutes| started + chrono::Duration::minutes(minutes);
        assert!(!scheduled_recheck_due(
            &source,
            at(0),
            started,
            overdue,
            false
        ));
        assert!(!scheduled_recheck_due(
            &source,
            at(9),
            started,
            overdue,
            false
        ));
        assert!(scheduled_recheck_due(
            &source,
            at(10),
            started,
            overdue,
            false
        ));
        assert!(scheduled_recheck_due(&source, at(10), started, None, false));

        let ran = at(10);
        let day = 24 * 60;
        assert!(!scheduled_recheck_due(
            &source,
            at(10 + 7 * day - 1),
            started,
            Some(ran),
            false
        ));
        assert!(scheduled_recheck_due(
            &source,
            at(10 + 7 * day),
            started,
            Some(ran),
            false
        ));
        let off = scheduled(None, SourceLifecycleState::Active);
        assert!(!scheduled_recheck_due(
            &off,
            at(10 + 7 * day),
            started,
            None,
            false
        ));
    }

    #[test]
    fn the_schedule_never_fires_for_a_paused_source_or_one_already_being_checked() {
        let started = DateTime::parse_from_rfc3339("2026-10-03T09:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let later = started + chrono::Duration::days(1);
        for lifecycle in [SourceLifecycleState::Paused, SourceLifecycleState::Removed] {
            let source = scheduled(Some(1), lifecycle);
            assert!(!scheduled_recheck_due(&source, later, started, None, false));
        }
        let active = scheduled(Some(1), SourceLifecycleState::Active);
        assert!(!scheduled_recheck_due(&active, later, started, None, true));
    }

    /// One check per source at a time, whichever caller started it, and the claim ends with it.
    #[test]
    fn a_source_has_at_most_one_check_running() {
        let first = RecheckClaim::acquire("claimed-source").expect("first claim");
        assert!(recheck_running("claimed-source"));
        assert!(RecheckClaim::acquire("claimed-source").is_none());
        let other = RecheckClaim::acquire("another-source");
        assert!(other.is_some());
        drop(first);
        assert!(!recheck_running("claimed-source"));
        assert!(RecheckClaim::acquire("claimed-source").is_some());
    }

    /// The loopback guard rules out a local fixture server, so the 404-versus-received split is
    /// proved against a live host: `cargo test --lib live_host -- --ignored`.
    #[tokio::test]
    #[ignore = "fetches from a live public host"]
    async fn a_live_host_404_is_gone_and_a_served_page_is_received() {
        let origin = "https://www.sandiego.gov";
        let registration = PortalRegistration {
            portal_key: "sandiego".to_string(),
            display_name: "San Diego".to_string(),
            canonical_url: origin.to_string(),
            origin_url: origin.to_string(),
            api_layout: PortalApiLayout {
                requests_endpoint: format!("{origin}/client/requests"),
                requests_pointer: "/requests".to_string(),
                metadata_pointer: None,
                page_parameter: "page".to_string(),
                page_size_parameter: "per_page".to_string(),
            },
            verified_at: "2026-10-02T00:00:00Z".to_string(),
            source_kind: SourceKind::NextRequest,
        };
        let client = ApiClient::new(HttpClientConfig::default()).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let gone = client
            .recheck_document_to_path(
                &registration,
                &format!("{origin}/onrecord-recheck-missing-page-7f3a"),
                &directory.path().join("gone"),
            )
            .await
            .unwrap();
        assert!(
            matches!(&gone, SourceRecheck::Gone(acquisition) if acquisition.http_status == 404),
            "{gone:?}"
        );
        let served = client
            .recheck_document_to_path(
                &registration,
                &format!("{origin}/city-clerk"),
                &directory.path().join("served"),
            )
            .await
            .unwrap();
        assert!(
            matches!(&served, SourceRecheck::Retrieved(payload) if payload.byte_count > 0),
            "{served:?}"
        );
    }
}
