//! Public registries an extracted identifier can be anchored to.
//!
//! An anchor is a statement about a number, never about a party: "licence 123456 is registered to
//! NAME in the snapshot imported on D" says nothing about whether any person or organization in
//! the documents is that registrant. Every match names the snapshot it was read from, so the same
//! lookup against the same file gives the same answer.

use serde::{Deserialize, Serialize};

use super::CorpusEntityKind;

/// A registry whose snapshot the operator can import. One snapshot per registry is held.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RegistryKind {
    /// CSLB "License Master" list: every licence currently renewed or expired but renewable, as
    /// the free CSV download publishes it. Not a history: a licence absent from it may have been
    /// cancelled, revoked, or never issued, and the list does not say which.
    CslbLicenseMaster,
}

impl RegistryKind {
    pub const ALL: [RegistryKind; 1] = [RegistryKind::CslbLicenseMaster];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CslbLicenseMaster => "CSLB_LICENSE_MASTER",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == value)
    }

    /// The extracted identifier kind this registry is keyed by.
    pub const fn identifier_kind(self) -> CorpusEntityKind {
        match self {
            Self::CslbLicenseMaster => CorpusEntityKind::ContractorLicense,
        }
    }

    pub fn for_identifier(kind: CorpusEntityKind) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|registry| registry.identifier_kind() == kind)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RegistrySnapshotSummary {
    pub registry: RegistryKind,
    pub identifier_kind: CorpusEntityKind,
    pub source_path: String,
    pub file_sha256: String,
    pub imported_at: String,
    pub record_count: u64,
    /// Rows the importer could not key: no identifier, a malformed one, or a repeat of one already
    /// taken. Reported rather than dropped silently, so a thin snapshot is visibly thin.
    pub skipped_row_count: u64,
}

/// What the registry says about one identifier, as the registry wrote it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RegistryRecord {
    pub registrant_name: String,
    pub status: String,
    pub detail: String,
}

/// The four answers a lookup can give. They are distinct facts and must stay distinct on screen:
/// "no snapshot has been imported" is not "the snapshot does not list this number", and neither
/// is "this identifier kind has no registry".
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "state",
    rename_all = "SCREAMING_SNAKE_CASE",
    rename_all_fields = "camelCase"
)]
pub enum RegistryAnchor {
    NoRegistry {
        identifier_kind: CorpusEntityKind,
    },
    NotImported {
        registry: RegistryKind,
    },
    NotListed {
        snapshot: RegistrySnapshotSummary,
        identifier: String,
    },
    Registered {
        snapshot: RegistrySnapshotSummary,
        identifier: String,
        record: RegistryRecord,
    },
}
