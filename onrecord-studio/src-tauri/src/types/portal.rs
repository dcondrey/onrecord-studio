use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SourceKind {
    #[default]
    NextRequest,
    WebCrawl,
    DirectFile,
    OpenDataCatalog,
    ArcGis,
    Tableau,
    GoogleDrive,
    LocalFile,
    PastedContent,
}

impl SourceKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NextRequest => "NEXT_REQUEST",
            Self::WebCrawl => "WEB_CRAWL",
            Self::DirectFile => "DIRECT_FILE",
            Self::OpenDataCatalog => "OPEN_DATA_CATALOG",
            Self::ArcGis => "ARC_GIS",
            Self::Tableau => "TABLEAU",
            Self::GoogleDrive => "GOOGLE_DRIVE",
            Self::LocalFile => "LOCAL_FILE",
            Self::PastedContent => "PASTED_CONTENT",
        }
    }

    /// Kinds whose documents the user put in by hand. Nothing was fetched from an agency, so such
    /// a document is never evidence of what an agency released, withheld or said. The
    /// `user_supplied_sources` view is generated from this list.
    pub const USER_SUPPLIED: [Self; 2] = [Self::LocalFile, Self::PastedContent];

    pub fn is_user_supplied(self) -> bool {
        Self::USER_SUPPLIED.contains(&self)
    }

    pub fn from_stored(value: &str) -> Self {
        match value {
            "WEB_CRAWL" => Self::WebCrawl,
            "DIRECT_FILE" => Self::DirectFile,
            "OPEN_DATA_CATALOG" => Self::OpenDataCatalog,
            "ARC_GIS" => Self::ArcGis,
            "TABLEAU" => Self::Tableau,
            "GOOGLE_DRIVE" => Self::GoogleDrive,
            "LOCAL_FILE" => Self::LocalFile,
            "PASTED_CONTENT" => Self::PastedContent,
            _ => Self::NextRequest,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PortalApiLayout {
    pub requests_endpoint: String,
    pub requests_pointer: String,
    pub metadata_pointer: Option<String>,
    pub page_parameter: String,
    pub page_size_parameter: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PortalRegistration {
    pub portal_key: String,
    pub display_name: String,
    pub canonical_url: String,
    pub origin_url: String,
    pub api_layout: PortalApiLayout,
    pub verified_at: String,
    #[serde(default)]
    pub source_kind: SourceKind,
}

/// Whether a saved source is still being synchronized, and if not, why not.
///
/// This is durable on purpose. Stopping a source used to live only in the in-memory pool map, so
/// the next launch found the source's frontier rows, read them as an interrupted crawl, and
/// resumed a source the operator had deliberately halted.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SourceLifecycleState {
    /// Synchronizes when started, and is offered for resumption when the app opens.
    #[default]
    Active,
    /// Halted by the operator. The frontier is kept so a resume continues rather than restarts.
    Paused,
    /// Withdrawn by the operator. Documents already ingested from it are retained; what is
    /// discarded is the crawl frontier, the connector recipe, and the source's place in the list.
    Removed,
}

impl SourceLifecycleState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "ACTIVE",
            Self::Paused => "PAUSED",
            Self::Removed => "REMOVED",
        }
    }

    /// Reads a stored marker. An absent or unrecognized marker means the source was never paused
    /// or removed, which is what `Active` says, so this is a fact about the row and not a default
    /// standing in for an unrecorded one.
    pub fn from_stored(value: &str) -> Self {
        match value {
            "PAUSED" => Self::Paused,
            "REMOVED" => Self::Removed,
            _ => Self::Active,
        }
    }
}

