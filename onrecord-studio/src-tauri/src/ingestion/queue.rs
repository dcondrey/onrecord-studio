//! Bounded, portal-isolated ingestion scheduling and document extraction.

use crate::blocking::spawn as spawn_blocking;
use crate::error::AppError;
use crate::file_name::sanitized_component;
use crate::ingestion::backoff;
use crate::ingestion::client::ApiClient;
use crate::ingestion::client::{
    AcquisitionRecord, DataCatalogEndpoint, DownloadedPayload, PublicEmbeddedConnector,
    PublicResourceInspection, PublicResourceKind, PublicResourceLink,
    WordPressGoogleDriveConnector,
};
use crate::ingestion::host_policy::RequestRefusal;
use crate::ingestion::limiter::RateLimiter;
use crate::integrity::{constant_time_equal, digest_hex, sha256_file_async, sha256_hex};
use crate::parsers::{
    assess_file_entropy, detect_document_format, execute_local_ocr_scan, extract_archive_container,
    extract_duckdb_text, extract_html_text, extract_msg_layers, extract_office_package,
    extract_plain_text, extract_rtf_text, extract_salvage_report, extract_spreadsheet_layers,
    extract_sql_dump_text, extract_sqlite_text, extract_svg_text, extract_text_layers,
    extract_zip_container_with_manifest, parser_capability, DocumentFormat, ExtractionRoute,
    MsgExtractionContext,
};
use crate::storage::corpus::{CorpusDocumentInput, CorpusIndex};
use crate::storage::sqlite::{
    DocumentDownloadFinalization, DocumentExtractionFinalization, DuckDbRepository,
    PortalSyncRunFinalization,
};
use crate::types::{
    CrawlPolicy, DocumentMetadata, DocumentPurge, FrontierExclusionCount, MetadataPageWindow,
    PastedContentInput, PortalApiLayout, PortalRegistration, PortalRuntimeSnapshot,
    PortalSyncStart, PublicRequest, RateLimitPolicy, SourceGuidance, SourceIngestionStart,
    SourceKind, SourceLifecycleState, SourcePurge, SourcePurgePreview, SourceRemoval, StoppedHost,
};
use chrono::{NaiveDate, Utc};
use regex::Regex;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{mpsc, watch, Mutex, RwLock, Semaphore};
use uuid::Uuid;

// The concurrent-portal ceiling is a runtime setting (`QueueManager::max_active_portals`),
// not a hard limit: a start beyond it parks until a running pool's slot frees rather than
// being refused. This constant is only the setting's default for a fresh install.
const DEFAULT_MAXIMUM_ACTIVE_PORTALS: usize = 5;
const WORKERS_PER_PORTAL: usize = 2;
const GLOBAL_WORKER_LIMIT: usize = 10;
const MAXIMUM_CPU_PARSERS: usize = 6;
const MAXIMUM_CONCURRENT_OCR: usize = 2;
const PARSER_MEMORY_PERMIT_BYTES: u64 = 16 * 1024 * 1024;
const PARSER_MEMORY_PERMITS: u32 = 64;
const PORTAL_QUEUE_CAPACITY: usize = 2_048;
const MAXIMUM_METADATA_PAGE: usize = 50_000;
const MAXIMUM_CONTAINER_DEPTH: usize = 8;
// Structured (JSON) expansion chains on its own axis, exactly like container nesting: a JSON
// payload yields downloads, one of which is JSON again. Site-crawl depth must not pay for that,
// or a catalog found deep in a portal silently stops yielding documents.
const MAXIMUM_STRUCTURED_CHAIN: usize = 8;
const MAXIMUM_CONTAINER_ENTRIES: usize = 2_000;
const MAXIMUM_CONTAINER_TREE_ENTRIES: usize = MAXIMUM_CONTAINER_ENTRIES * MAXIMUM_CONTAINER_DEPTH;
const MAXIMUM_CONTAINER_TREE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
// Synthetic child IDs occupy a dedicated range that remains exactly representable by JavaScript.
// Keep this namespace disjoint from upstream IDs when introducing another derived document kind.
const SYNTHETIC_DOCUMENT_NAMESPACE: u64 = 1_u64 << 52;
const SYNTHETIC_SOURCE_IDENTIFIER_MASK: u64 = (1_u64 << 51) - 1;
const MAXIMUM_LOCAL_IMPORT_BYTES: u64 = 512 * 1024 * 1024;
const MAXIMUM_LOCAL_IMPORT_FILES: usize = 1_000;
const MAXIMUM_PASTED_CONTENT_BYTES: usize = 16 * 1024 * 1024;

// LIMIT(ingestion): Structured discovery is intentionally unavailable above this fixed per-file
// ceiling; raising it alone would permit downloads that parsers cannot safely hold.
const MAXIMUM_STRUCTURED_DISCOVERY_BYTES: u64 = 64 * 1024 * 1024;
const MAXIMUM_STRUCTURED_DISCOVERY_LINKS: usize = 100_000;
const ARCGIS_FEATURE_PAGE_SIZE: usize = 500;
const MAXIMUM_DURABLE_TASK_ATTEMPTS: u64 = 4;

// Scheduler admission counters, durable task attempts, and retry eligibility persist in DuckDB.
// Channel position is rebuilt from monotonically increasing frontier identifiers after restart.

#[derive(Debug, Clone, serde::Serialize)]
#[serde(
    tag = "type",
    content = "payload",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum IngestionProgress {
    PortalVerified {
        portal: PortalRegistration,
    },
    SyncStarted {
        portal_key: String,
        sync_run_id: String,
    },
    PageStarted {
        portal_key: String,
        page: usize,
    },
    PageCompleted {
        portal_key: String,
        page: usize,
        records_found: usize,
    },
    DocumentStarted {
        portal_key: String,
        document_id: u64,
        file_name: String,
    },
    DocumentExtracted {
        portal_key: String,
        document_id: u64,
        character_count: usize,
        was_ocr_processed: bool,
    },
    DocumentExpanded {
        portal_key: String,
        document_id: u64,
        format: String,
        entry_count: usize,
    },
    DocumentDeferred {
        portal_key: String,
        document_id: u64,
        format: String,
        reason: String,
    },
    DocumentSalvaged {
        portal_key: String,
        document_id: u64,
        format: String,
        provenance_marker: String,
        source_sha256: String,
        recovered_run_count: usize,
        decoder_summary: String,
        primary_error: String,
    },
    DocumentCompleted {
        portal_key: String,
        document_id: u64,
        bytes_saved: u64,
    },
    SyncStopped {
        portal_key: String,
        sync_run_id: String,
    },
    TaskFailed {
        portal_key: String,
        target_id: String,
        error_msg: String,
    },
    TaskRetryScheduled {
        portal_key: String,
        target_id: String,
        attempt: u64,
        delay_seconds: u64,
        error_msg: String,
    },
    SourceNeedsGuidance {
        portal_key: String,
        source_url: String,
        reason: String,
    },
    /// This run will not contact a host again. Emitted once per host, because the alternative is
    /// the failure stream the operator actually saw: the same refusal repeated per URL, with
    /// nothing saying the run had stopped being welcome. `requests_spent` bounds what the run
    /// managed to read from the host before it stopped.
    HostStopped {
        portal_key: String,
        host: String,
        requests_spent: usize,
        reason: String,
    },
    /// A pre-DuckDB core database was converted during startup. Emitted once, before the window
    /// is shown, so the investigator learns where the retained original was kept.
    LegacyCoreDatabaseMigrated {
        backup_path: String,
        copied_tables: u64,
        copied_rows: u64,
        tables_absent_from_legacy: Vec<String>,
        tables_dropped_from_legacy: Vec<String>,
    },
    CorpusReconciliationStarted {},
    CorpusReconciliationCompleted {
        indexed_documents: u64,
        skipped_documents: u64,
        failed_documents: u64,
        first_error: Option<String>,
    },
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestionTelemetryEnvelope {
    pub protocol_version: u16,
    pub sequence: String,
    pub emitted_at: String,
    #[serde(flatten)]
    pub event: IngestionProgress,
}

// NOTE(telemetry): This sequence is process-local deduplication metadata, not a durable event ID;
// consumers must expect it to restart with the native process.
static INGESTION_TELEMETRY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub fn ingestion_telemetry_envelope(event: IngestionProgress) -> IngestionTelemetryEnvelope {
    IngestionTelemetryEnvelope {
        protocol_version: 1,
        sequence: INGESTION_TELEMETRY_SEQUENCE
            .fetch_add(1, Ordering::AcqRel)
            .saturating_add(1)
            .to_string(),
        emitted_at: Utc::now().to_rfc3339(),
        event,
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "taskKind", rename_all = "camelCase")]
pub enum IngestionTask {
    FetchMetadataPage {
        subdomain: String,
        page: usize,
        per_page: usize,
    },
    DownloadBinaryPayload {
        subdomain: String,
        request_id: u64,
        #[serde(with = "persisted_document")]
        document: Box<DocumentMetadata>,
        output_directory: String,
        container_depth: usize,
        /// Managed root of the recursive container tree. This is persisted so aggregate disk
        /// accounting remains effective when a durable child task resumes after restart.
        #[serde(default)]
        container_root_directory: Option<String>,
        /// Crawl-graph depth; see `PayloadDepth`. Defaulted because the durable frontier holds
        /// rows serialized before this field existed, and those resume at depth 0 as they did.
        #[serde(default)]
        crawl_depth: usize,
        /// Structured-chain depth; see `PayloadDepth`. Defaulted for the same reason.
        #[serde(default)]
        structured_depth: usize,
    },
    CrawlPublicResource {
        subdomain: String,
        url: String,
        depth: usize,
    },
    CrawlWordPressGoogleDrive {
        subdomain: String,
        connector: WordPressGoogleDriveConnector,
        depth: usize,
    },
    CrawlArcGisLayer {
        subdomain: String,
        layer_url: String,
        offset: usize,
        /// Crawl-graph depth; see `PayloadDepth`. Defaulted for the same reason.
        #[serde(default)]
        crawl_depth: usize,
        /// Structured-chain depth; see `PayloadDepth`. A layer reached from a JSON catalog is one
        /// chain hop, and its feature pages are `application/geo+json`, so they expand again.
        #[serde(default)]
        structured_depth: usize,
    },
    /// One page of a platform catalog index (CKAN, Socrata). Paging an index is not crawling: it
    /// visits one endpoint repeatedly with a cursor, so it holds no crawl depth.
    FetchDataCatalogPage {
        subdomain: String,
        endpoint: DataCatalogEndpoint,
        #[serde(default)]
        cursor: Option<String>,
    },
    ImportLocalPayload {
        subdomain: String,
        request_id: u64,
        #[serde(with = "persisted_document")]
        document: Box<DocumentMetadata>,
        source_path: String,
        output_directory: String,
    },
}

mod persisted_document {
    use serde::ser::SerializeStruct;
    use serde::Deserialize;

    use super::DocumentMetadata;

    #[derive(Deserialize)]
    struct StoredDocumentMetadata {
        id: u64,
        file_name: Option<String>,
        file_size: Option<u64>,
        description: Option<String>,
        content_type: Option<String>,
        download_url: Option<String>,
        created_at: String,
        updated_at: String,
        visibility: String,
        local_path: Option<String>,
        extracted_text_sha256: Option<String>,
        was_ocr_processed: bool,
    }

    pub(super) fn serialize<S>(
        document: &DocumentMetadata,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut state = serializer.serialize_struct("DocumentMetadata", 12)?;
        state.serialize_field("id", &document.id)?;
        state.serialize_field("file_name", &document.file_name)?;
        state.serialize_field("file_size", &document.file_size)?;
        state.serialize_field("description", &document.description)?;
        state.serialize_field("content_type", &document.content_type)?;
        state.serialize_field("download_url", &document.download_url)?;
        state.serialize_field("created_at", &document.created_at)?;
        state.serialize_field("updated_at", &document.updated_at)?;
        state.serialize_field("visibility", &document.visibility)?;
        state.serialize_field("local_path", &document.local_path)?;
        state.serialize_field("extracted_text_sha256", &document.extracted_text_sha256)?;
        state.serialize_field("was_ocr_processed", &document.was_ocr_processed)?;
        state.end()
    }

    pub(super) fn deserialize<'de, D>(deserializer: D) -> Result<Box<DocumentMetadata>, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let document = StoredDocumentMetadata::deserialize(deserializer)?;
        Ok(Box::new(DocumentMetadata {
            id: document.id,
            file_name: document.file_name,
            file_size: document.file_size,
            description: document.description,
            content_type: document.content_type,
            download_url: document.download_url,
            created_at: document.created_at,
            updated_at: document.updated_at,
            visibility: document.visibility,
            local_path: document.local_path,
            extracted_text_sha256: document.extracted_text_sha256,
            was_ocr_processed: document.was_ocr_processed,
        }))
    }
}

/// Whether a pool start may re-seed a request index the previous run was part-way through.
///
/// `OperatorRequested` is the answer for every start an operator performed: they chose the page
/// window that this start seeds, and an inherited frontier does not outrank what they just asked
/// for. `ContinueInterruptedWalk` is the answer for a resume, which is a continuation of an
/// earlier start and not a new instruction: a source paused at page 900 already carries page 901
/// in its durable frontier, so seeding its configured start page there re-fetches every page in
/// between. Deduplication makes that correct and not free -- 900 requests against a portal that
/// rate-limits.
#[derive(Clone, PartialEq, Eq, Debug)]
enum PoolSeedPolicy {
    OperatorRequested,
    ContinueInterruptedWalk,
    AutomaticRefresh,
    AutomaticDiscovery { submitted_url: String },
}

struct ScheduledTask {
    task: IngestionTask,
    frontier_id: Option<i64>,
    frontier_attempt: u64,
}

impl IngestionTask {
    fn portal_key(&self) -> &str {
        match self {
            Self::FetchMetadataPage { subdomain, .. }
            | Self::DownloadBinaryPayload { subdomain, .. }
            | Self::CrawlPublicResource { subdomain, .. }
            | Self::CrawlWordPressGoogleDrive { subdomain, .. }
            | Self::CrawlArcGisLayer { subdomain, .. }
            | Self::FetchDataCatalogPage { subdomain, .. }
            | Self::ImportLocalPayload { subdomain, .. } => subdomain,
        }
    }

    fn target_id(&self) -> String {
        match self {
            Self::FetchMetadataPage {
                subdomain, page, ..
            } => format!("{subdomain}-page-{page}"),
            Self::DownloadBinaryPayload {
                subdomain,
                document,
                ..
            } => format!("{subdomain}-document-{}", document.id),
            Self::CrawlPublicResource { subdomain, url, .. } => {
                format!("{subdomain}-crawl-{}", stable_source_identifier(url))
            }
            Self::CrawlWordPressGoogleDrive {
                subdomain,
                connector,
                ..
            } => format!(
                "{subdomain}-google-drive-{}",
                stable_source_identifier(&format!(
                    "{}|{}",
                    connector.endpoint, connector.folder_id
                ))
            ),
            Self::CrawlArcGisLayer {
                subdomain,
                layer_url,
                offset,
                ..
            } => format!(
                "{subdomain}-arcgis-{}-{offset}",
                stable_source_identifier(layer_url)
            ),
            Self::FetchDataCatalogPage {
                subdomain,
                endpoint,
                cursor,
            } => format!(
                "{subdomain}-catalog-{}-{}",
                stable_source_identifier(&endpoint.api_url),
                cursor.as_deref().unwrap_or("start")
            ),
            Self::ImportLocalPayload {
                subdomain,
                source_path,
                ..
            } => format!(
                "{subdomain}-local-{}",
                stable_source_identifier(source_path)
            ),
        }
    }

    /// The prefix every `FetchMetadataPage` target identifier for `source_key` carries.
    ///
    /// Written here rather than at the call site that queries the frontier, because it is the
    /// same construction `target_id` performs and the two must not drift: a resume decides
    /// whether to re-seed its request index by asking whether a row with this prefix survived,
    /// and a prefix that no longer matched would silently re-seed every time.
    fn metadata_page_target_prefix(source_key: &str) -> String {
        format!("{source_key}-page-")
    }

    /// The remote URL this task will contact, when it contacts one. Used to attribute a host stop
    /// to the host that issued it; `target_id` is an internal identifier and never a URL.
    fn target_url(&self) -> Option<&str> {
        match self {
            Self::DownloadBinaryPayload { document, .. } => document.download_url.as_deref(),
            Self::CrawlPublicResource { url, .. } => Some(url),
            Self::CrawlWordPressGoogleDrive { connector, .. } => Some(&connector.endpoint),
            Self::CrawlArcGisLayer { layer_url, .. } => Some(layer_url),
            Self::FetchDataCatalogPage { endpoint, .. } => Some(&endpoint.api_url),
            Self::FetchMetadataPage { .. } | Self::ImportLocalPayload { .. } => None,
        }
    }
}

struct CrawlState {
    policy: CrawlPolicy,
    guidance: SourceGuidance,
    visited_urls: Mutex<HashSet<String>>,
    resource_urls: Mutex<HashSet<String>>,
    /// One site path, one crawl node. See `claim_crawl_path`.
    crawl_path_claims: Mutex<HashSet<String>>,
    /// Hosts whose stop has already been announced, so one stop produces one event.
    announced_host_stops: Mutex<HashSet<String>>,
    /// How many links each frontier rule refused, keyed by rule. A crawl that silently drops
    /// links reports a corpus it cannot account for, which is the same false claim of coverage
    /// this codebase forbids elsewhere; these counts are what make the drop legible.
    frontier_exclusions: Mutex<BTreeMap<String, u64>>,
    pages_started: AtomicUsize,
    resources_started: AtomicUsize,
    frontier_refill_lock: Mutex<()>,
    arcgis_page_hashes: Mutex<HashSet<String>>,
    /// Regex patterns from `guidance`, compiled once when the crawl starts. See
    /// `CompiledGuidancePatterns` for why a pattern that fails to compile is silently dropped
    /// here rather than aborting the crawl.
    compiled_guidance: CompiledGuidancePatterns,
}

/// Regex predicates from `SourceGuidance`, compiled once per crawl instead of once per
/// evaluation.
///
/// `validate_source_guidance` already refuses to save a pattern that fails `Regex::new`, so
/// reaching this code with an invalid one means the guidance was written some other way (an
/// older save format, a direct database edit). That must not panic the ingestion pipeline: an
/// invalid pattern is simply absent from the compiled list, which makes the predicate it belongs
/// to contribute nothing rather than crash or, worse, silently exclude everything (an `include`
/// list that compiled to nothing would otherwise admit no document).
struct CompiledGuidancePatterns {
    include_filename_patterns: Vec<Regex>,
    exclude_filename_patterns: Vec<Regex>,
    include_content_patterns: Vec<Regex>,
    exclude_content_patterns: Vec<Regex>,
}

impl CompiledGuidancePatterns {
    fn compile(guidance: &SourceGuidance) -> Self {
        let compile_all = |patterns: &[String]| -> Vec<Regex> {
            patterns
                .iter()
                .filter_map(|pattern| Regex::new(pattern).ok())
                .collect()
        };
        Self {
            include_filename_patterns: compile_all(&guidance.include_filename_patterns),
            exclude_filename_patterns: compile_all(&guidance.exclude_filename_patterns),
            include_content_patterns: compile_all(&guidance.include_content_patterns),
            exclude_content_patterns: compile_all(&guidance.exclude_content_patterns),
        }
    }
}

/// Why the frontier refused a discovered link.
///
/// Returned rather than folded into a bare `false` so the count that reaches the operator names
/// the rule that fired. Scoping rules added later extend this enum instead of replacing it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum FrontierExclusion {
    /// The source's saved guidance excludes the URL by prefix or hostname.
    Guidance,
    /// The link differs from an already-scheduled crawl node only by its query string. A site
    /// crawl that follows these enumerates a faceted search, which is combinatorial and is what
    /// made the queue unbounded: `/dataset/?tags=species`, `?tags=wildlife`, `?tags=habitat` and
    /// so on are one page with a filter, not distinct records.
    QueryVariant { path: String },
    /// The host is off-scope for this source and no policy admits it.
    OffScopeHost,
    /// The payload is hosted off the source root and the source set
    /// `restrict_downloads_to_source_host`. Distinct from `OffScopeHost` because that rule governs
    /// which links are followed and this one governs only which payloads are fetched; a ledger that
    /// merged them could not say which policy dropped the document.
    OffSourceHostDownload,
    /// The crawl depth ceiling was reached.
    DepthCeiling,
    /// The link sits in the page's navigation chrome. See `is_navigation_chrome`.
    NavigationChrome,
    /// This run has stopped contacting the host. Refused before the task is scheduled so a
    /// stopped host does not accumulate tasks that each fail at dispatch.
    HostStopped { detail: String },
    /// The host's robots.txt excludes the path.
    RobotsExcluded { rule: String },
    /// The document's derived or API-reported filename fails a `SourceGuidance` filename glob or
    /// regex include/exclude rule. Evaluated before download.
    FilenamePattern,
    /// A known file size falls outside `minimum_file_size_bytes`/`maximum_file_size_bytes`.
    /// Evaluated before download, and only when a size is actually known pre-download.
    FileSize,
    /// The document's reported date falls outside `document_date_from`/`document_date_to`.
    /// Evaluated after extraction, and only when the source supplies a document-carried date.
    DocumentDate,
    /// The extracted text fails a content keyword include/exclude rule. Evaluated after
    /// extraction, once text is actually available.
    ContentKeyword,
    /// The extracted text fails a content regex include/exclude rule. Evaluated after extraction.
    ContentPattern,
    /// The owning request's agency fails an organization include/exclude rule. Evaluated after
    /// extraction, alongside the content rules above.
    Organization,
}

impl FrontierExclusion {
    fn rule(&self) -> &'static str {
        match self {
            Self::Guidance => "guidance",
            Self::QueryVariant { .. } => "query-variant",
            Self::OffScopeHost => "off-scope-host",
            Self::OffSourceHostDownload => "off-source-host-download",
            Self::DepthCeiling => "depth-ceiling",
            Self::NavigationChrome => "navigation-chrome",
            Self::HostStopped { .. } => "host-stopped",
            Self::RobotsExcluded { .. } => "robots-excluded",
            Self::FilenamePattern => "filename-pattern",
            Self::FileSize => "file-size",
            Self::DocumentDate => "document-date",
            Self::ContentKeyword => "content-keyword",
            Self::ContentPattern => "content-pattern",
            Self::Organization => "organization",
        }
    }
}

#[derive(Default)]
struct PortalStatistics {
    // Includes tasks persisted in the durable frontier, not only tasks currently in the channel.
    queued_tasks: AtomicUsize,
    active_tasks: AtomicUsize,
    live_workers: AtomicUsize,
    completed_tasks: AtomicU64,
    failed_tasks: AtomicU64,
    pages_processed: AtomicU64,
    requests_processed: AtomicU64,
    documents_processed: AtomicU64,
    data_documents_processed: AtomicU64,
    running: AtomicBool,
    stop_requested: AtomicBool,
    last_failure: StdMutex<Option<String>>,
}

struct PortalPool {
    portal: PortalRegistration,
    cancellation: watch::Sender<bool>,
    statistics: Arc<PortalStatistics>,
    sync_run_id: String,
    worker_handles: Vec<tokio::task::JoinHandle<()>>,
    crawl: Arc<CrawlState>,
}

/// Depth bookkeeping carried by a payload task.
///
/// Three independent axes, each with its own ceiling.
///
/// `container` counts archive nesting, bounded by `MAXIMUM_CONTAINER_DEPTH`.
///
/// `crawl` is the payload's depth in the site graph and has exactly the same meaning as
/// `IngestionTask::CrawlPublicResource::depth`: seeds are 0, anything derived from a node at
/// depth D is at D + 1, bounded by `CrawlPolicy::maximum_depth`. Paginating a single resource
/// (ArcGIS feature offsets, ArcGIS search pages) holds its depth, because it revisits one
/// resource rather than following a link.
///
/// `structured` counts JSON -> download -> JSON hops, bounded by `MAXIMUM_STRUCTURED_CHAIN`.
/// It is deliberately separate from `crawl`: a Socrata or ArcGIS catalog sitting eight pages
/// into a portal must still expand, and the chain it can build must still terminate. Laundering
/// a chain through an HTML page resets `structured` but spends `crawl`, since every HTML hop
/// goes through `reserve_crawl_page`, so the two axes bound the combination.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PayloadDepth {
    container: usize,
    crawl: usize,
    structured: usize,
}

impl PayloadDepth {
    /// A payload entering the pipeline from a crawl or listing rather than from inside another
    /// payload: no archive nesting, no structured chain behind it.
    fn discovered(crawl: usize, structured: usize) -> Self {
        Self {
            container: 0,
            crawl,
            structured,
        }
    }
}

#[derive(Clone)]
struct WorkerContext {
    portal: PortalRegistration,
    client: ApiClient,
    repository: DuckDbRepository,
    corpus_index: CorpusIndex,
    limiter: RateLimiter,
    dispatcher: mpsc::Sender<ScheduledTask>,
    progress_sender: mpsc::Sender<IngestionProgress>,
    global_semaphore: Arc<Semaphore>,
    parser_semaphore: Arc<Semaphore>,
    ocr_semaphore: Arc<Semaphore>,
    parser_memory: Arc<Semaphore>,
    download_root: PathBuf,
    statistics: Arc<PortalStatistics>,
    pools: Arc<RwLock<HashMap<String, PortalPool>>>,
    provisioning_lock: Arc<Mutex<()>>,
    // Bumped whenever this pool's slot frees, so a start parked on a full concurrency cap
    // wakes up and re-checks rather than staying refused forever. Read by `retire_portal_if_idle`
    // as well as the explicit stop/removal paths, which is why it lives on the context rather
    // than only on `QueueManager`: a worker retiring its own pool on natural completion has no
    // other handle back to the manager.
    slot_freed: watch::Sender<u64>,
    cancellation: watch::Sender<bool>,
    sync_run_id: String,
    crawl: Arc<CrawlState>,
}

pub struct QueueManager {
    client: ApiClient,
    repository: DuckDbRepository,
    corpus_index: CorpusIndex,
    limiter: RateLimiter,
    progress_sender: mpsc::Sender<IngestionProgress>,
    download_root: PathBuf,
    pools: Arc<RwLock<HashMap<String, PortalPool>>>,
    provisioning_lock: Arc<Mutex<()>>,
    // Runtime setting: how many portal task pools may run concurrently. Defaults to
    // `DEFAULT_MAXIMUM_ACTIVE_PORTALS` and is adjustable at runtime via
    // `max_active_portals`/`set_max_active_portals`. Not persisted across restarts; see the
    // doc comment on `set_max_active_portals` for why.
    max_active_portals: AtomicUsize,
    slot_freed: watch::Sender<u64>,
    global_semaphore: Arc<Semaphore>,
    parser_semaphore: Arc<Semaphore>,
    ocr_semaphore: Arc<Semaphore>,
    parser_memory: Arc<Semaphore>,
}

enum ExtractedDocument {
    Searchable {
        text: String,
        was_ocr_processed: bool,
    },
    Expanded {
        format: DocumentFormat,
        entry_count: usize,
    },
    Deferred {
        format: DocumentFormat,
        route: ExtractionRoute,
    },
    Salvaged {
        report: crate::parsers::SalvageReport,
        format: DocumentFormat,
    },
}

impl QueueManager {
    pub fn new<P: AsRef<Path>>(
        client: ApiClient,
        repository: DuckDbRepository,
        corpus_index: CorpusIndex,
        limiter: RateLimiter,
        progress_sender: mpsc::Sender<IngestionProgress>,
        download_root: P,
    ) -> Result<Self, AppError> {
        let download_root = download_root.as_ref();
        if download_root.as_os_str().is_empty() {
            return Err(AppError::DataParsing(
                "Queue download root must not be empty".to_string(),
            ));
        }
        Ok(Self {
            client,
            repository,
            corpus_index,
            limiter,
            progress_sender,
            download_root: download_root.to_path_buf(),
            pools: Arc::new(RwLock::new(HashMap::new())),
            provisioning_lock: Arc::new(Mutex::new(())),
            max_active_portals: AtomicUsize::new(DEFAULT_MAXIMUM_ACTIVE_PORTALS),
            slot_freed: watch::channel(0u64).0,
            global_semaphore: Arc::new(Semaphore::new(GLOBAL_WORKER_LIMIT)),
            parser_semaphore: Arc::new(Semaphore::new(MAXIMUM_CPU_PARSERS)),
            ocr_semaphore: Arc::new(Semaphore::new(MAXIMUM_CONCURRENT_OCR)),
            parser_memory: Arc::new(Semaphore::new(PARSER_MEMORY_PERMITS as usize)),
        })
    }

    /// The current concurrent-portal cap. Defaults to `DEFAULT_MAXIMUM_ACTIVE_PORTALS`.
    pub fn max_active_portals(&self) -> usize {
        self.max_active_portals.load(Ordering::Acquire)
    }

    /// Raises or lowers the concurrent-portal cap at runtime.
    ///
    /// This is process-lifetime only: the codebase has no settings persistence mechanism
    /// (no settings table, no `Settings` struct, nothing under `src-tauri/migrations/` for
    /// user preferences) to survive it across restarts, and building one is out of scope for
    /// this change. A restart resets the cap to `DEFAULT_MAXIMUM_ACTIVE_PORTALS`.
    ///
    /// A start already parked on the old cap is woken immediately, not just on the next pool
    /// removal, so raising the limit takes effect without waiting for unrelated churn.
    ///
    /// This does not change `GLOBAL_WORKER_LIMIT`: total worker concurrency across every pool
    /// remains capped at that constant regardless of how many pools are allowed to run, so
    /// raising this setting increases how many sources may be active at once without raising
    /// how much work happens in parallel across all of them.
    pub fn set_max_active_portals(&self, value: usize) -> Result<(), AppError> {
        if value == 0 {
            return Err(AppError::DataParsing(
                "Concurrent portal limit must be at least 1".to_string(),
            ));
        }
        self.max_active_portals.store(value, Ordering::Release);
        self.slot_freed.send_modify(|generation| {
            *generation = generation.wrapping_add(1);
        });
        Ok(())
    }

    pub async fn discover_and_start(
        &self,
        portal_url: &str,
        window: MetadataPageWindow,
        rate_limit_policy: RateLimitPolicy,
    ) -> Result<PortalSyncStart, AppError> {
        validate_page_parameters(window.start_page, window.page_size)?;
        let portal = self.client.discover_portal(portal_url).await?;
        let _ = self
            .progress_sender
            .send(IngestionProgress::PortalVerified {
                portal: portal.clone(),
            })
            .await;
        // Persisted before the pool starts, and only when the operator moved off the built-in
        // window, so a resume seeds the slice they chose instead of silently reverting to 1 and
        // 50. The registration is written first because the recipe row has a foreign key to it;
        // this mirrors what the public-source path does with a non-default crawl recipe.
        if window != MetadataPageWindow::default() {
            self.repository
                .insert_or_update_portal_registration(&portal)
                .await?;
            self.repository
                .save_source_metadata_page_window(&portal, portal_url, window)
                .await?;
        }
        let sync_run_id = self
            .start_verified_portal(portal.clone(), window, rate_limit_policy)
            .await?;
        Ok(PortalSyncStart {
            portal,
            sync_run_id,
        })
    }

    pub async fn start_verified_portal(
        &self,
        portal: PortalRegistration,
        window: MetadataPageWindow,
        rate_limit_policy: RateLimitPolicy,
    ) -> Result<String, AppError> {
        validate_page_parameters(window.start_page, window.page_size)?;
        let portal_key = portal.portal_key.clone();
        self.start_task_pool(
            portal,
            rate_limit_policy,
            vec![IngestionTask::FetchMetadataPage {
                subdomain: portal_key,
                page: window.start_page,
                per_page: window.page_size,
            }],
            CrawlPolicy::default(),
            SourceGuidance::default(),
            PoolSeedPolicy::OperatorRequested,
        )
        .await
    }

    /// `window` is the page window recorded for this source, or `None` when none was recorded and
    /// the built-in window applies. A resume that ignored it would re-seed page 1 at size 50 and
    /// walk the range the operator deliberately started past.
    pub async fn resume_registered_source(
        &self,
        source: PortalRegistration,
        crawl_policy: CrawlPolicy,
        guidance: SourceGuidance,
        window: Option<MetadataPageWindow>,
    ) -> Result<String, AppError> {
        let window = window.unwrap_or_default();
        validate_page_parameters(window.start_page, window.page_size)?;
        let initial_tasks = resume_initial_tasks(&source, window);
        self.start_task_pool(
            source,
            RateLimitPolicy::default(),
            initial_tasks,
            crawl_policy,
            guidance,
            PoolSeedPolicy::ContinueInterruptedWalk,
        )
        .await
    }

    /// Hold the same lock as source starts so an idle check cannot race a new extraction.
    pub async fn maintain_idle_storage(
        &self,
        effective_at: Option<&str>,
    ) -> Result<bool, AppError> {
        let _provisioning_guard = self.provisioning_lock.lock().await;
        if self
            .runtime_snapshots()
            .await?
            .iter()
            .any(|runtime| runtime.is_running)
        {
            return Ok(false);
        }
        match effective_at {
            Some(at) => {
                self.repository
                    .compact_one_document(&self.download_root, at)
                    .await
            }
            None => {
                self.repository
                    .recover_maintenance_storage(&self.download_root)
                    .await
            }
        }
    }

    /// Background rechecks must never reactivate a source paused or removed by its owner.
    pub async fn refresh_registered_source(
        &self,
        source: PortalRegistration,
        crawl_policy: CrawlPolicy,
        guidance: SourceGuidance,
        window: Option<MetadataPageWindow>,
    ) -> Result<String, AppError> {
        let window = window.unwrap_or_default();
        validate_page_parameters(window.start_page, window.page_size)?;
        let initial_tasks = resume_initial_tasks(&source, window);
        self.start_task_pool(
            source,
            RateLimitPolicy::default(),
            initial_tasks,
            crawl_policy,
            guidance,
            PoolSeedPolicy::AutomaticRefresh,
        )
        .await
    }

