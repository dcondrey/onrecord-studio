use crate::types::c2pa::{ProvenanceConfidence, RecordedDigestSource, SignatureVerificationState};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum StatutoryExtensionReason {
    SeparateFieldFacilities,
    VoluminousRecords,
    InteragencyConsultation,
    DataCompilation,
    CyberattackSystemsUnavailable,
    DeclaredEmergencyOperationalImpact,
}

impl StatutoryExtensionReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SeparateFieldFacilities => "SEPARATE_FIELD_FACILITIES",
            Self::VoluminousRecords => "VOLUMINOUS_RECORDS",
            Self::InteragencyConsultation => "INTERAGENCY_CONSULTATION",
            Self::DataCompilation => "DATA_COMPILATION",
            Self::CyberattackSystemsUnavailable => "CYBERATTACK_SYSTEMS_UNAVAILABLE",
            Self::DeclaredEmergencyOperationalImpact => "DECLARED_EMERGENCY_OPERATIONAL_IMPACT",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StatutoryExtensionRecord {
    pub notice_at: String,
    pub expected_determination_at: String,
    pub reason: StatutoryExtensionReason,
    pub notice_source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AuxiliaryStreamKind {
    Municipal311,
    CodeEnforcement,
    PoliceDispatch,
    SanitationOperations,
    Custom,
}

impl AuxiliaryStreamKind {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Municipal311 => "MUNICIPAL_311",
            Self::CodeEnforcement => "CODE_ENFORCEMENT",
            Self::PoliceDispatch => "POLICE_DISPATCH",
            Self::SanitationOperations => "SANITATION_OPERATIONS",
            Self::Custom => "CUSTOM",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AuxiliaryEventInput {
    pub external_id: String,
    pub occurred_at: String,
    pub neighborhood_block: String,
    pub event_type: String,
    pub description: String,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub source_payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AuxiliaryImportResult {
    pub source_id: String,
    pub imported_count: usize,
    pub updated_count: usize,
    pub rejected_count: usize,
    pub source_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CrossAgencyCorrelation {
    pub portal_key: String,
    pub request_id: u64,
    pub agency_id: Option<u64>,
    pub agency_name: String,
    pub request_title: String,
    pub request_timestamp: String,
    pub neighborhood_block: String,
    pub complaint_count: usize,
    pub correlation_window_hours: usize,
    pub distinct_source_count: usize,
    pub first_correlated_event_at: String,
    pub last_correlated_event_at: String,
    pub keyword_signal_count: usize,
    pub correlation_density: f64,
}

/// Result of comparing a stored document against whatever provenance evidence exists for it.
///
/// Read [`Self::provenance_confidence`] before presenting any of the booleans below. The boolean
/// fields report two different kinds of thing and only one of them is a check: `hash_matches` is a
/// result this application computed, while `manifest_present` and `is_cryptographically_signed`
/// mirror rows in local storage.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DocumentProvenanceVerification {
    pub document_id: u64,
    pub portal_key: String,
    pub expected_sha256: Option<String>,
    /// Digest of the stored bytes, or empty when no readable evidence file existed. The field
    /// stays a plain string rather than becoming optional so the serialized shape is unchanged;
    /// read [`Self::evidence_file_present`] to tell an empty digest from a real one.
    pub computed_sha256: String,
    /// Whether a readable evidence file was found and hashed. False means nothing was compared,
    /// and [`Self::hash_matches`] is false because no check ran, not because one failed.
    #[serde(default = "default_evidence_file_present")]
    pub evidence_file_present: bool,
    /// The stored bytes hash to [`Self::expected_sha256`]. A performed check.
    pub hash_matches: bool,
    /// Which recorded digest [`Self::expected_sha256`] came from. Matching an ingestion digest is
    /// local integrity; matching a manifest digest is a claim by a third party that was confirmed.
    #[serde(default = "default_recorded_digest_source")]
    pub recorded_digest_source: RecordedDigestSource,
    /// A manifest row exists locally. A stored claim.
    pub manifest_present: bool,
    /// The manifest *claims* a signature. A stored claim, never a verification result. See
    /// [`Self::signature_verification`] for whether anything was actually checked.
    pub is_cryptographically_signed: bool,
    /// Whether a signature was checked, and what the check said. `NOT_ATTEMPTED` means the asset
    /// carried no manifest the C2PA SDK could read, not that validation is unimplemented.
    #[serde(default = "default_signature_verification")]
    pub signature_verification: SignatureVerificationState,
    pub signature_issuer: Option<String>,
    pub signing_timestamp: Option<String>,
    pub assertion_count: usize,
    pub invalid_assertion_count: usize,
    /// Digest recorded for the normalized text projection when the document was extracted.
    ///
    /// `None` for a document extracted before the digest was recorded, or imported from a legacy
    /// core. Distinct from `extracted_text_sha256` on the document row, which covers the raw
    /// extracted text -- text that is not retained and so cannot be re-checked.
    #[serde(default)]
    pub text_projection_sha256: Option<String>,
    /// Whether the stored text projection still hashes to [`Self::text_projection_sha256`].
    ///
    /// `None` means the comparison could not be made, not that it passed: either no digest was
    /// recorded or no projection row exists. This is reported separately from
    /// [`Self::provenance_confidence`], which describes the asset as received; a document can be
    /// byte-identical to what was ingested while the text quoted from it has drifted.
    #[serde(default)]
    pub text_projection_matches: Option<bool>,
    /// The strongest evidence state supported by what was actually checked.
    #[serde(default = "default_provenance_confidence")]
    pub provenance_confidence: ProvenanceConfidence,
    /// Independent third-party evidence of chain of custody.
    ///
    /// This is true only at [`ProvenanceConfidence::TrustedChainOfCustody`], which requires the
    /// signer to chain to a configured trust anchor. No anchors are configured by default, so it
    /// is false in a default deployment. It previously reported true whenever
    /// recorded manifest claims looked complete, which presented stored assertions as validated
    /// evidence. Consumers wanting the old, weaker signal should read
    /// [`Self::provenance_confidence`] and decide explicitly what it is worth.
    pub chain_of_custody_confirmed: bool,
    pub verification_errors: Vec<String>,
}

fn default_evidence_file_present() -> bool {
    // Results serialized before this field existed were only ever produced by hashing a file.
    true
}

fn default_recorded_digest_source() -> RecordedDigestSource {
    RecordedDigestSource::None
}

fn default_signature_verification() -> SignatureVerificationState {
    SignatureVerificationState::NotAttempted
}

fn default_provenance_confidence() -> ProvenanceConfidence {
    ProvenanceConfidence::NoRecordedDigest
}

/// One row of the append-only provenance verification log.
///
/// Deliberately not a [`DocumentProvenanceVerification`]. A row written before classification
/// existed cannot be reconstructed into one, and presenting it as though it could would attach a
/// confidence level to a check that never recorded one. Those fields are `None` and
/// [`Self::predates_classification`] is true.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProvenanceVerificationRecord {
    pub verification_id: String,
    pub document_id: u64,
    pub portal_key: String,
    pub verified_at: String,
    pub expected_sha256: Option<String>,
    pub computed_sha256: String,
    /// `None` on a row written before the field was recorded.
    pub evidence_file_present: Option<bool>,
    pub hash_matches: bool,
    pub manifest_present: bool,
    pub is_cryptographically_signed: bool,
    pub signature_issuer: Option<String>,
    pub signing_timestamp: Option<String>,
    /// `None` on a row written before the field was recorded.
    pub assertion_count: Option<usize>,
    pub invalid_assertion_count: usize,
    pub recorded_digest_source: Option<RecordedDigestSource>,
    pub signature_verification: Option<SignatureVerificationState>,
    pub provenance_confidence: Option<ProvenanceConfidence>,
    pub text_projection_sha256: Option<String>,
    pub text_projection_matches: Option<bool>,
    pub chain_of_custody_confirmed: bool,
    pub verification_errors: Vec<String>,
    /// The row predates the classification columns, so its confidence was never recorded. Not the
    /// same as a recorded low-confidence result.
    pub predates_classification: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LegalBriefExportResult {
    pub request_id: u64,
    pub portal_key: String,
    pub output_path: String,
    pub report_sha256: String,
    pub generated_at: String,
    pub included_violation_markers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RequestSearchResult {
    pub request_id: u64,
    pub portal_key: String,
    pub agency_id: Option<u64>,
    pub agency_name: String,
    pub title: String,
    pub description_preview: String,
    pub status: String,
    pub created_at: String,
    pub closed_at: Option<String>,
    pub document_count: usize,
    pub integrity_flags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RequestDocumentSummary {
    pub document_id: u64,
    pub portal_key: String,
    pub file_name: String,
    pub file_size: Option<u64>,
    pub content_type: Option<String>,
    pub visibility: String,
    pub download_status: String,
    pub raw_file_sha256: Option<String>,
    pub extracted_text_sha256: Option<String>,
    pub extraction_status: String,
    pub extraction_format: Option<String>,
    pub extraction_deferred_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DocumentTextChunk {
    pub document_id: u64,
    pub portal_key: String,
    pub character_offset: usize,
    pub returned_character_count: usize,
    pub next_character_offset: Option<usize>,
    pub total_character_count: usize,
    pub masked_text: String,
    pub masked_finding_count: usize,
}
