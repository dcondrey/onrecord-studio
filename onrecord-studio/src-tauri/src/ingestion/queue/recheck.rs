//! Re-fetching collected documents to observe whether their source still serves the same bytes.
//!
//! The fetched bytes are hashed and discarded. Keeping them would put an object in the store that
//! no `documents` row references, which purge never releases.

use super::*;
use crate::ingestion::client::SourceRecheck;
use crate::storage::sqlite::{SourceCheckObservation, SourceRecheckTarget};
use crate::types::{SavedSource, SourceCheckOutcome, SourceLifecycleState, SourceRecheckSummary};

const MAXIMUM_RECHECK_DOCUMENTS: usize = 500;
/// Bytes one run may download before it stops, so a source of large exports cannot turn a check
/// into an unbounded re-download.
const MAXIMUM_RECHECK_BYTES: u64 = 2 * 1024 * 1024 * 1024;

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
            if let Err(error) = self
                .repository
                .check_retained_file_budget(&self.download_root)
                .await
            {
                summary.stopped_early = Some(error.to_string());
                break;
            }
            self.limiter.acquire(portal_key).await?;
            let observation = self.observe_source(&source, target).await?;
            if let SourceCheckObservation::Received { byte_count, .. } = &observation {
                received_bytes = received_bytes.saturating_add(*byte_count);
            }
            let outcome = self
                .repository
                .record_source_check(&summary.run_id, portal_key, target, observation)
                .await?;
            match outcome {
                SourceCheckOutcome::SameBytes => summary.same_bytes += 1,
                SourceCheckOutcome::DifferentBytes => summary.different_bytes += 1,
                SourceCheckOutcome::Gone => summary.gone += 1,
                SourceCheckOutcome::Unreachable => summary.unreachable += 1,
            }
            summary.not_checked -= 1;
        }
        Ok(summary)
    }

    /// A local failure (staging directory, file removal) is an error for the whole run; anything
    /// the network or the source policy did is an observation about this one document.
    async fn observe_source(
        &self,
        source: &SavedSource,
        target: &SourceRecheckTarget,
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
        if tokio::fs::try_exists(&staging_path).await.unwrap_or(false) {
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
            },
            Ok(SourceRecheck::Gone(acquisition)) => SourceCheckObservation::Gone { acquisition },
            Err(error) => SourceCheckObservation::Unreachable {
                detail: error.to_string(),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::ingestion::client::{ApiClient, HttpClientConfig, SourceRecheck};
    use crate::types::{PortalApiLayout, PortalRegistration, SourceKind};

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