    async fn start_task_pool(
        &self,
        portal: PortalRegistration,
        rate_limit_policy: RateLimitPolicy,
        initial_tasks: Vec<IngestionTask>,
        crawl_policy: CrawlPolicy,
        guidance: SourceGuidance,
        seed_policy: PoolSeedPolicy,
    ) -> Result<String, AppError> {
        crate::licensing::recheck()?;
        validate_crawl_policy(&crawl_policy)?;
        // At capacity, this parks rather than refusing: it releases `provisioning_lock` and
        // waits for `slot_freed` (bumped whenever a running pool is removed, or the cap is
        // raised) before re-checking. It must not hold the lock while parked, or the pool
        // removals it is waiting on could never take the lock to run.
        let mut was_parked = false;
        let _provisioning_guard = loop {
            let guard = self.provisioning_lock.lock().await;
            if let Some(existing) = self.pools.read().await.get(&portal.portal_key) {
                if existing.portal != portal {
                    return Err(AppError::PortalSecurity(format!(
                        "Portal key '{}' is already bound to a different verified endpoint",
                        portal.portal_key
                    )));
                }
                return Err(AppError::PortalCapacity(format!(
                    "Portal '{}' is already synchronizing",
                    portal.portal_key
                )));
            }
            // Subscribed before the cap is even read: `set_max_active_portals` sends outside
            // `provisioning_lock`, so a raise racing this check must be observable by the
            // subscription rather than possibly landing between the check and the subscribe,
            // where it would be a silently lost wakeup with no timeout to recover from it. A
            // raise that lands before this subscribe is at worst a spurious wake below, which
            // just re-checks and re-parks if the cap is still exceeded.
            let mut slot_freed = self.slot_freed.subscribe();
            let cap = self.max_active_portals.load(Ordering::Acquire);
            if self.pools.read().await.len() < cap {
                break guard;
            }
            was_parked = true;
            drop(guard);
            // `subscribe()` above baselines against the value at this instant, so a
            // `slot_freed` send that happens after it (even before this `await` starts
            // polling) is guaranteed to be observed here rather than lost.
            let _ = slot_freed.changed().await;
        };
        if matches!(&seed_policy, PoolSeedPolicy::AutomaticDiscovery { .. }) {
            // Discovery may have followed an alias or outlived a pause/removal. The
            // submitted URL is not the source identity. Check the resolved key AND
            // publication URL while holding the same lock as lifecycle mutations,
            // before registration or a recipe can overwrite the owner's choices.
            let key = portal.portal_key.clone();
            let canonical = portal.canonical_url.trim_end_matches('/').to_string();
            let already_registered = self.repository.query_connection(
                "Check automatic discovery identity", move |conn| {
                    conn.query_row(
                        "SELECT EXISTS (SELECT 1 FROM portals WHERE subdomain = ?1 OR rtrim(base_url, '/') = ?2)",
                        duckdb::params![key, canonical], |row| row.get::<_, bool>(0),
                    ).map_err(|error| AppError::Storage(error.to_string()))
                },
            ).await?;
            if already_registered {
                return Err(AppError::DataParsing(
                    "Automatic discovery resolved to an existing source. Its lifecycle and saved collection scope were preserved; use its existing refresh schedule or an explicit operator start.".into(),
                ));
            }
        }
        if was_parked || seed_policy == PoolSeedPolicy::AutomaticRefresh {
            // The park may have outlived the source: it could have been withdrawn or paused
            // while queued for a slot. Re-check before provisioning so a promoted start never
            // resurrects a source the operator removed while it waited. A brand-new source has
            // no lifecycle row yet (`None`), which is not a reason to refuse it.
            match self
                .repository
                .source_lifecycle_state(&portal.portal_key)
                .await?
            {
                Some(SourceLifecycleState::Removed) => {
                    return Err(AppError::DataParsing(format!(
                        "Source '{}' was withdrawn while its start was queued",
                        portal.portal_key
                    )));
                }
                Some(SourceLifecycleState::Paused) => {
                    return Err(AppError::DataParsing(format!(
                        "Source '{}' was paused while its start was queued",
                        portal.portal_key
                    )));
                }
                None if seed_policy == PoolSeedPolicy::AutomaticRefresh => {
                    return Err(AppError::DataParsing(
                        "Automatic refresh requires an active registered source".into(),
                    ));
                }
                Some(SourceLifecycleState::Active) | None => {}
            }
        }
        self.repository
            .insert_or_update_portal_registration(&portal)
            .await?;
        if let PoolSeedPolicy::AutomaticDiscovery { submitted_url } = &seed_policy {
            if guidance != SourceGuidance::default() || crawl_policy != CrawlPolicy::default() {
                self.repository
                    .save_source_connector_recipe(&portal, submitted_url, &crawl_policy, &guidance)
                    .await?;
            }
        }
        self.limiter
            .register_portal(&portal.portal_key, rate_limit_policy)
            .await?;
        let sync_run_id = Uuid::new_v4().to_string();
        if let Err(error) = self
            .repository
            .begin_portal_sync_run(&sync_run_id, &portal.portal_key)
            .await
        {
            self.limiter.unregister_portal(&portal.portal_key).await;
            return Err(error);
        }
        let adopted_frontier_tasks = match self
            .repository
            .adopt_orphaned_frontier_tasks(&portal.portal_key, &sync_run_id)
            .await
        {
            Ok(count) => count,
            Err(error) => {
                self.cancel_task_pool_start(&sync_run_id, &portal.portal_key)
                    .await;
                return Err(error);
            }
        };
        let scheduler_counts = match self
            .repository
            .ingestion_scheduler_counts(&sync_run_id, &portal.portal_key)
            .await
        {
            Ok(counts) => counts,
            Err(error) => {
                self.cancel_task_pool_start(&sync_run_id, &portal.portal_key)
                    .await;
                return Err(error);
            }
        };
        let recovered_pages_started = usize::try_from(scheduler_counts.crawl_pages_started)
            .map_err(|_| {
                AppError::PortalCapacity(
                    "Recovered crawl page counter exceeds platform capacity".to_string(),
                )
            })?;
        let recovered_resources_started = usize::try_from(scheduler_counts.resources_started)
            .map_err(|_| {
                AppError::PortalCapacity(
                    "Recovered resource counter exceeds platform capacity".to_string(),
                )
            })?;
        // Whether the adopted frontier still owes this source a request-index page, which is what
        // an interrupted walk looks like on disk: pagination enqueues page N+1 in the same durable
        // batch as page N's documents, and page N's row is deleted only after that batch is
        // written, so a surviving page row means the walk stopped part-way. Asked on every start
        // rather than only on a resume, so the condition is not written twice; `pool_seed_tasks`
        // owns what a start does with the answer.
        let walk_interrupted = if adopted_frontier_tasks > 0 {
            let prefix = IngestionTask::metadata_page_target_prefix(&portal.portal_key);
            match self
                .repository
                .frontier_holds_target_prefix(&sync_run_id, &portal.portal_key, &prefix)
                .await
            {
                Ok(interrupted) => interrupted,
                Err(error) => {
                    self.cancel_task_pool_start(&sync_run_id, &portal.portal_key)
                        .await;
                    return Err(error);
                }
            }
        } else {
            false
        };
        let initial_tasks = pool_seed_tasks(initial_tasks, seed_policy, walk_interrupted);
        if initial_tasks.is_empty() && adopted_frontier_tasks == 0 {
            self.cancel_task_pool_start(&sync_run_id, &portal.portal_key)
                .await;
            return Err(AppError::DataParsing(
                "An ingestion pool requires an initial or recoverable task".to_string(),
            ));
        }
        let (sender, receiver) = mpsc::channel(PORTAL_QUEUE_CAPACITY);
        let (cancellation, cancellation_receiver) = watch::channel(false);
        let statistics = Arc::new(PortalStatistics::default());
        statistics.queued_tasks.store(
            usize::try_from(adopted_frontier_tasks).map_err(|_| {
                AppError::PortalCapacity(
                    "Recovered ingestion frontier exceeds platform capacity".to_string(),
                )
            })?,
            Ordering::Release,
        );
        statistics.running.store(true, Ordering::Release);
        statistics
            .live_workers
            .store(WORKERS_PER_PORTAL, Ordering::Release);
        let receiver = Arc::new(Mutex::new(receiver));
        let compiled_guidance = CompiledGuidancePatterns::compile(&guidance);
        let crawl = Arc::new(CrawlState {
            policy: crawl_policy,
            guidance,
            compiled_guidance,
            visited_urls: Mutex::new(HashSet::new()),
            resource_urls: Mutex::new(HashSet::new()),
            crawl_path_claims: Mutex::new(HashSet::new()),
            announced_host_stops: Mutex::new(HashSet::new()),
            frontier_exclusions: Mutex::new(BTreeMap::new()),
            pages_started: AtomicUsize::new(recovered_pages_started),
            resources_started: AtomicUsize::new(recovered_resources_started),
            frontier_refill_lock: Mutex::new(()),
            arcgis_page_hashes: Mutex::new(HashSet::new()),
        });
        let context = WorkerContext {
            portal: portal.clone(),
            client: self.client.clone(),
            repository: self.repository.clone(),
            corpus_index: self.corpus_index.clone(),
            limiter: self.limiter.clone(),
            dispatcher: sender.clone(),
            progress_sender: self.progress_sender.clone(),
            global_semaphore: Arc::clone(&self.global_semaphore),
            parser_semaphore: Arc::clone(&self.parser_semaphore),
            ocr_semaphore: Arc::clone(&self.ocr_semaphore),
            parser_memory: Arc::clone(&self.parser_memory),
            download_root: self.download_root.clone(),
            statistics: Arc::clone(&statistics),
            pools: Arc::clone(&self.pools),
            provisioning_lock: Arc::clone(&self.provisioning_lock),
            slot_freed: self.slot_freed.clone(),
            cancellation: cancellation.clone(),
            sync_run_id: sync_run_id.clone(),
            crawl: Arc::clone(&crawl),
        };
        let worker_handles = (0..WORKERS_PER_PORTAL)
            .map(|_| {
                tokio::spawn(worker_loop(
                    context.clone(),
                    Arc::clone(&receiver),
                    cancellation_receiver.clone(),
                ))
            })
            .collect();
        let pool = PortalPool {
            portal: portal.clone(),
            cancellation: cancellation.clone(),
            statistics: Arc::clone(&statistics),
            sync_run_id: sync_run_id.clone(),
            worker_handles,
            crawl,
        };
        self.pools
            .write()
            .await
            .insert(portal.portal_key.clone(), pool);
        let enqueue_result = async {
            for task in initial_tasks {
                if let IngestionTask::CrawlPublicResource { url, .. } = &task {
                    if !mark_crawl_url(&context, url).await {
                        continue;
                    }
                    claim_crawl_path(&context, url).await;
                }
                enqueue_task_resilient(&context, task).await?;
            }
            refill_ingestion_frontier(&context).await?;
            Ok::<(), AppError>(())
        }
        .await;
        if let Err(error) = enqueue_result {
            let pool = self.pools.write().await.remove(&portal.portal_key);
            statistics.running.store(false, Ordering::Release);
            // Set before the workers stop, as `stop_source_pool_locked` does: the last worker out
            // announces `SyncStopped` unless a stop was requested, and this run never announced
            // `SyncStarted`.
            statistics.stop_requested.store(true, Ordering::Release);
            if let Some(pool) = pool {
                let _ = cancel_and_join_workers(&pool.cancellation, pool.worker_handles).await;
            }
            self.slot_freed.send_modify(|generation| {
                *generation = generation.wrapping_add(1);
            });
            self.cancel_task_pool_start(&context.sync_run_id, &portal.portal_key)
                .await;
            return Err(error);
        }
        let _ = self
            .progress_sender
            .send(IngestionProgress::SyncStarted {
                portal_key: portal.portal_key,
                sync_run_id: sync_run_id.clone(),
            })
            .await;
        Ok(sync_run_id)
    }

    async fn cancel_task_pool_start(&self, sync_run_id: &str, portal_key: &str) {
        self.limiter.unregister_portal(portal_key).await;
        let _ = self
            .repository
            .finish_portal_sync_run(PortalSyncRunFinalization {
                run_id: sync_run_id,
                portal_key,
                status: "CANCELLED",
                pages_processed: 0,
                requests_processed: 0,
                documents_processed: 0,
                failures: 1,
            })
            .await;
    }

    pub async fn discover_and_start_public_source(
        &self,
        source_url: &str,
        crawl_policy: CrawlPolicy,
        guidance: SourceGuidance,
        rate_limit_policy: RateLimitPolicy,
    ) -> Result<SourceIngestionStart, AppError> {
        self.discover_and_start_public_source_with_policy(
            source_url,
            crawl_policy,
            guidance,
            rate_limit_policy,
            PoolSeedPolicy::OperatorRequested,
        )
        .await
    }

    /// Automatic discovery admits only a new resolved source. Aliases must not
    /// reactivate a paused/removed source or replace an owner's saved recipe.
    pub async fn discover_and_start_public_source_automatically(
        &self,
        source_url: &str,
        crawl_policy: CrawlPolicy,
        guidance: SourceGuidance,
        rate_limit_policy: RateLimitPolicy,
    ) -> Result<SourceIngestionStart, AppError> {
        self.discover_and_start_public_source_with_policy(
            source_url,
            crawl_policy,
            guidance,
            rate_limit_policy,
            PoolSeedPolicy::AutomaticDiscovery {
                submitted_url: source_url.to_string(),
            },
        )
        .await
    }

    async fn discover_and_start_public_source_with_policy(
        &self,
        source_url: &str,
        crawl_policy: CrawlPolicy,
        guidance: SourceGuidance,
        rate_limit_policy: RateLimitPolicy,
        seed_policy: PoolSeedPolicy,
    ) -> Result<SourceIngestionStart, AppError> {
        validate_crawl_policy(&crawl_policy)?;
        validate_source_guidance(&guidance)?;
        // Platform detection runs ahead of the crawler, not as a fallback from it. A catalog
        // portal's index API names every asset it holds in a few paged requests; walking the HTML
        // that renders that index costs a request per page, per facet, and per navigation link,
        // which is what earned this client its 403s. Operator guidance overrides detection,
        // because a repair recipe is a statement about this source that a probe cannot outrank.
        let operator_supplied_recipe = !guidance.additional_seed_urls.is_empty()
            || guidance.link_selector.is_some()
            || guidance.download_selector.is_some();
        let catalog = if operator_supplied_recipe {
            None
        } else {
            self.client.detect_data_catalog(source_url).await
        };
        let (mut source, inspection) = self.client.discover_public_source(source_url).await?;
        if catalog.is_none()
            && matches!(inspection.kind, PublicResourceKind::HtmlPage)
            && inspection.links.is_empty()
            && inspection.embedded_connectors.is_empty()
            && !operator_supplied_recipe
        {
            return Err(AppError::PortalDiscovery(
                "The page was reachable but exposed no static data links. Open guided repair to select or paste the underlying dataset, download, folder, or dashboard URL."
                    .to_string(),
            ));
        }
        let initial_tasks = match catalog {
            Some(endpoint) => {
                source.source_kind = SourceKind::OpenDataCatalog;
                vec![IngestionTask::FetchDataCatalogPage {
                    subdomain: source.portal_key.clone(),
                    endpoint,
                    cursor: None,
                }]
            }
            None => {
                let mut seeds = vec![inspection.final_url.clone()];
                seeds.extend(guidance.additional_seed_urls.iter().cloned());
                seeds.sort();
                seeds.dedup();
                seeds
                    .into_iter()
                    .map(|url| IngestionTask::CrawlPublicResource {
                        subdomain: source.portal_key.clone(),
                        url,
                        depth: 0,
                    })
                    .collect()
            }
        };
        let detected_kind = source.source_kind;
        if seed_policy == PoolSeedPolicy::OperatorRequested
            && (guidance != SourceGuidance::default() || crawl_policy != CrawlPolicy::default())
        {
            self.repository
                .insert_or_update_portal_registration(&source)
                .await?;
            self.repository
                .save_source_connector_recipe(&source, source_url, &crawl_policy, &guidance)
                .await?;
        }
        let sync_run_id = self
            .start_task_pool(
                source.clone(),
                rate_limit_policy,
                initial_tasks,
                crawl_policy,
                guidance,
                seed_policy,
            )
            .await?;
        Ok(SourceIngestionStart {
            source,
            sync_run_id,
            detected_kind,
            used_saved_guidance: false,
        })
    }

    pub async fn preview_public_source(
        &self,
        source_url: &str,
        guidance: Option<SourceGuidance>,
    ) -> Result<crate::types::SourceDiscoveryPreview, AppError> {
        let guidance = guidance.unwrap_or_default();
        validate_source_guidance(&guidance)?;
        let selector_preview_applied =
            guidance.link_selector.is_some() || guidance.download_selector.is_some();
        if let Ok(source) = self.client.discover_portal(source_url).await {
            return Ok(crate::types::SourceDiscoveryPreview {
                detected_kind: SourceKind::NextRequest,
                source,
                confidence_basis_points: 10_000,
                candidate_links: Vec::new(),
                warnings: Vec::new(),
                repair_available: false,
                selector_preview_applied: false,
            });
        }
        let (source, default_inspection) = self.client.discover_public_source(source_url).await?;
        let inspection = if selector_preview_applied {
            self.client
                .inspect_public_resource_with_selectors(
                    source_url,
                    guidance.link_selector.as_deref(),
                    guidance.download_selector.as_deref(),
                )
                .await?
        } else {
            default_inspection
        };
        let has_embedded_connector = !inspection.embedded_connectors.is_empty();
        let candidate_links = inspection
            .links
            .into_iter()
            .take(500)
            .map(|link| crate::types::DiscoveredSourceLink {
                url: link.url,
                label: link.label,
                download_candidate: link.download_candidate,
                follow_candidate: link.follow_candidate && !link.navigation_chrome,
            })
            .collect::<Vec<_>>();
        let mut warnings = Vec::new();
        if matches!(inspection.kind, PublicResourceKind::HtmlPage)
            && candidate_links.is_empty()
            && !has_embedded_connector
        {
            warnings.push(
                "No static data links were visible; this source may need a dashboard, catalog, or custom-selector recipe."
                    .to_string(),
            );
        }
        let confidence_basis_points = if matches!(inspection.kind, PublicResourceKind::Download) {
            10_000
        } else if has_embedded_connector {
            9_500
        } else if candidate_links.iter().any(|link| link.download_candidate) {
            9_000
        } else if !candidate_links.is_empty() {
            7_500
        } else {
            4_000
        };
        Ok(crate::types::SourceDiscoveryPreview {
            detected_kind: source.source_kind,
            source,
            confidence_basis_points,
            candidate_links,
            repair_available: !warnings.is_empty(),
            selector_preview_applied,
            warnings,
        })
    }

    pub async fn start_local_files(
        &self,
        paths: Vec<String>,
    ) -> Result<SourceIngestionStart, AppError> {
        crate::licensing::recheck()?;
        if paths.is_empty() || paths.len() > MAXIMUM_LOCAL_IMPORT_FILES {
            return Err(AppError::DataParsing(format!(
                "Select between 1 and {MAXIMUM_LOCAL_IMPORT_FILES} files per import"
            )));
        }
        let mut canonical_paths = Vec::with_capacity(paths.len());
        for path in paths {
            if path.trim().is_empty() || path.len() > 32_768 {
                return Err(AppError::DataParsing(
                    "Selected file paths must not be empty".to_string(),
                ));
            }
            let path = tokio::fs::canonicalize(&path).await.map_err(|error| {
                AppError::Storage(format!("Failed to resolve selected file '{path}': {error}"))
            })?;
            let metadata = tokio::fs::symlink_metadata(&path).await.map_err(|error| {
                AppError::Storage(format!("Failed to inspect selected file: {error}"))
            })?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(AppError::DataParsing(format!(
                    "Selected path '{}' is not a regular file",
                    path.display()
                )));
            }
            if metadata.len() > MAXIMUM_LOCAL_IMPORT_BYTES {
                return Err(AppError::DataParsing(format!(
                    "Selected file '{}' exceeds the {}-byte import limit",
                    path.display(),
                    MAXIMUM_LOCAL_IMPORT_BYTES
                )));
            }
            canonical_paths.push(path);
        }
        canonical_paths.sort();
        canonical_paths.dedup();
        let identity = canonical_paths
            .iter()
            .map(|path| path.to_string_lossy())
            .collect::<Vec<_>>()
            .join("\n");
        let source = local_source_registration(
            SourceKind::LocalFile,
            if canonical_paths.len() == 1 {
                canonical_paths[0]
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "Local file".to_string())
            } else {
                format!("{} selected files", canonical_paths.len())
            },
            &identity,
        );
        self.repository
            .insert_or_update_portal_registration(&source)
            .await?;
        let request = synthetic_source_request(&source, "User-selected local evidence files");
        self.repository
            .insert_or_update_request(&source.portal_key, &request)
            .await?;
        let output_directory = self
            .download_root
            .join(format!("{}_documents", source.portal_key));
        let initial_tasks = canonical_paths
            .into_iter()
            .map(|path| {
                let path_string = path.to_string_lossy().into_owned();
                let document_id = stable_source_identifier(&format!("local|{path_string}"));
                let now = Utc::now().to_rfc3339();
                IngestionTask::ImportLocalPayload {
                    subdomain: source.portal_key.clone(),
                    request_id: request.id,
                    source_path: path_string,
                    output_directory: output_directory.to_string_lossy().into_owned(),
                    document: Box::new(DocumentMetadata {
                        id: document_id,
                        file_name: path
                            .file_name()
                            .map(|name| name.to_string_lossy().into_owned()),
                        file_size: None,
                        description: Some("User-selected local evidence file".to_string()),
                        content_type: None,
                        download_url: None,
                        created_at: now.clone(),
                        updated_at: now,
                        visibility: "public".to_string(),
                        local_path: None,
                        extracted_text_sha256: None,
                        was_ocr_processed: false,
                    }),
                }
            })
            .collect();
        let sync_run_id = self
            .start_task_pool(
                source.clone(),
                RateLimitPolicy::default(),
                initial_tasks,
                CrawlPolicy::default(),
                SourceGuidance::default(),
                PoolSeedPolicy::OperatorRequested,
            )
            .await?;
        Ok(SourceIngestionStart {
            source,
            sync_run_id,
            detected_kind: SourceKind::LocalFile,
            used_saved_guidance: false,
        })
    }

    pub async fn start_pasted_content(
        &self,
        input: PastedContentInput,
    ) -> Result<SourceIngestionStart, AppError> {
        crate::licensing::recheck()?;
        let title = input.title.trim();
        if title.is_empty() || title.len() > 256 {
            return Err(AppError::DataParsing(
                "Pasted content title must contain between 1 and 256 characters".to_string(),
            ));
        }
        if input.content.trim().is_empty() || input.content.len() > MAXIMUM_PASTED_CONTENT_BYTES {
            return Err(AppError::DataParsing(format!(
                "Pasted content must contain between 1 and {MAXIMUM_PASTED_CONTENT_BYTES} UTF-8 bytes"
            )));
        }
        let content_sha256 = sha256_hex(input.content.as_bytes());
        let source = local_source_registration(
            SourceKind::PastedContent,
            title.to_string(),
            &content_sha256,
        );
        self.repository
            .insert_or_update_portal_registration(&source)
            .await?;
        let request = synthetic_source_request(&source, "User-pasted content");
        self.repository
            .insert_or_update_request(&source.portal_key, &request)
            .await?;
        let extension = media_type_extension(input.declared_media_type.as_deref(), &input.content);
        let document_id = stable_source_identifier(&format!("pasted|{content_sha256}"));
        let managed_path =
            store_inline_payload(&self.download_root, document_id, input.content.as_bytes())
                .await?;
        let now = Utc::now().to_rfc3339();
        let document = DocumentMetadata {
            id: document_id,
            file_name: Some(format!(
                "{}.{}",
                sanitize_file_name(title, document_id),
                extension
            )),
            file_size: Some(input.content.len() as u64),
            description: Some("User-pasted content".to_string()),
            content_type: input.declared_media_type,
            download_url: None,
            created_at: now.clone(),
            updated_at: now,
            visibility: "public".to_string(),
            local_path: Some(managed_path.to_string_lossy().into_owned()),
            extracted_text_sha256: None,
            was_ocr_processed: false,
        };
        let output_directory = self
            .download_root
            .join(format!("{}_documents", source.portal_key));
        let sync_run_id = self
            .start_task_pool(
                source.clone(),
                RateLimitPolicy::default(),
                vec![IngestionTask::DownloadBinaryPayload {
                    subdomain: source.portal_key.clone(),
                    request_id: request.id,
                    document: Box::new(document),
                    output_directory: output_directory.to_string_lossy().into_owned(),
                    container_depth: 0,
                    container_root_directory: None,
                    crawl_depth: 0,
                    structured_depth: 0,
                }],
                CrawlPolicy::default(),
                SourceGuidance::default(),
                PoolSeedPolicy::OperatorRequested,
            )
            .await?;
        Ok(SourceIngestionStart {
            source,
            sync_run_id,
            detected_kind: SourceKind::PastedContent,
            used_saved_guidance: false,
        })
    }

    /// Halts a source and records the halt durably, so the next launch leaves it alone.
    ///
    /// The frontier is deliberately kept. It is the whole difference between a resume that
    /// continues and one that restarts from the first page.
    pub async fn pause_source(&self, source_key: &str) -> Result<bool, AppError> {
        let _provisioning_guard = self.provisioning_lock.lock().await;
        // Checked before anything is written, because pausing is a write to the same column
        // withdrawal uses, and an unguarded pause would put a withdrawn source back on the list.
        match self.repository.source_lifecycle_state(source_key).await? {
            None => {
                return Err(AppError::DataParsing(format!(
                    "No saved source is registered as '{source_key}'"
                )))
            }
            Some(SourceLifecycleState::Removed) => {
                return Err(AppError::DataParsing(format!(
                    "Source '{source_key}' has been removed and cannot be paused"
                )))
            }
            Some(SourceLifecycleState::Active | SourceLifecycleState::Paused) => {}
        }
        let stopped = self.stop_source_pool_locked(source_key).await?;
        self.repository
            .set_source_lifecycle_state(source_key, SourceLifecycleState::Paused)
            .await?;
        Ok(stopped)
    }

    /// Withdraws a source: stops it, discards the crawl frontier and the learned recipe, and
    /// marks the registration withdrawn.
    ///
    /// Documents already ingested are retained, and the count of them is returned so the caller
    /// can say so. The `portals` row is the parent of every request and document ingested from
    /// the source, so deleting it would be a corpus purge dressed up as a list operation.
    pub async fn remove_source(&self, source_key: &str) -> Result<SourceRemoval, AppError> {
        let _provisioning_guard = self.provisioning_lock.lock().await;
        if self
            .repository
            .source_lifecycle_state(source_key)
            .await?
            .is_none()
        {
            // Reporting zero discarded and zero retained for a key that was never registered
            // would read as a successful removal of an empty source.
            return Err(AppError::DataParsing(format!(
                "No saved source is registered as '{source_key}'"
            )));
        }
        // Workers are joined inside the stop before anything is deleted. A worker still running
        // could enqueue a link it just discovered and re-populate the frontier behind the delete.
        self.stop_source_pool_locked(source_key).await?;
        let discarded_frontier_tasks = self.repository.discard_source_frontier(source_key).await?;
        self.repository
            .delete_source_connector_recipe(source_key)
            .await?;
        let retained_documents = self.repository.count_source_documents(source_key).await?;
        self.repository
            .set_source_lifecycle_state(source_key, SourceLifecycleState::Removed)
            .await?;
        Ok(SourceRemoval {
            source_key: source_key.to_string(),
            discarded_frontier_tasks,
            retained_documents,
        })
    }

    /// Counts what [`Self::purge_source`] would destroy.
    ///
    /// Read-only, so it takes no provisioning lock and stops nothing. A crawl finishing between
    /// the preview and the purge makes the purge destroy more than the preview named; the purge
    /// reports what it actually destroyed, and that report is the authoritative one.
    pub async fn preview_source_purge(
        &self,
        source_key: &str,
    ) -> Result<SourcePurgePreview, AppError> {
        if self
            .repository
            .source_lifecycle_state(source_key)
            .await?
            .is_none()
        {
            return Err(AppError::DataParsing(format!(
                "No saved source is registered as '{source_key}'"
            )));
        }
        self.repository.preview_source_purge(source_key).await
    }

    /// Destroys a source and everything ingested under it, including the `portals` row.
    ///
    /// [`Self::remove_source`] withdraws a source and keeps its documents. This is the other
    /// operation, and the two are deliberately not one function with a flag: an operator who meant
    /// "stop crawling this" and an operator who meant "erase this" have opposite intentions, and a
    /// boolean parameter is a poor place for that distinction to live.
    ///
    /// Workers are joined before anything is deleted, for the same reason the withdrawal path does
    /// it: a worker still in flight can finalize a document behind the delete, which would leave
    /// rows referencing a `portals` row that no longer exists and make the next launch fail
    /// `verify_logical_relations`.
    ///
    /// The returned evidence paths are unlinked by the caller. The repository owns rows, not files.
    pub async fn purge_source(&self, source_key: &str) -> Result<SourcePurge, AppError> {
        let _provisioning_guard = self.provisioning_lock.lock().await;
        if self
            .repository
            .source_lifecycle_state(source_key)
            .await?
            .is_none()
        {
            return Err(AppError::DataParsing(format!(
                "No saved source is registered as '{source_key}'"
            )));
        }
        self.stop_source_pool_locked(source_key).await?;
        self.repository.purge_source(source_key).await
    }

    /// Destroys one ingested document and everything derived from it.
    ///
    /// Takes the provisioning lock and stops nothing else: a purge is rare and a running crawl for
    /// the same source could otherwise re-finalize the document between the delete and the commit.
    pub async fn purge_document(
        &self,
        source_key: &str,
        document_id: u64,
    ) -> Result<DocumentPurge, AppError> {
        let _provisioning_guard = self.provisioning_lock.lock().await;
        self.repository
            .purge_document(source_key, document_id)
            .await
    }

    /// Tears down a running pool and finalizes its run. Returns `false` when the source was not
    /// running, which is not an error: a saved source can be paused or withdrawn while idle.
    ///
    /// The caller must already hold `provisioning_lock`. It is a plain `tokio::sync::Mutex` and
    /// re-entering it deadlocks rather than erroring.
    async fn stop_source_pool_locked(&self, portal_key: &str) -> Result<bool, AppError> {
        let pool = self.pools.write().await.remove(portal_key);
        let Some(pool) = pool else {
            return Ok(false);
        };
        self.slot_freed.send_modify(|generation| {
            *generation = generation.wrapping_add(1);
        });
        pool.statistics
            .stop_requested
            .store(true, Ordering::Release);
        pool.statistics.running.store(false, Ordering::Release);
        let worker_result = cancel_and_join_workers(&pool.cancellation, pool.worker_handles).await;
        let finalization = self
            .repository
            .finish_portal_sync_run(PortalSyncRunFinalization {
                run_id: &pool.sync_run_id,
                portal_key,
                status: "CANCELLED",
                pages_processed: pool.statistics.pages_processed.load(Ordering::Acquire),
                requests_processed: pool.statistics.requests_processed.load(Ordering::Acquire),
                documents_processed: pool.statistics.documents_processed.load(Ordering::Acquire),
                failures: pool.statistics.failed_tasks.load(Ordering::Acquire),
            })
            .await;
        self.limiter.unregister_portal(portal_key).await;
        finalization?;
        worker_result?;
        let _ = self
            .progress_sender
            .send(IngestionProgress::SyncStopped {
                portal_key: portal_key.to_string(),
                sync_run_id: pool.sync_run_id,
            })
            .await;
        Ok(true)
    }

    pub async fn runtime_snapshots(&self) -> Result<Vec<PortalRuntimeSnapshot>, AppError> {
        let pools = {
            let pools = self.pools.read().await;
            pools
                .values()
                .map(|pool| {
                    (
                        pool.portal.clone(),
                        pool.sync_run_id.clone(),
                        Arc::clone(&pool.statistics),
                        Arc::clone(&pool.crawl),
                    )
                })
                .collect::<Vec<_>>()
        };
        let mut snapshots = Vec::with_capacity(pools.len());
        for (portal, sync_run_id, statistics, crawl) in pools {
            let frontier_exclusions = crawl
                .frontier_exclusions
                .lock()
                .await
                .iter()
                .map(|(rule, count)| FrontierExclusionCount {
                    rule: rule.clone(),
                    excluded_links: *count,
                })
                .collect();
            snapshots.push(PortalRuntimeSnapshot {
                rate_limit_tokens: self.limiter.available_tokens(&portal.portal_key).await?,
                portal,
                sync_run_id,
                queued_tasks: statistics.queued_tasks.load(Ordering::Acquire),
                active_tasks: statistics.active_tasks.load(Ordering::Acquire),
                completed_tasks: statistics.completed_tasks.load(Ordering::Acquire),
                failed_tasks: statistics.failed_tasks.load(Ordering::Acquire),
                pages_processed: statistics.pages_processed.load(Ordering::Acquire),
                requests_processed: statistics.requests_processed.load(Ordering::Acquire),
                documents_processed: statistics.documents_processed.load(Ordering::Acquire),
                is_running: statistics.running.load(Ordering::Acquire),
                frontier_exclusions,
                stopped_hosts: self
                    .client
                    .host_governor()
                    .snapshots()
                    .into_iter()
                    .filter_map(|host| {
                        host.stopped.map(|reason| StoppedHost {
                            host: host.host,
                            requests_spent: host.requests_spent,
                            reason: reason.describe(),
                        })
                    })
                    .collect(),
            });
        }
        snapshots.sort_by(|left, right| left.portal.portal_key.cmp(&right.portal.portal_key));
        Ok(snapshots)
    }

    pub async fn is_source_running(&self, source_key: &str) -> bool {
        self.pools.read().await.contains_key(source_key)
    }
}

