use super::research::ResearchSourceReference;
use crate::analysis::request_compiler::RequestFilingPlan;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct CaseworkCitation {
    pub id: String,
    pub report_id: String,
    pub evidence_id: String,
    pub label: String,
    pub quote: String,
    pub source_url: Option<String>,
    pub source_sha256: String,
    pub source_references: Vec<ResearchSourceReference>,
    pub portal_key: Option<String>,
    pub document_id: Option<u64>,
    pub character_offset: Option<usize>,
    /// SAVED_REPORT, HISTORICAL_REPORT or UNVERIFIED_PORTABLE. Recomputed by the repository.
    pub verification: String,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct CaseworkQuestion {
    pub id: String,
    pub question: String,
    pub hypothesis: String,
    pub next_test: String,
    pub disposition: String,
    pub owner: String,
    pub citation_ids: Vec<String>,
    pub counter_citation_ids: Vec<String>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct CaseworkProduction {
    pub id: String,
    pub received_at: String,
    pub description: String,
    pub record_types: Vec<String>,
    pub fields: Vec<String>,
    pub period: String,
    pub citation_ids: Vec<String>,
    pub notes: String,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct CaseworkRequest {
    pub id: String,
    pub title: String,
    pub custodian: String,
    pub gap: String,
    pub resolves: String,
    pub draft: String,
    pub state: String,
    pub expected_record_types: Vec<String>,
    pub expected_fields: Vec<String>,
    pub expected_period: String,
    pub submitted_at: Option<String>,
    pub response_due: Option<String>,
    pub external_reference: String,
    pub productions: Vec<CaseworkProduction>,
    pub unresolved_omissions: Vec<String>,
    pub follow_up_draft: String,
    pub citation_ids: Vec<String>,
    /// The CPRA request compiled from this request's scope and its identifying citations.
    /// Recomputed by the repository on every save; an operator edit belongs in `draft`.
    pub compiled_draft: String,
    /// The modeled determination clock for this request. `None` until a submission date is
    /// recorded: a request that was never filed has no clock, which is not a clock reading zero.
    pub filing_plan: Option<RequestFilingPlan>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct CaseworkDecision {
    pub id: String,
    pub body: String,
    pub date: String,
    pub agenda_item: String,
    pub proposed_decision: String,
    pub action: String,
    pub outcome: String,
    pub vote: String,
    pub state: String,
    pub citation_ids: Vec<String>,
    pub outcome_citation_ids: Vec<String>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct CaseworkCommitment {
    pub id: String,
    pub promise: String,
    pub responsible_party: String,
    pub due_date: Option<String>,
    pub completion_test: String,
    pub state: String,
    pub assessment: String,
    pub citation_ids: Vec<String>,
    pub completion_citation_ids: Vec<String>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct CaseworkCase {
    pub id: String,
    pub report_id: String,
    pub topic: String,
    pub title: String,
    pub owner: String,
    pub disposition: String,
    pub revision: u64,
    pub created_at: String,
    pub updated_at: String,
    pub questions: Vec<CaseworkQuestion>,
    pub requests: Vec<CaseworkRequest>,
    pub decisions: Vec<CaseworkDecision>,
    pub commitments: Vec<CaseworkCommitment>,
    pub evidence: Vec<CaseworkCitation>,
    pub notes: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaseworkEvent {
    pub case_id: String,
    pub revision: u64,
    pub author: String,
    pub at: String,
    pub summary: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaseworkRankFactor {
    pub label: String,
    pub score: u32,
    pub maximum: u32,
    pub reason: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaseworkLead {
    pub report_id: String,
    pub finding_index: usize,
    pub title: String,
    pub next_action: String,
    pub score: u32,
    pub factors: Vec<CaseworkRankFactor>,
    pub citations: Vec<CaseworkCitation>,
    pub case_id: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaseworkState {
    pub available_evidence: Vec<CaseworkCitation>,
    pub cases: Vec<CaseworkCase>,
    pub events: Vec<CaseworkEvent>,
    pub leads: Vec<CaseworkLead>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaseworkPacket {
    pub schema_version: u32,
    /// The case's operation log from its first revision, hash-chained; see
    /// `storage::casework_operations`. Absent in packets written before the log existed.
    #[serde(default)]
    pub operations: Vec<CaseOperation>,
    pub base_revision: u64,
    pub base_sha256: String,
    pub exported_at: String,
    pub contributor: String,
    pub case: CaseworkCase,
    pub history: Vec<CaseworkEvent>,
}
/// A frontend request to exclude one citation from a chain-of-custody export. `reason` must be
/// non-empty: a withheld citation is recorded by label and reason in the signed manifest, never
/// silently dropped (CLAUDE.md's absence rule applies here too — an omitted citation and a citation
/// nobody asked to withhold cannot share a representation).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WithheldCitationRequest {
    pub citation_id: String,
    pub reason: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WithheldCitation {
    pub label: String,
    pub reason: String,
}
/// A case's full chain-of-custody export: the case itself (fresh-read, so every citation's
/// `verification` reflects current storage, not a stale save) plus a rollup that a signature
/// cannot be allowed to launder into looking authoritative. An `UNVERIFIED_PORTABLE` citation is
/// exactly as unverified inside a signed package as outside one; the signature attests to who
/// exported these bytes and that they have not changed since, never to the truth of an unverified
/// claim within them. `previous_export_sha256` and `ledger_sequence` chain this export to the
/// installation's local append-only export ledger, always present (null only for sequence 1) so a
/// reader never has to guess whether the field was omitted or genuinely absent.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaseEvidenceManifest {
    pub schema_version: String,
    pub generated_at: String,
    pub case: CaseworkCase,
    pub verified_citation_count: usize,
    pub unverified_citation_labels: Vec<String>,
    pub withheld_citations: Vec<WithheldCitation>,
    pub previous_export_sha256: Option<String>,
    pub ledger_sequence: u64,
}
/// A signed record of one records request as filed: the exact draft, the scope it covers, the
/// digests of the records that identified it, and the modeled determination clock as of the
/// recorded filing date.
///
/// `filedAt` is the send instant the operator recorded after filing through their own channel.
/// This application transmits nothing, so the instant is a recorded operator statement, not an
/// observed transmission; the signature proves these bytes have not changed since this
/// installation produced them, never that the request was actually sent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RequestFilingRecord {
    pub schema_version: String,
    pub generated_at: String,
    pub case_id: String,
    pub case_revision: u64,
    pub request_id: String,
    pub title: String,
    pub custodian: String,
    pub external_reference: String,
    pub filed_at: String,
    pub request_text: String,
    pub request_text_sha256: String,
    pub expected_record_types: Vec<String>,
    pub expected_fields: Vec<String>,
    pub expected_period: String,
    /// The records already held that identify what was requested, by label and digest.
    pub identifying_sources: Vec<FilingIdentifyingSource>,
    pub filing_plan: RequestFilingPlan,
    /// Expectations the recorded productions do not cover. A comparison of this request against
    /// what was recorded as produced; never a finding that a record was unlawfully withheld.
    pub remaining_omissions: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FilingIdentifyingSource {
    pub label: String,
    pub source_sha256: String,
    pub verification: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RequestFilingRecordResult {
    pub case_id: String,
    pub request_id: String,
    pub output_path: String,
    pub package_sha256: String,
    pub signature_algorithm: String,
    pub signer_key_id: String,
    pub generated_at: String,
    pub determination_due_on: String,
    pub remaining_omissions: Vec<String>,
    pub verifier_script_path: String,
    pub verifier_page_path: String,
}
/// One entry of a case's local export ledger, joined with its recorded outcome (if any). `outcome`
/// is `"RESERVED"` when a sequence position was signed and committed to the chain but no outcome
/// was ever appended — most likely a process crash or a file write that failed between signing and
/// recording the result. That state must be surfaced as "reserved, not confirmed written," never
/// silently treated as delivered.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CaseExportLedgerEntry {
    pub sequence: u64,
    pub manifest_sha256: String,
    pub previous_manifest_sha256: Option<String>,
    pub signer_key_id: String,
    pub reserved_at: String,
    pub outcome: String,
    pub outcome_detail: Option<String>,
    pub outcome_recorded_at: Option<String>,
    /// `true` when this entry's `previousManifestSha256` matches the prior entry's
    /// `manifestSha256` (or this is sequence 1, which has none). Computed here, not left for the
    /// UI to eyeball hex strings against each other — "ship a way to actually verify the chain"
    /// means the app states a verdict, not just prints the raw rows.
    pub previous_link_valid: bool,
}
/// One previously imported revision of this case whose recorded signer this installation has
/// since revoked. `signerKeyId.slice(0, 12)`-style identification only, never a claim about the
/// content the import carried — the evidence itself may still be entirely accurate; this only
/// says the *channel* it arrived through is no longer trusted.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CompromisedImportWarning {
    pub revision: u64,
    pub signer_key_id: String,
    pub revoked_at: String,
    pub reason: String,
}
/// One entry of a case's operation log. `author` is the label the saving desk supplied, not an
/// authenticated identity; the packet signature binds the entry to a key.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CaseOperation {
    pub revision: u64,
    pub sequence: u64,
    pub author: String,
    pub at: String,
    /// `ADD`, `CHANGE`, `REMOVE` of a section item, or `SET` of a case field.
    pub kind: String,
    pub section: String,
    pub target: String,
    pub summary: String,
    pub content_sha256: Option<String>,
    pub previous_hash: Option<String>,
    pub hash: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaseworkImportPreview {
    pub mode: String,
    pub local_revision: Option<u64>,
    pub packet_sha256: String,
    pub warnings: Vec<String>,
    pub case: CaseworkCase,
    pub contributor: String,
    /// Set by the command layer once it has verified the packet's Ed25519 signature against its
    /// embedded public key. `None` means the packet was not (or could not be) verified as signed;
    /// the repository itself never sets this, since signing lives at the IPC boundary.
    pub signer_key_id: Option<String>,
    /// On a conflict: incoming operations since the shared base that touch nothing local work
    /// touched since then, so they can be applied as they are; and those that contend.
    pub commuting: Vec<CaseOperation>,
    pub contested: Vec<CaseOperation>,
}
