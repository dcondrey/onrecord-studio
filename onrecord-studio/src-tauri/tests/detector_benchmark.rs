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

use onrecord_studio_lib::analysis::claim_validation::{
    extract_claims, ClaimEvaluation, Evaluator, UnreconciledReason,
};
use onrecord_studio_lib::analysis::detectors::*;
use onrecord_studio_lib::analysis::finding::Finding;
use onrecord_studio_lib::error::AppError;
use onrecord_studio_lib::storage::sqlite::DuckDbRepository;
use onrecord_studio_lib::types::{FinancialMapping, PortalRegistration};
use serde::Deserialize;
use serde_json::json;
use std::collections::BTreeMap;
use std::future::Future;
use std::path::Path;
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

/// One hand-verified memo sentence over the real City exports in `fixtures/claim-eval`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EvalCase {
    id: String,
    text: String,
    expected: String,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    observed_contains: Option<String>,
}

fn unreconciled_reason_name(reason: &UnreconciledReason) -> &'static str {
    match reason {
        UnreconciledReason::NoMappedDataset => "NoMappedDataset",
        UnreconciledReason::AmbiguousScope => "AmbiguousScope",
        UnreconciledReason::MissingFiscalYear => "MissingFiscalYear",
        UnreconciledReason::PartialOrRejectedRows => "PartialOrRejectedRows",
        UnreconciledReason::UnsupportedClaim => "UnsupportedClaim",
    }
}