async fn worker_loop(
    context: WorkerContext,
    receiver: Arc<Mutex<mpsc::Receiver<ScheduledTask>>>,
    mut cancellation: watch::Receiver<bool>,
) {
    loop {
        let task = tokio::select! {
            changed = cancellation.changed() => {
                if changed.is_err() || *cancellation.borrow() {
                    break;
                }
                continue;
            }
            task = receive_task(&receiver) => task,
        };
        let Some(scheduled) = task else {
            break;
        };
        decrement_atomic(&context.statistics.queued_tasks);
        context
            .statistics
            .active_tasks
            .fetch_add(1, Ordering::AcqRel);
        let target_id = scheduled.task.target_id();
        let target_url = scheduled.task.target_url().map(str::to_string);
        let frontier_id = scheduled.frontier_id;
        let frontier_attempt = scheduled.frontier_attempt;
        let mut result = execute_task(&context, scheduled.task).await;
        let retry_delay =
            frontier_id
                .zip(result.as_ref().err())
                .and_then(|(frontier_id, error)| {
                    durable_retry_delay_for_error(
                        error,
                        frontier_attempt,
                        frontier_id.unsigned_abs(),
                    )
                });
        if let (Some(frontier_id), Some(delay_seconds)) = (frontier_id, retry_delay) {
            // Counted back in before the deferral commits, for the reason `enqueue_tasks_resilient`
            // gives: a refill may claim the row the moment it is available again.
            context
                .statistics
                .queued_tasks
                .fetch_add(1, Ordering::AcqRel);
            match context
                .repository
                .defer_ingestion_frontier_task(frontier_id, delay_seconds)
                .await
            {
                Ok(()) => {
                    let error_msg = result
                        .as_ref()
                        .err()
                        .map(ToString::to_string)
                        .unwrap_or_default();
                    let _ = context
                        .progress_sender
                        .send(IngestionProgress::TaskRetryScheduled {
                            portal_key: context.portal.portal_key.clone(),
                            target_id,
                            attempt: frontier_attempt,
                            delay_seconds,
                            error_msg,
                        })
                        .await;
                    decrement_atomic(&context.statistics.active_tasks);
                    if *context.cancellation.borrow() {
                        break;
                    }
                    // This wake is the only thing that re-dispatches a deferred row: workers
                    // idle in `receive_task`, and nothing ticks. A refill that fails once must
                    // therefore be retried and reported, or the pool holds `queued_tasks` above
                    // zero forever, never retires, and nobody is told why.
                    let retry_context = context.clone();
                    tokio::spawn(async move {
                        for _ in 0..MAXIMUM_DURABLE_TASK_ATTEMPTS {
                            tokio::time::sleep(std::time::Duration::from_secs(
                                delay_seconds.saturating_add(1),
                            ))
                            .await;
                            if *retry_context.cancellation.borrow() {
                                return;
                            }
                            match refill_ingestion_frontier(&retry_context).await {
                                Ok(()) => return,
                                Err(error) => {
                                    // A stop that landed mid-refill closes the channel; that is
                                    // the stop's outcome, not a failure of this pool.
                                    if *retry_context.cancellation.borrow() {
                                        return;
                                    }
                                    send_progress(
                                        &retry_context,
                                        IngestionProgress::TaskFailed {
                                            portal_key: retry_context.portal.portal_key.clone(),
                                            target_id: format!(
                                                "{}-frontier-refill",
                                                retry_context.sync_run_id
                                            ),
                                            error_msg: error.to_string(),
                                        },
                                    )
                                    .await;
                                }
                            }
                        }
                    });
                    retire_portal_if_idle(&context).await;
                    continue;
                }
                Err(error) => {
                    decrement_atomic(&context.statistics.queued_tasks);
                    result = Err(error);
                }
            }
        }
        // A terminal result leaves the retry frontier even when it failed; the failure is retained
        // in sync telemetry/statistics instead of making every future launch replay it forever.
        if let Some(frontier_id) = frontier_id {
            if let Err(error) = context
                .repository
                .complete_ingestion_frontier_task(frontier_id)
                .await
            {
                if result.is_ok() {
                    result = Err(error);
                }
            }
        }
        decrement_atomic(&context.statistics.active_tasks);
        match result {
            Ok(()) => {
                context
                    .statistics
                    .completed_tasks
                    .fetch_add(1, Ordering::AcqRel);
            }
            Err(error) => {
                context
                    .statistics
                    .failed_tasks
                    .fetch_add(1, Ordering::AcqRel);
                if let Ok(mut last_failure) = context.statistics.last_failure.lock() {
                    *last_failure = Some(error.to_string());
                }
                let _ = context
                    .repository
                    .record_source_task_failure(
                        &context.sync_run_id,
                        &context.portal.portal_key,
                        &target_id,
                        &error,
                    )
                    .await;
                if let Some(target_url) = target_url.as_deref() {
                    announce_host_stop(&context, target_url).await;
                }
                let _ = context
                    .progress_sender
                    .send(IngestionProgress::TaskFailed {
                        portal_key: context.portal.portal_key.clone(),
                        target_id,
                        error_msg: error.to_string(),
                    })
                    .await;
            }
        }
        if *context.cancellation.borrow() {
            break;
        }
        if let Err(error) = refill_ingestion_frontier(&context).await {
            context
                .statistics
                .failed_tasks
                .fetch_add(1, Ordering::AcqRel);
            let _ = context
                .progress_sender
                .send(IngestionProgress::TaskFailed {
                    portal_key: context.portal.portal_key.clone(),
                    target_id: format!("{}-frontier-refill", context.sync_run_id),
                    error_msg: error.to_string(),
                })
                .await;
        }
        retire_portal_if_idle(&context).await;
    }
    if context
        .statistics
        .live_workers
        .fetch_sub(1, Ordering::AcqRel)
        == 1
        && !context.statistics.stop_requested.load(Ordering::Acquire)
    {
        send_progress(
            &context,
            IngestionProgress::SyncStopped {
                portal_key: context.portal.portal_key.clone(),
                sync_run_id: context.sync_run_id.clone(),
            },
        )
        .await;
    }
}

fn durable_retry_delay_seconds(attempt: u64) -> u64 {
    backoff::exponential_delay(
        attempt.saturating_sub(1),
        Duration::from_secs(1),
        Duration::from_secs(30),
    )
    .as_secs()
}

fn durable_retry_delay_for_error(error: &AppError, attempt: u64, jitter_seed: u64) -> Option<u64> {
    (error.is_retryable() && attempt < MAXIMUM_DURABLE_TASK_ATTEMPTS).then(|| {
        let base = durable_retry_delay_seconds(attempt);
        // Bounded positive jitter keeps the exponential floor while distributing frontier rows
        // that fail together. The deterministic seed makes a persisted task's schedule explainable
        // and stable across restart; different frontier identifiers still spread over the window.
        let mixed = jitter_seed
            .wrapping_add(attempt.wrapping_mul(0x9e37_79b9_7f4a_7c15))
            .wrapping_mul(0xbf58_476d_1ce4_e5b9);
        let jitter_window = (base / 2).saturating_add(1);
        base.saturating_add(mixed % jitter_window)
    })
}

/// Signals cancellation, then waits for the workers. Workers leave their loop only on that signal
/// (the dispatcher sender they share keeps the task channel open), so a join that precedes the
/// signal waits forever, and every caller holds `provisioning_lock` while it waits.
async fn cancel_and_join_workers(
    cancellation: &watch::Sender<bool>,
    worker_handles: Vec<tokio::task::JoinHandle<()>>,
) -> Result<(), AppError> {
    let _ = cancellation.send(true);
    join_workers(worker_handles).await
}

/// Waits for workers to observe the cancellation signal at a task boundary. Aborting their async
/// handles would detach any `spawn_blocking` parser already in flight, allowing it to keep writing
/// after the source run had been finalized or withdrawn.
async fn join_workers(worker_handles: Vec<tokio::task::JoinHandle<()>>) -> Result<(), AppError> {
    for handle in worker_handles {
        if let Err(error) = handle.await {
            return Err(AppError::Internal(format!(
                "Ingestion worker failed while stopping: {error}"
            )));
        }
    }
    Ok(())
}

async fn retire_portal_if_idle(context: &WorkerContext) {
    // A pause/removal holds this lock while joining the workers. A worker that
    // finishes its current task must observe cancellation instead of waiting on
    // the lock held by its own joiner. Do not abort the task/parser: all task
    // writes have already completed before this retirement boundary.
    let mut cancellation = context.cancellation.subscribe();
    if *cancellation.borrow() {
        return;
    }
    let _provisioning_guard = tokio::select! {
        biased;
        _ = cancellation.changed() => return,
        guard = context.provisioning_lock.lock() => guard,
    };
    if *cancellation.borrow() {
        return;
    }
    if context.statistics.queued_tasks.load(Ordering::Acquire) != 0
        || context.statistics.active_tasks.load(Ordering::Acquire) != 0
    {
        return;
    }
    let removed = {
        let mut pools = context.pools.write().await;
        let is_current_pool = pools
            .get(&context.portal.portal_key)
            .is_some_and(|pool| Arc::ptr_eq(&pool.statistics, &context.statistics));
        is_current_pool
            .then(|| pools.remove(&context.portal.portal_key))
            .flatten()
    };
    if removed.is_none() {
        return;
    }
    context.slot_freed.send_modify(|generation| {
        *generation = generation.wrapping_add(1);
    });
    context.statistics.running.store(false, Ordering::Release);
    let _ = context.cancellation.send(true);
    let failures = context.statistics.failed_tasks.load(Ordering::Acquire);
    let documents_processed = context
        .statistics
        .documents_processed
        .load(Ordering::Acquire);
    let data_documents_processed = context
        .statistics
        .data_documents_processed
        .load(Ordering::Acquire);
    let terminal_status = if failures == 0 {
        "COMPLETED"
    } else {
        "COMPLETED_WITH_ERRORS"
    };
    if let Err(error) = context
        .repository
        .finish_portal_sync_run(PortalSyncRunFinalization {
            run_id: &context.sync_run_id,
            portal_key: &context.portal.portal_key,
            status: terminal_status,
            pages_processed: context.statistics.pages_processed.load(Ordering::Acquire),
            requests_processed: context
                .statistics
                .requests_processed
                .load(Ordering::Acquire),
            documents_processed,
            failures,
        })
        .await
    {
        send_progress(
            context,
            IngestionProgress::TaskFailed {
                portal_key: context.portal.portal_key.clone(),
                target_id: context.sync_run_id.clone(),
                error_msg: error.to_string(),
            },
        )
        .await;
    }
    let specialized_source_without_data = matches!(
        context.portal.source_kind,
        SourceKind::OpenDataCatalog
            | SourceKind::ArcGis
            | SourceKind::Tableau
            | SourceKind::GoogleDrive
    ) && data_documents_processed == 0;
    if context.portal.source_kind != SourceKind::NextRequest
        && (documents_processed == 0 || specialized_source_without_data)
    {
        let reason = context
            .statistics
            .last_failure
            .lock()
            .ok()
            .and_then(|value| value.clone())
            .unwrap_or_else(|| {
                "The source was reachable, but the completed crawl produced no usable documents."
                    .to_string()
            });
        send_progress(
            context,
            IngestionProgress::SourceNeedsGuidance {
                portal_key: context.portal.portal_key.clone(),
                source_url: context.portal.canonical_url.clone(),
                reason,
            },
        )
        .await;
    }
    context
        .limiter
        .unregister_portal(&context.portal.portal_key)
        .await;
}

async fn receive_task(
    receiver: &Arc<Mutex<mpsc::Receiver<ScheduledTask>>>,
) -> Option<ScheduledTask> {
    receiver.lock().await.recv().await
}

async fn execute_task(context: &WorkerContext, task: IngestionTask) -> Result<(), AppError> {
    if task.portal_key() != context.portal.portal_key {
        return Err(AppError::PortalSecurity(format!(
            "Task for portal '{}' reached isolated pool '{}'",
            task.portal_key(),
            context.portal.portal_key
        )));
    }
    let _permit =
        context.global_semaphore.acquire().await.map_err(|error| {
            AppError::Internal(format!("Global queue semaphore closed: {error}"))
        })?;
    match task {
        IngestionTask::FetchMetadataPage { page, per_page, .. } => {
            fetch_metadata_page(context, page, per_page).await
        }
        IngestionTask::DownloadBinaryPayload {
            request_id,
            document,
            output_directory,
            container_depth,
            container_root_directory,
            crawl_depth,
            structured_depth,
            ..
        } => {
            process_binary_payload(
                context,
                request_id,
                *document,
                PathBuf::from(output_directory),
                PayloadDepth {
                    container: container_depth,
                    crawl: crawl_depth,
                    structured: structured_depth,
                },
                container_root_directory.map(PathBuf::from),
            )
            .await
        }
        IngestionTask::FetchDataCatalogPage {
            endpoint, cursor, ..
        } => process_data_catalog_page(context, endpoint, cursor).await,
        IngestionTask::CrawlPublicResource { url, depth, .. } => {
            process_public_resource(context, url, depth).await
        }
        IngestionTask::CrawlWordPressGoogleDrive {
            connector, depth, ..
        } => process_wordpress_google_drive(context, connector, depth).await,
        IngestionTask::CrawlArcGisLayer {
            layer_url,
            offset,
            crawl_depth,
            structured_depth,
            ..
        } => process_arcgis_layer(context, layer_url, offset, crawl_depth, structured_depth).await,
        IngestionTask::ImportLocalPayload {
            request_id,
            mut document,
            source_path,
            output_directory,
            ..
        } => {
            let (managed_path, byte_count) =
                import_local_payload(&context.download_root, Path::new(&source_path), document.id)
                    .await?;
            document.local_path = Some(managed_path.to_string_lossy().into_owned());
            document.file_size = Some(byte_count);
            process_binary_payload(
                context,
                request_id,
                *document,
                PathBuf::from(output_directory),
                PayloadDepth::discovered(0, 0),
                None,
            )
            .await
        }
    }
}

async fn process_public_resource(
    context: &WorkerContext,
    url: String,
    depth: usize,
) -> Result<(), AppError> {
    if !reserve_crawl_page(context, depth).await? {
        return Ok(());
    }
    context
        .repository
        .check_retained_file_budget(&context.download_root)
        .await?;
    let inspection = context
        .client
        .inspect_public_resource_with_selectors(
            &url,
            context.crawl.guidance.link_selector.as_deref(),
            context.crawl.guidance.download_selector.as_deref(),
        )
        .await?;
    match inspection.kind {
        PublicResourceKind::Download => {
            enqueue_discovered_download(
                context,
                &inspection,
                None,
                &url,
                PayloadDepth::discovered(depth, 0),
            )
            .await?;
        }
        PublicResourceKind::HtmlPage => {
            // A disappearance lead is created only after a successfully retrieved listing.
            // Failed requests never imply that a publisher removed a record.
            if let Some(body) = inspection.body.as_deref() {
                context
                    .repository
                    .record_publication_listing(
                        inspection.final_url.clone(),
                        sha256_hex(body),
                        super::client::publication_listing_links(&inspection.final_url, body)?,
                    )
                    .await?;
            }
            let request_id = ensure_crawl_request(context, &inspection).await?;
            let mut discovered_tasks = Vec::new();
            for companion_url in arcgis_item_companion_urls(&inspection.final_url) {
                if let Some(task) =
                    prepare_crawl_url(context, companion_url, depth.saturating_add(1), 0).await
                {
                    discovered_tasks.push(task);
                }
            }
            for companion_url in tableau_export_companion_urls(&inspection.final_url) {
                if let Some(task) =
                    prepare_crawl_url(context, companion_url, depth.saturating_add(1), 0).await
                {
                    discovered_tasks.push(task);
                }
            }
            if let Some(body) = inspection.body.as_deref() {
                for companion_url in arcgis_hub_companion_urls(&inspection.final_url, body) {
                    if let Some(task) =
                        prepare_crawl_url(context, companion_url, depth.saturating_add(1), 0).await
                    {
                        discovered_tasks.push(task);
                    }
                }
                if let Some(task) = prepare_html_page_document(
                    context,
                    &inspection,
                    request_id,
                    body,
                    PayloadDepth::discovered(depth, 0),
                )
                .await?
                {
                    discovered_tasks.push(task);
                }
            }
            for connector in inspection.embedded_connectors.iter().cloned() {
                match connector {
                    PublicEmbeddedConnector::WordPressGoogleDrive(connector) => {
                        if let Some(task) = prepare_wordpress_google_drive(
                            context,
                            connector,
                            depth.saturating_add(1),
                        )
                        .await
                        {
                            discovered_tasks.push(task);
                        }
                    }
                }
            }
            for link in inspection.links {
                if let Err(exclusion) = guidance_allows_url(
                    &context.crawl.guidance,
                    &context.crawl.compiled_guidance,
                    &link.url,
                    None,
                ) {
                    record_frontier_exclusion(context, &exclusion).await;
                    continue;
                }
                for companion_url in arcgis_item_companion_urls(&link.url) {
                    if let Some(task) =
                        prepare_crawl_url(context, companion_url, depth.saturating_add(1), 0).await
                    {
                        discovered_tasks.push(task);
                    }
                }
                for companion_url in tableau_export_companion_urls(&link.url) {
                    if let Some(task) =
                        prepare_crawl_url(context, companion_url, depth.saturating_add(1), 0).await
                    {
                        discovered_tasks.push(task);
                    }
                }
                if link.download_candidate {
                    if let Some(task) = prepare_discovered_download(
                        context,
                        &PublicResourceInspection {
                            final_url: link.url,
                            content_type: None,
                            title: link.label,
                            kind: PublicResourceKind::Download,
                            links: Vec::new(),
                            body: None,
                            embedded_connectors: Vec::new(),
                            content_length: None,
                            acquisition: None,
                        },
                        Some(request_id),
                        &inspection.final_url,
                        PayloadDepth::discovered(depth.saturating_add(1), 0),
                    )
                    .await?
                    {
                        discovered_tasks.push(task);
                    }
                } else if link.follow_candidate {
                    match admit_crawl_link(context, &link, depth).await {
                        Ok(()) => {
                            if let Some(task) = prepare_crawl_url(
                                context,
                                link.url.clone(),
                                depth.saturating_add(1),
                                0,
                            )
                            .await
                            {
                                discovered_tasks.push(task);
                            }
                        }
                        Err(exclusion) => record_frontier_exclusion(context, &exclusion).await,
                    }
                }
            }
            enqueue_tasks_resilient(context, discovered_tasks).await?;
        }
    }
    context
        .statistics
        .pages_processed
        .fetch_add(1, Ordering::AcqRel);
    Ok(())
}

async fn process_wordpress_google_drive(
    context: &WorkerContext,
    connector: WordPressGoogleDriveConnector,
    depth: usize,
) -> Result<(), AppError> {
    if !reserve_crawl_page(context, depth).await? {
        return Ok(());
    }
    let items = context
        .client
        .list_wordpress_google_drive_folder(&connector)
        .await?;
    let request_id = stable_source_identifier(&format!(
        "google-drive-folder|{}|{}",
        connector.endpoint, connector.folder_id
    ));
    persist_crawl_request(
        context,
        request_id,
        format!("drive-{request_id}"),
        connector.folder_name.clone(),
        format!(
            "Public Google Drive folder discovered through {}",
            context.portal.canonical_url
        ),
    )
    .await?;

    let folder_origin = format!(
        "{} Google Drive folder {}",
        connector.endpoint, connector.folder_id
    );
    let mut discovered_tasks = Vec::with_capacity(items.len());
    for item in items {
        if item.is_folder {
            if depth < context.crawl.policy.maximum_depth {
                let mut child = connector.clone();
                child.folder_id = item.id;
                child.account_id = item.account_id;
                child.folder_name = item.name;
                if let Some(task) =
                    prepare_wordpress_google_drive(context, child, depth.saturating_add(1)).await
                {
                    discovered_tasks.push(task);
                }
            }
            continue;
        }
        let download_url = ApiClient::wordpress_google_drive_download_url(&connector, &item)?;
        if let Some(task) = prepare_discovered_download(
            context,
            &PublicResourceInspection {
                final_url: download_url,
                content_type: Some(item.media_type),
                title: item.name,
                kind: PublicResourceKind::Download,
                links: Vec::new(),
                body: None,
                embedded_connectors: Vec::new(),
                content_length: None,
                acquisition: None,
            },
            Some(request_id),
            &folder_origin,
            PayloadDepth::discovered(depth.saturating_add(1), 0),
        )
        .await?
        {
            discovered_tasks.push(task);
        }
    }
    enqueue_tasks_resilient(context, discovered_tasks).await?;
    context
        .statistics
        .pages_processed
        .fetch_add(1, Ordering::AcqRel);
    Ok(())
}

/// Reads one page of a platform catalog index and schedules the files it names.
///
/// This is the path that replaces crawling on a detected catalog. It never follows links: an index
/// page names assets, and every asset it names is either scheduled for download or counted as one
/// the index could not resolve to a file. Nothing in between is possible, which is what makes the
/// resulting corpus accountable to the catalog's own asset count.
async fn process_data_catalog_page(
    context: &WorkerContext,
    endpoint: DataCatalogEndpoint,
    cursor: Option<String>,
) -> Result<(), AppError> {
    if !reserve_paginated_page(context).await? {
        return Ok(());
    }
    let page = context
        .client
        .fetch_data_catalog_page(&endpoint, cursor.as_deref())
        .await?;
    let request_id = stable_source_identifier(&format!("catalog|{}", endpoint.api_url));
    persist_crawl_request(
        context,
        request_id,
        format!("catalog-{request_id}"),
        format!(
            "{} catalog at {}",
            endpoint.platform.as_str(),
            endpoint.domain
        ),
        match page.asset_count {
            Some(count) => format!(
                "{} index reporting {count} assets",
                endpoint.platform.as_str()
            ),
            None => format!(
                "{} index reporting no asset total",
                endpoint.platform.as_str()
            ),
        },
    )
    .await?;
    let mut tasks = Vec::new();
    for download in &page.downloads {
        if let Err(exclusion) = guidance_allows_url(
            &context.crawl.guidance,
            &context.crawl.compiled_guidance,
            &download.url,
            download.byte_count,
        ) {
            record_frontier_exclusion(context, &exclusion).await;
            continue;
        }
        let inspection = PublicResourceInspection {
            final_url: download.url.clone(),
            content_type: download.media_type.clone(),
            title: download.label.clone(),
            kind: PublicResourceKind::Download,
            links: Vec::new(),
            body: None,
            embedded_connectors: Vec::new(),
            content_length: download.byte_count,
            acquisition: None,
        };
        if let Some(task) = prepare_discovered_download(
            context,
            &inspection,
            Some(request_id),
            &endpoint.api_url,
            // A catalog asset is one structured hop from the index and occupies no crawl depth,
            // because paging an index is not walking a site graph.
            PayloadDepth::discovered(0, 1),
        )
        .await?
        {
            tasks.push(task);
        }
    }
    // An asset the index could not resolve to a file is a hole in coverage, so it is counted under
    // the reason it could not be resolved rather than dropped.
    for (reason, count) in &page.assets_without_downloads {
        *context
            .crawl
            .frontier_exclusions
            .lock()
            .await
            .entry(reason.clone())
            .or_insert(0) += *count;
    }
    if page.foreign_domain_assets > 0 {
        *context
            .crawl
            .frontier_exclusions
            .lock()
            .await
            .entry("catalog-asset-outside-portal-domain".to_string())
            .or_insert(0) += page.foreign_domain_assets;
    }
    if let Some(next_cursor) = page.next_cursor {
        tasks.push(IngestionTask::FetchDataCatalogPage {
            subdomain: context.portal.portal_key.clone(),
            endpoint: endpoint.clone(),
            cursor: Some(next_cursor),
        });
    }
    enqueue_tasks_resilient(context, tasks).await?;
    let _ = context
        .progress_sender
        .send(IngestionProgress::PageCompleted {
            portal_key: context.portal.portal_key.clone(),
            page: context.crawl.pages_started.load(Ordering::Acquire),
            records_found: page.downloads.len(),
        })
        .await;
    context
        .statistics
        .pages_processed
        .fetch_add(1, Ordering::AcqRel);
    Ok(())
}

async fn process_arcgis_layer(
    context: &WorkerContext,
    layer_url: String,
    offset: usize,
    crawl_depth: usize,
    structured_depth: usize,
) -> Result<(), AppError> {
    if !reserve_paginated_page(context).await? {
        return Ok(());
    }
    let page = context
        .client
        .fetch_arcgis_feature_page(&layer_url, offset, ARCGIS_FEATURE_PAGE_SIZE)
        .await?;
    if page.feature_count == 0 {
        if page.exceeded_transfer_limit {
            return Err(AppError::DataParsing(
                "ArcGIS reported more features but returned an empty page; coverage is incomplete."
                    .into(),
            ));
        }
        context
            .statistics
            .pages_processed
            .fetch_add(1, Ordering::AcqRel);
        return Ok(());
    }
    let page_hash = sha256_hex(&page.bytes);
    let page_identity = format!("{layer_url}|{page_hash}");
    if !context
        .crawl
        .arcgis_page_hashes
        .lock()
        .await
        .insert(page_identity)
    {
        return Err(AppError::DataParsing(
            "ArcGIS repeated a feature page without advancing; coverage is incomplete.".into(),
        ));
    }
    if !reserve_resource(context, &page.request_url).await? {
        return Ok(());
    }
    let request_id = stable_source_identifier(&format!("arcgis-layer|{layer_url}"));
    persist_crawl_request(
        context,
        request_id,
        format!("arcgis-{request_id}"),
        format!("ArcGIS layer {layer_url}"),
        "Paginated ArcGIS feature layer export".to_string(),
    )
    .await?;
    let document_id = stable_source_identifier(&format!("arcgis-page|{layer_url}|{offset}"));
    context
        .repository
        .check_retained_file_budget(&context.download_root)
        .await?;
    let managed_path =
        store_inline_payload(&context.download_root, document_id, &page.bytes).await?;
    let now = Utc::now().to_rfc3339();
    let document = DocumentMetadata {
        id: document_id,
        file_name: Some(format!("arcgis-{request_id}-{offset}.geojson")),
        file_size: Some(page.bytes.len() as u64),
        description: Some(format!(
            "ArcGIS features {} through {} from {layer_url}",
            offset,
            offset.saturating_add(page.feature_count)
        )),
        content_type: Some("application/geo+json".to_string()),
        download_url: Some(page.request_url),
        created_at: now.clone(),
        updated_at: now,
        visibility: "public".to_string(),
        local_path: Some(managed_path.to_string_lossy().into_owned()),
        extracted_text_sha256: None,
        was_ocr_processed: false,
    };
    record_inline_acquisition(
        context,
        request_id,
        &document,
        &page.bytes,
        &page.acquisition,
    )
    .await?;
    let mut tasks = vec![standard_document_task(
        context,
        request_id,
        document,
        PayloadDepth::discovered(crawl_depth, structured_depth),
    )];
    if page.exceeded_transfer_limit || page.feature_count == ARCGIS_FEATURE_PAGE_SIZE {
        let next_offset = offset.checked_add(page.feature_count).ok_or_else(|| {
            AppError::PortalCapacity("ArcGIS feature offset overflowed".to_string())
        })?;
        if next_offset <= 100_000_000 {
            tasks.push(IngestionTask::CrawlArcGisLayer {
                subdomain: context.portal.portal_key.clone(),
                layer_url,
                offset: next_offset,
                crawl_depth,
                structured_depth,
            });
        }
    }
    enqueue_tasks_resilient(context, tasks).await?;
    context
        .statistics
        .pages_processed
        .fetch_add(1, Ordering::AcqRel);
    Ok(())
}

async fn prepare_wordpress_google_drive(
    context: &WorkerContext,
    connector: WordPressGoogleDriveConnector,
    depth: usize,
) -> Option<IngestionTask> {
    let identity = format!(
        "wordpress-google-drive|{}|{}",
        connector.endpoint, connector.folder_id
    );
    if !mark_crawl_url(context, &identity).await {
        return None;
    }
    Some(IngestionTask::CrawlWordPressGoogleDrive {
        subdomain: context.portal.portal_key.clone(),
        connector,
        depth,
    })
}

async fn prepare_html_page_document(
    context: &WorkerContext,
    inspection: &PublicResourceInspection,
    request_id: u64,
    body: &[u8],
    depth: PayloadDepth,
) -> Result<Option<IngestionTask>, AppError> {
    if !reserve_resource(context, &inspection.final_url).await? {
        return Ok(None);
    }
    let document_id = stable_source_identifier(&format!("page|{}", inspection.final_url));
    let managed_path = store_inline_payload(&context.download_root, document_id, body).await?;
    let mut file_name = source_file_name(&inspection.final_url, document_id);
    let lower_file_name = file_name.to_ascii_lowercase();
    if ![".html", ".htm", ".xhtml"]
        .iter()
        .any(|extension| lower_file_name.ends_with(extension))
    {
        file_name.push_str(".html");
    }
    let now = Utc::now().to_rfc3339();
    let document = DocumentMetadata {
        id: document_id,
        file_name: Some(file_name),
        file_size: Some(body.len() as u64),
        description: Some(format!("Crawled HTML page: {}", inspection.final_url)),
        // The server's declared type, or none. The `.html` file name already routes extraction;
        // asserting `text/html` for a body the server never described would be a claim the
        // record does not support, and the stored type gates PDF-only detectors downstream.
        content_type: inspection.content_type.clone(),
        download_url: Some(inspection.final_url.clone()),
        created_at: now.clone(),
        updated_at: now,
        visibility: "public".to_string(),
        local_path: Some(managed_path.to_string_lossy().into_owned()),
        extracted_text_sha256: None,
        was_ocr_processed: false,
    };
    if let Some(acquisition) = &inspection.acquisition {
        record_inline_acquisition(context, request_id, &document, body, acquisition).await?;
    }
    Ok(Some(standard_document_task(
        context, request_id, document, depth,
    )))
}

/// A page fetched and stored inline is already an acquisition before its document task runs, and
/// that task reads the stored bytes as cached. The row is written first so the event always has a
/// document to belong to; the task's own write of the same metadata is an idempotent upsert.
async fn record_inline_acquisition(
    context: &WorkerContext,
    request_id: u64,
    document: &DocumentMetadata,
    bytes: &[u8],
    acquisition: &AcquisitionRecord,
) -> Result<(), AppError> {
    context
        .repository
        .insert_or_update_document(&context.portal.portal_key, request_id, document)
        .await?;
    if let Some(path) = &document.local_path {
        release_object_link(Path::new(path)).await;
    }
    context
        .repository
        .record_document_acquisition(
            &context.portal.portal_key,
            document.id,
            Some(&context.sync_run_id),
            &sha256_hex(bytes),
            bytes.len() as u64,
            acquisition,
        )
        .await
}

async fn ensure_crawl_request(
    context: &WorkerContext,
    inspection: &PublicResourceInspection,
) -> Result<u64, AppError> {
    let request_id = stable_source_identifier(&inspection.final_url);
    persist_crawl_request(
        context,
        request_id,
        format!("web-{request_id}"),
        inspection.title.clone(),
        format!("Crawled public source page: {}", inspection.final_url),
    )
    .await?;
    Ok(request_id)
}

async fn persist_crawl_request(
    context: &WorkerContext,
    request_id: u64,
    rpid: String,
    title: String,
    description: String,
) -> Result<(), AppError> {
    let now = Utc::now().to_rfc3339();
    let mut request = PublicRequest {
        id: request_id,
        rpid: Some(rpid),
        title: Some(title),
        description: Some(description),
        status: "published".to_string(),
        current_status_id: None,
        created_at: now.clone(),
        updated_at: now,
        closed_at: None,
        due_date: None,
        determination_at: None,
        determination_source: None,
        neighborhood_block: None,
        statutory_extension: None,
        documents: Vec::new(),
        agency: None,
        category: None,
        timeline_anomalies: Vec::new(),
    };
    request.compute_timeline_anomalies()?;
    context
        .repository
        .insert_or_update_request(&context.portal.portal_key, &request)
        .await?;
    context
        .statistics
        .requests_processed
        .fetch_add(1, Ordering::AcqRel);
    Ok(())
}

async fn enqueue_discovered_download(
    context: &WorkerContext,
    inspection: &PublicResourceInspection,
    parent_request_id: Option<u64>,
    discovered_from: &str,
    depth: PayloadDepth,
) -> Result<(), AppError> {
    let Some(task) = prepare_discovered_download(
        context,
        inspection,
        parent_request_id,
        discovered_from,
        depth,
    )
    .await?
    else {
        return Ok(());
    };
    enqueue_task_resilient(context, task).await
}

async fn prepare_discovered_download(
    context: &WorkerContext,
    inspection: &PublicResourceInspection,
    parent_request_id: Option<u64>,
    discovered_from: &str,
    depth: PayloadDepth,
) -> Result<Option<IngestionTask>, AppError> {
    // Before `reserve_resource`, so a refused host does not spend a resource slot, and reported
    // rather than dropped: a crawl that silently declines documents claims coverage it does not
    // have. This is the only place a remote URL becomes a fetch task -- the other
    // `DownloadBinaryPayload` births carry bytes already on disk (`local_path`) or come from the
    // source's own metadata API.
    if !download_host_allowed(
        &context.crawl.guidance,
        &context.portal.canonical_url,
        &inspection.final_url,
    ) {
        record_frontier_exclusion(context, &FrontierExclusion::OffSourceHostDownload).await;
        return Ok(None);
    }
    if !reserve_resource(context, &inspection.final_url).await? {
        return Ok(None);
    }
    let request_id = match parent_request_id {
        Some(request_id) => request_id,
        None => ensure_crawl_request(context, inspection).await?,
    };
    let document_id = stable_source_identifier(&format!("document|{}", inspection.final_url));
    let now = Utc::now().to_rfc3339();
    let document = DocumentMetadata {
        id: document_id,
        file_name: Some(discovered_file_name(inspection, document_id)),
        file_size: None,
        description: Some(describe_discovery_origin(
            discovered_from,
            &inspection.final_url,
        )),
        content_type: inspection.content_type.clone(),
        download_url: Some(inspection.final_url.clone()),
        created_at: now.clone(),
        updated_at: now,
        visibility: "public".to_string(),
        local_path: None,
        extracted_text_sha256: None,
        was_ocr_processed: false,
    };
    Ok(Some(standard_document_task(
        context, request_id, document, depth,
    )))
}

async fn prepare_crawl_url(
    context: &WorkerContext,
    url: String,
    depth: usize,
    structured_depth: usize,
) -> Option<IngestionTask> {
    // CKAN and Socrata are detected before this path and use their native paginated catalog task;
    // ArcGIS uses the layer task below. Other public catalogs remain bounded generic discovery,
    // while credentialed SharePoint/HUD tenant systems are manual-import-only by policy.
    if let Some(layer_url) = arcgis_layer_url(&url) {
        let identity = format!("arcgis-layer|{layer_url}");
        if !mark_crawl_url(context, &identity).await {
            return None;
        }
        return Some(IngestionTask::CrawlArcGisLayer {
            subdomain: context.portal.portal_key.clone(),
            layer_url,
            offset: 0,
            crawl_depth: depth,
            structured_depth,
        });
    }
    let url = arcgis_metadata_url(&url).unwrap_or(url);
    if !mark_crawl_url(context, &url).await {
        return None;
    }
    Some(IngestionTask::CrawlPublicResource {
        subdomain: context.portal.portal_key.clone(),
        url,
        depth,
    })
}

/// Claims the site path a crawl node occupies, refusing a second URL that reaches the same path
/// with a different query string, except the known San Diego policy category/page keys.
///
/// The first URL to reach a path wins, which is normally the one linked from the seed. Later
/// query variants are the faceted-search shape: `/dataset/` and `/dataset/?tags=species` are the
/// same listing under a filter, and following every filter combination is what made the queue
/// unbounded on data.ca.gov.
///
/// Scoped to site crawling on purpose. Structured expansion is untouched, because a catalog API
/// legitimately addresses distinct records through the query string
/// (`/api/3/action/package_show?id=...`), and that chain is already bounded by
/// `MAXIMUM_STRUCTURED_CHAIN`.
async fn claim_crawl_path(context: &WorkerContext, url: &str) -> bool {
    context
        .crawl
        .crawl_path_claims
        .lock()
        .await
        .insert(crawl_path_identity(url))
}

/// The site path a crawl node occupies: scheme, host, and path with the query dropped and a
/// trailing slash normalized away, so `/dataset`, `/dataset/`, and `/dataset/?tags=species` are
/// one identity.
fn crawl_path_identity(url: &str) -> String {
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return url.to_string();
    };
    if let Some(identity) = super::client::san_diego_policy_listing_identity(&parsed) {
        return identity;
    }
    if let Some(identity) = super::regional::meeting_identity(&parsed) {
        return identity;
    }
    format!(
        "{}://{}{}",
        parsed.scheme(),
        parsed.host_str().unwrap_or_default(),
        parsed.path().trim_end_matches('/')
    )
}

/// Announces, once, that the run has stopped contacting the host a failed task targeted.
async fn announce_host_stop(context: &WorkerContext, target_id: &str) {
    let Ok(url) = reqwest::Url::parse(target_id) else {
        return;
    };
    let governor = context.client.host_governor();
    let Some(reason) = governor.stop_reason(&url) else {
        return;
    };
    let host = crate::ingestion::host_policy::HostGovernor::host_key(&url);
    if !context
        .crawl
        .announced_host_stops
        .lock()
        .await
        .insert(host.clone())
    {
        return;
    }
    let requests_spent = governor
        .snapshots()
        .into_iter()
        .find(|snapshot| snapshot.host == host)
        .map_or(0, |snapshot| snapshot.requests_spent);
    let _ = context
        .progress_sender
        .send(IngestionProgress::HostStopped {
            portal_key: context.portal.portal_key.clone(),
            host,
            requests_spent,
            reason: reason.describe(),
        })
        .await;
}

