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

use crate::{
    analysis::finding::Finding,
    error::AppError,
    storage::{vector::VectorIndex, DuckDbRepository},
};

pub mod absence_of_records;
pub mod boilerplate_denial;
pub mod budget_line_comparison;
pub mod claim_data_mismatch;
pub mod contradictory_statement;
pub mod correlated_findings;
pub mod cross_corpus_orphan_topic;
pub mod cross_corpus_reused_denial_grounds;
pub mod cross_corpus_shared_language;
pub mod cross_corpus_temporal_shift;
pub mod differential_treatment;
pub mod email_thread_gap;
pub mod encampment_notice;
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
pub mod practice_departure;
mod public_documents;
pub mod redaction_inconsistency;
pub mod regenerated_response;
pub mod release_timing;
pub mod response_time_claim;
pub mod rolling_production;
pub mod spending_cases;
mod text_shared;

/// Summary returned by [`refresh_all_findings`].
#[derive(Debug, Clone, Default)]
pub struct RefreshSummary {
    pub detectors_run: usize,
    pub detectors_failed: usize,
    pub findings_recorded: usize,
    pub stale_cleared: usize,
    /// Detectors that could not perform their test this run; their earlier findings are kept.
    pub not_run: Vec<DetectorNotRun>,
}

#[derive(Debug, Clone)]
pub struct DetectorNotRun {
    pub detector: &'static str,
    pub reason: String,
}

/// Records one detector's findings and sweeps its superseded rows. A detector that errs is
/// counted as failed and its stale rows are left alone, since an absent result is not proof the
/// finding no longer holds.
async fn record_detector_run(
    repository: &DuckDbRepository,
    summary: &mut RefreshSummary,
    run_started: chrono::DateTime<chrono::Utc>,
    slug: &'static str,
    findings: Result<Vec<Finding>, AppError>,
) {
    let outcome: Result<(usize, usize), AppError> = async {
        let findings = findings?;
        let recorded = if findings.is_empty() {
            0
        } else {
            repository.record_findings(findings).await?
        };
        let cleared = repository
            .clear_stale_findings(slug.to_string(), run_started)
            .await?;
        Ok((recorded, cleared))
    }
    .await;
    match outcome {
        Ok((recorded, cleared)) => {
            summary.findings_recorded += recorded;
            summary.stale_cleared += cleared;
        }
        Err(error) => {
            summary.detectors_failed += 1;
            eprintln!("detector {slug} failed: {error}");
        }
    }
}

macro_rules! run_detector {
    ($repository:expr, $summary:expr, $run_started:expr, $module:ident) => {{
        $summary.detectors_run += 1;
        let findings = $module::run($repository).await;
        record_detector_run(
            $repository,
            &mut $summary,
            $run_started,
            $module::SLUG,
            findings,
        )
        .await;
    }};
}

/// [`refresh_all_findings_with_embeddings`] without the semantic index, so detectors that need
/// it report not-run.
pub async fn refresh_all_findings(
    repository: &DuckDbRepository,
) -> Result<RefreshSummary, AppError> {
    refresh_all_findings_with_embeddings(repository, None).await
}