/// Registers a sliced City export under the snapshot id and source URI the automatic budget
/// and actual mappings key on, exactly as an operator's import would leave it.
async fn import_city_export(
    repo: &DuckDbRepository,
    snapshot_id: &str,
    table: &str,
    source_uri: &str,
    csv: &Path,
) {
    let conn = repo.analytics_connection().await.unwrap();
    let path = csv.to_str().expect("fixture path is UTF-8");
    assert!(
        !path.contains('\''),
        "fixture path must not contain a quote: {path}"
    );
    conn.execute_batch(&format!(
        "CREATE TABLE {table} AS SELECT * FROM read_csv('{path}', all_varchar = true, header = true)"
    ))
    .unwrap();
    let rows: i64 = conn
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap();
    conn.execute(
        "INSERT INTO analytical_snapshots(snapshot_id,dataset_kind,display_name,table_name,source_uri,local_path,source_sha256,retrieved_at,validation_summary_json,imported_rows) VALUES (?1,'OPERATING_BUDGET',?1,?2,?3,?4,repeat('0',64),'2026-09-30','{}',?5)",
        duckdb::params![snapshot_id, table, source_uri, path, rows],
    )
    .unwrap();
    let mut statement = conn
        .prepare("SELECT column_name, ordinal_position FROM information_schema.columns WHERE table_name = ?1 ORDER BY ordinal_position")
        .unwrap();
    let columns = statement
        .query_map(duckdb::params![table], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    for (name, ordinal) in columns {
        conn.execute(
            "INSERT INTO analytical_columns(snapshot_id,table_name,column_name,logical_type,ordinal) VALUES (?1,?2,?3,'TEXT',?4)",
            duckdb::params![snapshot_id, table, name, ordinal],
        )
        .unwrap();
    }
}

/// Every sentence in `fixtures/claim-eval/cases.json` must evaluate to its verified outcome
/// against the real FY22–23 General Fund rows for the Library and Police departments. The
/// scorecard it prints is the claim-validation benchmark; a change to extraction, binding or
/// the bands that moves any case is a change to what the detector tells a reader.
#[tokio::test]
async fn claim_validation_eval_set() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/claim-eval");
    let cases: Vec<EvalCase> =
        serde_json::from_str(&std::fs::read_to_string(fixtures.join("cases.json")).unwrap())
            .unwrap();
    assert!(!cases.is_empty());

    let temp = tempfile::tempdir().unwrap();
    let repo = DuckDbRepository::new(temp.path().join("core.db")).unwrap();
    repo.run_migrations(include_str!("../migrations/0001_initial_schema.sql"))
        .await
        .unwrap();
    let portal: PortalRegistration = serde_json::from_value(json!({
        "portalKey": "sandiego", "displayName": "City of San Diego",
        "canonicalUrl": "https://sandiego.nextrequest.com", "originUrl": "https://sandiego.nextrequest.com",
        "verifiedAt": "2026-09-13T00:00:00Z", "sourceKind": "NEXT_REQUEST",
        "apiLayout": {"requestsEndpoint": "https://sandiego.nextrequest.com/api", "requestsPointer": "/resources",
                      "pageParameter": "page", "pageSizeParameter": "limit"}
    }))
    .unwrap();
    repo.insert_or_update_portal_registration(&portal)
        .await
        .unwrap();
    import_city_export(
        &repo,
        "budget",
        "analysis_eval_budget",
        "https://seshat.datasd.org/operating_budget/budget_operating_datasd.csv",
        &fixtures.join("budget_operating_datasd_fy22-23_gf_library_police.csv"),
    )
    .await;
    import_city_export(
        &repo,
        "actual",
        "analysis_eval_actual",
        "https://seshat.datasd.org/operating_actuals/actuals_operating_datasd.csv",
        &fixtures.join("actuals_operating_datasd_fy22-23_gf_library_police.csv"),
    )
    .await;
    // The actuals export carries receipts under 4xxxxx accounts; an operator maps it to the
    // revenue role explicitly, and the official-export filter keeps expenditure rows out.
    repo.save_financial_mapping(FinancialMapping {
        role: "revenue".to_string(),
        snapshot_id: "actual".to_string(),
        columns: [
            ("fy", "report_fy"),
            ("fund", "fund_number"),
            ("account", "account_number"),
            ("department", "dept_name"),
            ("amount", "amount"),
        ]
        .into_iter()
        .map(|(field, column)| (field.to_string(), column.to_string()))
        .collect::<BTreeMap<_, _>>(),
        currency: Some("USD".to_string()),
        agency: Some("City of San Diego".to_string()),
    })
    .await
    .unwrap();

    let mut evaluator = Evaluator::load(&repo).await.unwrap();
    let mut failures = Vec::new();
    let mut tally: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    for case in &cases {
        let claims = extract_claims(&case.text);
        let entry = tally.entry(case.expected.as_str()).or_default();
        entry.1 += 1;
        let [claim] = claims.as_slice() else {
            failures.push(format!(
                "{}: extracted {} claims, expected exactly one",
                case.id,
                claims.len()
            ));
            continue;
        };
        let evaluation = evaluator.evaluate(&repo, "sandiego", claim).await.unwrap();
        let (outcome, observed, reason) = match &evaluation {
            ClaimEvaluation::Consistent { observed, .. } => ("consistent", observed.as_str(), None),
            ClaimEvaluation::Mismatch { observed, .. } => ("mismatch", observed.as_str(), None),
            ClaimEvaluation::Unreconciled { reason, detail } => (
                "unreconciled",
                detail.as_str(),
                Some(unreconciled_reason_name(reason)),
            ),
        };
        let mut problems = Vec::new();
        if outcome != case.expected {
            problems.push(format!("outcome {outcome}, expected {}", case.expected));
        }
        if let Some(expected_reason) = &case.reason {
            if reason != Some(expected_reason.as_str()) {
                problems.push(format!("reason {reason:?}, expected {expected_reason}"));
            }
        }
        if let Some(fragment) = &case.observed_contains {
            if !observed.contains(fragment) {
                problems.push(format!("observed '{observed}' lacks '{fragment}'"));
            }
        }
        if problems.is_empty() {
            entry.0 += 1;
            println!("ok   {:<32} {outcome:<12} {observed}", case.id);
        } else {
            println!("FAIL {:<32} {}", case.id, problems.join("; "));
            failures.push(format!("{}: {}", case.id, problems.join("; ")));
        }
    }
    for (expected, (passed, total)) in &tally {
        println!("{expected:<12} {passed}/{total}");
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