async fn record_frontier_exclusion(context: &WorkerContext, exclusion: &FrontierExclusion) {
    *context
        .crawl
        .frontier_exclusions
        .lock()
        .await
        .entry(exclusion.rule().to_string())
        .or_insert(0) += 1;
}

/// Decides whether a link discovered on an HTML page becomes a crawl node, naming the rule that
/// refused it when it does not.
async fn admit_crawl_link(
    context: &WorkerContext,
    link: &PublicResourceLink,
    depth: usize,
) -> Result<(), FrontierExclusion> {
    let url = link.url.as_str();
    guidance_allows_url(
        &context.crawl.guidance,
        &context.crawl.compiled_guidance,
        url,
        None,
    )?;
    if link.navigation_chrome {
        return Err(FrontierExclusion::NavigationChrome);
    }
    if depth >= context.crawl.policy.maximum_depth {
        return Err(FrontierExclusion::DepthCeiling);
    }
    if let Ok(parsed) = reqwest::Url::parse(url) {
        let governor = context.client.host_governor();
        // The cached rules if this host's robots.txt has already been read; `None` while it is
        // still being fetched, in which case the same check runs again at dispatch. Reading them
        // here is what keeps a robots exclusion counted under its own rule instead of surfacing
        // as a dispatch failure.
        let cell = governor.robots_cell(&parsed);
        let rules = cell.get().cloned();
        if let Err(refusal) = governor.admits(&parsed, rules.as_deref()) {
            return Err(match refusal {
                RequestRefusal::RobotsExcluded { rule, .. } => {
                    FrontierExclusion::RobotsExcluded { rule }
                }
                refusal => FrontierExclusion::HostStopped {
                    detail: refusal.describe(),
                },
            });
        }
    }
    if !crawl_host_in_scope(
        &context.portal.canonical_url,
        &context.crawl.guidance,
        &context.crawl.policy,
        url,
    ) {
        return Err(FrontierExclusion::OffScopeHost);
    }
    if !claim_crawl_path(context, url).await {
        return Err(FrontierExclusion::QueryVariant {
            path: crawl_path_identity(url),
        });
    }
    Ok(())
}

/// The host-scope rule every crawl node must satisfy, whether the link was found in HTML or in a
/// structured payload: the source's own host, a host the operator named in `allowed_hostnames`,
/// or a supported external data platform when the policy opts in. Downloads are governed
/// separately by `download_host_allowed`, because real portals serve documents from storage hosts
/// this rule does not enumerate.
fn crawl_host_in_scope(
    canonical_url: &str,
    guidance: &SourceGuidance,
    policy: &CrawlPolicy,
    url: &str,
) -> bool {
    source_host_matches(canonical_url, url)
        || guidance_explicitly_allows_host(guidance, url)
        || (policy.follow_supported_external_platforms && is_supported_external_data_platform(url))
}

async fn mark_crawl_url(context: &WorkerContext, url: &str) -> bool {
    let mut visited = context.crawl.visited_urls.lock().await;
    insert_bounded_identity(&mut visited, url, context.crawl.policy.maximum_pages)
}

async fn mark_resource_url(context: &WorkerContext, url: &str) -> bool {
    let mut resources = context.crawl.resource_urls.lock().await;
    insert_bounded_identity(&mut resources, url, context.crawl.policy.maximum_resources)
}

fn insert_bounded_identity(identities: &mut HashSet<String>, value: &str, limit: usize) -> bool {
    if identities.contains(value) || identities.len() >= limit {
        return false;
    }
    identities.insert(value.to_string())
}

async fn reserve_crawl_page(context: &WorkerContext, depth: usize) -> Result<bool, AppError> {
    if depth > context.crawl.policy.maximum_depth {
        return Ok(false);
    }
    reserve_paginated_page(context).await
}

/// The page budget and rate limit without the depth check, for walking one resource's pages.
/// Paginating a single ArcGIS feature layer is not descending the site, so it must not spend
/// `maximum_depth`; `maximum_pages`, the per-layer page-hash dedup and the 100M offset ceiling
/// bound it instead.
async fn reserve_paginated_page(context: &WorkerContext) -> Result<bool, AppError> {
    let Some(page_number) = context
        .repository
        .reserve_ingestion_crawl_page(
            &context.sync_run_id,
            &context.portal.portal_key,
            context.crawl.policy.maximum_pages,
        )
        .await?
    else {
        return Ok(false);
    };
    let page_number = usize::try_from(page_number).map_err(|_| {
        AppError::PortalCapacity("Crawl page counter exceeds platform capacity".to_string())
    })?;
    context
        .crawl
        .pages_started
        .fetch_max(page_number, Ordering::AcqRel);
    context.limiter.acquire(&context.portal.portal_key).await?;
    Ok(true)
}

async fn reserve_resource(context: &WorkerContext, url: &str) -> Result<bool, AppError> {
    if !mark_resource_url(context, url).await {
        return Ok(false);
    }
    let Some(resources_started) = context
        .repository
        .reserve_ingestion_resource(
            &context.sync_run_id,
            &context.portal.portal_key,
            context.crawl.policy.maximum_resources,
        )
        .await?
    else {
        return Ok(false);
    };
    let resources_started = usize::try_from(resources_started).map_err(|_| {
        AppError::PortalCapacity("Resource counter exceeds platform capacity".to_string())
    })?;
    context
        .crawl
        .resources_started
        .fetch_max(resources_started, Ordering::AcqRel);
    Ok(true)
}

fn standard_document_task(
    context: &WorkerContext,
    request_id: u64,
    document: DocumentMetadata,
    depth: PayloadDepth,
) -> IngestionTask {
    let output_directory = context
        .download_root
        .join(format!("{}_documents", context.portal.portal_key));
    IngestionTask::DownloadBinaryPayload {
        subdomain: context.portal.portal_key.clone(),
        request_id,
        document: Box::new(document),
        output_directory: output_directory.to_string_lossy().into_owned(),
        container_depth: depth.container,
        container_root_directory: None,
        crawl_depth: depth.crawl,
        structured_depth: depth.structured,
    }
}

async fn fetch_metadata_page(
    context: &WorkerContext,
    page: usize,
    per_page: usize,
) -> Result<(), AppError> {
    validate_page_parameters(page, per_page)?;
    context.limiter.acquire(&context.portal.portal_key).await?;
    send_progress(
        context,
        IngestionProgress::PageStarted {
            portal_key: context.portal.portal_key.clone(),
            page,
        },
    )
    .await;
    let response = context
        .client
        .fetch_requests(&context.portal, page, per_page)
        .await?;
    let record_count = response.requests.len();
    let processed_requests = u64::try_from(record_count).map_err(|error| {
        AppError::Internal(format!("Request page count cannot be represented: {error}"))
    })?;
    let mut discovered_tasks = Vec::new();
    for mut record in response.requests {
        record.compute_timeline_anomalies()?;
        context
            .repository
            .insert_or_update_request(&context.portal.portal_key, &record)
            .await?;
        for document in record.documents {
            discovered_tasks.push(standard_document_task(
                context,
                record.id,
                document,
                PayloadDepth::discovered(0, 0),
            ));
        }
    }
    send_progress(
        context,
        IngestionProgress::PageCompleted {
            portal_key: context.portal.portal_key.clone(),
            page,
            records_found: record_count,
        },
    )
    .await;
    context
        .statistics
        .pages_processed
        .fetch_add(1, Ordering::AcqRel);
    context
        .statistics
        .requests_processed
        .fetch_add(processed_requests, Ordering::AcqRel);
    if record_count > 0 {
        let next_page = response
            .meta
            .as_ref()
            .and_then(|meta| meta.next_page)
            .or_else(|| (record_count == per_page).then_some(page.saturating_add(1)));
        if let Some(next_page) =
            next_page.filter(|next| *next > page && *next <= MAXIMUM_METADATA_PAGE)
        {
            discovered_tasks.push(IngestionTask::FetchMetadataPage {
                subdomain: context.portal.portal_key.clone(),
                page: next_page,
                per_page,
            });
        }
    }
    enqueue_tasks_resilient(context, discovered_tasks).await?;
    Ok(())
}

async fn process_binary_payload(
    context: &WorkerContext,
    request_id: u64,
    mut document: DocumentMetadata,
    output_directory: PathBuf,
    depth: PayloadDepth,
    container_root_directory: Option<PathBuf>,
) -> Result<(), AppError> {
    validate_internal_output_directory(&context.download_root, &output_directory)?;
    if let Some(root) = container_root_directory.as_deref() {
        validate_internal_output_directory(&context.download_root, root)?;
    }
    let file_name = sanitize_file_name(document.file_name.as_deref().unwrap_or(""), document.id);
    send_progress(
        context,
        IngestionProgress::DocumentStarted {
            portal_key: context.portal.portal_key.clone(),
            document_id: document.id,
            file_name,
        },
    )
    .await;
    let (local_path, byte_count, raw_file_sha256, acquisition) =
        if let Some(local_path) = &document.local_path {
            let (path, count) = validate_cached_payload(
                &context.download_root,
                Path::new(local_path),
                document.file_size,
            )
            .await?;
            let hash = sha256_file(&path).await?;
            // An object's file name is the digest it was committed under. Re-hashing a damaged or
            // edited object and recording the new digest would make the corruption look like the
            // evidence, so a mismatch stops here.
            if let Some(committed) = content_address(&context.download_root, &path) {
                if !constant_time_equal(committed.as_bytes(), hash.as_bytes()) {
                    return Err(AppError::Storage(format!(
                        "Stored evidence '{}' no longer matches the SHA-256 it was committed under",
                        path.display()
                    )));
                }
            }
            (path, count, hash, None)
        } else {
            context
                .repository
                .check_retained_file_budget(&context.download_root)
                .await?;
            context.limiter.acquire(&context.portal.portal_key).await?;
            let download_url = document
                .download_url
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| {
                    AppError::DataParsing(format!("Document {} has no download URL", document.id))
                })?;
            let downloaded = stream_download(
                &context.client,
                &context.portal,
                download_url,
                &context.download_root,
                document.id,
            )
            .await?;
            (
                downloaded.path,
                downloaded.byte_count,
                downloaded.sha256,
                Some(downloaded.acquisition),
            )
        };
    let entropy_path = local_path.clone();
    let entropy_file_name = document.file_name.clone();
    let entropy = spawn_blocking("Entropy assessment", move || {
        assess_file_entropy(&entropy_path, entropy_file_name.as_deref())
    })
    .await??;
    document.local_path = Some(local_path.to_string_lossy().into_owned());
    document.file_size = Some(byte_count);
    let format_path = local_path.clone();
    let declared_file_name = document.file_name.clone();
    let content_type = document.content_type.clone();
    let format = spawn_blocking("Format detector", move || {
        detect_document_format(
            &format_path,
            declared_file_name.as_deref(),
            content_type.as_deref(),
        )
    })
    .await??;
    let capability = parser_capability(format);
    if byte_count > capability.maximum_source_bytes {
        return Err(AppError::DataParsing(format!(
            "PARSER_SOURCE_SIZE_EXCEEDED: {format:?} source contains {byte_count} bytes; capability limit is {}",
            capability.maximum_source_bytes
        )));
    }
    let memory_permits = parser_memory_permits(byte_count, capability.working_set_multiplier);
    let _memory_permit = context
        .parser_memory
        .clone()
        .acquire_many_owned(memory_permits)
        .await
        .map_err(|_| AppError::Internal("Parser memory governor is unavailable".to_string()))?;
    context
        .repository
        .insert_or_update_document(&context.portal.portal_key, request_id, &document)
        .await?;
    release_object_link(&local_path).await;
    // Recorded once the row exists and before extraction rewrites its digest, so the event
    // captures the digest this fetch replaced.
    if let Some(acquisition) = &acquisition {
        context
            .repository
            .record_document_acquisition(
                &context.portal.portal_key,
                document.id,
                Some(&context.sync_run_id),
                &raw_file_sha256,
                byte_count,
                acquisition,
            )
            .await?;
    }
    expand_structured_document_links(context, request_id, &document, &local_path, depth).await?;
    let extracted_document = extract_document(
        context,
        DocumentExtractionInput {
            request_id,
            document: &document,
            local_path: &local_path,
            format,
            depth,
            raw_file_sha256: &raw_file_sha256,
            container_root_directory: container_root_directory.as_deref(),
        },
    )
    .await?;
    let download_finalization = DocumentDownloadFinalization {
        document_id: document.id,
        portal_subdomain: &context.portal.portal_key,
        raw_file_size: byte_count,
        raw_file_sha256: &raw_file_sha256,
        local_path: &local_path,
        shannon_entropy: Some(entropy.shannon_entropy),
        entropy_sample_bytes: entropy.sampled_bytes,
        suspicious_data_density: entropy.suspicious_data_density,
    };
    // NextRequest's API supplies the document's own `created_at`. Every other source kind in this
    // queue currently fills `DocumentMetadata::created_at` with `Utc::now()` when it discovers or
    // materializes a payload (including local files, HTML pages, and ArcGIS pages). Treating those
    // ingestion timestamps as document dates would make a bounded historical range refuse nearly
    // every non-NextRequest document. Container members inherit their parent's value, so this gate
    // remains correct for nested documents too.
    let document_date =
        document_date_for_guidance(context.portal.source_kind, document.created_at.as_str());
    if matches!(
        &extracted_document,
        ExtractedDocument::Expanded { .. } | ExtractedDocument::Deferred { .. }
    ) {
        context
            .repository
            .mark_document_downloaded(download_finalization)
            .await?;
    }
    match extracted_document {
        ExtractedDocument::Searchable {
            text,
            was_ocr_processed,
        } => {
            let Some(character_count) = finalize_and_index_extracted_document(
                context,
                request_id,
                download_finalization,
                text,
                was_ocr_processed,
                document_date,
            )
            .await?
            else {
                return Ok(());
            };
            send_progress(
                context,
                IngestionProgress::DocumentExtracted {
                    portal_key: context.portal.portal_key.clone(),
                    document_id: document.id,
                    character_count,
                    was_ocr_processed,
                },
            )
            .await;
        }
        ExtractedDocument::Expanded {
            format,
            entry_count,
        } => {
            let format_label = format!("{format:?}");
            context
                .repository
                .mark_document_extraction_state(
                    document.id,
                    &context.portal.portal_key,
                    "EXPANDED",
                    Some(&format_label),
                    None,
                )
                .await?;
            send_progress(
                context,
                IngestionProgress::DocumentExpanded {
                    portal_key: context.portal.portal_key.clone(),
                    document_id: document.id,
                    format: format_label,
                    entry_count,
                },
            )
            .await;
        }
        ExtractedDocument::Deferred { format, route } => {
            let format_label = format!("{format:?}");
            let reason = extraction_route_label(route);
            context
                .repository
                .mark_document_extraction_state(
                    document.id,
                    &context.portal.portal_key,
                    "DEFERRED",
                    Some(&format_label),
                    Some(&reason),
                )
                .await?;
            send_progress(
                context,
                IngestionProgress::DocumentDeferred {
                    portal_key: context.portal.portal_key.clone(),
                    document_id: document.id,
                    format: format_label,
                    reason,
                },
            )
            .await;
        }
        ExtractedDocument::Salvaged { report, format } => {
            let decoder_summary = report.decoder_summary();
            let provenance_marker = report.provenance_marker.to_string();
            let source_sha256 = report.source_sha256.clone();
            let recovered_run_count = report.runs.len();
            let primary_error = report.primary_error.clone();
            if finalize_and_index_extracted_document(
                context,
                request_id,
                download_finalization,
                report.text,
                false,
                document_date,
            )
            .await?
            .is_none()
            {
                return Ok(());
            }
            let format_label = format!("{format:?}");
            context
                .repository
                .mark_document_extraction_state(
                    document.id,
                    &context.portal.portal_key,
                    "SALVAGED",
                    Some(&format_label),
                    None,
                )
                .await?;
            send_progress(
                context,
                IngestionProgress::DocumentSalvaged {
                    portal_key: context.portal.portal_key.clone(),
                    document_id: document.id,
                    format: format_label,
                    provenance_marker,
                    source_sha256,
                    recovered_run_count,
                    decoder_summary,
                    primary_error,
                },
            )
            .await;
        }
    }
    send_progress(
        context,
        IngestionProgress::DocumentCompleted {
            portal_key: context.portal.portal_key.clone(),
            document_id: document.id,
            bytes_saved: byte_count,
        },
    )
    .await;
    context
        .statistics
        .documents_processed
        .fetch_add(1, Ordering::AcqRel);
    if !document_is_html(&document) {
        context
            .statistics
            .data_documents_processed
            .fetch_add(1, Ordering::AcqRel);
    }
    Ok(())
}

fn document_is_html(document: &DocumentMetadata) -> bool {
    document.content_type.as_deref().is_some_and(|value| {
        matches!(
            value.split(';').next().unwrap_or(value).trim(),
            "text/html" | "application/xhtml+xml"
        )
    }) || document.file_name.as_deref().is_some_and(|name| {
        let name = name.to_ascii_lowercase();
        [".html", ".htm", ".xhtml"]
            .iter()
            .any(|extension| name.ends_with(extension))
    })
}

async fn index_extracted_document(
    context: &WorkerContext,
    request_id: u64,
    document_id: u64,
    raw_file_sha256: &str,
    extracted_text_sha256: &str,
    text: String,
) -> Result<(), AppError> {
    context
        .corpus_index
        .index_document(CorpusDocumentInput {
            portal_key: context.portal.portal_key.clone(),
            request_id,
            document_id,
            raw_file_sha256: raw_file_sha256.to_string(),
            extracted_text_sha256: extracted_text_sha256.to_string(),
            text,
        })
        .await
}

/// Content keyword/regex, organization, and document-date guidance predicates, evaluated once
/// extracted text (and, via `agency_label_for_request`, the owning agency) are actually available
/// -- unlike the pre-download predicates in `guidance_allows_url`. `agency_label` is `None` both
/// when the source has no organization guidance (never fetched) and when the request has no
/// agency on file; either way the organization predicate contributes nothing. `document_created_at`
/// must be `None` unless the caller can attest it is a date the document itself actually carries
/// -- see the call site's comment on `SourceKind::NextRequest` for why most ingestion paths stamp
/// this field with the ingestion instant instead, which would make the date predicate exclude
/// nearly everything on any bounded range.
fn evaluate_extracted_document_guidance(
    guidance: &SourceGuidance,
    compiled: &CompiledGuidancePatterns,
    text: &str,
    agency_label: Option<&str>,
    document_created_at: Option<&str>,
) -> Result<(), FrontierExclusion> {
    if guidance
        .exclude_content_keywords
        .iter()
        .any(|keyword| text.contains(keyword.as_str()))
    {
        return Err(FrontierExclusion::ContentKeyword);
    }
    if !guidance.include_content_keywords.is_empty()
        && !guidance
            .include_content_keywords
            .iter()
            .any(|keyword| text.contains(keyword.as_str()))
    {
        return Err(FrontierExclusion::ContentKeyword);
    }
    if compiled
        .exclude_content_patterns
        .iter()
        .any(|pattern| pattern.is_match(text))
    {
        return Err(FrontierExclusion::ContentPattern);
    }
    if !compiled.include_content_patterns.is_empty()
        && !compiled
            .include_content_patterns
            .iter()
            .any(|pattern| pattern.is_match(text))
    {
        return Err(FrontierExclusion::ContentPattern);
    }
    if let Some(agency) = agency_label {
        let agency_lower = agency.to_ascii_lowercase();
        if guidance
            .exclude_organizations
            .iter()
            .any(|name| agency_lower.contains(&name.to_ascii_lowercase()))
        {
            return Err(FrontierExclusion::Organization);
        }
        if !guidance.include_organizations.is_empty()
            && !guidance
                .include_organizations
                .iter()
                .any(|name| agency_lower.contains(&name.to_ascii_lowercase()))
        {
            return Err(FrontierExclusion::Organization);
        }
    }
    if guidance.document_date_from.is_some() || guidance.document_date_to.is_some() {
        // The first 10 bytes of an ISO-8601 timestamp are its calendar date; a value too short to
        // hold one carries no date to compare, so the predicate contributes nothing rather than
        // excluding on an absence.
        if let Some(date) = document_created_at.and_then(|value| value.get(0..10)) {
            let before_start = guidance
                .document_date_from
                .as_deref()
                .is_some_and(|from| date < from);
            let after_end = guidance
                .document_date_to
                .as_deref()
                .is_some_and(|to| date > to);
            if before_start || after_end {
                return Err(FrontierExclusion::DocumentDate);
            }
        }
    }
    Ok(())
}

fn document_date_for_guidance(source_kind: SourceKind, created_at: &str) -> Option<&str> {
    if source_kind != SourceKind::NextRequest {
        return None;
    }
    let date = created_at.get(0..10)?;
    NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()?;
    Some(date)
}

async fn finalize_and_index_extracted_document(
    context: &WorkerContext,
    request_id: u64,
    download: DocumentDownloadFinalization<'_>,
    text: String,
    was_ocr_processed: bool,
    document_created_at: Option<&str>,
) -> Result<Option<usize>, AppError> {
    let guidance = &context.crawl.guidance;
    let agency_label =
        if guidance.include_organizations.is_empty() && guidance.exclude_organizations.is_empty() {
            None
        } else {
            context
                .repository
                .agency_label_for_request(&context.portal.portal_key, request_id)
                .await?
        };
    if let Err(exclusion) = evaluate_extracted_document_guidance(
        guidance,
        &context.crawl.compiled_guidance,
        &text,
        agency_label.as_deref(),
        document_created_at,
    ) {
        // A resumed sync re-extracts every document regardless of prior completion, so this
        // document may already carry a `documents_fts` row from an earlier pass. Guidance edited
        // between syncs must not retroactively purge evidence a prior sync already finalized --
        // `delete_unextracted_document` enforces that itself and reports whether it actually
        // removed anything. When it didn't, the document stays and falls through to the normal
        // finalize path below rather than being counted as a refusal that did not happen.
        let discarded = context
            .repository
            .delete_unextracted_document(download.portal_subdomain, download.document_id)
            .await?;
        if discarded {
            record_frontier_exclusion(context, &exclusion).await;
            return Ok(None);
        }
    }
    let extracted_text_sha256 = sha256_hex(text.as_bytes());
    let character_count = text.chars().count();
    context
        .repository
        .finalize_document_extraction(DocumentExtractionFinalization {
            document_id: download.document_id,
            portal_subdomain: download.portal_subdomain,
            extracted_text: &text,
            extracted_text_sha256: &extracted_text_sha256,
            raw_file_sha256: download.raw_file_sha256,
            was_ocr_processed,
            raw_file_size: download.raw_file_size,
            local_path: download.local_path,
            shannon_entropy: download.shannon_entropy,
            entropy_sample_bytes: download.entropy_sample_bytes,
            suspicious_data_density: download.suspicious_data_density,
        })
        .await?;
    context
        .repository
        .capture_monitoring_document(download.portal_subdomain, download.document_id)
        .await?;
    index_extracted_document(
        context,
        request_id,
        download.document_id,
        download.raw_file_sha256,
        &extracted_text_sha256,
        text,
    )
    .await?;
    Ok(Some(character_count))
}

#[derive(Clone)]
struct StructuredLinkCandidate {
    url: String,
    label: String,
    media_type: Option<String>,
    download_candidate: bool,
}

struct StructuredDocumentDiscovery {
    candidates: Vec<StructuredLinkCandidate>,
    next_page: Option<String>,
    companion_urls: Vec<String>,
    layer_urls: Vec<String>,
}

async fn expand_structured_document_links(
    context: &WorkerContext,
    request_id: u64,
    document: &DocumentMetadata,
    local_path: &Path,
    depth: PayloadDepth,
) -> Result<(), AppError> {
    if !document_is_json(document) {
        return Ok(());
    }
    // Discovered downloads never pass through `reserve_crawl_page`, so before this the chain
    // JSON -> download -> JSON honored no depth ceiling at all. It was still finite, bounded by
    // `maximum_resources` and `maximum_pages`, but nothing capped the chain itself. The cap is
    // its own axis so that a catalog deep in a portal keeps yielding documents.
    if !may_expand_structured_links(depth.structured) {
        return Ok(());
    }
    let metadata = tokio::fs::metadata(local_path).await.map_err(|error| {
        AppError::Storage(format!(
            "Failed to inspect structured source '{}': {error}",
            local_path.display()
        ))
    })?;
    if metadata.len() > MAXIMUM_STRUCTURED_DISCOVERY_BYTES {
        return Ok(());
    }
    let bytes = tokio::fs::read(local_path).await.map_err(|error| {
        AppError::Storage(format!(
            "Failed to read structured source '{}': {error}",
            local_path.display()
        ))
    })?;
    let source_url = document.download_url.clone();
    // Parsing a 64 MB JSON value and recursively walking it are CPU/allocation work. Tauri IPC
    // shares this Tokio runtime, so doing either directly in this future can stall unrelated UI
    // commands even though the file read itself is asynchronous.
    let Some(discovery) = spawn_blocking("Structured document discovery", move || {
        Ok(discover_structured_document_links(
            &bytes,
            source_url.as_deref(),
        ))
    })
    .await??
    else {
        return Ok(());
    };
    let mut tasks = Vec::with_capacity(
        discovery.candidates.len()
            + discovery.companion_urls.len()
            + discovery.layer_urls.len()
            + usize::from(discovery.next_page.is_some()),
    );
    // Paging one search result set revisits a single resource, so it holds its own depth and is
    // bounded by `maximum_pages` and `visited_urls` instead.
    if let Some(next_page) = discovery.next_page {
        if let Some(task) =
            prepare_crawl_url(context, next_page, depth.crawl, depth.structured).await
        {
            tasks.push(task);
        }
    }
    for companion_url in discovery.companion_urls {
        if let Some(task) = prepare_crawl_url(
            context,
            companion_url,
            depth.crawl.saturating_add(1),
            depth.structured.saturating_add(1),
        )
        .await
        {
            tasks.push(task);
        }
    }
    for layer_url in discovery.layer_urls {
        let identity = format!("arcgis-layer|{layer_url}");
        if mark_crawl_url(context, &identity).await {
            tasks.push(IngestionTask::CrawlArcGisLayer {
                subdomain: context.portal.portal_key.clone(),
                layer_url,
                offset: 0,
                crawl_depth: depth.crawl.saturating_add(1),
                structured_depth: depth.structured.saturating_add(1),
            });
        }
    }
    let discovered_from = document
        .download_url
        .clone()
        .unwrap_or_else(|| format!("local document {}", document.id));
    for candidate in discovery.candidates {
        if let Err(exclusion) = guidance_allows_url(
            &context.crawl.guidance,
            &context.crawl.compiled_guidance,
            &candidate.url,
            None,
        ) {
            record_frontier_exclusion(context, &exclusion).await;
            continue;
        }
        if candidate.download_candidate {
            let inspection = PublicResourceInspection {
                final_url: candidate.url,
                content_type: candidate.media_type,
                title: candidate.label,
                kind: PublicResourceKind::Download,
                links: Vec::new(),
                body: None,
                embedded_connectors: Vec::new(),
                content_length: None,
                acquisition: None,
            };
            if let Some(task) = prepare_discovered_download(
                context,
                &inspection,
                Some(request_id),
                &discovered_from,
                PayloadDepth::discovered(
                    depth.crawl.saturating_add(1),
                    depth.structured.saturating_add(1),
                ),
            )
            .await?
            {
                tasks.push(task);
            }
        } else {
            // A JSON payload is content the remote site chose, exactly like an HTML page, so a
            // crawl node it names is held to the host rule `admit_crawl_link` applies to HTML
            // links. Without this, `{"type":"dataset","url":"https://anywhere/"}` in any JSON
            // the crawl reads became a `CrawlPublicResource` on that host, and the page fetched
            // there was recorded as a document of this source.
            if !crawl_host_in_scope(
                &context.portal.canonical_url,
                &context.crawl.guidance,
                &context.crawl.policy,
                &candidate.url,
            ) {
                record_frontier_exclusion(context, &FrontierExclusion::OffScopeHost).await;
                continue;
            }
            if let Some(task) = prepare_crawl_url(
                context,
                candidate.url,
                depth.crawl.saturating_add(1),
                depth.structured.saturating_add(1),
            )
            .await
            {
                tasks.push(task);
            }
        }
    }
    enqueue_tasks_resilient(context, tasks).await
}

fn document_is_json(document: &DocumentMetadata) -> bool {
    document.content_type.as_deref().is_some_and(|value| {
        let value = value.split(';').next().unwrap_or(value).trim();
        value == "application/json" || value == "application/ld+json" || value.ends_with("+json")
    }) || document.file_name.as_deref().is_some_and(|name| {
        let name = name.to_ascii_lowercase();
        [".json", ".jsonld"]
            .iter()
            .any(|extension| name.ends_with(extension))
    })
}

/// Parses and walks one structured payload without retaining the potentially large JSON tree in
/// the async worker. The caller runs this function on the blocking executor and receives only the
/// bounded link/layer summaries needed for asynchronous policy checks and task scheduling.
fn discover_structured_document_links(
    bytes: &[u8],
    source_url: Option<&str>,
) -> Option<StructuredDocumentDiscovery> {
    let value = serde_json::from_slice::<serde_json::Value>(bytes).ok()?;
    let base_url = source_url.and_then(|value| reqwest::Url::parse(value).ok());
    let mut candidates = HashMap::new();
    collect_structured_link_candidates(&value, base_url.as_ref(), 0, None, &mut candidates);
    let mut candidates = candidates.into_values().collect::<Vec<_>>();
    candidates.sort_by(|left, right| left.url.cmp(&right.url));
    Some(StructuredDocumentDiscovery {
        candidates,
        next_page: arcgis_search_next_page(source_url, &value),
        companion_urls: arcgis_structured_item_companion_urls(&value),
        layer_urls: arcgis_layers_from_metadata(source_url, &value),
    })
}

fn collect_structured_link_candidates(
    value: &serde_json::Value,
    base_url: Option<&reqwest::Url>,
    depth: usize,
    inherited_label: Option<&str>,
    candidates: &mut HashMap<String, StructuredLinkCandidate>,
) {
    if depth > 64 || candidates.len() >= MAXIMUM_STRUCTURED_DISCOVERY_LINKS {
        return;
    }
    match value {
        serde_json::Value::Object(object) => {
            let local_label = ["title", "name", "identifier"]
                .into_iter()
                .find_map(|key| object.get(key).and_then(serde_json::Value::as_str))
                .or(inherited_label)
                .unwrap_or("Structured data resource");
            let declared_format = object
                .get("format")
                .and_then(serde_json::Value::as_str)
                .and_then(super::client::catalog_format_media_type);
            let media_type = ["mediaType", "contentType", "mimeType", "mimetype"]
                .into_iter()
                .find_map(|key| object.get(key).and_then(serde_json::Value::as_str))
                .or(declared_format);
            let object_is_data_descriptor = object
                .get("type")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|value| {
                    let value = value.to_ascii_lowercase();
                    value.contains("service")
                        || value.contains("dataset")
                        || value.contains("csv")
                        || value.contains("geojson")
                });
            for (key, nested) in object {
                if let Some(raw_url) = nested.as_str() {
                    let normalized_key = key
                        .chars()
                        .filter(|character| character.is_ascii_alphanumeric())
                        .flat_map(char::to_lowercase)
                        .collect::<String>();
                    let declared_download = matches!(
                        normalized_key.as_str(),
                        "downloadurl" | "downloaduri" | "contenturl" | "fileurl" | "describedby"
                    ) || (normalized_key == "url"
                        && declared_format.is_some());
                    let declared_access = matches!(
                        normalized_key.as_str(),
                        "accessurl" | "endpointurl" | "apiurl" | "serviceurl"
                    );
                    let generic_link = matches!(normalized_key.as_str(), "url" | "href");
                    if declared_download || declared_access || generic_link {
                        if let Some(url) = resolve_structured_url(base_url, raw_url) {
                            let extension_download = has_likely_document_extension(url.path());
                            if declared_download
                                || declared_access
                                || extension_download
                                || (generic_link && object_is_data_descriptor)
                            {
                                let canonical = url.to_string();
                                let download_candidate = declared_download || extension_download;
                                candidates
                                    .entry(canonical.clone())
                                    .and_modify(|candidate| {
                                        candidate.download_candidate |= download_candidate;
                                        if candidate.media_type.is_none() {
                                            candidate.media_type = media_type.map(str::to_string);
                                        }
                                    })
                                    .or_insert_with(|| StructuredLinkCandidate {
                                        url: canonical,
                                        label: local_label.chars().take(256).collect(),
                                        media_type: media_type.map(str::to_string),
                                        download_candidate,
                                    });
                            }
                        }
                    }
                }
                collect_structured_link_candidates(
                    nested,
                    base_url,
                    depth.saturating_add(1),
                    Some(local_label),
                    candidates,
                );
                if candidates.len() >= MAXIMUM_STRUCTURED_DISCOVERY_LINKS {
                    return;
                }
            }
        }
        serde_json::Value::Array(values) => {
            for nested in values {
                collect_structured_link_candidates(
                    nested,
                    base_url,
                    depth.saturating_add(1),
                    inherited_label,
                    candidates,
                );
                if candidates.len() >= MAXIMUM_STRUCTURED_DISCOVERY_LINKS {
                    return;
                }
            }
        }
        _ => {}
    }
}

fn resolve_structured_url(base_url: Option<&reqwest::Url>, value: &str) -> Option<reqwest::Url> {
    if value.is_empty() || value.len() > 4_096 {
        return None;
    }
    let mut url = reqwest::Url::parse(value)
        .ok()
        .or_else(|| base_url?.join(value).ok())?;
    url.set_fragment(None);
    (url.scheme() == "https").then_some(url)
}

fn arcgis_item_companion_urls(value: &str) -> Vec<String> {
    let Ok(url) = reqwest::Url::parse(value) else {
        return Vec::new();
    };
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    let has_arcgis_signature = host.contains("arcgis.com")
        || host.contains("arcgisonline.com")
        || url.path().to_ascii_lowercase().contains("/arcgis/")
        || url
            .query_pairs()
            .any(|(key, _)| key.eq_ignore_ascii_case("webmap"));
    if !has_arcgis_signature {
        return Vec::new();
    }
    let item_id = url
        .query_pairs()
        .find(|(key, value)| {
            matches!(
                key.to_ascii_lowercase().as_str(),
                "webmap" | "id" | "itemid"
            ) && is_arcgis_item_id(value)
        })
        .map(|(_, value)| value.into_owned())
        .or_else(|| {
            url.path_segments()?
                .find(|segment| is_arcgis_item_id(segment))
                .map(str::to_string)
        });
    let Some(item_id) = item_id else {
        return Vec::new();
    };
    let root = format!("https://www.arcgis.com/sharing/rest/content/items/{item_id}");
    vec![format!("{root}?f=json"), format!("{root}/data?f=json")]
}

