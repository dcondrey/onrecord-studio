//! Standing searches: a saved query, the corpus state it last ran against, and what it returned.
//!
//! A run is reproducible evidence only if it names the corpus it saw. Every run therefore carries
//! the corpus-state digest (sha256 over the ordered portal, document id and extracted-text digest
//! set) and the digest of its own match set, so "query Q against corpus C returned R" can be
//! re-derived by anyone holding the same corpus.

use serde::{Deserialize, Serialize};

/// The flat native filter the query AST compiled to. Stored beside the AST because this is what
/// executes; the AST is what the operator wrote.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StandingQueryFilters {
    pub search_term: String,
    pub portal_key: Option<String>,
    pub agency_id: Option<u64>,
    pub status: Option<String>,
    pub policy_deviation: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StandingQueryInput {
    pub label: String,
    /// The `QueryAst` from `src/query`, serialized verbatim. Refused when it holds a filter the
    /// flat native query cannot execute, so a saved query never silently runs as a looser one.
    pub ast_json: String,
    pub filters: StandingQueryFilters,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StandingQuery {
    pub id: String,
    pub label: String,
    pub ast_json: String,
    pub filters: StandingQueryFilters,
    pub created_at: String,
}

/// Whether a count was measured over every match or scaled up from a sample. The two are not
/// the same claim and must not be rendered alike.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "basis",
    rename_all = "SCREAMING_SNAKE_CASE",
    rename_all_fields = "camelCase"
)]
pub enum CountBasis {
    Exact,
    EstimatedFromSample { sample_size: u64 },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FacetValue {
    pub value: String,
    pub count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Facet {
    pub dimension: String,
    /// Largest first. `truncated` says more values exist than are listed.
    pub values: Vec<FacetValue>,
    pub truncated: bool,
}

/// Matching requests that produced byte-identical extracted text: the same document released
/// under several requests, found by digest rather than by any similarity judgment.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DocumentCluster {
    pub extracted_text_sha256: String,
    pub request_count: u64,
    pub document_count: u64,
    pub example_file_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StandingQueryResult {
    /// Always exact: a count over the whole match set is cheap, only the breakdowns are sampled.
    pub match_count: u64,
    /// sha256 over the ordered (portal key, request id) match set.
    pub matches_digest: String,
    /// Governs `facets` and `clusters`.
    pub count_basis: CountBasis,
    pub facets: Vec<Facet>,
    pub clusters: Vec<DocumentCluster>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StandingQueryRun {
    pub run_id: String,
    pub query_id: String,
    pub ran_at: String,
    pub corpus_digest: String,
    pub corpus_document_count: u64,
    pub result: StandingQueryResult,
    /// `None` on the first run: there is nothing to have changed from.
    pub changed_since_previous: Option<bool>,
    pub previous_match_count: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StandingQuerySummary {
    pub query: StandingQuery,
    pub last_run: Option<StandingQueryRun>,
    pub current_corpus_digest: String,
    /// The corpus has changed since the last run, or there has been none.
    pub due: bool,
}

/// What one run request did for one query.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "outcome",
    rename_all = "SCREAMING_SNAKE_CASE",
    rename_all_fields = "camelCase"
)]
pub enum StandingQueryOutcome {
    Ran {
        run: StandingQueryRun,
    },
    /// The corpus is the one the last run saw; nothing to re-run against.
    NotDue {
        query_id: String,
    },
    /// Could not be run honestly right now; `reason` says why.
    Deferred {
        query_id: String,
        reason: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SeriesMember {
    pub entity_id: String,
    pub display_value: String,
    pub document_count: u64,
}

/// Numbers of one labelled series that no document in the corpus carries, bounded by the two
/// members on either side. Absence from the corpus is not absence from the agency's files.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SeriesGap {
    pub label: String,
    pub prefix: String,
    pub from_number: u64,
    pub to_number: u64,
    /// Digit width the series writes its numbers with, for display.
    pub width: usize,
    pub before: SeriesMember,
    pub after: SeriesMember,
    pub series_member_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SeriesGapReport {
    pub gaps: Vec<SeriesGap>,
    /// Series with enough members to be examined for gaps.
    pub series_count: u64,
    pub truncated: bool,
}
