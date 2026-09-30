//! Autonomous detector registry.
//!
//! Each detector is invoked by the `refresh_findings` command (fired on ingest completion and by
//! explicit user request; no background heartbeat, per the current product decision). Detectors
//! run sequentially against a shared repository; each is expected to be bounded (query with a
//! limit, cap work by document count) so the refresh call stays responsive.
//!
//! **Adding a detector:** create a new submodule that exposes `pub const SLUG: &str = "..."` and
//! `pub async fn run(repository: &DuckDbRepository) -> Result<Vec<Finding>, AppError>`, then add
//! it to `refresh_all_findings`. The slug is what `clear_stale_findings` uses to sweep this
//! detector's superseded rows without touching other detectors' output.

use crate::{error::AppError, storage::DuckDbRepository};

pub mod absence_of_records;
pub mod boilerplate_denial;
pub mod claim_data_mismatch;
pub mod contradictory_statement;
pub mod cross_corpus_orphan_topic;
pub mod cross_corpus_reused_denial_grounds;
pub mod cross_corpus_shared_language;
pub mod cross_corpus_temporal_shift;
pub mod differential_treatment;
pub mod email_thread_gap;
pub mod evasion_late_arriving_records;
pub mod evasion_paraphrased_denials;
pub mod evasion_redaction_at_limit;
pub mod evasion_round_number_thresholds;
pub mod extension_abuse;
pub mod held_back_response;
pub mod inferred_redaction_identity;
pub mod just_in_time_compliance;
pub mod metadata_vs_content;
pub mod no_records_contradicted;
pub mod policy_violation;
mod public_documents;
pub mod regenerated_response;

/// Summary returned by [`refresh_all_findings`].
#[derive(Debug, Clone, Copy, Default)]
pub struct RefreshSummary {
    pub detectors_run: usize,
    pub findings_recorded: usize,
    pub stale_cleared: usize,
}