pub(crate) fn arcgis_hub_companion_urls(page_url: &str, body: &[u8]) -> Vec<String> {
    let is_hub = reqwest::Url::parse(page_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
        .is_some_and(|host| {
            host == "hub.arcgis.com"
                || host.ends_with(".hub.arcgis.com")
                || host.ends_with(".opendata.arcgis.com")
        });
    if !is_hub {
        return Vec::new();
    }
    let decoded = percent_decode_bytes(body);
    let mut urls = BTreeSet::new();
    let decoded = String::from_utf8_lossy(&decoded);
    let identifiers =
        regex::Regex::new(r#"(?i)"(contentGroupId|siteId|itemId|webmap)"\s*:\s*"([a-f0-9]{32})""#)
            .expect("static ArcGIS property pattern");
    for capture in identifiers.captures_iter(&decoded).take(128) {
        let item_id = &capture[2];
        if capture[1].eq_ignore_ascii_case("contentGroupId") {
            let mut search = reqwest::Url::parse("https://www.arcgis.com/sharing/rest/search")
                .expect("static ArcGIS search URL is valid");
            search.query_pairs_mut().extend_pairs([
                ("f", "json"),
                ("q", &format!("group:{item_id}")),
                ("num", "100"),
                ("start", "1"),
            ]);
            urls.insert(search.to_string());
        } else {
            let root = format!("https://www.arcgis.com/sharing/rest/content/items/{item_id}");
            urls.insert(format!("{root}?f=json"));
            urls.insert(format!("{root}/data?f=json"));
        }
    }
    urls.into_iter().collect()
}

fn arcgis_structured_item_companion_urls(value: &serde_json::Value) -> Vec<String> {
    fn visit(value: &serde_json::Value, depth: usize, urls: &mut BTreeSet<String>) {
        if depth > 64 || urls.len() >= MAXIMUM_STRUCTURED_DISCOVERY_LINKS {
            return;
        }
        match value {
            serde_json::Value::Object(object) => {
                if object
                    .get("type")
                    .and_then(serde_json::Value::as_str)
                    .is_some()
                {
                    if let Some(item_id) = object
                        .get("id")
                        .and_then(serde_json::Value::as_str)
                        .filter(|value| is_arcgis_item_id(value))
                    {
                        let root =
                            format!("https://www.arcgis.com/sharing/rest/content/items/{item_id}");
                        urls.insert(format!("{root}?f=json"));
                        urls.insert(format!("{root}/data?f=json"));
                    }
                }
                for nested in object.values() {
                    visit(nested, depth.saturating_add(1), urls);
                }
            }
            serde_json::Value::Array(values) => {
                for nested in values {
                    visit(nested, depth.saturating_add(1), urls);
                }
            }
            _ => {}
        }
    }

    let mut urls = BTreeSet::new();
    visit(value, 0, &mut urls);
    urls.into_iter().collect()
}

fn arcgis_search_next_page(source_url: Option<&str>, value: &serde_json::Value) -> Option<String> {
    let next_start = value.get("nextStart")?.as_u64()?;
    if next_start == 0 {
        return None;
    }
    let mut url = reqwest::Url::parse(source_url?).ok()?;
    let host = url.host_str()?.to_ascii_lowercase();
    if !(host == "arcgis.com" || host.ends_with(".arcgis.com"))
        || !url.path().ends_with("/sharing/rest/search")
    {
        return None;
    }
    let retained = url
        .query_pairs()
        .filter(|(key, _)| !key.eq_ignore_ascii_case("start"))
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    url.set_query(None);
    url.query_pairs_mut()
        .extend_pairs(retained)
        .append_pair("start", &next_start.to_string());
    Some(url.to_string())
}

fn percent_decode_bytes(value: &[u8]) -> Vec<u8> {
    fn hex_nibble(value: u8) -> Option<u8> {
        match value {
            b'0'..=b'9' => Some(value - b'0'),
            b'a'..=b'f' => Some(value - b'a' + 10),
            b'A'..=b'F' => Some(value - b'A' + 10),
            _ => None,
        }
    }

    let mut decoded = Vec::with_capacity(value.len());
    let mut index = 0;
    while index < value.len() {
        if value[index] == b'%' && index.saturating_add(2) < value.len() {
            if let (Some(high), Some(low)) =
                (hex_nibble(value[index + 1]), hex_nibble(value[index + 2]))
            {
                decoded.push((high << 4) | low);
                index += 3;
                continue;
            }
        }
        decoded.push(value[index]);
        index += 1;
    }
    decoded
}

fn tableau_export_companion_urls(value: &str) -> Vec<String> {
    let Ok(url) = reqwest::Url::parse(value) else {
        return Vec::new();
    };
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    if !host.contains("tableau.com") {
        return Vec::new();
    }
    let segments = url
        .path_segments()
        .map(|segments| segments.collect::<Vec<_>>())
        .unwrap_or_default();
    let view_pair = segments
        .iter()
        .position(|segment| *segment == "viz")
        .and_then(|index| Some((*segments.get(index + 1)?, *segments.get(index + 2)?)))
        .or_else(|| {
            segments
                .iter()
                .position(|segment| *segment == "views")
                .and_then(|index| Some((*segments.get(index + 1)?, *segments.get(index + 2)?)))
        });
    let Some((workbook, raw_sheet)) = view_pair else {
        return Vec::new();
    };
    if workbook.is_empty() || raw_sheet.is_empty() {
        return Vec::new();
    }
    let sheet = raw_sheet
        .trim_end_matches(".csv")
        .trim_end_matches(".pdf")
        .trim_end_matches(".png");
    let origin = format!("{}://{}", url.scheme(), host);
    let base = format!("{origin}/views/{workbook}/{sheet}");
    vec![
        format!("{base}.csv?:showVizHome=no"),
        format!("{base}.pdf?:showVizHome=no"),
    ]
}

fn is_arcgis_item_id(value: &str) -> bool {
    value.len() == 32 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn arcgis_layer_url(value: &str) -> Option<String> {
    let mut url = reqwest::Url::parse(value).ok()?;
    let segments = url.path_segments()?.collect::<Vec<_>>();
    let server_index = segments
        .iter()
        .position(|segment| matches!(*segment, "FeatureServer" | "MapServer"))?;
    let layer_id = segments.get(server_index.saturating_add(1))?;
    if segments
        .iter()
        .skip(server_index + 2)
        .any(|part| !part.is_empty())
        || url
            .query_pairs()
            .any(|(key, value)| key != "f" || !matches!(value.as_ref(), "json" | "pjson"))
    {
        // A supplied /query is a selected export, possibly filtered. Do not turn it into 1=1.
        return None;
    }
    if layer_id.is_empty() || !layer_id.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let path = format!("/{}", segments[..=server_index + 1].join("/"));
    url.set_path(&path);
    url.set_query(None);
    url.set_fragment(None);
    Some(url.to_string())
}

fn arcgis_metadata_url(value: &str) -> Option<String> {
    let mut url = reqwest::Url::parse(value).ok()?;
    let segments = url.path_segments()?.collect::<Vec<_>>();
    let server_index = segments
        .iter()
        .position(|segment| matches!(*segment, "FeatureServer" | "MapServer"))?;
    if segments
        .get(server_index.saturating_add(1))
        .is_some_and(|segment| !segment.is_empty())
    {
        return None;
    }
    let path = format!("/{}", segments[..=server_index].join("/"));
    url.set_path(&path);
    url.set_query(None);
    url.set_fragment(None);
    url.query_pairs_mut().append_pair("f", "json");
    Some(url.to_string())
}

fn arcgis_layers_from_metadata(source_url: Option<&str>, value: &serde_json::Value) -> Vec<String> {
    let Some(source_url) = source_url else {
        return Vec::new();
    };
    let Ok(mut base) = reqwest::Url::parse(source_url) else {
        return Vec::new();
    };
    base.set_query(None);
    base.set_fragment(None);
    let path = base.path().trim_end_matches('/');
    if !path.ends_with("/FeatureServer") && !path.ends_with("/MapServer") {
        return Vec::new();
    }
    ["layers", "tables"]
        .into_iter()
        .filter_map(|key| value.get(key).and_then(serde_json::Value::as_array))
        .flatten()
        .filter_map(|layer| layer.get("id").and_then(serde_json::Value::as_u64))
        .take(10_000)
        .map(|layer_id| {
            let mut layer_url = base.clone();
            layer_url.set_path(&format!("{path}/{layer_id}"));
            layer_url.to_string()
        })
        .collect()
}

#[derive(Clone, Copy)]
struct DocumentExtractionInput<'a> {
    request_id: u64,
    document: &'a DocumentMetadata,
    local_path: &'a Path,
    format: DocumentFormat,
    depth: PayloadDepth,
    raw_file_sha256: &'a str,
    container_root_directory: Option<&'a Path>,
}

async fn extract_document(
    context: &WorkerContext,
    input: DocumentExtractionInput<'_>,
) -> Result<ExtractedDocument, AppError> {
    let DocumentExtractionInput {
        request_id,
        document,
        local_path,
        format,
        depth,
        raw_file_sha256: _,
        container_root_directory,
    } = input;
    let _parser_permit = context
        .parser_semaphore
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| AppError::Internal("Parser scheduler is unavailable".to_string()))?;
    let primary_result = async {
        match format {
        DocumentFormat::Pdf => {
            // Ordinary ingestion remains searchable-text-only. Route the
            // immutable residual-redaction audit through the dedicated forensic command so OCR
            // fallback cannot silently replace structural evidence.
            let path = local_path.to_path_buf();
            let extraction = spawn_blocking("PDF parser", move || extract_text_layers(&path))
                .await?;
            match extraction {
                Ok(text) => Ok(ExtractedDocument::Searchable {
                    text,
                    was_ocr_processed: false,
                }),
                Err(primary_error) => {
                    let _ocr_permit = context
                        .ocr_semaphore
                        .clone()
                        .acquire_owned()
                        .await
                        .map_err(|_| {
                            AppError::Internal("OCR scheduler is unavailable".to_string())
                        })?;
                    let path = local_path.to_path_buf();
                    let ocr_result =
                        spawn_blocking("OCR", move || execute_local_ocr_scan(&path)).await?;
                    match ocr_result {
                        Ok(text) => Ok(ExtractedDocument::Searchable {
                            text,
                            was_ocr_processed: true,
                        }),
                        Err(ocr_error) => Err(AppError::DataParsing(format!(
                            "PDF_PRIMARY_AND_OCR_FAILED: primary parser: {primary_error}; OCR: {ocr_error}"
                        ))),
                    }
                }
            }
        }
        DocumentFormat::Spreadsheet => {
            let path = local_path.to_path_buf();
            let text = spawn_blocking("Spreadsheet parser", move || {
                extract_spreadsheet_layers(&path)
            })
            .await??;
            Ok(ExtractedDocument::Searchable {
                text,
                was_ocr_processed: false,
            })
        }
        DocumentFormat::OutlookMessage => {
            let attachment_container_depth = next_msg_attachment_container_depth(depth.container);
            let attachment_cache = local_path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(format!("{}_attachments", document.id));
            let attachment_root_directory = container_root_directory
                .map(Path::to_path_buf)
                .unwrap_or_else(|| attachment_cache.clone());
            let payload = extract_msg_layers(
                local_path,
                MsgExtractionContext {
                    source_document: document,
                    cache_directory: &attachment_cache,
                    collect_attachments: attachment_container_depth.is_some(),
                },
            )
            .await?;
            let attachment_tasks = if let Some(container_depth) = attachment_container_depth {
                if let Err(error) = validate_container_tree_budget(&attachment_root_directory) {
                    let _ = std::fs::remove_dir_all(&attachment_cache);
                    return Err(error);
                }
                let output_directory = attachment_cache.to_string_lossy().into_owned();
                let container_root_directory =
                    Some(attachment_root_directory.to_string_lossy().into_owned());
                payload
                    .attachment_documents
                    .into_iter()
                    .map(|attachment| IngestionTask::DownloadBinaryPayload {
                        subdomain: context.portal.portal_key.clone(),
                        request_id,
                        document: Box::new(attachment.metadata),
                        output_directory: output_directory.clone(),
                        container_depth,
                        container_root_directory: container_root_directory.clone(),
                        crawl_depth: depth.crawl,
                        structured_depth: depth.structured,
                    })
                    .collect()
            } else {
                Vec::new()
            };
            enqueue_tasks_resilient(context, attachment_tasks).await?;
            // The body is still evidence, so this is not deferred like an archive at the same
            // depth; but the record must say the attachments exist and were not followed.
            let withheld_note = (payload.withheld_attachment_count > 0).then(|| {
                format!(
                    "{} attachment(s) not extracted: maximum nested-container depth reached.",
                    payload.withheld_attachment_count
                )
            });
            let text = [
                payload.metadata_summary,
                payload.body_text,
                withheld_note.unwrap_or_default(),
            ]
            .into_iter()
                .filter(|value| !value.trim().is_empty())
                .collect::<Vec<_>>()
                .join("\n\n");
            Ok(ExtractedDocument::Searchable {
                text,
                was_ocr_processed: false,
            })
        }
        DocumentFormat::PlainText => {
            let path = local_path.to_path_buf();
            let is_html = document
                .content_type
                .as_deref()
                .is_some_and(|value| {
                    matches!(
                        value.split(';').next().unwrap_or(value).trim(),
                        "text/html" | "application/xhtml+xml"
                    )
                })
                || document
                    .file_name
                    .as_deref()
                    .is_some_and(|name| {
                        let name = name.to_ascii_lowercase();
                        [".html", ".htm", ".xhtml"]
                            .iter()
                            .any(|extension| name.ends_with(extension))
                    });
            // The PlainText route carries three markup families. SVG needs its own collector:
            // `extract_plain_text` would index `<path d="M0 0 L400 200"/>` as if it were prose, and
            // `extract_html_text` drops every `svg` subtree by design.
            let is_svg = document
                .content_type
                .as_deref()
                .is_some_and(|value| {
                    value.split(';').next().unwrap_or(value).trim() == "image/svg+xml"
                })
                || document
                    .file_name
                    .as_deref()
                    .is_some_and(|name| name.to_ascii_lowercase().ends_with(".svg"));
            let text = spawn_blocking("Text parser", move || {
                if is_svg {
                    extract_svg_text(&path)
                } else if is_html {
                    extract_html_text(&path)
                } else {
                    extract_plain_text(&path)
                }
            })
            .await??;
            Ok(ExtractedDocument::Searchable {
                text,
                was_ocr_processed: false,
            })
        }
        DocumentFormat::RichText => {
            let path = local_path.to_path_buf();
            let text = spawn_blocking("RTF parser", move || extract_rtf_text(&path)).await??;
            Ok(ExtractedDocument::Searchable {
                text,
                was_ocr_processed: false,
            })
        }
        DocumentFormat::Image => {
            let _ocr_permit = context
                .ocr_semaphore
                .clone()
                .acquire_owned()
                .await
                .map_err(|_| AppError::Internal("OCR scheduler is unavailable".to_string()))?;
            let path = local_path.to_path_buf();
            let text = spawn_blocking("OCR", move || execute_local_ocr_scan(&path)).await??;
            Ok(ExtractedDocument::Searchable {
                text,
                was_ocr_processed: true,
            })
        }
        DocumentFormat::WordPackage
        | DocumentFormat::PresentationPackage
        | DocumentFormat::OpenDocumentText
        | DocumentFormat::OpenDocumentPresentation
        | DocumentFormat::Ebook => {
            // Search extraction stays separate from the attributed forensic command, which
            // retains part names, relationship IDs, comments, and revision markers.
            let path = local_path.to_path_buf();
            let text = spawn_blocking("Package parser", move || {
                extract_office_package(&path, format)
            })
            .await??;
            Ok(ExtractedDocument::Searchable {
                text,
                was_ocr_processed: false,
            })
        }
        DocumentFormat::Sqlite => {
            let path = local_path.to_path_buf();
            let text =
                spawn_blocking("SQLite parser", move || extract_sqlite_text(&path)).await??;
            Ok(ExtractedDocument::Searchable {
                text,
                was_ocr_processed: false,
            })
        }
        DocumentFormat::DuckDb => {
            let path = local_path.to_path_buf();
            let text =
                spawn_blocking("DuckDB parser", move || extract_duckdb_text(&path)).await??;
            Ok(ExtractedDocument::Searchable {
                text,
                was_ocr_processed: false,
            })
        }
        DocumentFormat::SqlDump => {
            let path = local_path.to_path_buf();
            let text =
                spawn_blocking("SQL dump parser", move || extract_sql_dump_text(&path)).await??;
            Ok(ExtractedDocument::Searchable {
                text,
                was_ocr_processed: false,
            })
        }
        DocumentFormat::Zip | DocumentFormat::EmailArchive => {
            expand_container(context, input).await
        }
        DocumentFormat::Tar
        | DocumentFormat::TarGzip
        | DocumentFormat::Gzip
        | DocumentFormat::TarBzip2
        | DocumentFormat::Bzip2
        | DocumentFormat::TarXz
        | DocumentFormat::Xz
        | DocumentFormat::TarZstd
        | DocumentFormat::Zstd => {
            expand_container(context, input).await
        }
            _ => Ok(ExtractedDocument::Deferred {
                format,
                route: format.extraction_route(),
            }),
        }
    }
    .await;

    match primary_result {
        Err(error)
            if matches!(
                &error,
                AppError::DataParsing(_) | AppError::OcrRequired { .. }
            ) && salvage_allowed(format) =>
        {
            let path = local_path.to_path_buf();
            let primary_error = error.to_string();
            match spawn_blocking("Salvage parser", move || {
                extract_salvage_report(&path, primary_error)
            })
            .await
            {
                Ok(Ok(report)) => Ok(ExtractedDocument::Salvaged { report, format }),
                _ => Err(error),
            }
        }
        result => result,
    }
}

fn salvage_allowed(format: DocumentFormat) -> bool {
    matches!(
        format,
        DocumentFormat::Pdf
            | DocumentFormat::PlainText
            | DocumentFormat::RichText
            | DocumentFormat::Spreadsheet
            | DocumentFormat::OutlookMessage
            | DocumentFormat::Sqlite
            | DocumentFormat::DuckDb
            | DocumentFormat::SqlDump
    )
}

async fn expand_container(
    context: &WorkerContext,
    input: DocumentExtractionInput<'_>,
) -> Result<ExtractedDocument, AppError> {
    let DocumentExtractionInput {
        request_id,
        document: source_document,
        local_path,
        format,
        depth,
        raw_file_sha256,
        container_root_directory,
    } = input;
    if depth.container >= MAXIMUM_CONTAINER_DEPTH {
        return Ok(ExtractedDocument::Deferred {
            format,
            route: ExtractionRoute::ToolRequired("maximum nested-container depth reached"),
        });
    }
    let parent = local_path.parent().ok_or_else(|| {
        AppError::DataParsing(format!(
            "CONTAINER_PATH_INVALID: '{}' has no parent directory",
            local_path.display()
        ))
    })?;
    let hash_prefix = raw_file_sha256.get(..16).ok_or_else(|| {
        AppError::DataParsing(
            "CONTAINER_HASH_INVALID: raw file SHA-256 is not normalized".to_string(),
        )
    })?;
    let destination = parent.join(format!("{}_{}_contents", source_document.id, hash_prefix));
    let archive_path = local_path.to_path_buf();
    let extraction_destination = destination.clone();
    let extracted_files = spawn_blocking("Container parser", move || {
        if extraction_destination.try_exists().map_err(|error| {
            AppError::Storage(format!(
                "Failed to inspect cached container '{}': {error}",
                extraction_destination.display()
            ))
        })? {
            return existing_container_manifest(&extraction_destination);
        }
        match format {
            DocumentFormat::Zip | DocumentFormat::EmailArchive => {
                extract_zip_container_with_manifest(&archive_path, &extraction_destination)
            }
            _ => extract_archive_container(&archive_path, &extraction_destination, format),
        }
    })
    .await??;

    let root_directory = container_root_directory
        .map(Path::to_path_buf)
        .unwrap_or_else(|| destination.clone());
    if let Err(error) = validate_container_tree_budget(&root_directory) {
        let _ = std::fs::remove_dir_all(&destination);
        return Err(error);
    }

    let mut identifiers = HashSet::with_capacity(extracted_files.len());
    let mut tasks = Vec::with_capacity(extracted_files.len());
    for extracted_path in extracted_files {
        let metadata = std::fs::symlink_metadata(&extracted_path).map_err(|error| {
            AppError::Internal(format!(
                "CONTAINER_ENTRY_INSPECTION_FAILED: failed to inspect '{}': {error}",
                extracted_path.display()
            ))
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(AppError::DataParsing(format!(
                "CONTAINER_ENTRY_INVALID: '{}' is not a regular file",
                extracted_path.display()
            )));
        }
        let relative_path = extracted_path.strip_prefix(&destination).map_err(|error| {
            AppError::Internal(format!(
                "CONTAINER_ENTRY_PATH_INVALID: '{}' is outside '{}': {error}",
                extracted_path.display(),
                destination.display()
            ))
        })?;
        let identifier = archive_entry_id(
            source_document.id,
            raw_file_sha256,
            depth.container,
            relative_path,
        );
        if !identifiers.insert(identifier) {
            let _ = std::fs::remove_dir_all(&destination);
            return Err(AppError::DataParsing(format!(
                "CONTAINER_IDENTIFIER_COLLISION: '{}' produced a duplicate document identifier",
                relative_path.display()
            )));
        }
        let file_name = extracted_path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .ok_or_else(|| {
                AppError::DataParsing(format!(
                    "CONTAINER_ENTRY_NAME_INVALID: '{}' has no file name",
                    extracted_path.display()
                ))
            })?;
        let document = DocumentMetadata {
            id: identifier,
            file_name: Some(file_name),
            file_size: Some(metadata.len()),
            description: Some(format!(
                "Extracted from document {} at {}",
                source_document.id,
                relative_path.display()
            )),
            content_type: None,
            download_url: None,
            created_at: source_document.created_at.clone(),
            updated_at: source_document.updated_at.clone(),
            visibility: source_document.visibility.clone(),
            local_path: Some(extracted_path.to_string_lossy().into_owned()),
            extracted_text_sha256: None,
            was_ocr_processed: false,
        };
        tasks.push(IngestionTask::DownloadBinaryPayload {
            subdomain: context.portal.portal_key.clone(),
            request_id,
            document: Box::new(document),
            output_directory: destination.to_string_lossy().into_owned(),
            container_depth: depth.container + 1,
            container_root_directory: Some(root_directory.to_string_lossy().into_owned()),
            crawl_depth: depth.crawl,
            structured_depth: depth.structured,
        });
    }

    enqueue_tasks_resilient(context, tasks).await?;
    Ok(ExtractedDocument::Expanded {
        format,
        entry_count: identifiers.len(),
    })
}

fn existing_container_manifest(destination: &Path) -> Result<Vec<PathBuf>, AppError> {
    let root_metadata = std::fs::symlink_metadata(destination).map_err(|error| {
        AppError::Storage(format!(
            "Failed to inspect cached container '{}': {error}",
            destination.display()
        ))
    })?;
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return Err(AppError::PortalSecurity(format!(
            "Cached container '{}' is not a managed directory",
            destination.display()
        )));
    }

    let mut directories = vec![destination.to_path_buf()];
    let mut files = Vec::new();
    let mut entry_count = 0_usize;
    while let Some(directory) = directories.pop() {
        for entry in std::fs::read_dir(&directory).map_err(|error| {
            AppError::Storage(format!(
                "Failed to enumerate cached container '{}': {error}",
                directory.display()
            ))
        })? {
            let entry = entry.map_err(|error| {
                AppError::Storage(format!(
                    "Failed to enumerate cached container '{}': {error}",
                    directory.display()
                ))
            })?;
            entry_count = entry_count.checked_add(1).ok_or_else(|| {
                AppError::DataParsing("Cached container entry count overflowed".to_string())
            })?;
            if entry_count > MAXIMUM_CONTAINER_ENTRIES {
                return Err(AppError::DataParsing(format!(
                    "Cached container '{}' exceeds {MAXIMUM_CONTAINER_ENTRIES} entries",
                    destination.display()
                )));
            }
            let path = entry.path();
            let metadata = std::fs::symlink_metadata(&path).map_err(|error| {
                AppError::Storage(format!(
                    "Failed to inspect cached container entry '{}': {error}",
                    path.display()
                ))
            })?;
            if metadata.file_type().is_symlink() {
                return Err(AppError::PortalSecurity(format!(
                    "Cached container entry '{}' is a symbolic link",
                    path.display()
                )));
            }
            if metadata.is_dir() {
                directories.push(path);
            } else if metadata.is_file() {
                files.push(path);
            } else {
                return Err(AppError::PortalSecurity(format!(
                    "Cached container entry '{}' is not a regular file or directory",
                    path.display()
                )));
            }
        }
    }
    files.sort();
    Ok(files)
}

/// Measures the real managed extraction tree without following links. Unlike each format parser's
/// per-archive expansion ceiling, this bound covers every recursively extracted descendant and is
/// recomputed from disk, so a durable child task cannot reset it by resuming after process restart.
fn validate_container_tree_budget(root: &Path) -> Result<u64, AppError> {
    validate_container_tree_budget_with_limits(
        root,
        MAXIMUM_CONTAINER_TREE_ENTRIES,
        MAXIMUM_CONTAINER_TREE_BYTES,
    )
}

fn validate_container_tree_budget_with_limits(
    root: &Path,
    maximum_entries: usize,
    maximum_bytes: u64,
) -> Result<u64, AppError> {
    let root_metadata = std::fs::symlink_metadata(root).map_err(|error| {
        AppError::Storage(format!(
            "CONTAINER_TREE_INSPECTION_FAILED: failed to inspect '{}': {error}",
            root.display()
        ))
    })?;
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return Err(AppError::PortalSecurity(format!(
            "CONTAINER_TREE_INVALID: '{}' is not a managed directory",
            root.display()
        )));
    }

    let mut directories = vec![root.to_path_buf()];
    let mut entry_count = 0_usize;
    let mut total_bytes = 0_u64;
    while let Some(directory) = directories.pop() {
        for entry in std::fs::read_dir(&directory).map_err(|error| {
            AppError::Storage(format!(
                "CONTAINER_TREE_ENUMERATION_FAILED: failed to enumerate '{}': {error}",
                directory.display()
            ))
        })? {
            let entry = entry.map_err(|error| {
                AppError::Storage(format!(
                    "CONTAINER_TREE_ENUMERATION_FAILED: failed to enumerate '{}': {error}",
                    directory.display()
                ))
            })?;
            entry_count = entry_count.checked_add(1).ok_or_else(|| {
                AppError::DataParsing(
                    "CONTAINER_TREE_ENTRY_COUNT_OVERFLOW: entry count overflowed".to_string(),
                )
            })?;
            if entry_count > maximum_entries {
                return Err(AppError::DataParsing(format!(
                    "CONTAINER_TREE_ENTRY_COUNT_EXCEEDED: recursive extraction exceeds {maximum_entries} entries"
                )));
            }
            let path = entry.path();
            let metadata = std::fs::symlink_metadata(&path).map_err(|error| {
                AppError::Storage(format!(
                    "CONTAINER_TREE_ENTRY_INSPECTION_FAILED: failed to inspect '{}': {error}",
                    path.display()
                ))
            })?;
            if metadata.file_type().is_symlink() {
                return Err(AppError::PortalSecurity(format!(
                    "CONTAINER_TREE_ENTRY_INVALID: '{}' is a symbolic link",
                    path.display()
                )));
            }
            if metadata.is_dir() {
                directories.push(path);
                continue;
            }
            if !metadata.is_file() {
                return Err(AppError::PortalSecurity(format!(
                    "CONTAINER_TREE_ENTRY_INVALID: '{}' is not a regular file or directory",
                    path.display()
                )));
            }
            total_bytes = total_bytes.checked_add(metadata.len()).ok_or_else(|| {
                AppError::DataParsing(
                    "CONTAINER_TREE_SIZE_OVERFLOW: recursive extraction size overflowed"
                        .to_string(),
                )
            })?;
            if total_bytes > maximum_bytes {
                return Err(AppError::DataParsing(format!(
                    "CONTAINER_TREE_SIZE_EXCEEDED: recursive extraction exceeds {maximum_bytes} bytes"
                )));
            }
        }
    }
    Ok(total_bytes)
}

fn archive_entry_id(
    source_document_id: u64,
    raw_file_sha256: &str,
    container_depth: usize,
    relative_path: &Path,
) -> u64 {
    let mut digest = Sha256::new();
    digest.update(b"onrecord-container-entry-v1");
    digest.update(source_document_id.to_le_bytes());
    digest.update((container_depth as u64).to_le_bytes());
    digest.update(raw_file_sha256.as_bytes());
    digest.update(relative_path.as_os_str().as_encoded_bytes());
    let hash = digest.finalize();
    let mut identifier_bytes = [0_u8; 8];
    identifier_bytes.copy_from_slice(&hash[..8]);
    let identifier = u64::from_le_bytes(identifier_bytes);
    SYNTHETIC_DOCUMENT_NAMESPACE | (identifier & (SYNTHETIC_DOCUMENT_NAMESPACE - 1))
}

fn extraction_route_label(route: ExtractionRoute) -> String {
    match route {
        ExtractionRoute::Container(name) => format!("container extractor required: {name}"),
        ExtractionRoute::ToolRequired(name) => format!("specialist tool required: {name}"),
        ExtractionRoute::KeyRequired => "decryption key required".to_string(),
        ExtractionRoute::Unknown => "unrecognized document format".to_string(),
        ExtractionRoute::DuckDb | ExtractionRoute::SqlDump => {
            "trusted database extractor unexpectedly deferred".to_string()
        }
        _ => "native extractor unavailable".to_string(),
    }
}

async fn enqueue_task_resilient(
    context: &WorkerContext,
    task: IngestionTask,
) -> Result<(), AppError> {
    enqueue_tasks_resilient(context, vec![task]).await
}

async fn enqueue_tasks_resilient(
    context: &WorkerContext,
    tasks: Vec<IngestionTask>,
) -> Result<(), AppError> {
    if tasks.is_empty() {
        return Ok(());
    }
    let mut encoded = Vec::with_capacity(tasks.len());
    for task in tasks {
        let target_id = task.target_id();
        let payload = serde_json::to_string(&task).map_err(|error| {
            AppError::Internal(format!(
                "Failed to encode a durable ingestion task: {error}"
            ))
        })?;
        encoded.push((target_id, payload));
    }
    // Counted before the rows become visible. Once the batch commits, any worker's refill can
    // claim a row and its dequeue decrements `queued_tasks`; a decrement that lands before this
    // increment saturates at zero and leaves the counter one too high for the rest of the run,
    // and `retire_portal_if_idle` then never sees the pool idle. Rows the UNIQUE constraint
    // ignored, or a failed batch, are given back below.
    let reserved = encoded.len();
    context
        .statistics
        .queued_tasks
        .fetch_add(reserved, Ordering::AcqRel);
    let inserted = match context
        .repository
        .persist_ingestion_frontier_tasks(
            &context.sync_run_id,
            &context.portal.portal_key,
            &encoded,
        )
        .await
    {
        Ok(inserted) => usize::try_from(inserted).unwrap_or(reserved).min(reserved),
        Err(error) => {
            context
                .statistics
                .queued_tasks
                .fetch_sub(reserved, Ordering::AcqRel);
            return Err(error);
        }
    };
    if inserted < reserved {
        context
            .statistics
            .queued_tasks
            .fetch_sub(reserved - inserted, Ordering::AcqRel);
    }
    if inserted > 0 {
        refill_ingestion_frontier(context).await?;
    }
    Ok(())
}

async fn refill_ingestion_frontier(context: &WorkerContext) -> Result<(), AppError> {
    // Claim and channel reservation must remain serialized per portal. Concurrent refillers could
    // otherwise lease more rows than the bounded dispatcher can accept.
    let _refill_guard = context.crawl.frontier_refill_lock.lock().await;
    loop {
        let permit = match context.dispatcher.try_reserve() {
            Ok(permit) => permit,
            Err(mpsc::error::TrySendError::Full(_)) => return Ok(()),
            Err(mpsc::error::TrySendError::Closed(_)) => {
                return Err(AppError::Internal(
                    "Portal task queue closed while refilling its durable frontier".to_string(),
                ));
            }
        };
        let Some(claimed) = context
            .repository
            .claim_ingestion_frontier_task(&context.sync_run_id, &context.portal.portal_key)
            .await?
        else {
            return Ok(());
        };
        let task: IngestionTask = match serde_json::from_str(&claimed.payload_json) {
            Ok(task) => task,
            Err(error) => {
                context
                    .repository
                    .complete_ingestion_frontier_task(claimed.frontier_id)
                    .await?;
                decrement_atomic(&context.statistics.queued_tasks);
                return Err(AppError::Storage(format!(
                    "Stored ingestion frontier task {} is invalid and was quarantined: {error}",
                    claimed.frontier_id
                )));
            }
        };
        if task.portal_key() != context.portal.portal_key {
            let wrong_source = task.portal_key().to_string();
            context
                .repository
                .complete_ingestion_frontier_task(claimed.frontier_id)
                .await?;
            decrement_atomic(&context.statistics.queued_tasks);
            return Err(AppError::PortalSecurity(format!(
                "Stored ingestion frontier task {} belonged to '{wrong_source}' rather than '{}' and was quarantined",
                claimed.frontier_id, context.portal.portal_key
            )));
        }
        // Retries scheduled in-process stop at the cap, so a lease beyond it means every earlier
        // attempt was interrupted, typically because processing the task ended the process.
        // Leasing it again would repeat that on every launch.
        if claimed.attempts > MAXIMUM_DURABLE_TASK_ATTEMPTS {
            context
                .repository
                .complete_ingestion_frontier_task(claimed.frontier_id)
                .await?;
            decrement_atomic(&context.statistics.queued_tasks);
            let error = AppError::Internal(format!(
                "Ingestion task was interrupted on {} consecutive attempts and was abandoned",
                claimed.attempts - 1
            ));
            context
                .statistics
                .failed_tasks
                .fetch_add(1, Ordering::AcqRel);
            context
                .repository
                .record_source_task_failure(
                    &context.sync_run_id,
                    &context.portal.portal_key,
                    &task.target_id(),
                    &error,
                )
                .await?;
            continue;
        }
        permit.send(ScheduledTask {
            task,
            frontier_id: Some(claimed.frontier_id),
            frontier_attempt: claimed.attempts,
        });
    }
}

#[cfg(test)]
fn enqueue_task(
    sender: &mpsc::Sender<ScheduledTask>,
    statistics: &PortalStatistics,
    task: IngestionTask,
) -> Result<(), AppError> {
    statistics.queued_tasks.fetch_add(1, Ordering::AcqRel);
    sender
        .try_send(ScheduledTask {
            task,
            frontier_id: None,
            frontier_attempt: 0,
        })
        .map_err(|error| {
            decrement_atomic(&statistics.queued_tasks);
            match error {
                mpsc::error::TrySendError::Full(_) => AppError::PortalCapacity(
                    "Portal task queue reached its bounded backpressure limit".to_string(),
                ),
                mpsc::error::TrySendError::Closed(_) => {
                    AppError::Internal("Portal task queue is closed".to_string())
                }
            }
        })
}

