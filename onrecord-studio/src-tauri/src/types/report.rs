//! Shapes of the verifiable "show your work" report. The manifest is the signed payload embedded
//! in the report file; everything a reader sees is rendered from these fields and nothing else.

use serde::{Deserialize, Serialize};

pub const REPORT_SCHEMA_VERSION: &str = "onrecord-verifiable-report-v1";

/// Why a claim is or is not a legal statement. Derived from `FindingKind::is_legal_claim`, never
/// chosen by the caller.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReportEvidenceBasis {
    /// A counted comparison against a rule or commitment the record itself states.
    CountedViolation,
    /// Slower-than-peers, reuse, or clustering. Evidence for review, not a legal conclusion.
    PatternSignal,
}

impl ReportEvidenceBasis {
    /// The sentence a reader sees beside a claim. Fixed wording, so every rendering of a claim
    /// states its basis the same way.
    pub const fn label(self) -> &'static str {
        match self {
            Self::CountedViolation => {
                "Counted violation: a counted comparison against a rule or commitment the record itself states"
            }
            Self::PatternSignal => "Pattern signal: evidence for review, not a legal conclusion",
        }
    }
}

/// Mirrors `FindingConfidence` with an explicit wire shape so a verifier in another language can
/// tell "not assessed" from a measured zero without inferring from a missing field.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReportConfidence {
    Unassessed,
    #[serde(rename_all = "camelCase")]
    Measured {
        basis_points: u16,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum QuoteStatus {
    /// The detector's quote is contained in the stored text at the cited range; the range text is
    /// what is shown.
    VerifiedAgainstStoredText,
    /// The detector supplied no quote; the text at the cited range is shown.
    FromStoredTextRange,
    /// No range, or no stored text to compare with. The detector's quote is shown unchecked.
    UncheckedDetectorQuote,
    /// The detector's quote does not appear at the cited range, so it is not published.
    WithheldMismatch,
    /// The document is not public, so no excerpt is published.
    WithheldNonPublicDocument,
    /// The citation carries neither a quote nor a usable range.
    NoQuote,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReportCitation {
    pub portal_key: String,
    pub document_id: i64,
    pub label: Option<String>,
    pub file_name: Option<String>,
    pub character_start: Option<i64>,
    pub character_end: Option<i64>,
    pub quote: Option<String>,
    pub quote_status: QuoteStatus,
    pub quote_truncated: bool,
    pub pii_masked_count: usize,
    /// `None` is "not recorded", never an empty digest.
    pub raw_file_sha256: Option<String>,
    pub extracted_text_projection_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReportClaim {
    pub finding_id: String,
    pub kind: String,
    pub severity: String,
    pub evidence_basis: ReportEvidenceBasis,
    pub confidence: ReportConfidence,
    pub headline: String,
    pub explanation: String,
    pub source_detector: String,
    pub discovered_at: String,
    pub last_seen_at: String,
    pub portal_key: Option<String>,
    pub request_id: Option<i64>,
    pub citations: Vec<ReportCitation>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WithheldReason {
    InferredRedactionIdentity,
    DismissedByReviewer,
    LegalClaimWithoutMeasuredConfidence,
    QuoteMismatch,
    QuoteNonPublicDocument,
}

impl WithheldReason {
    pub const fn explanation(self) -> &'static str {
        match self {
            Self::InferredRedactionIdentity => {
                "Findings that infer who a redaction hides are hypotheses, not facts. They are \
                 never included, and their text is not reproduced here."
            }
            Self::DismissedByReviewer => {
                "Findings a reviewer dismissed are not presented as claims."
            }
            Self::LegalClaimWithoutMeasuredConfidence => {
                "A legal-claim finding with no measured confidence cannot be stated as a legal \
                 claim."
            }
            Self::QuoteMismatch => {
                "A quoted excerpt that does not appear at the cited character range of the \
                 stored text was not published; the citation remains without the quote."
            }
            Self::QuoteNonPublicDocument => {
                "Excerpts from documents that are not public are not published; the citation \
                 remains without the excerpt."
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WithheldItem {
    pub reason: WithheldReason,
    pub count: usize,
    pub explanation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VerifiableReportManifest {
    pub schema_version: String,
    /// SHA-256 of the report template with the payload slot left as its literal placeholder. It is
    /// inside the signed bytes so a change to the page's own markup or script breaks a
    /// command-line verification, not just a change to the payload.
    pub template_sha256: String,
    pub generated_at: String,
    pub title: String,
    pub author_name: Option<String>,
    pub scope_portal_key: Option<String>,
    pub claims: Vec<ReportClaim>,
    pub withheld: Vec<WithheldItem>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EvidencePacketExportResult {
    pub markdown_path: String,
    pub json_path: String,
    pub content_sha256: String,
    pub claim_count: usize,
    pub gap_count: usize,
    pub withheld_count: usize,
    pub generated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VerifiableReportExportResult {
    pub output_path: String,
    pub file_sha256: String,
    pub payload_sha256: String,
    pub signer_key_id: String,
    pub identity_assurance: String,
    pub claim_count: usize,
    pub withheld_count: usize,
    pub generated_at: String,
}
