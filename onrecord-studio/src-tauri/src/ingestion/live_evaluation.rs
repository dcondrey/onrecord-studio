//! End-to-end evaluation against a live portal: the same collection pipeline the app runs, then
//! every detector, so finding quality is judged on real records rather than fixtures.
//!
//! Network-dependent and slow, so ignored by default:
//! `ONRECORD_EVAL_DIR=<dir> ONRECORD_EVAL_SUBJECT=encampment cargo test --lib live_evaluation
//! -- --ignored --nocapture`. The database is left in `<dir>` for inspection.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use crate::ingestion::client::{ApiClient, HttpClientConfig};
use crate::ingestion::limiter::RateLimiter;
use crate::ingestion::queue::{IngestionProgress, QueueManager};
use crate::storage::corpus::CorpusIndex;
use crate::storage::sqlite::DuckDbRepository;
use crate::types::{MetadataPageWindow, RateLimitPolicy};

const PORTAL: &str = "https://sandiego.nextrequest.com/";
const MAXIMUM_RUN: Duration = Duration::from_secs(3 * 60 * 60);

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "collects from a live portal for up to three hours"]
async fn collect_a_subject_and_report_findings() {
    let root = PathBuf::from(std::env::var("ONRECORD_EVAL_DIR").expect("ONRECORD_EVAL_DIR"));
    let subject = std::env::var("ONRECORD_EVAL_SUBJECT").unwrap_or_else(|_| "encampment".into());
    std::fs::create_dir_all(root.join("downloads")).unwrap();
    let repository = DuckDbRepository::new(root.join("core.db")).unwrap();
    repository
        .run_migrations(include_str!("../../migrations/0001_initial_schema.sql"))
        .await
        .unwrap();
    let corpus = CorpusIndex::open(root.join("corpus.db")).unwrap();
    // ONRECORD_EVAL_REUSE=1 re-runs the detectors over a database an earlier run collected.
    let failures = if std::env::var("ONRECORD_EVAL_REUSE").is_ok() {
        Vec::new()
    } else {
        collect(&root, &repository, corpus, &subject).await
    };
    let summary = crate::analysis::detectors::refresh_all_findings(&repository)
        .await
        .unwrap();
    eprintln!("detector summary: {summary:?}");
    eprintln!("{} task failures; first 20:", failures.len());
    for failure in failures.iter().take(20) {
        eprintln!("  {failure}");
    }
    let report = repository
        .analytics_connection()
        .await
        .unwrap()
        .prepare(
            "SELECT source_detector, severity, headline FROM findings
             ORDER BY source_detector, severity",
        )
        .unwrap()
        .query_map([], |row| {
            Ok(format!(
                "{} [{}] {}",
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    eprintln!("{} findings for '{subject}':", report.len());
    for line in report {
        eprintln!("  {line}");
    }
}

async fn collect(
    root: &std::path::Path,
    repository: &DuckDbRepository,
    corpus: CorpusIndex,
    subject: &str,
) -> Vec<String> {
    let (sender, mut receiver) = mpsc::channel::<IngestionProgress>(4_096);
    let failures = tokio::spawn(async move {
        let mut failures = Vec::new();
        while let Some(progress) = receiver.recv().await {
            if let IngestionProgress::TaskFailed { error_msg, .. } = progress {
                failures.push(error_msg);
            }
        }
        failures
    });
    let manager = QueueManager::new(
        ApiClient::new(HttpClientConfig::default()).unwrap(),
        repository.clone(),
        corpus,
        RateLimiter::new(5).unwrap(),
        sender,
        root.join("downloads"),
    )
    .unwrap();
    let started = Instant::now();
    manager
        .discover_and_start(
            PORTAL,
            MetadataPageWindow::default(),
            RateLimitPolicy::default(),
            Some(subject.to_string()),
        )
        .await
        .expect("the portal must start");
    loop {
        tokio::time::sleep(Duration::from_secs(30)).await;
        let runtimes = manager.runtime_snapshots().await.unwrap();
        let running = runtimes.iter().any(|runtime| runtime.is_running);
        for runtime in &runtimes {
            eprintln!(
                "[{:>5}s] queued {} completed {} failed {}",
                started.elapsed().as_secs(),
                runtime.queued_tasks,
                runtime.completed_tasks,
                runtime.failed_tasks
            );
        }
        if !running || started.elapsed() > MAXIMUM_RUN {
            break;
        }
    }
    drop(manager);
    failures.await.unwrap_or_default()
}
