//! Detector benchmark harness. Runs every registered detector against a COPY of a real core
//! database and writes findings, per-detector counts and timings as JSON. Never records findings,
//! so the database it opens is only read (DuckDB may still checkpoint its WAL: use a copy).
//!
//! ```text
//! ONRECORD_BENCH_DB=/path/to/copy/onrecord_studio_core.db \
//! ONRECORD_BENCH_OUT=/path/to/out.json \
//! cargo test --manifest-path src-tauri/Cargo.toml --test detector_benchmark -- --ignored --nocapture
//! ```
//!
//! `ONRECORD_BENCH_ONLY=slug1,slug2` restricts the run; `ONRECORD_BENCH_SKIP` excludes slugs. The detector list mirrors
//! `analysis::detectors::refresh_all_findings`; `registry_matches_refresh_all_findings` fails if
//! they drift.

use onrecord_studio_lib::analysis::detectors::*;
use onrecord_studio_lib::analysis::finding::Finding;
use onrecord_studio_lib::error::AppError;
use onrecord_studio_lib::storage::sqlite::DuckDbRepository;
use serde_json::json;
use std::future::Future;
use std::pin::Pin;
use std::time::Instant;

type Run = for<'a> fn(
    &'a DuckDbRepository,
) -> Pin<Box<dyn Future<Output = Result<Vec<Finding>, AppError>> + 'a>>;

macro_rules! detectors {
    ($($m:ident),* $(,)?) => {
        vec![$(($m::SLUG, (|r| Box::pin($m::run(r))) as Run)),*]
    };
}

fn registry() -> Vec<(&'static str, Run)> {
    detectors![
        held_back_response,
        boilerplate_denial,
        just_in_time_compliance,
        extension_abuse,
        contradictory_statement,
        claim_data_mismatch,
        no_records_contradicted,
        absence_of_records,
        differential_treatment,
        metadata_vs_content,
        regenerated_response,
        email_thread_gap,
        evasion_paraphrased_denials,
        evasion_round_number_thresholds,
        evasion_redaction_at_limit,
        evasion_late_arriving_records,
        inferred_redaction_identity,
        cross_corpus_shared_language,
        cross_corpus_temporal_shift,
        cross_corpus_orphan_topic,
        cross_corpus_reused_denial_grounds,
        policy_violation,
        encampment_notice,
    ]
}

#[test]
fn registry_matches_refresh_all_findings() {
    let source = include_str!("../src/analysis/detectors/mod.rs");
    let called = source.matches("::run(repository).await?").count();
    assert_eq!(registry().len(), called, "update registry() in this file");
}

#[tokio::test]
#[ignore = "Runs every detector against a real database copy named by ONRECORD_BENCH_DB"]
async fn detector_benchmark() {
    let db = std::env::var("ONRECORD_BENCH_DB").expect("set ONRECORD_BENCH_DB to a database COPY");
    let out =
        std::env::var("ONRECORD_BENCH_OUT").unwrap_or_else(|_| "detector-benchmark.json".into());
    let only: Option<Vec<String>> = std::env::var("ONRECORD_BENCH_ONLY")
        .ok()
        .map(|v| v.split(',').map(str::to_owned).collect());
    let skip: Vec<String> = std::env::var("ONRECORD_BENCH_SKIP")
        .map(|v| v.split(',').map(str::to_owned).collect())
        .unwrap_or_default();
    let repo = DuckDbRepository::new(&db).expect("open database copy");
    // The app runs this on every launch; a database from an older build lacks the findings tables.
    repo.run_migrations(include_str!("../migrations/0001_initial_schema.sql"))
        .await
        .expect("run migrations on the copy");
    let mut report = Vec::new();
    for (slug, run) in registry() {
        if only.as_ref().is_some_and(|o| !o.iter().any(|s| s == slug))
            || skip.iter().any(|s| s == slug)
        {
            continue;
        }
        let started = Instant::now();
        let result = run(&repo).await;
        let seconds = started.elapsed().as_secs_f64();
        match result {
            Ok(findings) => {
                println!("{slug}: {} findings in {seconds:.2}s", findings.len());
                report.push(json!({"detector": slug, "count": findings.len(), "seconds": seconds, "findings": findings}));
            }
            Err(error) => {
                println!("{slug}: ERROR {error} after {seconds:.2}s");
                report.push(json!({"detector": slug, "error": error.to_string(), "seconds": seconds, "findings": []}));
            }
        }
    }
    std::fs::write(&out, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    println!("wrote {out}");
}
