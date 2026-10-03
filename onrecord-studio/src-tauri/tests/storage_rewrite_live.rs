//! Storage rewrite of a copy of a real installation's database, then the application's own startup
//! sequence against the result. Never point it at a live installation: it replaces the file.
use onrecord_studio_lib::storage::storage_rewrite::{rewrite_to_current_storage, StorageRewrite};
use onrecord_studio_lib::storage::DuckDbRepository;
use std::path::PathBuf;
use std::time::Instant;

#[test]
#[ignore = "Set ONRECORD_STORAGE_REWRITE_COPY to a COPY of onrecord_studio_core.db; rewrites it in place"]
fn rewrite_a_copied_core_then_run_startup_migrations() {
    let path = PathBuf::from(
        std::env::var("ONRECORD_STORAGE_REWRITE_COPY").expect("path to a copied core database"),
    );
    assert!(
        !path.to_string_lossy().contains("Application Support"),
        "refusing to rewrite what looks like a live installation: {}",
        path.display()
    );

    let StorageRewrite::Rewritten(report) =
        rewrite_to_current_storage(&path).expect("rewrite must succeed")
    else {
        panic!("the copy was already at the pinned storage version");
    };
    println!(
        "rewrite: storage {} -> current, {} -> {} bytes, {:.1}s, {} rows in {} tables verified",
        report.previous_storage,
        report.bytes_before,
        report.bytes_after,
        report.elapsed.as_secs_f64(),
        report.total_rows(),
        report.tables.len()
    );
    for (table, rows) in &report.tables {
        println!("  {table}: {rows}");
    }

    let started = Instant::now();
    let repository = DuckDbRepository::new(&path).expect("rewritten core must open");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        repository
            .run_migrations(include_str!("../migrations/0001_initial_schema.sql"))
            .await
            .expect("migrations must run on the rewritten core");
        repository.initialize_research().await.unwrap();
        repository.initialize_review().await.unwrap();
        repository.initialize_casework().await.unwrap();
        repository.initialize_monitoring().await.unwrap();
        repository.initialize_watchlists().await.unwrap();
        repository.initialize_standing_queries().await.unwrap();
        repository.checkpoint().await.unwrap();
    });
    println!(
        "startup sequence on the rewritten core: {:.1}s",
        started.elapsed().as_secs_f64()
    );
}

#[test]
#[ignore = "Set ONRECORD_STORAGE_REWRITE_CORPUS_COPY to a COPY of onrecord_studio_corpus.db; rewrites it in place"]
fn rewrite_a_copied_corpus_then_open_it() {
    let path = PathBuf::from(
        std::env::var("ONRECORD_STORAGE_REWRITE_CORPUS_COPY").expect("path to a copied corpus"),
    );
    assert!(
        !path.to_string_lossy().contains("Application Support"),
        "refusing to rewrite what looks like a live installation: {}",
        path.display()
    );
    let StorageRewrite::Rewritten(report) =
        rewrite_to_current_storage(&path).expect("rewrite must succeed")
    else {
        panic!("the copy was already at the pinned storage version");
    };
    println!(
        "corpus rewrite: {} -> {} bytes, {:.1}s, {} rows in {} tables verified",
        report.bytes_before,
        report.bytes_after,
        report.elapsed.as_secs_f64(),
        report.total_rows(),
        report.tables.len()
    );
    onrecord_studio_lib::storage::corpus::CorpusIndex::open(&path)
        .expect("rewritten corpus must open");
}