/// Runs every registered detector against the repository, upserts their findings, and clears
/// stale rows each detector no longer emits (from before this run started).
pub async fn refresh_all_findings_with_embeddings(
    repository: &DuckDbRepository,
    semantic: Option<&VectorIndex>,
) -> Result<RefreshSummary, AppError> {
    let run_started = chrono::Utc::now();
    let mut summary = RefreshSummary::default();
    run_detector!(repository, summary, run_started, held_back_response);
    run_detector!(repository, summary, run_started, boilerplate_denial);
    run_detector!(repository, summary, run_started, just_in_time_compliance);
    run_detector!(repository, summary, run_started, extension_abuse);
    run_detector!(repository, summary, run_started, contradictory_statement);
    summary.detectors_run += 1;
    let findings = match no_records_contradicted::run(repository, semantic).await {
        Ok(no_records_contradicted::Outcome::NotRun(reason)) => {
            eprintln!(
                "detector {} not run: {reason}",
                no_records_contradicted::SLUG
            );
            summary.not_run.push(DetectorNotRun {
                detector: no_records_contradicted::SLUG,
                reason,
            });
            None
        }
        Ok(no_records_contradicted::Outcome::Ran(findings)) => Some(Ok(findings)),
        Err(error) => Some(Err(error)),
    };
    if let Some(findings) = findings {
        record_detector_run(
            repository,
            &mut summary,
            run_started,
            no_records_contradicted::SLUG,
            findings,
        )
        .await;
    }
    run_detector!(repository, summary, run_started, absence_of_records);
    run_detector!(repository, summary, run_started, differential_treatment);
    run_detector!(repository, summary, run_started, metadata_vs_content);
    run_detector!(repository, summary, run_started, regenerated_response);
    run_detector!(repository, summary, run_started, response_time_claim);
    run_detector!(repository, summary, run_started, redaction_inconsistency);
    run_detector!(repository, summary, run_started, email_thread_gap);
    run_detector!(
        repository,
        summary,
        run_started,
        evasion_paraphrased_denials
    );
    run_detector!(
        repository,
        summary,
        run_started,
        evasion_round_number_thresholds
    );
    run_detector!(repository, summary, run_started, evasion_redaction_at_limit);
    run_detector!(
        repository,
        summary,
        run_started,
        evasion_late_arriving_records
    );
    run_detector!(
        repository,
        summary,
        run_started,
        inferred_redaction_identity
    );
    run_detector!(
        repository,
        summary,
        run_started,
        cross_corpus_shared_language
    );
    run_detector!(
        repository,
        summary,
        run_started,
        cross_corpus_temporal_shift
    );
    run_detector!(repository, summary, run_started, cross_corpus_orphan_topic);
    run_detector!(
        repository,
        summary,
        run_started,
        cross_corpus_reused_denial_grounds
    );
    run_detector!(repository, summary, run_started, policy_violation);
    run_detector!(repository, summary, run_started, claim_data_mismatch);
    run_detector!(repository, summary, run_started, spending_cases);
    run_detector!(repository, summary, run_started, practice_departure);
    run_detector!(repository, summary, run_started, budget_line_comparison);
    run_detector!(repository, summary, run_started, encampment_notice);
    run_detector!(repository, summary, run_started, rolling_production);
    run_detector!(repository, summary, run_started, release_timing);
    // Last: correlates the settled output of every detector above.
    run_detector!(repository, summary, run_started, correlated_findings);

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

    /// One records release and five hand-added imports hold the same text: a numbering gap and a
    /// stock denial repeated across five requests. Each import is its own source with one
    /// synthetic request, which is how `start_local_files` stores it.
    #[tokio::test]
    async fn a_hand_added_file_is_never_reported_as_something_an_agency_released() {
        const DENIAL: &str = "After a diligent search the department has determined that there \
            are no responsive records for this request. The department does not maintain the \
            material described and considers the request closed as of the date of this letter.";
        let repo = fresh_repo().await;
        {
            let conn = repo.analytics_connection().await.unwrap();
            let insert_document = |portal: &str, request: i64, id: i64, text: &str| {
                conn.execute(
                    "INSERT INTO documents (id, request_id, portal_subdomain, file_name,
                        visibility, created_at, updated_at, extracted_text_sha256)
                     VALUES (?1, ?2, ?3, ?4, 'public', '2026-01-01T00:00:00Z',
                        '2026-01-01T00:00:00Z', 'aa')",
                    duckdb::params![id, request, portal, format!("doc-{id}.pdf")],
                )
                .unwrap();
                conn.execute(
                    "INSERT INTO documents_fts (document_id, portal_subdomain, file_name, extracted_text)
                     VALUES (?1, ?2, ?3, ?4)",
                    duckdb::params![id, portal, format!("doc-{id}.pdf"), text],
                )
                .unwrap();
            };
            let insert_source = |portal: &str, kind: &str| {
                conn.execute(
                    "INSERT INTO portals (subdomain, display_name, base_url, source_kind)
                     VALUES (?1, ?1, 'https://example.test', ?2)",
                    duckdb::params![portal, kind],
                )
                .unwrap();
            };
            let insert_request = |portal: &str, id: i64, title: &str, status: &str| {
                conn.execute(
                    "INSERT INTO requests (id, portal_subdomain, title, status, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
                    duckdb::params![id, portal, title, status],
                )
                .unwrap();
            };
            let invoices = [(1_i64, 101), (2, 102), (3, 104), (4, 105), (5, 106)];

            insert_source("release", "NEXT_REQUEST");
            insert_request("release", 7, "Invoices", "closed");
            for (id, number) in invoices {
                insert_document("release", 7, id, &format!("Invoice #{number} approved."));
            }
            for index in 0..5_i64 {
                insert_request("release", 20 + index, &format!("Topic {index}"), "closed");
                insert_document("release", 20 + index, 20 + index, DENIAL);
            }

            for index in 0..5_i64 {
                let portal = format!("source-local-{index}");
                insert_source(&portal, "LOCAL_FILE");
                insert_request(&portal, 7, &format!("upload-{index}.pdf"), "imported");
                insert_document(&portal, 7, 40 + index, DENIAL);
                if index == 0 {
                    for (id, number) in invoices {
                        insert_document(&portal, 7, id, &format!("Invoice #{number} approved."));
                    }
                }
            }
        }

        let summary = refresh_all_findings(&repo).await.expect("refresh runs");
        assert_eq!(summary.detectors_failed, 0);

        let conn = repo.analytics_connection().await.unwrap();
        let rows: Vec<(String, String)> = conn
            .prepare(
                "SELECT source_detector,
                        concat_ws(' ', subject_portal_key, headline, explanation, citations_json)
                 FROM findings",
            )
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        for detector in [absence_of_records::SLUG, boilerplate_denial::SLUG] {
            assert!(
                rows.iter()
                    .any(|(slug, text)| slug == detector && text.contains("release")),
                "{detector} must still report the release: {rows:#?}"
            );
        }
        assert!(
            rows.iter().all(|(_, text)| !text.contains("source-local")),
            "no finding may cite a hand-added file: {rows:#?}"
        );
    }
}