fn decrement_atomic(value: &AtomicUsize) {
    let _ = value.fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
        Some(current.saturating_sub(1))
    });
}

/// The seed a pool start actually enqueues, once the adopted frontier has been consulted.
///
/// Dropping the request-index seed is confined to a resume, which is the continuation of an
/// earlier start rather than a new instruction. A source paused at page 900 already carries page
/// 901 in its frontier, so seeding its configured start page re-fetches all 900 pages in between;
/// deduplication makes that correct but not free, and the cost lands on a portal that rate-limits.
///
/// The seed stands in the two cases that are not that one. An operator-requested start is an
/// instruction: they chose the page window on this start, and an inherited frontier does not
/// outrank it. And a walk that ran to the end leaves no page row at all, so `walk_interrupted` is
/// false there and the source re-enters at its start page, which is how records filed since the
/// last run are found. Recording a high-water page instead would have ended that: page numbers
/// here are positions in whatever order the portal returns, not cursors, and seeding past the last
/// page walked means never looking at the first page again.
///
/// Only the request-index seed is dropped. A crawl seed is left alone deliberately; a crawl's
/// visited set is rebuilt per pool and its re-walk is a separate, unclosed question.
fn pool_seed_tasks(
    initial_tasks: Vec<IngestionTask>,
    seed_policy: PoolSeedPolicy,
    walk_interrupted: bool,
) -> Vec<IngestionTask> {
    if seed_policy == PoolSeedPolicy::OperatorRequested || !walk_interrupted {
        return initial_tasks;
    }
    initial_tasks
        .into_iter()
        .filter(|task| !matches!(task, IngestionTask::FetchMetadataPage { .. }))
        .collect()
}

/// The seed a resume adds alongside whatever the durable frontier already holds.
///
/// Split out so the window a resume applies is provable without provisioning a pool: the seed is
/// enqueued unconditionally by `start_task_pool`, so a resume that ignored the operator's saved
/// window would re-fetch page 1 at size 50 and walk the range they started past.
fn resume_initial_tasks(
    source: &PortalRegistration,
    window: MetadataPageWindow,
) -> Vec<IngestionTask> {
    match source.source_kind {
        SourceKind::NextRequest => vec![IngestionTask::FetchMetadataPage {
            subdomain: source.portal_key.clone(),
            page: window.start_page,
            per_page: window.page_size,
        }],
        SourceKind::LocalFile | SourceKind::PastedContent => Vec::new(),
        _ => vec![IngestionTask::CrawlPublicResource {
            subdomain: source.portal_key.clone(),
            url: source.canonical_url.clone(),
            depth: 0,
        }],
    }
}

fn validate_page_parameters(page: usize, per_page: usize) -> Result<(), AppError> {
    if page == 0 || page > MAXIMUM_METADATA_PAGE {
        return Err(AppError::DataParsing(format!(
            "Metadata page must be between 1 and {MAXIMUM_METADATA_PAGE}"
        )));
    }
    if !(1..=100).contains(&per_page) {
        return Err(AppError::DataParsing(
            "Metadata page size must be between 1 and 100".to_string(),
        ));
    }
    Ok(())
}

fn validate_internal_output_directory(root: &Path, output: &Path) -> Result<(), AppError> {
    if output.as_os_str().is_empty()
        || output
            .components()
            .any(|component| matches!(component, Component::ParentDir))
        || !output.starts_with(root)
    {
        return Err(AppError::PortalSecurity(format!(
            "Document output directory '{}' escapes the managed download root",
            output.display()
        )));
    }
    Ok(())
}

/// The digest a file in the content-addressed object store was committed under, if `path` is one.
fn content_address<'a>(download_root: &Path, path: &'a Path) -> Option<&'a str> {
    let relative = path.strip_prefix(download_root.join("objects")).ok()?;
    let mut components = relative.components();
    let prefix = components.next()?.as_os_str().to_str()?;
    let digest = components.next()?.as_os_str().to_str()?;
    (components.next().is_none()
        && digest.len() == 64
        && digest
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        && digest.starts_with(prefix))
    .then_some(digest)
}

async fn validate_cached_payload(
    managed_root: &Path,
    path: &Path,
    expected_size: Option<u64>,
) -> Result<(PathBuf, u64), AppError> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
        || !path.starts_with(managed_root)
    {
        return Err(AppError::PortalSecurity(format!(
            "Cached payload '{}' escapes the managed download root",
            path.display()
        )));
    }
    let metadata = tokio::fs::symlink_metadata(path).await.map_err(|error| {
        AppError::Storage(format!(
            "Failed to inspect cached payload '{}': {error}",
            path.display()
        ))
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(AppError::DataParsing(format!(
            "Cached payload '{}' is not a regular file",
            path.display()
        )));
    }
    if expected_size.is_some_and(|expected| metadata.len() != expected) {
        return Err(AppError::DataParsing(format!(
            "Cached payload '{}' size does not match its metadata",
            path.display()
        )));
    }
    Ok((path.to_path_buf(), metadata.len()))
}

#[cfg(test)]
async fn persist_download(
    output_directory: &Path,
    document_id: u64,
    original_file_name: &str,
    bytes: &[u8],
) -> Result<PathBuf, AppError> {
    tokio::fs::create_dir_all(output_directory)
        .await
        .map_err(|error| {
            AppError::Storage(format!(
                "Failed to create download directory '{}': {error}",
                output_directory.display()
            ))
        })?;
    let file_name = sanitize_file_name(original_file_name, document_id);
    let final_path = output_directory.join(format!("{document_id}_{file_name}"));
    if tokio::fs::try_exists(&final_path)
        .await
        .map_err(|error| AppError::Storage(format!("Failed to inspect download: {error}")))?
    {
        let existing_hash = sha256_file(&final_path).await?;
        if constant_time_equal(existing_hash.as_bytes(), sha256_hex(bytes).as_bytes()) {
            return Ok(final_path);
        }
        return Err(AppError::Storage(format!(
            "Download collision at '{}': existing bytes differ",
            final_path.display()
        )));
    }
    let temporary_path = output_directory.join(format!(".{document_id}.{}.tmp", Uuid::new_v4()));
    let mut temporary_file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary_path)
        .await
        .map_err(|error| {
            AppError::Storage(format!("Failed to create temporary download: {error}"))
        })?;
    if let Err(error) = temporary_file.write_all(bytes).await {
        drop(temporary_file);
        let _ = tokio::fs::remove_file(&temporary_path).await;
        return Err(AppError::Storage(format!(
            "Failed to write temporary download: {error}"
        )));
    }
    if let Err(error) = temporary_file.sync_all().await {
        drop(temporary_file);
        let _ = tokio::fs::remove_file(&temporary_path).await;
        return Err(AppError::Storage(format!(
            "Failed to sync temporary download: {error}"
        )));
    }
    drop(temporary_file);
    match tokio::fs::hard_link(&temporary_path, &final_path).await {
        Ok(()) => {
            tokio::fs::remove_file(&temporary_path)
                .await
                .map_err(|error| {
                    AppError::Storage(format!("Failed to remove temporary link: {error}"))
                })?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let _ = tokio::fs::remove_file(&temporary_path).await;
            let existing_hash = sha256_file(&final_path).await?;
            if !constant_time_equal(existing_hash.as_bytes(), sha256_hex(bytes).as_bytes()) {
                return Err(AppError::Storage(format!(
                    "Concurrent download collision at '{}'",
                    final_path.display()
                )));
            }
        }
        Err(error) => {
            let _ = tokio::fs::remove_file(&temporary_path).await;
            return Err(AppError::Storage(format!(
                "Failed to commit download: {error}"
            )));
        }
    }
    Ok(final_path)
}

async fn stream_download(
    client: &ApiClient,
    portal: &PortalRegistration,
    download_url: &str,
    managed_root: &Path,
    document_id: u64,
) -> Result<DownloadedPayload, AppError> {
    let staging_directory = managed_root.join(".staging");
    tokio::fs::create_dir_all(&staging_directory)
        .await
        .map_err(|error| {
            AppError::Storage(format!(
                "Failed to create download staging directory '{}': {error}",
                staging_directory.display()
            ))
        })?;
    let temporary_path =
        staging_directory.join(format!(".{document_id}.{}.partial", Uuid::new_v4()));
    let downloaded = client
        .download_document_to_path(portal, download_url, &temporary_path)
        .await?;
    if downloaded.byte_count == 0 {
        let _ = tokio::fs::remove_file(&temporary_path).await;
        return Err(AppError::DataParsing(format!(
            "Document {document_id} download was empty"
        )));
    }
    let path = commit_staged_object(managed_root, &temporary_path, &downloaded.sha256).await?;
    Ok(DownloadedPayload { path, ..downloaded })
}

async fn import_local_payload(
    download_root: &Path,
    source_path: &Path,
    document_id: u64,
) -> Result<(PathBuf, u64), AppError> {
    let metadata = tokio::fs::symlink_metadata(source_path)
        .await
        .map_err(|error| {
            AppError::Storage(format!(
                "Failed to inspect selected file '{}': {error}",
                source_path.display()
            ))
        })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(AppError::DataParsing(format!(
            "Selected path '{}' is not a regular file",
            source_path.display()
        )));
    }
    if metadata.len() > MAXIMUM_LOCAL_IMPORT_BYTES {
        return Err(AppError::DataParsing(format!(
            "Selected file '{}' exceeds the {}-byte limit",
            source_path.display(),
            MAXIMUM_LOCAL_IMPORT_BYTES
        )));
    }
    let staging_directory = download_root.join(".staging");
    tokio::fs::create_dir_all(&staging_directory)
        .await
        .map_err(|error| {
            AppError::Storage(format!("Failed to create import staging area: {error}"))
        })?;
    let staging_path = staging_directory.join(format!("{document_id}.{}.local", Uuid::new_v4()));
    let mut source = tokio::fs::File::open(source_path).await.map_err(|error| {
        AppError::Storage(format!(
            "Failed to open selected file '{}': {error}",
            source_path.display()
        ))
    })?;
    let mut destination = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staging_path)
        .await
        .map_err(|error| AppError::Storage(format!("Failed to create staged import: {error}")))?;
    let mut digest = Sha256::new();
    let mut byte_count = 0_u64;
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let count = source.read(&mut buffer).await.map_err(|error| {
            AppError::Storage(format!("Failed while reading selected file: {error}"))
        })?;
        if count == 0 {
            break;
        }
        byte_count = byte_count.checked_add(count as u64).ok_or_else(|| {
            AppError::DataParsing("Selected file byte count overflowed".to_string())
        })?;
        if byte_count > MAXIMUM_LOCAL_IMPORT_BYTES {
            drop(destination);
            let _ = tokio::fs::remove_file(&staging_path).await;
            return Err(AppError::DataParsing(format!(
                "Selected file exceeded the {MAXIMUM_LOCAL_IMPORT_BYTES}-byte limit while reading"
            )));
        }
        destination
            .write_all(&buffer[..count])
            .await
            .map_err(|error| {
                AppError::Storage(format!("Failed while staging selected file: {error}"))
            })?;
        digest.update(&buffer[..count]);
    }
    destination.sync_all().await.map_err(|error| {
        AppError::Storage(format!("Failed to sync staged selected file: {error}"))
    })?;
    drop(destination);
    let sha256 = digest_hex(digest.finalize());
    let final_path = commit_staged_object(download_root, &staging_path, &sha256).await?;
    Ok((final_path, byte_count))
}

async fn store_inline_payload(
    download_root: &Path,
    document_id: u64,
    bytes: &[u8],
) -> Result<PathBuf, AppError> {
    let staging_directory = download_root.join(".staging");
    tokio::fs::create_dir_all(&staging_directory)
        .await
        .map_err(|error| {
            AppError::Storage(format!("Failed to create paste staging area: {error}"))
        })?;
    let staging_path = staging_directory.join(format!("{document_id}.{}.paste", Uuid::new_v4()));
    let mut destination = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staging_path)
        .await
        .map_err(|error| AppError::Storage(format!("Failed to create staged paste: {error}")))?;
    destination.write_all(bytes).await.map_err(|error| {
        AppError::Storage(format!("Failed to write staged pasted content: {error}"))
    })?;
    destination.sync_all().await.map_err(|error| {
        AppError::Storage(format!("Failed to sync staged pasted content: {error}"))
    })?;
    drop(destination);
    commit_staged_object(download_root, &staging_path, &sha256_hex(bytes)).await
}

/// Objects an ingestion task linked, or found already present, and may not yet reference from a
/// committed row. Purge unlinks only under this lock and skips these, so it can never remove a file
/// between a concurrent sync's link and that sync's row; the grace outlasts any task.
static RECENT_OBJECT_LINKS: std::sync::LazyLock<Mutex<HashMap<PathBuf, std::time::Instant>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));
const RECENT_OBJECT_LINK_GRACE: Duration = Duration::from_secs(6 * 60 * 60);

/// Unlinks evidence files a committed purge released, except any a running sync has linked and
/// not yet referenced. Returns the paths it left in place, so a caller never reports them deleted.
pub(crate) async fn remove_released_evidence_files(paths: &[String]) -> Vec<String> {
    let mut recent = RECENT_OBJECT_LINKS.lock().await;
    recent.retain(|_, linked_at| linked_at.elapsed() < RECENT_OBJECT_LINK_GRACE);
    let mut kept = Vec::new();
    for path in paths {
        if recent.contains_key(Path::new(path)) {
            kept.push(path.clone());
            continue;
        }
        let _ = tokio::fs::remove_file(path).await;
    }
    kept
}

/// A committed row now references `path`, so the purge-time reference check covers it.
async fn release_object_link(path: &Path) {
    RECENT_OBJECT_LINKS.lock().await.remove(path);
}

async fn commit_staged_object(
    download_root: &Path,
    staging_path: &Path,
    sha256: &str,
) -> Result<PathBuf, AppError> {
    let prefix = sha256.get(..2).ok_or_else(|| {
        AppError::Internal("Content-addressed digest was not normalized".to_string())
    })?;
    let object_directory = download_root.join("objects").join(prefix);
    tokio::fs::create_dir_all(&object_directory)
        .await
        .map_err(|error| AppError::Storage(format!("Failed to create object store: {error}")))?;
    let final_path = object_directory.join(sha256);
    let linked = {
        let mut recent = RECENT_OBJECT_LINKS.lock().await;
        let linked = tokio::fs::hard_link(staging_path, &final_path).await;
        if linked.as_ref().map_or_else(
            |error| error.kind() == std::io::ErrorKind::AlreadyExists,
            |()| true,
        ) {
            recent.insert(final_path.clone(), std::time::Instant::now());
        }
        linked
    };
    match linked {
        Ok(()) => {
            // The object's bytes were synced while staged; its new directory entry is not durable
            // until the directory is, and a committed row may point at it after power loss.
            let linked = final_path.clone();
            let _ =
                tokio::task::spawn_blocking(move || crate::atomic_write::sync_directory(&linked))
                    .await;
            tokio::fs::remove_file(staging_path)
                .await
                .map_err(|error| {
                    AppError::Storage(format!("Failed to clear staged input: {error}"))
                })?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let existing_hash = sha256_file(&final_path).await?;
            let _ = tokio::fs::remove_file(staging_path).await;
            if !constant_time_equal(existing_hash.as_bytes(), sha256.as_bytes()) {
                return Err(AppError::Storage(format!(
                    "Content-addressed collision at '{}'",
                    final_path.display()
                )));
            }
        }
        Err(error) => {
            let _ = tokio::fs::remove_file(staging_path).await;
            return Err(AppError::Storage(format!(
                "Failed to commit staged input: {error}"
            )));
        }
    }
    Ok(final_path)
}

fn sanitize_file_name(file_name: &str, document_id: u64) -> String {
    let sanitized = sanitized_component(file_name, 180);
    if sanitized.is_empty() {
        format!("document_{document_id}.bin")
    } else {
        sanitized
    }
}

async fn sha256_file(path: &Path) -> Result<String, AppError> {
    sha256_file_async(path)
        .await
        .map_err(|error| AppError::Storage(format!("Failed to hash '{}': {error}", path.display())))
}

fn parser_memory_permits(source_bytes: u64, working_set_multiplier: u32) -> u32 {
    let estimated_bytes = source_bytes
        .saturating_mul(u64::from(working_set_multiplier.max(1)))
        .max(PARSER_MEMORY_PERMIT_BYTES);
    estimated_bytes
        .saturating_add(PARSER_MEMORY_PERMIT_BYTES - 1)
        .checked_div(PARSER_MEMORY_PERMIT_BYTES)
        .unwrap_or(u64::from(PARSER_MEMORY_PERMITS))
        .clamp(1, u64::from(PARSER_MEMORY_PERMITS)) as u32
}

fn validate_crawl_policy(policy: &CrawlPolicy) -> Result<(), AppError> {
    if !(1..=100_000).contains(&policy.maximum_pages) {
        return Err(AppError::DataParsing(
            "Crawl page limit must be between 1 and 100000".to_string(),
        ));
    }
    if policy.maximum_depth > 32 {
        return Err(AppError::DataParsing(
            "Crawl depth must not exceed 32".to_string(),
        ));
    }
    if !(1..=1_000_000).contains(&policy.maximum_resources) {
        return Err(AppError::DataParsing(
            "Crawl resource limit must be between 1 and 1000000".to_string(),
        ));
    }
    Ok(())
}

fn validate_source_guidance(guidance: &SourceGuidance) -> Result<(), AppError> {
    if guidance.additional_seed_urls.len() > 100
        || guidance.allowed_hostnames.len() > 100
        || guidance.include_url_prefixes.len() > 100
        || guidance.exclude_url_prefixes.len() > 100
    {
        return Err(AppError::DataParsing(
            "Source repair guidance contains too many rules".to_string(),
        ));
    }
    for value in guidance
        .additional_seed_urls
        .iter()
        .chain(guidance.include_url_prefixes.iter())
        .chain(guidance.exclude_url_prefixes.iter())
    {
        if value.trim().is_empty() || value.len() > 4_096 {
            return Err(AppError::DataParsing(
                "Source repair URLs and prefixes must contain between 1 and 4096 characters"
                    .to_string(),
            ));
        }
    }
    for hostname in &guidance.allowed_hostnames {
        if hostname.trim().is_empty()
            || hostname.len() > 253
            || !hostname
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
        {
            return Err(AppError::DataParsing(format!(
                "Source repair hostname '{hostname}' is invalid"
            )));
        }
    }
    for selector in [
        guidance.link_selector.as_deref(),
        guidance.download_selector.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        if selector.trim().is_empty() || selector.len() > 1_024 {
            return Err(AppError::DataParsing(
                "Source repair selectors must contain between 1 and 1024 characters".to_string(),
            ));
        }
        scraper::Selector::parse(selector).map_err(|error| {
            AppError::DataParsing(format!("Source repair selector is invalid: {error}"))
        })?;
    }
    if guidance.include_filename_globs.len() > 100
        || guidance.exclude_filename_globs.len() > 100
        || guidance.include_filename_patterns.len() > 100
        || guidance.exclude_filename_patterns.len() > 100
        || guidance.include_content_keywords.len() > 100
        || guidance.exclude_content_keywords.len() > 100
        || guidance.include_content_patterns.len() > 100
        || guidance.exclude_content_patterns.len() > 100
        || guidance.include_organizations.len() > 100
        || guidance.exclude_organizations.len() > 100
    {
        return Err(AppError::DataParsing(
            "Source repair guidance contains too many rules".to_string(),
        ));
    }
    for value in guidance
        .include_filename_globs
        .iter()
        .chain(guidance.exclude_filename_globs.iter())
        .chain(guidance.include_filename_patterns.iter())
        .chain(guidance.exclude_filename_patterns.iter())
        .chain(guidance.include_content_keywords.iter())
        .chain(guidance.exclude_content_keywords.iter())
        .chain(guidance.include_content_patterns.iter())
        .chain(guidance.exclude_content_patterns.iter())
        .chain(guidance.include_organizations.iter())
        .chain(guidance.exclude_organizations.iter())
    {
        if value.trim().is_empty() || value.len() > 1_024 {
            return Err(AppError::DataParsing(
                "Source repair scope rules must contain between 1 and 1024 characters".to_string(),
            ));
        }
    }
    // Regex patterns are compiled here so a bad one is refused at save time with a message that
    // names the pattern, rather than silently contributing nothing once the crawl is already
    // running. `CompiledGuidancePatterns::compile` still tolerates an invalid pattern at runtime
    // (dropping it rather than panicking), which covers guidance saved before this check existed.
    for pattern in guidance
        .include_filename_patterns
        .iter()
        .chain(guidance.exclude_filename_patterns.iter())
        .chain(guidance.include_content_patterns.iter())
        .chain(guidance.exclude_content_patterns.iter())
    {
        Regex::new(pattern).map_err(|error| {
            AppError::DataParsing(format!(
                "Source repair pattern '{pattern}' is invalid: {error}"
            ))
        })?;
    }
    if let (Some(minimum), Some(maximum)) = (
        guidance.minimum_file_size_bytes,
        guidance.maximum_file_size_bytes,
    ) {
        if minimum > maximum {
            return Err(AppError::DataParsing(
                "Source repair minimum file size must not exceed the maximum".to_string(),
            ));
        }
    }
    let date_from = guidance
        .document_date_from
        .as_deref()
        .map(|value| {
            NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|error| {
                AppError::DataParsing(format!(
                    "Source repair document start date '{value}' is invalid: {error}"
                ))
            })
        })
        .transpose()?;
    let date_to = guidance
        .document_date_to
        .as_deref()
        .map(|value| {
            NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|error| {
                AppError::DataParsing(format!(
                    "Source repair document end date '{value}' is invalid: {error}"
                ))
            })
        })
        .transpose()?;
    if date_from.zip(date_to).is_some_and(|(from, to)| from > to) {
        return Err(AppError::DataParsing(
            "Source repair document start date must not be after the end date".to_string(),
        ));
    }
    Ok(())
}

/// A minimal glob matcher: `*` matches any run of characters (including none), every other byte
/// matches literally. Sufficient for filename rules like `*.pdf` or `budget-*-final.xlsx` without
/// pulling in a glob crate for a single wildcard character.
///
/// Iterative with a single backtrack point, so it is O(pattern x text). The filename is taken from
/// a URL a remote site chose, and this runs on a worker thread with no await point: a recursive
/// matcher takes exponential time on `*a*a*a*a*a*b` against a long run of `a`.
fn glob_matches(pattern: &str, text: &str) -> bool {
    let (pattern, text) = (pattern.as_bytes(), text.as_bytes());
    let (mut pattern_index, mut text_index) = (0, 0);
    // Where to resume after the most recent `*`: the pattern just past it, and the next text
    // position it should swallow.
    let mut resume: Option<(usize, usize)> = None;
    while text_index < text.len() {
        if pattern.get(pattern_index) == Some(&b'*') {
            pattern_index += 1;
            resume = Some((pattern_index, text_index));
        } else if pattern.get(pattern_index) == Some(&text[text_index]) {
            pattern_index += 1;
            text_index += 1;
        } else if let Some((resume_pattern, resume_text)) = resume {
            pattern_index = resume_pattern;
            text_index = resume_text + 1;
            resume = Some((resume_pattern, text_index));
        } else {
            return false;
        }
    }
    pattern[pattern_index..].iter().all(|byte| *byte == b'*')
}

/// The immediate last path segment of a URL, treated as the filename a `SourceGuidance` filename
/// rule evaluates against. Mirrors `client::file_name_from_url`'s convention: a trailing slash
/// (a directory, not a file) is an empty segment and yields `None`, never the directory name one
/// level up -- an include-glob like `*.pdf` must not silently prune a crawl into `/reports/2024/`
/// because "2024" was mistaken for a filename. A non-URL value falls back to its immediate last
/// slash-delimited segment for compatibility with the pre-parser path. Absence means the filename
/// predicates contribute nothing, not that they exclude, per the absence-versus-measurement rule.
fn filename_from_url(value: &str) -> Option<String> {
    let last_segment = reqwest::Url::parse(value)
        .ok()
        .and_then(|url| {
            url.path_segments()
                .and_then(|mut segments| segments.next_back())
                .map(str::to_string)
        })
        .unwrap_or_else(|| value.rsplit('/').next().unwrap_or(value).to_string());
    (!last_segment.is_empty()).then_some(last_segment)
}

/// Whether a discovered URL survives the source's saved scoping guidance, naming the specific
/// rule that refused it. `content_length` is `Some` only at call sites where a size is already
/// known pre-download (a catalog listing that reports it); elsewhere it is `None` and the size
/// predicate contributes nothing rather than excluding on an unmeasured value.
fn guidance_allows_url(
    guidance: &SourceGuidance,
    compiled: &CompiledGuidancePatterns,
    value: &str,
    content_length: Option<u64>,
) -> Result<(), FrontierExclusion> {
    if guidance
        .exclude_url_prefixes
        .iter()
        .any(|prefix| value.starts_with(prefix))
    {
        return Err(FrontierExclusion::Guidance);
    }
    if !guidance.include_url_prefixes.is_empty()
        && !guidance
            .include_url_prefixes
            .iter()
            .any(|prefix| value.starts_with(prefix))
    {
        return Err(FrontierExclusion::Guidance);
    }
    if !guidance.allowed_hostnames.is_empty() {
        let host_allowed = reqwest::Url::parse(value)
            .ok()
            .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
            .is_some_and(|host| {
                guidance.allowed_hostnames.iter().any(|allowed| {
                    let allowed = allowed.to_ascii_lowercase();
                    host == allowed || host.ends_with(&format!(".{allowed}"))
                })
            });
        if !host_allowed {
            return Err(FrontierExclusion::Guidance);
        }
    }
    if let Some(filename) = filename_from_url(value) {
        let filename_lower = filename.to_ascii_lowercase();
        let excluded = guidance
            .exclude_filename_globs
            .iter()
            .any(|pattern| glob_matches(&pattern.to_ascii_lowercase(), &filename_lower))
            || compiled
                .exclude_filename_patterns
                .iter()
                .any(|pattern| pattern.is_match(&filename));
        if excluded {
            return Err(FrontierExclusion::FilenamePattern);
        }
        let has_includes = !guidance.include_filename_globs.is_empty()
            || !compiled.include_filename_patterns.is_empty();
        if has_includes {
            let included = guidance
                .include_filename_globs
                .iter()
                .any(|pattern| glob_matches(&pattern.to_ascii_lowercase(), &filename_lower))
                || compiled
                    .include_filename_patterns
                    .iter()
                    .any(|pattern| pattern.is_match(&filename));
            if !included {
                return Err(FrontierExclusion::FilenamePattern);
            }
        }
    }
    if let Some(length) = content_length {
        let out_of_range = guidance
            .minimum_file_size_bytes
            .is_some_and(|minimum| length < minimum)
            || guidance
                .maximum_file_size_bytes
                .is_some_and(|maximum| length > maximum);
        if out_of_range {
            return Err(FrontierExclusion::FileSize);
        }
    }
    Ok(())
}

/// True while a payload `structured_depth` hops into a JSON chain may still expand the links it
/// contains. Extracted so the bound is testable without a live `WorkerContext`. Deliberately
/// takes no `CrawlPolicy`: site depth does not fund structured expansion.
fn may_expand_structured_links(structured_depth: usize) -> bool {
    structured_depth < MAXIMUM_STRUCTURED_CHAIN
}

/// Returns the depth assigned to an Outlook attachment when the container budget permits another
/// descent. `.msg` files deliberately emit only their immediate attachments; if one attachment is
/// itself a message, the ordinary format detector classifies it on the next task. This keeps all
/// nesting on the queue's single container-depth axis instead of hiding recursion inside a parser.
fn next_msg_attachment_container_depth(container_depth: usize) -> Option<usize> {
    (container_depth < MAXIMUM_CONTAINER_DEPTH).then(|| container_depth + 1)
}

/// Provenance for a discovered payload. The portal is not the origin: a crawled page can point
/// at any host, and in an evidence tool the recorded origin has to be the page the link was on
/// plus the host the bytes actually came from.
fn describe_discovery_origin(discovered_from: &str, download_url: &str) -> String {
    match (url_host(discovered_from), url_host(download_url)) {
        (Some(origin), Some(payload)) if origin != payload => {
            format!("Discovered from {discovered_from}; payload hosted on {payload}")
        }
        _ => format!("Discovered from {discovered_from}"),
    }
}

fn url_host(value: &str) -> Option<String> {
    reqwest::Url::parse(value)
        .ok()
        .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
}

fn source_host_matches(source_url: &str, candidate_url: &str) -> bool {
    let source_host = url_host(source_url);
    let candidate_host = url_host(candidate_url);
    source_host.is_some() && source_host == candidate_host
}

fn guidance_explicitly_allows_host(guidance: &SourceGuidance, value: &str) -> bool {
    if guidance.allowed_hostnames.is_empty() {
        return false;
    }
    reqwest::Url::parse(value)
        .ok()
        .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
        .is_some_and(|host| {
            guidance.allowed_hostnames.iter().any(|allowed| {
                let allowed = allowed.to_ascii_lowercase();
                host == allowed || host.ends_with(&format!(".{allowed}"))
            })
        })
}

/// Whether a payload hosted at `candidate_url` may be downloaded for a source rooted at
/// `source_url`.
///
/// Unset (the default) this admits every host, which is what the crawl has always done: the link
/// that reached the payload was already admitted by the follow gate, and real portals serve their
/// documents from storage hosts that gate does not enumerate. Set, the source root's own host and
/// any host the operator wrote into `allowed_hostnames` are the whole permitted set. The explicit
/// allowlist has to stay in the permitted set, or setting the flag would refuse hosts the operator
/// named on purpose.
fn download_host_allowed(guidance: &SourceGuidance, source_url: &str, candidate_url: &str) -> bool {
    !guidance.restrict_downloads_to_source_host
        || source_host_matches(source_url, candidate_url)
        || guidance_explicitly_allows_host(guidance, candidate_url)
}

fn is_supported_external_data_platform(value: &str) -> bool {
    // Host policy fetches, caches, and enforces robots rules before any supported external host is
    // crawled. This list decides only which off-source platforms are eligible for that policy.
    let Some(host) = reqwest::Url::parse(value)
        .ok()
        .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
    else {
        return false;
    };
    [
        "arcgis.com",
        "arcgisonline.com",
        "tableau.com",
        "google.com",
        "googleusercontent.com",
        "socrata.com",
        "granicus.com",
        "hylandcloud.com",
        "netfile.com",
        "github.com",
        "githubusercontent.com",
        "box.com",
        "dropbox.com",
        "sharepoint.com",
        "onedrive.live.com",
    ]
    .iter()
    .any(|platform| host == *platform || host.ends_with(&format!(".{platform}")))
}

fn stable_source_identifier(value: &str) -> u64 {
    let digest = Sha256::digest(value.as_bytes());
    let prefix = u64::from_be_bytes(digest[..8].try_into().expect("SHA-256 prefix is 8 bytes"));
    SYNTHETIC_DOCUMENT_NAMESPACE | (prefix & SYNTHETIC_SOURCE_IDENTIFIER_MASK)
}

fn source_file_name(value: &str, document_id: u64) -> String {
    let candidate = reqwest::Url::parse(value)
        .ok()
        .and_then(|url| {
            url.path_segments()
                .and_then(|mut segments| segments.next_back())
                .map(str::to_string)
        })
        .unwrap_or_default();
    sanitize_file_name(&candidate, document_id)
}

fn discovered_file_name(inspection: &PublicResourceInspection, document_id: u64) -> String {
    let from_url = source_file_name(&inspection.final_url, document_id);
    let title = inspection.title.trim();
    if title.is_empty() || has_likely_document_extension(&from_url) {
        from_url
    } else {
        title.chars().take(512).collect()
    }
}

fn has_likely_document_extension(value: &str) -> bool {
    let value = value.to_ascii_lowercase();
    [
        ".pdf", ".csv", ".tsv", ".json", ".jsonl", ".geojson", ".xml", ".xlsx", ".xls", ".ods",
        ".docx", ".doc", ".odt", ".pptx", ".ppt", ".rtf", ".txt", ".md", ".zip", ".tar", ".gz",
        ".db", ".sqlite", ".msg", ".eml", ".png", ".jpg", ".jpeg", ".tif", ".tiff", ".webp",
    ]
    .iter()
    .any(|extension| value.ends_with(extension))
}

fn local_source_registration(
    kind: SourceKind,
    title: String,
    identity: &str,
) -> PortalRegistration {
    let digest = sha256_hex(identity.as_bytes());
    let source_key = format!("source-{}", &digest[..24]);
    let source_uri = format!("onrecord://local/{source_key}");
    PortalRegistration {
        portal_key: source_key,
        display_name: title,
        canonical_url: source_uri.clone(),
        origin_url: "onrecord://local/".to_string(),
        api_layout: PortalApiLayout {
            requests_endpoint: source_uri,
            requests_pointer: "/resources".to_string(),
            metadata_pointer: None,
            page_parameter: "import_page".to_string(),
            page_size_parameter: "import_limit".to_string(),
        },
        verified_at: Utc::now().to_rfc3339(),
        source_kind: kind,
    }
}

fn synthetic_source_request(source: &PortalRegistration, description: &str) -> PublicRequest {
    let now = Utc::now().to_rfc3339();
    PublicRequest {
        id: stable_source_identifier(&format!("request|{}", source.portal_key)),
        rpid: Some(format!(
            "source-{}",
            &source.portal_key[source.portal_key.len().saturating_sub(12)..]
        )),
        title: Some(source.display_name.clone()),
        description: Some(description.to_string()),
        status: "imported".to_string(),
        current_status_id: None,
        created_at: now.clone(),
        updated_at: now,
        closed_at: None,
        due_date: None,
        determination_at: None,
        determination_source: None,
        neighborhood_block: None,
        statutory_extension: None,
        documents: Vec::new(),
        agency: None,
        category: None,
        timeline_anomalies: Vec::new(),
    }
}

fn media_type_extension(media_type: Option<&str>, content: &str) -> &'static str {
    match media_type.unwrap_or_default().to_ascii_lowercase().as_str() {
        "application/json" | "application/geo+json" => "json",
        "application/x-ndjson" => "ndjson",
        "text/csv" => "csv",
        "text/tab-separated-values" => "tsv",
        "text/html" | "application/xhtml+xml" => "html",
        "application/xml" | "text/xml" => "xml",
        "text/markdown" => "md",
        _ if serde_json::from_str::<serde_json::Value>(content).is_ok() => "json",
        _ => "txt",
    }
}

async fn send_progress(context: &WorkerContext, progress: IngestionProgress) {
    let _ = context.progress_sender.send(progress).await;
}

#[cfg(test)]
mod tests {
    // Release-scale coverage is orchestrated by `scripts/validate-release-performance.mjs`: the
    // high-link and parser-pressure cases below run beside the million-row analytical harness and
    // the durable interrupted-frontier recovery test.
    use super::*;
    use crate::ingestion::client::HttpClientConfig;
    use tempfile::tempdir;