/// Runs every registered detector against the repository, upserts their findings, and clears
/// stale rows each detector no longer emits (from before this run started).
pub async fn refresh_all_findings(
    repository: &DuckDbRepository,
) -> Result<RefreshSummary, AppError> {
    let run_started = chrono::Utc::now();
    let mut summary = RefreshSummary::default();

    // held_back_response
    summary.detectors_run += 1;
    let findings = held_back_response::run(repository).await?;
    if !findings.is_empty() {
        summary.findings_recorded += repository.record_findings(findings).await?;
    }
    summary.stale_cleared += repository
        .clear_stale_findings(held_back_response::SLUG.to_string(), run_started)
        .await?;

    // boilerplate_denial
    summary.detectors_run += 1;
    let findings = boilerplate_denial::run(repository).await?;
    if !findings.is_empty() {
        summary.findings_recorded += repository.record_findings(findings).await?;
    }
    summary.stale_cleared += repository
        .clear_stale_findings(boilerplate_denial::SLUG.to_string(), run_started)
        .await?;

    // just_in_time_compliance
    summary.detectors_run += 1;
    let findings = just_in_time_compliance::run(repository).await?;
    if !findings.is_empty() {
        summary.findings_recorded += repository.record_findings(findings).await?;
    }
    summary.stale_cleared += repository
        .clear_stale_findings(just_in_time_compliance::SLUG.to_string(), run_started)
        .await?;

    // extension_abuse
    summary.detectors_run += 1;
    let findings = extension_abuse::run(repository).await?;
    if !findings.is_empty() {
        summary.findings_recorded += repository.record_findings(findings).await?;
    }
    summary.stale_cleared += repository
        .clear_stale_findings(extension_abuse::SLUG.to_string(), run_started)
        .await?;

    // contradictory_statement (Wave 2A)
    summary.detectors_run += 1;
    let findings = contradictory_statement::run(repository).await?;
    if !findings.is_empty() {
        summary.findings_recorded += repository.record_findings(findings).await?;
    }
    summary.stale_cleared += repository
        .clear_stale_findings(contradictory_statement::SLUG.to_string(), run_started)
        .await?;

    // no_records_contradicted (Wave 2A)
    summary.detectors_run += 1;
    let findings = no_records_contradicted::run(repository).await?;
    if !findings.is_empty() {
        summary.findings_recorded += repository.record_findings(findings).await?;
    }
    summary.stale_cleared += repository
        .clear_stale_findings(no_records_contradicted::SLUG.to_string(), run_started)
        .await?;

    // absence_of_records (Wave 2D)
    summary.detectors_run += 1;
    let findings = absence_of_records::run(repository).await?;
    if !findings.is_empty() {
        summary.findings_recorded += repository.record_findings(findings).await?;
    }
    summary.stale_cleared += repository
        .clear_stale_findings(absence_of_records::SLUG.to_string(), run_started)
        .await?;

    // differential_treatment (Wave 2D)
    summary.detectors_run += 1;
    let findings = differential_treatment::run(repository).await?;
    if !findings.is_empty() {
        summary.findings_recorded += repository.record_findings(findings).await?;
    }
    summary.stale_cleared += repository
        .clear_stale_findings(differential_treatment::SLUG.to_string(), run_started)
        .await?;

    // metadata_vs_content (Wave 2D)
    summary.detectors_run += 1;
    let findings = metadata_vs_content::run(repository).await?;
    if !findings.is_empty() {
        summary.findings_recorded += repository.record_findings(findings).await?;
    }
    summary.stale_cleared += repository
        .clear_stale_findings(metadata_vs_content::SLUG.to_string(), run_started)
        .await?;

    // regenerated_response (Wave 2C)
    summary.detectors_run += 1;
    let findings = regenerated_response::run(repository).await?;
    if !findings.is_empty() {
        summary.findings_recorded += repository.record_findings(findings).await?;
    }
    summary.stale_cleared += repository
        .clear_stale_findings(regenerated_response::SLUG.to_string(), run_started)
        .await?;

    // email_thread_gap (Wave 2C)
    summary.detectors_run += 1;
    let findings = email_thread_gap::run(repository).await?;
    if !findings.is_empty() {
        summary.findings_recorded += repository.record_findings(findings).await?;
    }
    summary.stale_cleared += repository
        .clear_stale_findings(email_thread_gap::SLUG.to_string(), run_started)
        .await?;

    // evasion_paraphrased_denials (Wave 6)
    summary.detectors_run += 1;
    let findings = evasion_paraphrased_denials::run(repository).await?;
    if !findings.is_empty() {
        summary.findings_recorded += repository.record_findings(findings).await?;
    }
    summary.stale_cleared += repository
        .clear_stale_findings(evasion_paraphrased_denials::SLUG.to_string(), run_started)
        .await?;

    // evasion_round_number_thresholds (Wave 6)
    summary.detectors_run += 1;
    let findings = evasion_round_number_thresholds::run(repository).await?;
    if !findings.is_empty() {
        summary.findings_recorded += repository.record_findings(findings).await?;
    }
    summary.stale_cleared += repository
        .clear_stale_findings(
            evasion_round_number_thresholds::SLUG.to_string(),
            run_started,
        )
        .await?;

    // evasion_redaction_at_limit (Wave 6)
    summary.detectors_run += 1;
    let findings = evasion_redaction_at_limit::run(repository).await?;
    if !findings.is_empty() {
        summary.findings_recorded += repository.record_findings(findings).await?;
    }
    summary.stale_cleared += repository
        .clear_stale_findings(evasion_redaction_at_limit::SLUG.to_string(), run_started)
        .await?;

    // evasion_late_arriving_records (Wave 6)
    summary.detectors_run += 1;
    let findings = evasion_late_arriving_records::run(repository).await?;
    if !findings.is_empty() {
        summary.findings_recorded += repository.record_findings(findings).await?;
    }
    summary.stale_cleared += repository
        .clear_stale_findings(evasion_late_arriving_records::SLUG.to_string(), run_started)
        .await?;

    // inferred_redaction_identity (Wave 9)
    summary.detectors_run += 1;
    let findings = inferred_redaction_identity::run(repository).await?;
    if !findings.is_empty() {
        summary.findings_recorded += repository.record_findings(findings).await?;
    }
    summary.stale_cleared += repository
        .clear_stale_findings(inferred_redaction_identity::SLUG.to_string(), run_started)
        .await?;

    // cross_corpus_shared_language (Wave 8)
    summary.detectors_run += 1;
    let findings = cross_corpus_shared_language::run(repository).await?;
    if !findings.is_empty() {
        summary.findings_recorded += repository.record_findings(findings).await?;
    }
    summary.stale_cleared += repository
        .clear_stale_findings(cross_corpus_shared_language::SLUG.to_string(), run_started)
        .await?;

    // cross_corpus_temporal_shift (Wave 8)
    summary.detectors_run += 1;
    let findings = cross_corpus_temporal_shift::run(repository).await?;
    if !findings.is_empty() {
        summary.findings_recorded += repository.record_findings(findings).await?;
    }
    summary.stale_cleared += repository
        .clear_stale_findings(cross_corpus_temporal_shift::SLUG.to_string(), run_started)
        .await?;

    // cross_corpus_orphan_topic (Wave 8)
    summary.detectors_run += 1;
    let findings = cross_corpus_orphan_topic::run(repository).await?;
    if !findings.is_empty() {
        summary.findings_recorded += repository.record_findings(findings).await?;
    }
    summary.stale_cleared += repository
        .clear_stale_findings(cross_corpus_orphan_topic::SLUG.to_string(), run_started)
        .await?;

    // cross_corpus_reused_denial_grounds (Wave 8)
    summary.detectors_run += 1;
    let findings = cross_corpus_reused_denial_grounds::run(repository).await?;
    if !findings.is_empty() {
        summary.findings_recorded += repository.record_findings(findings).await?;
    }
    summary.stale_cleared += repository
        .clear_stale_findings(
            cross_corpus_reused_denial_grounds::SLUG.to_string(),
            run_started,
        )
        .await?;

    // policy_violation (Wave 4)
    summary.detectors_run += 1;
    let findings = policy_violation::run(repository).await?;
    if !findings.is_empty() {
        summary.findings_recorded += repository.record_findings(findings).await?;
    }
    summary.stale_cleared += repository
        .clear_stale_findings(policy_violation::SLUG.to_string(), run_started)
        .await?;

    // claim_data_mismatch
    summary.detectors_run += 1;
    let findings = claim_data_mismatch::run(repository).await?;
    if !findings.is_empty() {
        summary.findings_recorded += repository.record_findings(findings).await?;
    }
    summary.stale_cleared += repository
        .clear_stale_findings(claim_data_mismatch::SLUG.to_string(), run_started)
        .await?;

    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn fresh_repo() -> DuckDbRepository {
        let repo = DuckDbRepository::new(":memory:").expect("open in-memory core");
        repo.run_migrations(include_str!("../../../migrations/0001_initial_schema.sql"))
            .await
            .expect("run migrations");
        repo
    }

    #[tokio::test]
    async fn refresh_all_findings_runs_registered_detectors_against_empty_corpus() {
        // Runs each detector against an empty corpus. Every detector must be safe to call with
        // no data present; this test guards that invariant.
        let repo = fresh_repo().await;
        let summary = refresh_all_findings(&repo).await.expect("refresh runs");
        assert!(
            summary.detectors_run >= 1,
            "at least held_back_response is registered"
        );
        assert_eq!(
            summary.findings_recorded, 0,
            "empty corpus yields no findings"
        );
        assert_eq!(
            summary.stale_cleared, 0,
            "nothing to clear on first run against empty corpus"
        );
    }
}
