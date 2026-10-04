//! Re-fetch observations of collected documents. See `document_source_checks` in the schema for
//! what each outcome does and does not claim.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SourceCheckOutcome {
    SameBytes,
    DifferentBytes,
    Gone,
    Unreachable,
}

impl SourceCheckOutcome {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SameBytes => "SAME_BYTES",
            Self::DifferentBytes => "DIFFERENT_BYTES",
            Self::Gone => "GONE",
            Self::Unreachable => "UNREACHABLE",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        [
            Self::SameBytes,
            Self::DifferentBytes,
            Self::Gone,
            Self::Unreachable,
        ]
        .into_iter()
        .find(|outcome| outcome.as_str() == value)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SourceCheck {
    pub check_id: u64,
    pub run_id: String,
    pub portal_key: String,
    pub document_id: u64,
    pub requested_url: String,
    /// Local clock when the check finished; not a trusted timestamp.
    pub checked_at: String,
    pub outcome: SourceCheckOutcome,
    pub http_status: Option<u16>,
    pub collected_sha256: String,
    pub observed_sha256: Option<String>,
    pub detail: Option<String>,
    /// The acquisition this check's received bytes appended. `None`: no bytes arrived, or the
    /// check predates the link.
    pub acquisition_id: Option<u64>,
    /// Whether the different bytes this check received are kept in the evidence store. `false`
    /// on a DIFFERENT_BYTES check means they were not kept, never that they matched.
    pub observed_copy_retained: bool,
}

/// One row of `document_acquisitions`: a network fetch that received a document's bytes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DocumentAcquisition {
    pub acquisition_id: u64,
    pub sync_run_id: Option<String>,
    pub requested_url: String,
    pub resolved_url: String,
    pub http_status: u16,
    pub content_type: Option<String>,
    pub raw_file_sha256: String,
    pub file_size: u64,
    /// The digest the document held when this fetch began; `None` on its first fetch.
    pub previous_raw_file_sha256: Option<String>,
    /// Local clock at receipt; not a trusted timestamp.
    pub retrieved_at: String,
    /// The source check that made this fetch, when one did.
    pub source_check_id: Option<u64>,
}

/// Everything recorded about fetching one document, newest first in each list.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DocumentSourceHistory {
    pub checks: Vec<SourceCheck>,
    pub acquisitions: Vec<DocumentAcquisition>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SourceRecheckSummary {
    pub run_id: String,
    pub portal_key: String,
    pub same_bytes: u32,
    pub different_bytes: u32,
    pub gone: u32,
    pub unreachable: u32,
    /// Collected documents with a download URL that this run did not reach. They have no row for
    /// this run, which means not checked, not unchanged.
    pub not_checked: u64,
    /// Why the run ended before checking every eligible document, when it did.
    pub stopped_early: Option<String>,
}