    /// A purge that released an object must not unlink it while a sync that just linked the same
    /// bytes has yet to commit the row referencing it.
    #[tokio::test]
    async fn purge_leaves_an_object_a_running_sync_just_linked() {
        let root = tempdir().unwrap();
        let staging = root.path().join("staged");
        tokio::fs::write(&staging, b"shared evidence")
            .await
            .unwrap();
        let object = commit_staged_object(root.path(), &staging, &sha256_hex(b"shared evidence"))
            .await
            .unwrap();
        let path = object.to_string_lossy().into_owned();
        assert_eq!(
            remove_released_evidence_files(std::slice::from_ref(&path)).await,
            vec![path.clone()]
        );
        assert!(object.exists());

        // Once a row references it, the purge-time reference check is the guard instead.
        release_object_link(&object).await;
        assert!(remove_released_evidence_files(&[path]).await.is_empty());
        assert!(!object.exists());
    }

    fn metadata_task(page: usize) -> IngestionTask {
        IngestionTask::FetchMetadataPage {
            subdomain: "records".to_string(),
            page,
            per_page: 50,
        }
    }

    /// A crawl node named by a structured payload obeys the same host rule as one found in HTML.
    /// Proven against the pre-fix file by inspection: `expand_structured_document_links` handed
    /// every non-download candidate to `prepare_crawl_url` with no host check, while
    /// `admit_crawl_link` refused the identical URL as `OffScopeHost`.
    #[test]
    fn structured_crawl_candidates_obey_the_html_host_scope_rule() {
        let source = "https://records.example.gov/portal";
        let guidance = SourceGuidance::default();
        let mut policy = CrawlPolicy {
            follow_supported_external_platforms: false,
            ..Default::default()
        };

        assert!(crawl_host_in_scope(
            source,
            &guidance,
            &policy,
            "https://records.example.gov/api/3/action/package_show?id=x"
        ));
        assert!(!crawl_host_in_scope(
            source,
            &guidance,
            &policy,
            "https://anywhere.example/landing"
        ));
        assert!(
            !crawl_host_in_scope(
                source,
                &guidance,
                &policy,
                "https://services.arcgis.com/abc/FeatureServer"
            ),
            "a supported platform is off scope until the policy opts in"
        );

        policy.follow_supported_external_platforms = true;
        assert!(crawl_host_in_scope(
            source,
            &guidance,
            &policy,
            "https://services.arcgis.com/abc/FeatureServer"
        ));

        let allowing = SourceGuidance {
            allowed_hostnames: vec!["anywhere.example".to_string()],
            ..Default::default()
        };
        assert!(crawl_host_in_scope(
            source,
            &allowing,
            &policy,
            "https://data.anywhere.example/landing"
        ));
    }

    #[test]
    fn release_stress_harness_bounds_high_link_discovery() {
        use std::fmt::Write as _;

        let mut payload = String::from("{\"resources\":[");
        for index in 0..=MAXIMUM_STRUCTURED_DISCOVERY_LINKS {
            if index > 0 {
                payload.push(',');
            }
            write!(
                payload,
                "{{\"downloadUrl\":\"https://files.example.test/{index}.pdf\"}}"
            )
            .unwrap();
        }
        payload.push_str("]}");
        assert!(payload.len() < MAXIMUM_STRUCTURED_DISCOVERY_BYTES as usize);
        let discovery = discover_structured_document_links(payload.as_bytes(), None).unwrap();
        assert_eq!(
            discovery.candidates.len(),
            MAXIMUM_STRUCTURED_DISCOVERY_LINKS
        );
    }

    #[tokio::test]
    async fn release_stress_harness_applies_parser_memory_backpressure() {
        assert_eq!(
            parser_memory_permits(u64::MAX, u32::MAX),
            PARSER_MEMORY_PERMITS
        );
        let semaphore = Arc::new(Semaphore::new(PARSER_MEMORY_PERMITS as usize));
        let all_memory = Arc::clone(&semaphore)
            .acquire_many_owned(PARSER_MEMORY_PERMITS)
            .await
            .unwrap();
        assert!(semaphore.try_acquire().is_err());
        drop(all_memory);
        assert_eq!(
            semaphore.available_permits(),
            PARSER_MEMORY_PERMITS as usize
        );
    }

    /// Builds a real `WorkerContext` so the download gate is exercised on the path a crawl
    /// actually takes, rather than on the predicate alone.
    async fn crawl_worker_context(
        directory: &tempfile::TempDir,
        guidance: SourceGuidance,
    ) -> WorkerContext {
        let repository =
            DuckDbRepository::new(Path::new(":memory:")).expect("DuckDB core must open");
        repository
            .run_migrations(include_str!("../../migrations/0001_initial_schema.sql"))
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

    fn off_host_download() -> PublicResourceInspection {
        PublicResourceInspection {
            final_url: "https://records-files.s3.amazonaws.com/report.pdf".to_string(),
            content_type: Some("application/pdf".to_string()),
            title: "Report".to_string(),
            kind: PublicResourceKind::Download,
            links: Vec::new(),
            body: None,
            embedded_connectors: Vec::new(),
            content_length: None,
            acquisition: None,
        }
    }

    // REGRESSION(queue): a crawled page could steer a download to any host, and there was no way
    // for an operator to say otherwise. Proven by deleting the `download_host_allowed` branch in
    // `prepare_discovered_download`: the restricted cases below then produce a task and the
    // exclusion ledger stays empty.
    #[tokio::test]
    async fn a_cross_host_download_is_admitted_by_default_and_refused_once_restricted() {
        let directory = tempdir().expect("temp dir must be created");
        let permissive = crawl_worker_context(&directory, SourceGuidance::default()).await;
        let task = prepare_discovered_download(
            &permissive,
            &off_host_download(),
            Some(7),
            "https://records.example.test/portal/reports",
            PayloadDepth::discovered(1, 0),
        )
        .await
        .expect("the permissive default must not error");
        assert!(
            task.is_some(),
            "today's behaviour admits a payload hosted off the source root"
        );
        assert!(permissive.crawl.frontier_exclusions.lock().await.is_empty());

        let restricted = crawl_worker_context(
            &directory,
            SourceGuidance {
                restrict_downloads_to_source_host: true,
                ..SourceGuidance::default()
            },
        )
        .await;
        let task = prepare_discovered_download(
            &restricted,
            &off_host_download(),
            Some(7),
            "https://records.example.test/portal/reports",
            PayloadDepth::discovered(1, 0),
        )
        .await
        .expect("a refused host is a policy outcome, not an error");
        assert!(task.is_none(), "the restricted source must refuse the host");
        assert_eq!(
            restricted
                .crawl
                .frontier_exclusions
                .lock()
                .await
                .get("off-source-host-download"),
            Some(&1),
            "a refused download has to be countable, not silent"
        );
    }

    #[tokio::test]
    async fn restricting_downloads_still_admits_the_source_root_and_the_explicit_allowlist() {
        let directory = tempdir().expect("temp dir must be created");
        let context = crawl_worker_context(
            &directory,
            SourceGuidance {
                restrict_downloads_to_source_host: true,
                allowed_hostnames: vec!["s3.amazonaws.com".to_string()],
                ..SourceGuidance::default()
            },
        )
        .await;
        for url in [
            "https://records.example.test/files/report.pdf",
            "https://records-files.s3.amazonaws.com/report.pdf",
        ] {
            let inspection = PublicResourceInspection {
                final_url: url.to_string(),
                ..off_host_download()
            };
            let task = prepare_discovered_download(
                &context,
                &inspection,
                Some(7),
                "https://records.example.test/portal/reports",
                PayloadDepth::discovered(1, 0),
            )
            .await
            .expect("an admitted host must not error");
            assert!(task.is_some(), "{url} must survive the download gate");
        }
    }

    // The rejected fix was to apply the follow-path gate to downloads. This pins that the new flag
    // did not do that by another route: `is_supported_external_data_platform` omits S3, Azure Blob,
    // Granicus and CivicPlus, so a source that restricts its downloads must still follow links onto
    // the platforms that gate does admit.
    #[test]
    fn restricting_downloads_does_not_narrow_which_links_are_followed() {
        let guidance = SourceGuidance {
            restrict_downloads_to_source_host: true,
            ..SourceGuidance::default()
        };
        assert!(is_supported_external_data_platform(
            "https://services.arcgis.com/abc/FeatureServer/0"
        ));
        assert!(guidance.allowed_hostnames.is_empty());
        assert!(!download_host_allowed(
            &guidance,
            "https://records.example.test/portal",
            "https://services.arcgis.com/abc/FeatureServer/0"
        ));
    }

    // The seed is what decides where a resumed portal re-enters its request index whenever one is
    // enqueued at all: `start_task_pool` drops it only when the adopted frontier proves the walk
    // was interrupted mid-pagination (see `pool_seed_tasks`), and otherwise adds it alongside
    // every adopted row. Before the window was persisted this was hardcoded to page 1 at size 50,
    // which meant an operator who started at page 400 got the first 399 pages crawled on the next
    // launch.
    #[test]
    fn a_resumed_portal_seeds_the_page_window_the_operator_saved() {
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
            verified_at: "2026-08-29T00:00:00Z".to_string(),
            source_kind: SourceKind::NextRequest,
        };

        let saved = resume_initial_tasks(
            &source,
            MetadataPageWindow {
                start_page: 400,
                page_size: 25,
            },
        );
        assert!(
            matches!(
                saved.as_slice(),
                [IngestionTask::FetchMetadataPage {
                    page: 400,
                    per_page: 25,
                    ..
                }]
            ),
            "a resume must re-enter at the saved window, got: {saved:?}"
        );

        let unset = resume_initial_tasks(&source, MetadataPageWindow::default());
        assert!(
            matches!(
                unset.as_slice(),
                [IngestionTask::FetchMetadataPage {
                    page: 1,
                    per_page: 50,
                    ..
                }]
            ),
            "a source with no saved window must resume exactly as it did before, got: {unset:?}"
        );
    }

    // REGRESSION(queue): a resume enqueued its start-page seed alongside the adopted frontier
    // unconditionally, so a NextRequest source paused at page 900 re-fetched pages 1..900 on every
    // launch while page 901 sat pending in its own frontier. Proven by removing the
    // `walk_interrupted` arm from `pool_seed_tasks` and confirming the first case below fails.
    //
    // The two cases that keep the seed are the point of the fix as much as the case that drops it:
    // an operator start is an instruction, and a walk with no surviving page row finished, so
    // re-entering at the start page is the only way it ever sees records filed since.
    #[test]
    fn a_resume_defers_to_an_interrupted_walk_and_re_seeds_every_other_start() {
        let seed = || {
            vec![
                IngestionTask::FetchMetadataPage {
                    subdomain: "records".to_string(),
                    page: 400,
                    per_page: 25,
                },
                IngestionTask::CrawlPublicResource {
                    subdomain: "records".to_string(),
                    url: "https://records.example.test".to_string(),
                    depth: 0,
                },
            ]
        };

        let resumed = pool_seed_tasks(seed(), PoolSeedPolicy::ContinueInterruptedWalk, true);
        assert!(
            !resumed
                .iter()
                .any(|task| matches!(task, IngestionTask::FetchMetadataPage { .. })),
            "a resume whose frontier still owes a page must not re-seed the index: {resumed:?}"
        );
        assert_eq!(
            resumed.len(),
            1,
            "only the request-index seed is dropped; a crawl seed is a separate question: {resumed:?}"
        );

        assert_eq!(
            pool_seed_tasks(seed(), PoolSeedPolicy::ContinueInterruptedWalk, false).len(),
            2,
            "a walk that left no page row finished, and a resume must re-enter at its start page"
        );
        assert_eq!(
            pool_seed_tasks(seed(), PoolSeedPolicy::OperatorRequested, true).len(),
            2,
            "an operator who just chose a start page outranks an inherited frontier"
        );
    }

    // The prefix and the identifier are two constructions of the same string, and the frontier
    // query that decides the above matches rows by it. A drift between them would not fail any
    // test that only exercised `pool_seed_tasks`: the query would simply never match, and every
    // resume would silently re-seed again.
    #[test]
    fn the_metadata_page_prefix_matches_the_identifier_such_a_task_is_stored_under() {
        let prefix = IngestionTask::metadata_page_target_prefix("records");
        let page = IngestionTask::FetchMetadataPage {
            subdomain: "records".to_string(),
            page: 901,
            per_page: 50,
        };
        assert!(
            page.target_id().starts_with(&prefix),
            "a stored page task must be findable by the prefix, got '{}' against '{prefix}'",
            page.target_id()
        );
        assert!(
            !IngestionTask::CrawlPublicResource {
                subdomain: "records".to_string(),
                url: "https://records.example.test".to_string(),
                depth: 0,
            }
            .target_id()
            .starts_with(&prefix),
            "no other task kind may answer for the request index"
        );
        assert!(
            !IngestionTask::metadata_page_target_prefix("records-2").starts_with(&prefix),
            "one source key must not carry another's prefix"
        );
    }

