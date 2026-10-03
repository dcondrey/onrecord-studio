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
use onrecord_studio_lib::test_support::register_analytical_table;
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

/// The benchmark has no semantic index, so this detector reports not-run, surfaced as an error
/// rather than as zero findings.
fn no_records_without_embeddings(
    repository: &DuckDbRepository,
) -> Pin<Box<dyn Future<Output = Result<Vec<Finding>, AppError>> + '_>> {
    Box::pin(async move {
        match no_records_contradicted::run(repository, None).await? {
            no_records_contradicted::Outcome::Ran(findings) => Ok(findings),
            no_records_contradicted::Outcome::NotRun(reason) => {
                Err(AppError::Analysis(format!("not run: {reason}")))
            }
        }
    })
}

fn registry() -> Vec<(&'static str, Run)> {
    let mut registry = detectors![
        held_back_response,
        boilerplate_denial,
        just_in_time_compliance,
        extension_abuse,
        contradictory_statement,
        claim_data_mismatch,
        absence_of_records,
        differential_treatment,
        metadata_vs_content,
        regenerated_response,
        response_time_claim,
        redaction_inconsistency,
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
        spending_cases,
        practice_departure,
        budget_line_comparison,
        rolling_production,
        correlated_findings,
    ];
    registry.insert(
        6,
        (
            no_records_contradicted::SLUG,
            no_records_without_embeddings as Run,
        ),
    );
    registry
}

#[test]
fn registry_matches_refresh_all_findings() {
    let source = include_str!("../src/analysis/detectors/mod.rs");
    let called = source.matches("run_detector!(").count()
        + source.matches("no_records_contradicted::run(").count();
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
    drop(conn);
    register_analytical_table(repo, snapshot_id, table, source_uri, '0')
        .await
        .unwrap();
}

/// A fresh repository holding the City portal, the two sliced exports under the automatic
/// budget and actual mappings, and the actuals export mapped again as revenue.
async fn eval_repository(fixtures: &Path) -> (tempfile::TempDir, DuckDbRepository) {
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

    (temp, repo)
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

    let (_temp, repo) = eval_repository(&fixtures).await;

    let mut evaluator = Evaluator::load(&repo).await.unwrap();
    let mut failures = Vec::new();
    let mut tally: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    let mut rows = Vec::new();
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
        rows.push(json!({
            "id": case.id,
            "text": case.text,
            "expected": case.expected,
            "outcome": outcome,
            "observed": observed,
            "passed": problems.is_empty(),
        }));
    }
    for (expected, (passed, total)) in &tally {
        println!("{expected:<12} {passed}/{total}");
    }
    // The scorecard is committed next to the cases so the methods page can cite the run that
    // produced it; it holds nothing time-dependent, so a clean run leaves it unchanged.
    let scorecard = json!({
        "evalSet": "claim-eval",
        "totals": tally.iter().map(|(expected, (passed, total))| {
            json!({"expected": expected, "passed": passed, "total": total})
        }).collect::<Vec<_>>(),
        "cases": rows,
    });
    std::fs::write(
        fixtures.join("scorecard.json"),
        serde_json::to_string_pretty(&scorecard).unwrap() + "\n",
    )
    .unwrap();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The sweep the app runs after every ingest, over the same exports: a memo and a press release
/// stored as public documents. Only the mismatching sentences become findings, the press
/// release's are labelled as an official statement, and consistent or unreconciled sentences
/// leave nothing behind.
#[tokio::test]
async fn claim_sweep_end_to_end_on_stored_documents() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/claim-eval");
    let (_temp, repo) = eval_repository(&fixtures).await;
    let memo = "Library budget memo. The Library Department spent $65 million in FY 2023 from fund 100000. \
                The adopted budget of $66.8 million for FY 2023 funded the Library Department in fund 100000. \
                The Fire-Rescue Department spent $350 million in FY 2023 from fund 100000.";
    let release = "FOR IMMEDIATE RELEASE. Statement from the City of San Diego. \
                   The Police Department reported a surplus of $2 million in fund 100000 for FY 2023.";
    let conn = repo.analytics_connection().await.unwrap();
    conn.execute_batch(&format!(
        "INSERT INTO requests (id, portal_subdomain, title, status, created_at, updated_at)
           VALUES (1, 'sandiego', 'Budget records', 'closed', '2026-01-01T00:00:00Z', '2026-01-02T00:00:00Z');
         INSERT INTO documents (id, request_id, portal_subdomain, file_name, visibility, created_at, updated_at, extracted_text_sha256)
           VALUES (1, 1, 'sandiego', 'library-budget-memo.pdf', 'public', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', 'h1'),
                  (2, 1, 'sandiego', 'press-release.pdf', 'public', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', 'h2');
         INSERT INTO documents_fts (document_id, portal_subdomain, file_name, extracted_text)
           VALUES (1, 'sandiego', 'library-budget-memo.pdf', '{memo}'),
                  (2, 'sandiego', 'press-release.pdf', '{release}');"
    ))
    .unwrap();
    drop(conn);

    let findings = claim_data_mismatch::run(&repo).await.unwrap();
    let mut headlines: Vec<&str> = findings.iter().map(|f| f.headline.as_str()).collect();
    headlines.sort();
    assert_eq!(findings.len(), 2, "{headlines:?}");
    let memo_finding = findings
        .iter()
        .find(|f| f.subject.document_id == Some(1))
        .expect("the understated Library actual is a finding");
    assert!(
        memo_finding.explanation.contains("$65"),
        "{}",
        memo_finding.explanation
    );
    assert!(!memo_finding.headline.starts_with("Official statement"));
    let release_finding = findings
        .iter()
        .find(|f| f.subject.document_id == Some(2))
        .expect("the surplus claim in the press release is a finding");
    assert!(
        release_finding
            .headline
            .starts_with("Official statement vs released record"),
        "{}",
        release_finding.headline
    );
    for finding in &findings {
        let text = format!("{} {}", finding.headline, finding.explanation).to_lowercase();
        for word in ["false", "lied", "misled"] {
            assert!(!text.contains(word), "{text}");
        }
    }
}
