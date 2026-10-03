// Release-scale coverage is orchestrated by `scripts/validate-release-performance.mjs`: the
// high-link and parser-pressure cases below run beside the million-row analytical harness and
// the durable interrupted-frontier recovery test.
use super::*;

use crate::ingestion::client::HttpClientConfig;

use tempfile::tempdir;

mod crawl;
mod extraction;
mod frontier;
mod guidance;
mod intake;
mod manager;
mod objects;
mod payload;
mod progress;
mod resources;
mod structured;
mod worker;

fn metadata_task(page: usize) -> IngestionTask {
    IngestionTask::FetchMetadataPage {
        subdomain: "records".to_string(),
        page,
        per_page: 50,
        search: None,
    }
}

/// Builds a real `WorkerContext` so the download gate is exercised on the path a crawl
/// actually takes, rather than on the predicate alone.
async fn crawl_worker_context(
    directory: &tempfile::TempDir,
    guidance: SourceGuidance,
) -> WorkerContext {
    let repository = DuckDbRepository::new(Path::new(":memory:")).expect("DuckDB core must open");
    repository
        .run_migrations(include_str!(
            "../../../../migrations/0001_initial_schema.sql"
        ))
        .await
        .expect("migrations must apply");
    let corpus_index =
        CorpusIndex::open(directory.path().join("corpus.db")).expect("corpus index must open");
    let (progress_sender, _progress_receiver) = mpsc::channel(64);
    let (dispatcher, _task_receiver) = mpsc::channel(64);
    let portal = PortalRegistration {
        portal_key: "records".to_string(),
        display_name: "Records".to_string(),
        canonical_url: "https://records.example.test/portal".to_string(),
        origin_url: "https://records.example.test/portal".to_string(),
        api_layout: PortalApiLayout {
            requests_endpoint: "https://records.example.test/client/requests".to_string(),
            requests_pointer: "/requests".to_string(),
            metadata_pointer: None,
            page_parameter: "page".to_string(),
            page_size_parameter: "per_page".to_string(),
        },
        verified_at: "2026-08-26T00:00:00Z".to_string(),
        source_kind: SourceKind::NextRequest,
    };
    repository
        .insert_or_update_portal_registration(&portal)
        .await
        .expect("source registration must persist");
    repository
        .begin_portal_sync_run("download-gate-run", &portal.portal_key)
        .await
        .expect("sync run must begin");
    let compiled_guidance = CompiledGuidancePatterns::compile(&guidance);
    WorkerContext {
        portal,
        client: ApiClient::new(HttpClientConfig::default()).expect("client must build"),
        repository,
        corpus_index,
        limiter: RateLimiter::new(5).expect("limiter must build"),
        dispatcher,
        progress_sender,
        global_semaphore: Arc::new(Semaphore::new(4)),
        parser_semaphore: Arc::new(Semaphore::new(4)),
        ocr_semaphore: Arc::new(Semaphore::new(4)),
        parser_memory: Arc::new(Semaphore::new(PARSER_MEMORY_PERMITS as usize)),
        download_root: directory.path().to_path_buf(),
        statistics: Arc::new(PortalStatistics::default()),
        pools: Arc::new(RwLock::new(HashMap::new())),
        provisioning_lock: Arc::new(Mutex::new(())),
        slot_freed: watch::channel(0).0,
        cancellation: watch::channel(false).0,
        sync_run_id: "download-gate-run".to_string(),
        crawl: Arc::new(CrawlState {
            policy: CrawlPolicy::default(),
            guidance,
            subject: None,
            visited_urls: Mutex::new(HashSet::new()),
            resource_urls: Mutex::new(HashSet::new()),
            crawl_path_claims: Mutex::new(HashSet::new()),
            announced_host_stops: Mutex::new(HashSet::new()),
            frontier_exclusions: Mutex::new(BTreeMap::new()),
            pages_started: AtomicUsize::new(0),
            resources_started: AtomicUsize::new(0),
            frontier_refill_lock: Mutex::new(()),
            arcgis_page_hashes: Mutex::new(HashSet::new()),
            compiled_guidance,
        }),
    }
}

async fn manager_with_source(
    directory: &tempfile::TempDir,
) -> (
    Arc<QueueManager>,
    DuckDbRepository,
    PortalRegistration,
    mpsc::Receiver<IngestionProgress>,
) {
    let repository = DuckDbRepository::new(Path::new(":memory:")).expect("DuckDB core must open");
    repository
        .run_migrations(include_str!(
            "../../../../migrations/0001_initial_schema.sql"
        ))
        .await
        .expect("migrations must apply");
    let corpus_index =
        CorpusIndex::open(directory.path().join("corpus.db")).expect("corpus index must open");
    let (progress_sender, progress_receiver) = mpsc::channel(64);
    let manager = Arc::new(
        QueueManager::new(
            ApiClient::new(HttpClientConfig::default()).expect("client must build"),
            repository.clone(),
            corpus_index,
            RateLimiter::new(5).expect("limiter must build"),
            progress_sender,
            directory.path(),
        )
        .expect("queue manager must build"),
    );
    let source = PortalRegistration {
        portal_key: "records".to_string(),
        display_name: "Records".to_string(),
        canonical_url: "https://records.example.test".to_string(),
        origin_url: "https://records.example.test".to_string(),
        api_layout: PortalApiLayout {
            requests_endpoint: "https://records.example.test/client/requests".to_string(),
            requests_pointer: "/requests".to_string(),
            metadata_pointer: None,
            page_parameter: "page".to_string(),
            page_size_parameter: "per_page".to_string(),
        },
        verified_at: "2026-08-26T00:00:00Z".to_string(),
        source_kind: SourceKind::NextRequest,
    };
    repository
        .insert_or_update_portal_registration(&source)
        .await
        .expect("source registration must persist");
    // The receiver is handed back so the caller keeps the channel open for the test's
    // lifetime; nothing here reads telemetry.
    (manager, repository, source, progress_receiver)
}