    async fn manager_with_source(
        directory: &tempfile::TempDir,
    ) -> (
        Arc<QueueManager>,
        DuckDbRepository,
        PortalRegistration,
        mpsc::Receiver<IngestionProgress>,
    ) {
        let repository =
            DuckDbRepository::new(Path::new(":memory:")).expect("DuckDB core must open");
        repository
            .run_migrations(include_str!("../../migrations/0001_initial_schema.sql"))
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

    // The input is the canonical registration returned AFTER redirect/platform discovery,
    // while submitted_url deliberately names a different alias. No network is needed to
    // exercise the mutation boundary: disabling its identity guard must alter the stored
    // lifecycle/recipe and fail these assertions.
    #[tokio::test]
    async fn automatic_discovery_preserves_resolved_source_lifecycle_and_recipe() {
        let directory = tempdir().unwrap();
        let (manager, repository, source, _telemetry) = manager_with_source(&directory).await;
        manager.global_semaphore.close(); // Mutants must never issue an actual network request.
        let saved_guidance = SourceGuidance {
            link_selector: Some("main a[href]".into()),
            ..Default::default()
        };
        repository
            .save_source_connector_recipe(
                &source,
                "https://records.example.test/original-alias",
                &CrawlPolicy::default(),
                &saved_guidance,
            )
            .await
            .unwrap();
        let original_recipe = repository
            .load_source_connector_recipe_for_source(&source)
            .await
            .unwrap()
            .unwrap();
        for state in [
            SourceLifecycleState::Paused,
            SourceLifecycleState::Removed,
            SourceLifecycleState::Active,
        ] {
            repository
                .set_source_lifecycle_state(&source.portal_key, state)
                .await
                .unwrap();
            for reclassified in [false, true] {
                let mut resolved = source.clone();
                resolved.display_name = "Unwanted automatic replacement".into();
                if reclassified {
                    // A changed platform classification can change the derived key;
                    // the canonical publication still belongs to the saved source.
                    resolved.portal_key = "source-reclassified".into();
                    resolved.source_kind = SourceKind::WebCrawl;
                    resolved.canonical_url.push('/');
                }
                let result = manager
                    .start_task_pool(
                        resolved.clone(),
                        RateLimitPolicy::default(),
                        resume_initial_tasks(&resolved, MetadataPageWindow::default()),
                        CrawlPolicy {
                            maximum_pages: 2,
                            maximum_depth: 1,
                            maximum_resources: 8,
                            follow_supported_external_platforms: false,
                        },
                        SourceGuidance::default(),
                        PoolSeedPolicy::AutomaticDiscovery {
                            submitted_url: "https://records.example.test/redirecting-alias".into(),
                        },
                    )
                    .await;
                assert!(
                    matches!(result, Err(AppError::DataParsing(ref message)) if message.contains("existing source")),
                    "Automatic discovery must reject the resolved registered identity: {result:?}"
                );
                assert_eq!(
                    repository
                        .source_lifecycle_state(&source.portal_key)
                        .await
                        .unwrap(),
                    Some(state)
                );
                let retained = repository
                    .load_source_connector_recipe_for_source(&source)
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(
                    (
                        &retained.crawl_policy,
                        &retained.guidance,
                        &retained.metadata_page_window
                    ),
                    (
                        &original_recipe.crawl_policy,
                        &original_recipe.guidance,
                        &original_recipe.metadata_page_window
                    ),
                    "Rejected discovery must not replace the owner's collection recipe"
                );
                if reclassified {
                    assert_eq!(
                        repository
                            .source_lifecycle_state(&resolved.portal_key)
                            .await
                            .unwrap(),
                        None
                    );
                }
            }
        }
        let conn = repository.analytics_connection().await.unwrap();
        let (registrations, runs, label): (i64, i64, String) = conn.query_row(
            "SELECT (SELECT count(*) FROM portals), (SELECT count(*) FROM portal_sync_runs), display_name FROM portals WHERE subdomain = ?1",
            [&source.portal_key], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).unwrap();
        assert_eq!((registrations, runs, label), (1, 0, source.display_name));
    }

    #[tokio::test]
    async fn automatic_discovery_rechecks_identity_after_a_concurrent_pause() {
        let directory = tempdir().unwrap();
        let (manager, repository, source, _telemetry) = manager_with_source(&directory).await;
        manager.global_semaphore.close();
        let held = manager.provisioning_lock.lock().await;
        let (ready, waiting) = tokio::sync::oneshot::channel();
        let pause_manager = Arc::clone(&manager);
        let pause_key = source.portal_key.clone();
        let pause = tokio::spawn(async move {
            ready.send(()).unwrap();
            pause_manager.pause_source(&pause_key).await
        });
        waiting.await.unwrap();
        // Tokio's mutex is FIFO: pause has reached the lock before discovery is polled.
        let start_manager = Arc::clone(&manager);
        let resolved = source.clone();
        let start = tokio::spawn(async move {
            start_manager
                .start_task_pool(
                    resolved.clone(),
                    RateLimitPolicy::default(),
                    resume_initial_tasks(&resolved, MetadataPageWindow::default()),
                    CrawlPolicy::default(),
                    SourceGuidance::default(),
                    PoolSeedPolicy::AutomaticDiscovery {
                        submitted_url: "https://records.example.test/alias".into(),
                    },
                )
                .await
        });
        tokio::task::yield_now().await;
        assert!(
            !start.is_finished(),
            "The identity check must share the lifecycle mutation lock"
        );
        drop(held);
        pause.await.unwrap().unwrap();
        let result = tokio::time::timeout(std::time::Duration::from_secs(5), start)
            .await
            .unwrap()
            .unwrap();
        assert!(
            result.is_err(),
            "A discovery resolved before pause cannot reactivate its source"
        );
        assert_eq!(
            repository
                .source_lifecycle_state(&source.portal_key)
                .await
                .unwrap(),
            Some(SourceLifecycleState::Paused)
        );
        let conn = repository.analytics_connection().await.unwrap();
        let runs: i64 = conn
            .query_row("SELECT count(*) FROM portal_sync_runs", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(
            runs, 0,
            "Rejected discovery cannot leave even a cancelled automatic run"
        );
    }

    #[tokio::test]
    async fn finishing_worker_observes_cancellation_while_lifecycle_lock_is_held() {
        let directory = tempdir().unwrap();
        let context = crawl_worker_context(&directory, SourceGuidance::default()).await;
        // A real worker_loop owns this receiver independently of retirement. Keep
        // it alive so the negative control reaches the lock timeout, not SendError.
        let _worker_cancellation = context.cancellation.subscribe();
        let lifecycle = context.provisioning_lock.lock().await;
        let worker = context.clone();
        let (ready, waiting) = tokio::sync::oneshot::channel();
        let retirement = tokio::spawn(async move {
            ready.send(()).unwrap();
            retire_portal_if_idle(&worker).await;
        });
        waiting.await.unwrap();
        tokio::task::yield_now().await;
        assert!(
            !retirement.is_finished(),
            "An uncancelled worker must wait for normal lifecycle finalization"
        );
        context
            .statistics
            .stop_requested
            .store(true, Ordering::Release);
        context.cancellation.send(true).unwrap();
        // Keep the lifecycle lock held, exactly as stop_source_pool_locked does
        // while joining this worker. Cancellation must break the lock cycle.
        tokio::time::timeout(std::time::Duration::from_secs(3), retirement)
            .await
            .expect("A stopped worker must not wait on the lifecycle lock held by its joiner")
            .unwrap();
        assert!(context.provisioning_lock.try_lock().is_err());
        drop(lifecycle);
        // Cancellation sent before retirement subscribes must also be honored.
        let held_again = context.provisioning_lock.lock().await;
        tokio::time::timeout(
            std::time::Duration::from_secs(3),
            retire_portal_if_idle(&context),
        )
        .await
        .expect("Already-cancelled retirement must return without acquiring the lifecycle lock");
        drop(held_again);
    }

    #[tokio::test]
    async fn automatic_discovery_accepts_new_sources_and_operator_starts_can_resume_them() {
        let directory = tempdir().unwrap();
        let (manager, repository, _source, _telemetry) = manager_with_source(&directory).await;
        manager.global_semaphore.close(); // Starts are real; payload workers remain network-free.
        let fresh = second_source("new-automatic-source");
        let crawl = CrawlPolicy {
            maximum_pages: 2,
            maximum_depth: 1,
            maximum_resources: 8,
            follow_supported_external_platforms: false,
        };
        let alias = "https://second.example.test/alias";
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            manager.start_task_pool(
                fresh.clone(),
                RateLimitPolicy::default(),
                resume_initial_tasks(&fresh, MetadataPageWindow::default()),
                crawl.clone(),
                SourceGuidance::default(),
                PoolSeedPolicy::AutomaticDiscovery {
                    submitted_url: alias.into(),
                },
            ),
        )
        .await
        .expect("A new automatic source must finish provisioning")
        .unwrap();
        assert_eq!(
            repository
                .source_lifecycle_state(&fresh.portal_key)
                .await
                .unwrap(),
            Some(SourceLifecycleState::Active)
        );
        let recipe = repository
            .load_source_connector_recipe(alias)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(recipe.crawl_policy, crawl);
        tokio::time::timeout(std::time::Duration::from_secs(5), manager.pause_source(&fresh.portal_key))
            .await.expect("Pausing the new automatic source must join its workers without a lifecycle-lock deadlock").unwrap();
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            manager.start_task_pool(
                fresh.clone(),
                RateLimitPolicy::default(),
                resume_initial_tasks(&fresh, MetadataPageWindow::default()),
                CrawlPolicy::default(),
                SourceGuidance::default(),
                PoolSeedPolicy::OperatorRequested,
            ),
        )
        .await
        .expect("An explicit operator resume must finish provisioning")
        .unwrap();
        assert_eq!(
            repository
                .source_lifecycle_state(&fresh.portal_key)
                .await
                .unwrap(),
            Some(SourceLifecycleState::Active)
        );
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            manager.pause_source(&fresh.portal_key),
        )
        .await
        .expect(
            "Pausing the resumed source must join its workers without a lifecycle-lock deadlock",
        )
        .unwrap();
    }

    /// Covers the lifecycle writes as one sequence, and the lock discipline underneath them.
    ///
    /// `pause_source` and `remove_source` both take `provisioning_lock` and then call the shared
    /// teardown. That lock is a plain `tokio::sync::Mutex`, so re-entering it deadlocks rather
    /// than erroring, and a deadlock is invisible to review and fatal in the app. This test hangs
    /// instead of failing if either grows a second acquisition.
    #[tokio::test]
    async fn automatic_refresh_never_reactivates_paused_removed_or_purged_sources() {
        let directory = tempdir().unwrap();
        let (manager, repository, source, _telemetry) = manager_with_source(&directory).await;
        manager.pause_source(&source.portal_key).await.unwrap();
        assert!(manager
            .refresh_registered_source(
                source.clone(),
                CrawlPolicy::default(),
                SourceGuidance::default(),
                None
            )
            .await
            .is_err());
        assert_eq!(
            repository
                .source_lifecycle_state(&source.portal_key)
                .await
                .unwrap(),
            Some(SourceLifecycleState::Paused)
        );
        manager.remove_source(&source.portal_key).await.unwrap();
        assert!(manager
            .refresh_registered_source(
                source.clone(),
                CrawlPolicy::default(),
                SourceGuidance::default(),
                None
            )
            .await
            .is_err());
        assert_eq!(
            repository
                .source_lifecycle_state(&source.portal_key)
                .await
                .unwrap(),
            Some(SourceLifecycleState::Removed)
        );
        manager.purge_source(&source.portal_key).await.unwrap();
        assert!(manager
            .refresh_registered_source(
                source.clone(),
                CrawlPolicy::default(),
                SourceGuidance::default(),
                None
            )
            .await
            .is_err());
        assert_eq!(
            repository
                .source_lifecycle_state(&source.portal_key)
                .await
                .unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn pausing_keeps_the_queue_and_removing_discards_it() {
        let directory = tempdir().expect("temp dir must be created");
        let (manager, repository, source, _telemetry) = manager_with_source(&directory).await;
        repository
            .begin_portal_sync_run("run-1", &source.portal_key)
            .await
            .expect("sync run must begin");
        repository
            .persist_ingestion_frontier_task(
                "run-1",
                &source.portal_key,
                "page-2",
                r#"{"taskKind":"fetchMetadataPage","subdomain":"records","page":2,"perPage":50}"#,
            )
            .await
            .expect("queued work must persist");

        assert!(
            !manager
                .pause_source(&source.portal_key)
                .await
                .expect("pausing an idle source must succeed"),
            "no pool was running, and saying otherwise would be a false report"
        );
        assert_eq!(
            repository
                .source_lifecycle_state(&source.portal_key)
                .await
                .expect("lifecycle lookup must succeed"),
            Some(SourceLifecycleState::Paused)
        );
        assert!(
            repository
                .recover_interrupted_source_keys()
                .await
                .expect("startup recovery scan must succeed")
                .is_empty(),
            "a paused source must not be resumed by the next launch"
        );

        let removal = manager
            .remove_source(&source.portal_key)
            .await
            .expect("removal must succeed");
        assert_eq!(removal.source_key, source.portal_key);
        assert_eq!(
            removal.discarded_frontier_tasks, 1,
            "the queued page was the only frontier row, and removal discards it"
        );
        assert_eq!(
            removal.retained_documents, 0,
            "nothing was ingested, and the count must report what is actually kept"
        );
        assert!(
            repository
                .list_portal_registrations()
                .await
                .expect("listing must succeed")
                .is_empty(),
            "a removed source leaves the list"
        );
        assert!(
            manager.pause_source(&source.portal_key).await.is_err(),
            "pausing a removed source would put it back on the list"
        );

        // Re-adding is the same write every start path performs, and it must restore the source
        // rather than leave it invisible forever.
        repository
            .insert_or_update_portal_registration(&source)
            .await
            .expect("re-registration must persist");
        assert_eq!(
            repository
                .source_lifecycle_state(&source.portal_key)
                .await
                .expect("lifecycle lookup must succeed"),
            Some(SourceLifecycleState::Active)
        );
    }

    // REGRESSION(queue): a start whose initial enqueue failed joined its workers before signalling
    // them, so the join never returned while `provisioning_lock` was held and every later start,
    // pause and removal blocked. To prove it, make `start_task_pool`'s failure branch join before
    // signalling again: the start below then exceeds the timeout.
    #[tokio::test]
    async fn a_failed_initial_enqueue_stops_its_workers_and_releases_the_lifecycle_lock() {
        let directory = tempdir().expect("temp dir must be created");
        let (manager, repository, source, _telemetry) = manager_with_source(&directory).await;
        manager.global_semaphore.close(); // Keep any dispatched task network-free.
        repository
            .begin_portal_sync_run("run-1", &source.portal_key)
            .await
            .expect("sync run must begin");
        repository
            .persist_ingestion_frontier_task("run-1", &source.portal_key, "corrupt", "{not json")
            .await
            .expect("queued work must persist");
        repository
            .finish_portal_sync_run(PortalSyncRunFinalization {
                run_id: "run-1",
                portal_key: &source.portal_key,
                status: "CANCELLED",
                pages_processed: 0,
                requests_processed: 0,
                documents_processed: 0,
                failures: 0,
            })
            .await
            .expect("interrupted run must finalize");

        // The corrupt row is adopted, claimed first during the start's own refill, and refused as
        // invalid: an error from inside the enqueue step, after the workers are already running.
        let started = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            manager.start_verified_portal(
                source.clone(),
                MetadataPageWindow::default(),
                RateLimitPolicy::default(),
            ),
        )
        .await
        .expect("a failed start must return instead of waiting on workers nobody cancelled");
        assert!(
            started.is_err(),
            "the invalid frontier row must fail the start"
        );
        assert!(!manager.is_source_running(&source.portal_key).await);
        assert!(
            manager.provisioning_lock.try_lock().is_ok(),
            "a failed start must not leave the lifecycle lock held"
        );
    }

    #[test]
    fn glob_matching_is_linear_and_keeps_its_wildcard_semantics() {
        assert!(glob_matches("*.pdf", "report.pdf"));
        assert!(glob_matches(
            "budget-*-final.xlsx",
            "budget-2024-final.xlsx"
        ));
        assert!(glob_matches("*", ""));
        assert!(glob_matches("a**b", "ab"));
        assert!(glob_matches("*a*b*", "xxaxxbxx"));
        assert!(!glob_matches("*.pdf", "report.pdf.exe"));
        assert!(!glob_matches("a*b", "ba"));
        assert!(!glob_matches("abc", "ab"));
        assert!(!glob_matches("", "a"));
        assert!(glob_matches("", ""));
        // A recursive matcher needs on the order of C(60, 12) steps to reject this.
        let pattern = format!("{}b", "*a".repeat(12));
        let text = "a".repeat(60);
        let started = std::time::Instant::now();
        assert!(!glob_matches(&pattern, &text));
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
    }

    fn second_source(portal_key: &str) -> PortalRegistration {
        PortalRegistration {
            portal_key: portal_key.to_string(),
            display_name: "Second".to_string(),
            canonical_url: "https://second.example.test".to_string(),
            origin_url: "https://second.example.test".to_string(),
            api_layout: PortalApiLayout {
                requests_endpoint: "https://second.example.test/client/requests".to_string(),
                requests_pointer: "/requests".to_string(),
                metadata_pointer: None,
                page_parameter: "page".to_string(),
                page_size_parameter: "per_page".to_string(),
            },
            verified_at: "2026-08-26T00:00:00Z".to_string(),
            source_kind: SourceKind::NextRequest,
        }
    }

    fn placeholder_pool(portal_key: &str) -> PortalPool {
        // No worker handles and no real network activity: this pool exists only to occupy a slot
        // in `manager.pools` deterministically, so the park/promote test below is not racing a
        // real worker's `retire_portal_if_idle` self-teardown (which the first version of this
        // test did, and which flaked under load: the seeded fetch task can fail fast against a
        // fake host and the pool retires itself before the test observes the parked state).
        PortalPool {
            portal: second_source(portal_key),
            cancellation: watch::channel(false).0,
            statistics: Arc::new(PortalStatistics::default()),
            sync_run_id: "placeholder-run".to_string(),
            worker_handles: Vec::new(),
            crawl: Arc::new(CrawlState {
                policy: CrawlPolicy::default(),
                guidance: SourceGuidance::default(),
                visited_urls: Mutex::new(HashSet::new()),
                resource_urls: Mutex::new(HashSet::new()),
                crawl_path_claims: Mutex::new(HashSet::new()),
                announced_host_stops: Mutex::new(HashSet::new()),
                frontier_exclusions: Mutex::new(BTreeMap::new()),
                pages_started: AtomicUsize::new(0),
                resources_started: AtomicUsize::new(0),
                frontier_refill_lock: Mutex::new(()),
                arcgis_page_hashes: Mutex::new(HashSet::new()),
                compiled_guidance: CompiledGuidancePatterns::compile(&SourceGuidance::default()),
            }),
        }
    }

    // REGRESSION(queue): registration used to be capped by MAXIMUM_ACTIVE_PORTALS, so a source
    // beyond the concurrency ceiling was refused outright rather than parked. Proven by reverting
    // `start_task_pool`'s park loop to an immediate `Err(AppError::PortalCapacity(..))` on a full
    // pool map and confirming the parked start below returns that error instead of completing once
    // the occupying pool is removed.
    #[tokio::test]
    async fn a_start_beyond_the_concurrency_cap_parks_and_is_promoted_when_a_slot_frees() {
        let directory = tempdir().expect("temp dir must be created");
        let (manager, _repository, _first, _telemetry) = manager_with_source(&directory).await;
        let second = second_source("second-records");
        _repository
            .insert_or_update_portal_registration(&second)
            .await
            .expect("second source registration must persist");

        manager
            .set_max_active_portals(1)
            .expect("a positive cap must be accepted");
        let placeholder = second_source("placeholder");
        _repository
            .insert_or_update_portal_registration(&placeholder)
            .await
            .expect("placeholder source registration must persist");
        _repository
            .begin_portal_sync_run("placeholder-run", "placeholder")
            .await
            .expect("placeholder sync run must begin");
        manager
            .pools
            .write()
            .await
            .insert("placeholder".to_string(), placeholder_pool("placeholder"));

        let parked_manager = Arc::clone(&manager);
        let parked_second = second.clone();
        let parked = tokio::spawn(async move {
            parked_manager
                .start_verified_portal(
                    parked_second,
                    MetadataPageWindow::default(),
                    RateLimitPolicy::default(),
                )
                .await
        });

        // Give the parked start a moment to actually reach the park point rather than merely
        // asserting it eventually succeeds, which would also pass if it never parked at all. The
        // placeholder pool above never tears itself down, so there is nothing racing this check.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(
            !parked.is_finished(),
            "a start over the cap must park rather than resolve immediately"
        );

        // Through the real removal path, not a hand-fired signal: a regression where a removal
        // site forgets to signal `slot_freed` (this happened once during development, on the
        // natural-completion path) must fail this test, not merely the park mechanism in
        // isolation.
        let removal_guard = manager.provisioning_lock.lock().await;
        manager
            .stop_source_pool_locked("placeholder")
            .await
            .expect("stopping the placeholder pool must succeed");
        drop(removal_guard);

        let promoted = tokio::time::timeout(std::time::Duration::from_secs(5), parked)
            .await
            .expect("the parked start must be promoted once the slot frees")
            .expect("the spawned task must not panic");
        assert!(
            promoted.is_ok(),
            "a start parked on a full cap must succeed once a slot frees, not be refused: {:?}",
            promoted.err()
        );
    }

    #[test]
    fn serializes_telemetry_variants_with_camel_case_payload_fields() {
        let page = serde_json::to_value(IngestionProgress::PageCompleted {
            portal_key: "records".to_string(),
            page: 3,
            records_found: 50,
        })
        .unwrap();
        assert_eq!(
            page,
            serde_json::json!({
                "type": "pageCompleted",
                "payload": {
                    "portalKey": "records",
                    "page": 3,
                    "recordsFound": 50
                }
            })
        );

        let stopped = serde_json::to_value(IngestionProgress::SyncStopped {
            portal_key: "records".to_string(),
            sync_run_id: "run-7".to_string(),
        })
        .unwrap();
        assert_eq!(
            stopped,
            serde_json::json!({
                "type": "syncStopped",
                "payload": {
                    "portalKey": "records",
                    "syncRunId": "run-7"
                }
            })
        );

        let envelope = serde_json::to_value(ingestion_telemetry_envelope(
            IngestionProgress::CorpusReconciliationStarted {},
        ))
        .unwrap();
        assert_eq!(envelope["protocolVersion"], 1);
        assert!(envelope["sequence"]
            .as_str()
            .is_some_and(|value| value.parse::<u64>().is_ok()));
        assert!(envelope["emittedAt"].as_str().is_some());
        assert_eq!(envelope["type"], "corpusReconciliationStarted");
    }

    #[test]
    fn san_diego_policy_categories_and_pages_keep_distinct_crawl_nodes() {
        let procedures =
            "https://www.sandiego.gov/police/data-transparency/policies-procedures/procedures";
        assert_eq!(
            crawl_path_identity(procedures),
            crawl_path_identity(&format!("{procedures}/?page=0"))
        );
        assert_ne!(
            crawl_path_identity(procedures),
            crawl_path_identity(&format!("{procedures}?page=1"))
        );
        assert_eq!(
            crawl_path_identity(&format!("{procedures}?page=1")),
            crawl_path_identity(&format!("{procedures}?page=1&tracking=123"))
        );
        let categories =
            "https://www.sandiego.gov/police/data-transparency/policies-procedures/category";
        let orders = format!("{categories}?category=Department%20Orders");
        let policies = format!("{categories}?category=Policies");
        assert_ne!(crawl_path_identity(&orders), crawl_path_identity(&policies));
        assert_ne!(
            crawl_path_identity(&orders),
            crawl_path_identity(&format!("{orders}&page=1"))
        );
        assert_eq!(
            crawl_path_identity(&format!("{procedures}?page=100")),
            crawl_path_identity(&format!("{procedures}?page=99999"))
        );
        // The exception never applies to a lookalike host or the City's other sections.
        let other = procedures.replace("www.sandiego.gov", "www.sandiego.gov.example.test");
        assert_eq!(
            crawl_path_identity(&other),
            crawl_path_identity(&format!("{other}?page=1"))
        );
        let calendar = "https://www.sandiego.gov/cpp/meetingsagendas";
        assert_eq!(
            crawl_path_identity(calendar),
            crawl_path_identity(&format!("{calendar}?page=1"))
        );
    }

    #[test]
    fn faceted_search_variants_collapse_onto_one_crawl_path() {
        // Verbatim from https://data.ca.gov/dataset fetched 2026-08-25. Ten of these appear on a
        // single page and each one links ten more, which is the combinatorial expansion that made
        // the frontier unbounded.
        let seed = crawl_path_identity("https://data.ca.gov/dataset");
        for facet in [
            "https://data.ca.gov/dataset/?tags=species",
            "https://data.ca.gov/dataset/?tags=wildlife",
            "https://data.ca.gov/dataset/?tags=habitat",
            "https://data.ca.gov/dataset/?tags=CDFW",
            "https://data.ca.gov/dataset/",
        ] {
            assert_eq!(
                crawl_path_identity(facet),
                seed,
                "{facet} claimed its own node"
            );
        }
        // A genuinely different record keeps its own node.
        assert_ne!(
            crawl_path_identity("https://data.ca.gov/dataset/fish-hatchery-annual-report"),
            seed
        );
        // Host is part of the identity, so two sites never share a claim.
        assert_ne!(
            crawl_path_identity("https://data.cityofnewyork.us/dataset"),
            seed
        );
    }

    #[test]
    fn validates_page_bounds_at_the_queue_boundary() {
        assert!(validate_page_parameters(1, 1).is_ok());
        assert!(validate_page_parameters(MAXIMUM_METADATA_PAGE, 100).is_ok());
        assert!(validate_page_parameters(0, 50).is_err());
        assert!(validate_page_parameters(MAXIMUM_METADATA_PAGE + 1, 50).is_err());
        assert!(validate_page_parameters(1, 0).is_err());
        assert!(validate_page_parameters(1, 101).is_err());
    }

    #[test]
    fn sanitizes_names_to_a_portable_byte_bound() {
        assert_eq!(
            sanitize_file_name("../../evidence:final.pdf", 7),
            "evidence_final.pdf"
        );
        assert_eq!(sanitize_file_name("...", 7), "document_7.bin");

        let unicode_name = format!("{}.txt", "🔎".repeat(100));
        let sanitized = sanitize_file_name(&unicode_name, 9);
        assert!(sanitized.len() <= 180);
        assert!(sanitized.is_char_boundary(sanitized.len()));
    }

    #[test]
    fn preserves_queue_statistics_when_backpressure_rejects_a_task() {
        let statistics = PortalStatistics::default();
        let (sender, _receiver) = mpsc::channel(1);
        enqueue_task(&sender, &statistics, metadata_task(1)).unwrap();
        assert_eq!(statistics.queued_tasks.load(Ordering::Acquire), 1);

        assert!(enqueue_task(&sender, &statistics, metadata_task(2)).is_err());
        assert_eq!(statistics.queued_tasks.load(Ordering::Acquire), 1);
    }

    #[test]
    fn preserves_queue_statistics_when_the_receiver_is_closed() {
        let statistics = PortalStatistics::default();
        let (sender, receiver) = mpsc::channel(1);
        drop(receiver);

        assert!(enqueue_task(&sender, &statistics, metadata_task(1)).is_err());
        assert_eq!(statistics.queued_tasks.load(Ordering::Acquire), 0);
    }

    #[test]
    fn durable_frontier_round_trip_preserves_paths_and_ignores_the_legacy_msg_backstop() {
        let task = IngestionTask::DownloadBinaryPayload {
            subdomain: "records".to_string(),
            request_id: 41,
            document: Box::new(DocumentMetadata {
                id: 73,
                file_name: Some("evidence.pdf".to_string()),
                file_size: Some(128),
                description: None,
                content_type: Some("application/pdf".to_string()),
                download_url: None,
                created_at: "2026-08-24T00:00:00Z".to_string(),
                updated_at: "2026-08-24T00:00:00Z".to_string(),
                visibility: "public".to_string(),
                local_path: Some("/managed/objects/abc".to_string()),
                extracted_text_sha256: None,
                was_ocr_processed: false,
            }),
            output_directory: "/managed/records".to_string(),
            container_depth: 0,
            container_root_directory: Some("/managed/root_contents".to_string()),
            crawl_depth: 5,
            structured_depth: 3,
        };
        let encoded = serde_json::to_string(&task).unwrap();
        let decoded: IngestionTask = serde_json::from_str(&encoded).unwrap();
        let IngestionTask::DownloadBinaryPayload {
            document,
            container_root_directory,
            crawl_depth,
            structured_depth,
            ..
        } = decoded
        else {
            panic!("durable task kind changed during round trip");
        };
        assert_eq!(document.local_path.as_deref(), Some("/managed/objects/abc"));
        assert_eq!(
            container_root_directory.as_deref(),
            Some("/managed/root_contents")
        );
        assert_eq!(crawl_depth, 5);
        assert_eq!(structured_depth, 3);

        // Rows written before `.msg` expansion became format- and depth-driven carry this field.
        // Serde's default unknown-field behavior must let those durable tasks resume after upgrade.
        let mut legacy_encoded = serde_json::to_value(&task).unwrap();
        legacy_encoded.as_object_mut().unwrap().insert(
            "skip_msg_attachment_expansion".to_string(),
            serde_json::Value::Bool(true),
        );
        let legacy_decoded: IngestionTask = serde_json::from_value(legacy_encoded).unwrap();
        let IngestionTask::DownloadBinaryPayload {
            document,
            container_depth,
            ..
        } = legacy_decoded
        else {
            panic!("legacy durable task kind changed during resume");
        };
        assert_eq!(document.id, 73);
        assert_eq!(container_depth, 0);
    }

    #[test]
    fn a_frontier_row_written_before_the_depth_fields_existed_resumes_at_depth_zero() {
        let task = IngestionTask::DownloadBinaryPayload {
            subdomain: "records".to_string(),
            request_id: 41,
            document: Box::new(DocumentMetadata {
                id: 73,
                file_name: Some("evidence.pdf".to_string()),
                file_size: Some(128),
                description: None,
                content_type: Some("application/pdf".to_string()),
                download_url: None,
                created_at: "2026-08-24T00:00:00Z".to_string(),
                updated_at: "2026-08-24T00:00:00Z".to_string(),
                visibility: "public".to_string(),
                local_path: Some("/managed/objects/abc".to_string()),
                extracted_text_sha256: None,
                was_ocr_processed: false,
            }),
            output_directory: "/managed/records".to_string(),
            container_depth: 0,
            container_root_directory: Some("/managed/root_contents".to_string()),
            crawl_depth: 5,
            structured_depth: 3,
        };
        let mut encoded = serde_json::to_value(&task).unwrap();
        let object = encoded.as_object_mut().unwrap();
        for field in [
            "crawl_depth",
            "structured_depth",
            "container_root_directory",
        ] {
            assert!(
                object.remove(field).is_some(),
                "the durable payload must carry {field}"
            );
        }

        let decoded: IngestionTask = serde_json::from_value(encoded).unwrap();
        let IngestionTask::DownloadBinaryPayload {
            container_root_directory,
            crawl_depth,
            structured_depth,
            ..
        } = decoded
        else {
            panic!("durable task kind changed during round trip");
        };
        assert_eq!(crawl_depth, 0);
        assert_eq!(structured_depth, 0);
        assert_eq!(container_root_directory, None);
    }

    #[test]
    fn structured_expansion_is_bounded_by_the_chain_budget_not_by_crawl_depth() {
        assert!(may_expand_structured_links(0));
        assert!(may_expand_structured_links(MAXIMUM_STRUCTURED_CHAIN - 1));
        assert!(!may_expand_structured_links(MAXIMUM_STRUCTURED_CHAIN));
        assert!(!may_expand_structured_links(MAXIMUM_STRUCTURED_CHAIN + 7));
    }

    #[test]
    fn every_msg_attachment_descent_spends_the_shared_container_budget() {
        assert_eq!(next_msg_attachment_container_depth(0), Some(1));
        assert_eq!(
            next_msg_attachment_container_depth(MAXIMUM_CONTAINER_DEPTH - 1),
            Some(MAXIMUM_CONTAINER_DEPTH)
        );
        assert_eq!(
            next_msg_attachment_container_depth(MAXIMUM_CONTAINER_DEPTH),
            None
        );
        assert_eq!(
            next_msg_attachment_container_depth(MAXIMUM_CONTAINER_DEPTH + 1),
            None
        );
    }

    /// A catalog eight pages into a portal is exactly the case the previous gate killed: its
    /// crawl depth sits at the policy ceiling while its structured chain is untouched.
    #[test]
    fn a_catalog_at_the_crawl_ceiling_still_expands() {
        let policy = CrawlPolicy::default();
        let depth = PayloadDepth::discovered(policy.maximum_depth, 0);

        assert!(may_expand_structured_links(depth.structured));
    }

    #[test]
    fn discovery_origin_names_the_page_and_flags_a_cross_host_payload() {
        assert_eq!(
            describe_discovery_origin(
                "https://records.example.gov/page/4",
                "https://records.example.gov/files/report.pdf"
            ),
            "Discovered from https://records.example.gov/page/4"
        );
        assert_eq!(
            describe_discovery_origin(
                "https://records.example.gov/page/4",
                "https://cdn.attacker.example/report.pdf"
            ),
            "Discovered from https://records.example.gov/page/4; payload hosted on cdn.attacker.example"
        );
        assert_eq!(
            describe_discovery_origin("local document 73", "https://cdn.example/report.pdf"),
            "Discovered from local document 73"
        );
    }

    #[test]
    fn expands_dcat_distributions_without_following_licenses_or_context_urls() {
        let catalog = serde_json::json!({
            "@context": "https://project-open-data.cio.gov/schema/catalog.jsonld",
            "dataset": [{
                "title": "Public records",
                "license": "https://example.gov/license",
                "distribution": [{
                    "title": "CSV export",
                    "downloadURL": "https://data.example.gov/records.csv",
                    "mediaType": "text/csv"
                }, {
                    "title": "Query endpoint",
                    "accessURL": "https://data.example.gov/api/records",
                    "mediaType": "application/json"
                }]
            }]
        });
        let mut candidates = HashMap::new();
        collect_structured_link_candidates(&catalog, None, 0, None, &mut candidates);

        assert_eq!(candidates.len(), 2);
        assert!(candidates["https://data.example.gov/records.csv"].download_candidate);
        assert!(!candidates["https://data.example.gov/api/records"].download_candidate);
        assert!(!candidates.contains_key("https://example.gov/license"));
    }

    #[test]
    fn structured_discovery_returns_only_scheduling_summaries_and_ignores_malformed_json() {
        let source_url =
            "https://gis.example.gov/arcgis/rest/services/Records/FeatureServer?f=json";
        let payload = serde_json::json!({
            "type": "Feature Service",
            "id": "e75d5c7a59ff4d6a90e862a481c3ba96",
            "title": "Records service",
            "downloadURL": "https://files.example.gov/records.csv",
            "mediaType": "text/csv",
            "layers": [{"id": 0}],
            "tables": [{"id": 2}]
        });
        let bytes = serde_json::to_vec(&payload).unwrap();

        let discovery = discover_structured_document_links(&bytes, Some(source_url))
            .expect("valid structured payload must produce a scheduling summary");
        assert_eq!(discovery.candidates.len(), 1);
        assert_eq!(
            discovery.candidates[0].url,
            "https://files.example.gov/records.csv"
        );
        assert!(discovery.candidates[0].download_candidate);
        assert_eq!(discovery.companion_urls.len(), 2);
        assert_eq!(
            discovery.layer_urls,
            vec![
                "https://gis.example.gov/arcgis/rest/services/Records/FeatureServer/0".to_string(),
                "https://gis.example.gov/arcgis/rest/services/Records/FeatureServer/2".to_string(),
            ]
        );
        assert!(discovery.next_page.is_none());

        let replay = include_bytes!("../../fixtures/open-data/arcgis-item.json");
        let replay_discovery = discover_structured_document_links(
            replay,
            Some("https://hub.example.gov/datasets/facilities"),
        )
        .expect("recorded ArcGIS item fixture must replay");
        assert_eq!(replay_discovery.candidates.len(), 1);
        assert!(replay_discovery.candidates[0]
            .url
            .ends_with("/PublicFacilities/FeatureServer/0"));
        assert!(discover_structured_document_links(b"{broken", Some(source_url)).is_none());
    }

    #[test]
    fn derives_arcgis_item_and_paginated_layer_endpoints() {
        let package = discover_structured_document_links(br#"{"success":true,"result":{"resources":[{"name":"Measurements","format":"CSV","url":"https://data.example.test/export?id=42"}]}}"#, Some("https://data.ca.gov/api/3/action/package_show?id=sample")).unwrap();
        assert_eq!(package.candidates.len(), 1);
        assert!(package.candidates[0].download_candidate);
        assert_eq!(
            package.candidates[0].media_type.as_deref(),
            Some("text/csv")
        );
        assert!(arcgis_layer_url(
            "https://services.arcgis.com/a/FeatureServer/0/query?where=county%3D73&f=geojson"
        )
        .is_none());
        assert!(arcgis_layer_url(
            "https://services.arcgis.com/a/FeatureServer/0?where=county%3D73"
        )
        .is_none());
        let companions = arcgis_item_companion_urls(
            "https://gis-portal.example.gov/arcgis/home/webmap/viewer.html?webmap=e75d5c7a59ff4d6a90e862a481c3ba96",
        );
        assert_eq!(companions.len(), 2);
        assert!(companions[0].contains("/items/e75d5c7a59ff4d6a90e862a481c3ba96?f=json"));

        let service =
            "https://services.arcgis.com/example/arcgis/rest/services/Records/FeatureServer";
        let metadata = serde_json::json!({"layers": [{"id": 0}, {"id": 4}], "tables": [{"id": 7}]});
        let layers = arcgis_layers_from_metadata(Some(service), &metadata);
        assert_eq!(layers.len(), 3);
        assert_eq!(arcgis_layer_url(&layers[1]), Some(layers[1].clone()));
        assert_eq!(
            arcgis_metadata_url(service),
            Some(format!("{service}?f=json"))
        );
    }

    #[test]
    fn discovers_arcgis_hub_catalogs_and_structured_items() {
        let hub = arcgis_hub_companion_urls(
            "https://records.hub.arcgis.com/pages/data",
            b"%7B%22contentGroupId%22%3A%22fc3e345822664a3cb7ffb771453f1ba8%22%2C%22siteId%22%3A%2254054311055a4758b9d641896521648e%22%7D",
        );
        assert!(hub.iter().any(|url| {
            url.contains("/sharing/rest/search?")
                && url.contains("group%3Afc3e345822664a3cb7ffb771453f1ba8")
        }));
        assert!(hub
            .iter()
            .any(|url| { url.contains("/items/54054311055a4758b9d641896521648e/data?f=json") }));

        let search = serde_json::json!({
            "results": [{
                "id": "e75d5c7a59ff4d6a90e862a481c3ba96",
                "type": "Feature Service",
                "title": "Public records"
            }]
        });
        let companions = arcgis_structured_item_companion_urls(&search);
        assert_eq!(companions.len(), 2);
        assert!(companions
            .iter()
            .any(|url| url.ends_with("e75d5c7a59ff4d6a90e862a481c3ba96?f=json")));
        let next = arcgis_search_next_page(
            Some("https://www.arcgis.com/sharing/rest/search?f=json&q=group%3Aabc&num=100&start=1"),
            &serde_json::json!({"nextStart": 101}),
        )
        .unwrap();
        assert!(next.contains("q=group%3Aabc"));
        assert!(next.ends_with("start=101"));
    }

    #[test]
    fn follows_only_explicitly_supported_external_data_platforms() {
        assert!(is_supported_external_data_platform(
            "https://records.hub.arcgis.com/pages/data"
        ));
        assert!(is_supported_external_data_platform(
            "https://public.tableau.com/views/workbook/view"
        ));
        assert!(!is_supported_external_data_platform(
            "https://unrelated.example/blog"
        ));
    }

    #[test]
    fn derives_tableau_public_csv_and_pdf_exports() {
        let exports = tableau_export_companion_urls(
            "https://public.tableau.com/app/profile/chsu/viz/CommunityProfiles/HHSA?publish=yes",
        );
        assert_eq!(exports.len(), 2);
        assert_eq!(
            exports[0],
            "https://public.tableau.com/views/CommunityProfiles/HHSA.csv?:showVizHome=no"
        );
        assert_eq!(
            exports[1],
            "https://public.tableau.com/views/CommunityProfiles/HHSA.pdf?:showVizHome=no"
        );
    }

    #[test]
    fn rejects_output_paths_outside_the_managed_root() {
        let root = Path::new("/managed/documents");
        assert!(validate_internal_output_directory(root, &root.join("portal")).is_ok());
        assert!(validate_internal_output_directory(root, Path::new("/managed/escape")).is_err());
        assert!(validate_internal_output_directory(root, &root.join("../escape")).is_err());
    }

    #[test]
    fn derives_stable_javascript_safe_archive_entry_identifiers() {
        let digest = "a".repeat(64);
        let first = archive_entry_id(42, &digest, 1, Path::new("folder/evidence.txt"));
        let repeated = archive_entry_id(42, &digest, 1, Path::new("folder/evidence.txt"));
        let different = archive_entry_id(42, &digest, 1, Path::new("folder/other.txt"));

        assert_eq!(first, repeated);
        assert_ne!(first, different);
        assert!(first >= SYNTHETIC_DOCUMENT_NAMESPACE);
        assert!(first < (1_u64 << 53));
    }

    #[test]
    fn reuses_only_regular_files_from_committed_container_directories() {
        let temporary = tempdir().unwrap();
        let nested = temporary.path().join("nested");
        std::fs::create_dir(&nested).unwrap();
        let first = temporary.path().join("one.txt");
        let second = nested.join("two.txt");
        std::fs::write(&first, b"one").unwrap();
        std::fs::write(&second, b"two").unwrap();

        assert_eq!(
            existing_container_manifest(temporary.path()).unwrap(),
            vec![second, first]
        );
    }

    #[tokio::test]
    async fn commits_downloads_idempotently_and_detects_collisions() {
        let temporary = tempdir().unwrap();
        let output = temporary.path().join("portal");
        let first = persist_download(&output, 42, "evidence.txt", b"first")
            .await
            .unwrap();
        let repeated = persist_download(&output, 42, "evidence.txt", b"first")
            .await
            .unwrap();
        assert_eq!(first, repeated);
        assert_eq!(tokio::fs::read(&first).await.unwrap(), b"first");

        let collision = persist_download(&output, 42, "evidence.txt", b"different").await;
        assert!(collision.is_err());
        assert_eq!(tokio::fs::read(&first).await.unwrap(), b"first");
    }

    #[tokio::test]
    async fn cached_payloads_must_be_regular_managed_files() {
        let managed = tempdir().unwrap();
        let outside = tempdir().unwrap();
        let managed_file = managed.path().join("cached.txt");
        let outside_file = outside.path().join("outside.txt");
        tokio::fs::write(&managed_file, b"cached").await.unwrap();
        tokio::fs::write(&outside_file, b"outside").await.unwrap();

        assert_eq!(
            validate_cached_payload(managed.path(), &managed_file, Some(6))
                .await
                .unwrap()
                .1,
            6
        );
        assert!(validate_cached_payload(managed.path(), &outside_file, None)
            .await
            .is_err());
        assert!(
            validate_cached_payload(managed.path(), &managed_file, Some(99))
                .await
                .is_err()
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cached_payloads_reject_symbolic_links() {
        let managed = tempdir().unwrap();
        let target = managed.path().join("target.txt");
        let link = managed.path().join("link.txt");
        tokio::fs::write(&target, b"target").await.unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();

        assert!(validate_cached_payload(managed.path(), &link, None)
            .await
            .is_err());
    }

    #[test]
    fn durable_retry_backoff_doubles_then_saturates() {
        // The live domain. Every call site is gated by
        // `frontier_attempt < MAXIMUM_DURABLE_TASK_ATTEMPTS`, so only attempts 0..4 reach
        // production and the observable schedule is 1s, 1s, 2s, 4s.
        assert_eq!(MAXIMUM_DURABLE_TASK_ATTEMPTS, 4);
        let live: Vec<u64> = (0..MAXIMUM_DURABLE_TASK_ATTEMPTS)
            .map(durable_retry_delay_seconds)
            .collect();
        assert_eq!(live, vec![1, 1, 2, 4]);

        // Past that gate the pure function keeps doubling and saturates at 30s. The cap is NOT
        // reachable within 0..=5: `attempt.saturating_sub(1).min(5)` first yields a shift of 5 at
        // attempt 6, and 1 << 5 == 32 is the first value `.min(30)` actually clamps. Raising
        // MAXIMUM_DURABLE_TASK_ATTEMPTS is what would put the tail below into service.
        let full: Vec<u64> = (0..=5).map(durable_retry_delay_seconds).collect();
        assert_eq!(full, vec![1, 1, 2, 4, 8, 16]);
        assert_eq!(durable_retry_delay_seconds(6), 30);
        assert_eq!(durable_retry_delay_seconds(u64::MAX), 30);
    }

    #[test]
    fn durable_retry_policy_never_replays_permanent_parser_or_security_failures() {
        for error in [
            AppError::DataParsing("malformed document".to_string()),
            AppError::PortalSecurity("unsafe path".to_string()),
            AppError::Storage("database constraint".to_string()),
            AppError::Internal("blocking worker panicked".to_string()),
        ] {
            assert_eq!(durable_retry_delay_for_error(&error, 1, 7), None);
        }

        for error in [
            AppError::Network("connection reset".to_string()),
            AppError::RateLimiter("temporary throttle".to_string()),
            AppError::PortalCapacity("queue is full".to_string()),
        ] {
            assert!(matches!(
                durable_retry_delay_for_error(&error, 1, 7),
                Some(1 | 2)
            ));
            assert_eq!(
                durable_retry_delay_for_error(&error, MAXIMUM_DURABLE_TASK_ATTEMPTS, 7,),
                None
            );
        }
    }

    #[test]
    fn durable_retry_jitter_spreads_rows_without_lowering_the_backoff_floor() {
        let error = AppError::Network("connection reset".to_string());
        let delays = (1..=32)
            .filter_map(|seed| durable_retry_delay_for_error(&error, 3, seed))
            .collect::<HashSet<_>>();
        assert_eq!(durable_retry_delay_seconds(3), 4);
        assert_eq!(delays, HashSet::from([4, 5, 6]));
    }

    #[test]
    fn crawl_identity_sets_never_grow_past_the_policy_budget() {
        let mut identities = HashSet::new();
        assert!(insert_bounded_identity(&mut identities, "first", 2));
        assert!(!insert_bounded_identity(&mut identities, "first", 2));
        assert!(insert_bounded_identity(&mut identities, "second", 2));
        assert!(!insert_bounded_identity(&mut identities, "third", 2));
        assert_eq!(
            identities,
            HashSet::from(["first".to_string(), "second".to_string()])
        );
    }

    #[test]
    fn recursive_container_budget_measures_the_persisted_tree() {
        let fixture = tempdir().expect("fixture directory must open");
        let nested = fixture.path().join("nested");
        std::fs::create_dir(&nested).expect("nested fixture directory must be created");
        std::fs::write(fixture.path().join("first.bin"), b"abc")
            .expect("first fixture must be written");
        std::fs::write(nested.join("second.bin"), b"de").expect("second fixture must be written");

        assert_eq!(
            validate_container_tree_budget_with_limits(fixture.path(), 3, 5)
                .expect("tree at both limits must be accepted"),
            5
        );
        assert!(
            validate_container_tree_budget_with_limits(fixture.path(), 3, 4)
                .unwrap_err()
                .to_string()
                .contains("CONTAINER_TREE_SIZE_EXCEEDED")
        );
        assert!(
            validate_container_tree_budget_with_limits(fixture.path(), 2, 5)
                .unwrap_err()
                .to_string()
                .contains("CONTAINER_TREE_ENTRY_COUNT_EXCEEDED")
        );
    }

    #[tokio::test]
    async fn stopping_workers_does_not_detach_in_flight_blocking_work() {
        let completed = Arc::new(AtomicBool::new(false));
        let worker_completed = Arc::clone(&completed);
        let worker = tokio::spawn(async move {
            tokio::task::spawn_blocking(|| {
                std::thread::sleep(std::time::Duration::from_millis(10));
            })
            .await
            .expect("fixture blocking worker must finish");
            worker_completed.store(true, Ordering::Release);
        });

        join_workers(vec![worker])
            .await
            .expect("worker shutdown must join in-flight work");
        assert!(completed.load(Ordering::Acquire));
    }

    // LIMIT(protocol): the two synthetic identifier generators are NOT disjoint. Both set
    // SYNTHETIC_DOCUMENT_NAMESPACE as the high bit, but `stable_source_identifier` masks the
    // remainder to 51 bits while `archive_entry_id` masks to 52. The crawled-source span is a
    // strict subset of the archive-entry span, so a crawled page and an archive entry can be
    // handed the same `documents.id`.
    //
    // Pinned rather than fixed, deliberately. `documents(id, portal_subdomain)` is a primary key
    // that `provenance_manifests` and the corpus tables carry as a foreign key, so re-masking
    // either generator orphans every already-ingested document and its provenance manifest: a
    // migration, not a one-line mask change. It would also buy very little. Both values are
    // truncated SHA-256, so collisions *within* either span already arrive at roughly 2^26
    // documents by the birthday bound -- setting a partition bit buys one bit against a limit
    // that is two dozen bits lower. Widening the identifier past 53 bits, which means giving up
    // exact representability in JavaScript, is the fix that would actually matter.
    #[test]
    fn synthetic_identifier_spans_overlap_rather_than_partition() {
        let source_span_end = SYNTHETIC_DOCUMENT_NAMESPACE | SYNTHETIC_SOURCE_IDENTIFIER_MASK;
        let digest = "b".repeat(64);

        // Crawled sources occupy only the lower half of the namespace: [2^52, 2^52 + 2^51).
        for value in [
            "page|https://records.example.gov/a",
            "local|/evidence/one.pdf",
            "pasted|abc",
        ] {
            let identifier = stable_source_identifier(value);
            assert!(identifier >= SYNTHETIC_DOCUMENT_NAMESPACE);
            assert!(identifier <= source_span_end);
        }

        // Archive entries occupy the whole namespace, so about half of them land inside the span
        // above. Finding one is the collision surface, stated as an assertion rather than prose.
        let overlapping = (0..64_u64)
            .map(|index| archive_entry_id(index, &digest, 1, Path::new("evidence.txt")))
            .find(|identifier| *identifier <= source_span_end);
        assert!(
            overlapping.is_some(),
            "archive entry identifiers reach into the crawled-source span",
        );
        assert!(overlapping.unwrap() >= SYNTHETIC_DOCUMENT_NAMESPACE);
        assert!(overlapping.unwrap() < (1_u64 << 53));
    }

    /// A filename rule must name itself in the refusal count, not fall back to the generic
    /// `Guidance` variant that predates it. Proven against the pre-wiring behavior: before the
    /// filename glob/regex checks were added, every guidance refusal -- including this one --
    /// returned `FrontierExclusion::Guidance`, which would make this assertion fail.
    #[test]
    fn guidance_allows_url_reports_the_specific_filename_rule_not_the_generic_one() {
        let guidance = SourceGuidance {
            exclude_filename_globs: vec!["*.exe".to_string()],
            ..Default::default()
        };
        let compiled = CompiledGuidancePatterns::compile(&guidance);
        let result = guidance_allows_url(
            &guidance,
            &compiled,
            "https://records.example.test/files/installer.exe",
            None,
        );
        assert_eq!(result, Err(FrontierExclusion::FilenamePattern));
    }

    /// The size predicate must not fire on an unknown size: excluding on an absent measurement is
    /// exactly the absence-versus-measurement violation this project forbids elsewhere. A caller
    /// that has no `content_length` (most discovered links) must see the URL admitted even when a
    /// size range is configured.
    #[test]
    fn guidance_allows_url_does_not_exclude_on_an_unknown_size() {
        let guidance = SourceGuidance {
            minimum_file_size_bytes: Some(1_000),
            maximum_file_size_bytes: Some(2_000),
            ..Default::default()
        };
        let compiled = CompiledGuidancePatterns::compile(&guidance);
        let result = guidance_allows_url(
            &guidance,
            &compiled,
            "https://records.example.test/files/report.pdf",
            None,
        );
        assert_eq!(result, Ok(()));
    }

    #[test]
    fn source_guidance_requires_valid_ordered_calendar_dates() {
        let invalid_date = SourceGuidance {
            document_date_from: Some("2026-02-30".to_string()),
            ..Default::default()
        };
        assert!(validate_source_guidance(&invalid_date).is_err());

        let reversed_range = SourceGuidance {
            document_date_from: Some("2026-08-28".to_string()),
            document_date_to: Some("2026-08-27".to_string()),
            ..Default::default()
        };
        assert!(validate_source_guidance(&reversed_range).is_err());

        let valid_range = SourceGuidance {
            document_date_from: Some("2024-02-29".to_string()),
            document_date_to: Some("2026-08-27".to_string()),
            ..Default::default()
        };
        assert!(validate_source_guidance(&valid_range).is_ok());
    }

    #[test]
    fn only_nextrequest_metadata_supplies_a_document_date_for_guidance() {
        let timestamp = "2024-06-01T00:00:00Z";
        assert_eq!(
            document_date_for_guidance(SourceKind::NextRequest, timestamp),
            Some("2024-06-01")
        );
        assert_eq!(
            document_date_for_guidance(SourceKind::NextRequest, "2024-02-30T00:00:00Z"),
            None
        );
        assert_eq!(
            document_date_for_guidance(SourceKind::NextRequest, "not-a-date"),
            None
        );
        for kind in [
            SourceKind::WebCrawl,
            SourceKind::DirectFile,
            SourceKind::OpenDataCatalog,
            SourceKind::ArcGis,
            SourceKind::Tableau,
            SourceKind::GoogleDrive,
            SourceKind::LocalFile,
            SourceKind::PastedContent,
        ] {
            assert_eq!(document_date_for_guidance(kind, timestamp), None);
        }
    }

    /// Each post-extraction predicate must report the rule that actually fired rather than a
    /// shared or wrong variant -- a content-pattern refusal reported as `ContentKeyword` (or vice
    /// versa) would misname the rule an operator sees in the refusal count. Proven per predicate;
    /// deleting any one branch's distinct return would collapse two of these to the same variant
    /// and fail the corresponding assertion.
    #[test]
    fn evaluate_extracted_document_guidance_names_the_predicate_that_fired() {
        let base = SourceGuidance::default();
        let compiled_base = CompiledGuidancePatterns::compile(&base);

        let keyword_guidance = SourceGuidance {
            exclude_content_keywords: vec!["classified".to_string()],
            ..SourceGuidance::default()
        };
        assert_eq!(
            evaluate_extracted_document_guidance(
                &keyword_guidance,
                &CompiledGuidancePatterns::compile(&keyword_guidance),
                "this record is classified pending review",
                None,
                Some("2024-06-01T00:00:00Z"),
            ),
            Err(FrontierExclusion::ContentKeyword)
        );

        let pattern_guidance = SourceGuidance {
            exclude_content_patterns: vec![r"\bSSN\b".to_string()],
            ..SourceGuidance::default()
        };
        assert_eq!(
            evaluate_extracted_document_guidance(
                &pattern_guidance,
                &CompiledGuidancePatterns::compile(&pattern_guidance),
                "applicant SSN on file",
                None,
                Some("2024-06-01T00:00:00Z"),
            ),
            Err(FrontierExclusion::ContentPattern)
        );

        let organization_guidance = SourceGuidance {
            include_organizations: vec!["Parks".to_string()],
            ..SourceGuidance::default()
        };
        assert_eq!(
            evaluate_extracted_document_guidance(
                &organization_guidance,
                &CompiledGuidancePatterns::compile(&organization_guidance),
                "unrelated content",
                Some("Department of Public Works"),
                Some("2024-06-01T00:00:00Z"),
            ),
            Err(FrontierExclusion::Organization)
        );

        let date_guidance = SourceGuidance {
            document_date_from: Some("2025-01-01".to_string()),
            ..SourceGuidance::default()
        };
        assert_eq!(
            evaluate_extracted_document_guidance(
                &date_guidance,
                &CompiledGuidancePatterns::compile(&date_guidance),
                "unrelated content",
                None,
                Some("2024-06-01T00:00:00Z"),
            ),
            Err(FrontierExclusion::DocumentDate)
        );

        assert_eq!(
            evaluate_extracted_document_guidance(
                &base,
                &compiled_base,
                "unrelated content",
                None,
                Some("2024-06-01T00:00:00Z"),
            ),
            Ok(())
        );

        // A missing document-carried date is an absent measurement, not evidence that the
        // document falls outside the range. Non-NextRequest ingestion paths rely on this behavior
        // because their metadata date is only the ingestion instant.
        assert_eq!(
            evaluate_extracted_document_guidance(
                &date_guidance,
                &CompiledGuidancePatterns::compile(&date_guidance),
                "unrelated content",
                None,
                None,
            ),
            Ok(())
        );
    }
}