/// A registration paired with its lifecycle state.
///
/// The two are carried side by side rather than merged because `PortalRegistration` is compared
/// for equality to decide whether a portal key still names the same verified endpoint. Folding
/// mutable runtime state into that identity would let a pause raise an endpoint-changed security
/// refusal, which would be a false statement about the source.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SavedSource {
    pub registration: PortalRegistration,
    pub lifecycle_state: SourceLifecycleState,
    /// Durable source-health state. Recovery failures are carried here because startup telemetry
    /// can be emitted before the webview has attached its event listener.
    pub health_status: String,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CollectionPlanSource {
    pub portal_key: String,
    pub display_name: String,
    pub canonical_url: String,
    pub source_kind: SourceKind,
    /// The last session queued work here that never ran.
    pub has_unfinished_work: bool,
    /// The subject this source was last collected for; `None` collected everything in scope.
    pub subject: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CollectionPlan {
    pub sources: Vec<CollectionPlanSource>,
}

/// What withdrawing a source actually discarded, and what it kept.
///
/// The retained document count is returned rather than assumed because "Remove" that silently
/// keeps hundreds of ingested records is the same false statement as one that silently destroys
/// them. The caller is expected to say which happened.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SourceRemoval {
    pub source_key: String,
    pub discarded_frontier_tasks: u64,
    pub retained_documents: u64,
}

/// What purging a single document destroyed.
///
/// `local_path` is reported rather than unlinked by the repository, because the repository owns
/// rows and not the filesystem. It is `None` when another surviving row still references the same
/// content-addressed file, which must then outlive this purge. A caller that ignores it leaves an orphaned evidence file, which is
/// recoverable; a repository that deleted the file inside its transaction would leave an
/// unrecoverable gap whenever the transaction rolled back.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DocumentPurge {
    pub document_id: u64,
    pub source_key: String,
    pub local_path: Option<String>,
    pub additional_local_paths: Vec<String>,
    pub search_rows_removed: u64,
    pub provenance_rows_removed: u64,
}

/// What purging an entire source destroyed.
///
/// Counts are reported for the same reason [`SourceRemoval`] reports its retained documents: a
/// destructive action that does not say what it destroyed is as misleading as one that silently
/// keeps what the operator asked to remove.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SourcePurge {
    pub source_key: String,
    pub documents_removed: u64,
    pub requests_removed: u64,
    /// Files no surviving row references; a content-addressed file shared with another source is
    /// kept and not listed.
    pub local_paths: Vec<String>,
}

/// What purging a source *would* destroy, counted without destroying anything.
///
/// The fields are counts and are named as counts. A preview field named `documentsRemoved` would
/// claim a removal that has not happened, which is the same class of false statement as rendering
/// an absence as a measurement. The counts are read with the predicates
/// [`crate::storage::sqlite::DuckDbRepository::purge_source`] deletes on, and a test purges immediately
/// after previewing to prove the two agree; a confirmation that names a number the purge does not
/// honour is worse than one that names none.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SourcePurgePreview {
    pub source_key: String,
    pub document_count: u64,
    pub request_count: u64,
    pub evidence_file_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PortalSyncStart {
    pub portal: PortalRegistration,
    pub sync_run_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PortalRuntimeSnapshot {
    pub portal: PortalRegistration,
    pub sync_run_id: String,
    pub queued_tasks: usize,
    pub active_tasks: usize,
    pub completed_tasks: u64,
    pub failed_tasks: u64,
    pub pages_processed: u64,
    pub requests_processed: u64,
    pub documents_processed: u64,
    pub is_running: bool,
    pub rate_limit_tokens: f64,
    /// How many discovered links each frontier rule refused. Present so a filtered crawl reports
    /// what it declined rather than presenting the survivors as the whole of what was there.
    pub frontier_exclusions: Vec<FrontierExclusionCount>,
    /// Hosts this run stopped contacting, and why. A stopped host is a bound on coverage, so it
    /// is reported rather than folded into the failure count.
    pub stopped_hosts: Vec<StoppedHost>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FrontierExclusionCount {
    pub rule: String,
    pub excluded_links: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StoppedHost {
    pub host: String,
    pub requests_spent: usize,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RateLimitPolicy {
    pub burst_capacity: u32,
    pub refill_tokens_per_second: f64,
}

impl Default for RateLimitPolicy {
    fn default() -> Self {
        Self {
            burst_capacity: 5,
            refill_tokens_per_second: 1.0,
        }
    }
}
