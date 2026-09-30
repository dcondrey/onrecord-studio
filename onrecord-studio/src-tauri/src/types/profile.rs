//! The local user profile and the declared organization affiliations that drive the "you" lens.
//!
//! Nothing here is transmitted, and nothing here is an input to any finding, confidence, severity,
//! statutory evaluation or ranking. A profile filters, highlights and discloses.

use super::corpus::EntityValidationGrade;
use serde::{Deserialize, Serialize};

pub const PROFILE_SCHEMA_VERSION: u32 = 1;
pub const MAXIMUM_DISPLAY_NAME_CHARACTERS: usize = 120;
pub const MAXIMUM_AFFILIATIONS: usize = 8;
pub const MAXIMUM_AFFILIATION_NAME_CHARACTERS: usize = 150;
pub const MAXIMUM_ALIASES_PER_AFFILIATION: usize = 8;
pub const MAXIMUM_ALIAS_CHARACTERS: usize = 80;
pub const MAXIMUM_MATCH_DECISIONS: usize = 256;
pub const MAXIMUM_VERIFIED_DOMAINS: usize = 16;

/// How the app knows the user belongs to the organization.
///
/// `EmailDomainVerified` means a one-time code reached a mailbox on a domain whose name matches
/// this affiliation and the user linked them (`profile::verified`). It proves control of a
/// mailbox, not employment. Nothing else sets it.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AffiliationBasis {
    SelfDeclared,
    EmailDomainVerified,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Affiliation {
    pub id: String,
    pub name: String,
    pub aliases: Vec<String>,
    pub basis: AffiliationBasis,
    pub created_at: String,
    /// The verified domain this entry was linked to by the user. `Some` exactly when `basis` is
    /// `EmailDomainVerified`.
    #[serde(default)]
    pub verified_domain: Option<String>,
}

/// A domain a mailbox code was confirmed for. Only the domain is kept: the address and the code
/// are never written to the profile file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VerifiedDomain {
    pub domain: String,
    pub verified_at: String,
    /// A consumer mailbox provider (server flag). Recorded, never usable as organization proof.
    #[serde(default)]
    pub is_free_mail: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AffiliationMatchKind {
    /// A registered portal, matched on its display name.
    Portal,
    /// An agency recorded under a portal, matched on its name.
    Agency,
    /// An Organization or Acronym entity read from documents in the corpus.
    Organization,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MatchDecisionKind {
    Confirmed,
    Rejected,
}

/// A user's answer about one candidate. Keyed by kind and normalized name, never by `entity_id`:
/// the corpus file is rebuilt and its ids change.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MatchDecision {
    pub affiliation_id: String,
    pub kind: AffiliationMatchKind,
    pub normalized_name: String,
    pub decision: MatchDecisionKind,
    pub decided_at: String,
}

/// The stored document. `display_name` is `None` when the user gave none: a blank is not a name.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UserProfile {
    pub schema_version: u32,
    pub display_name: Option<String>,
    pub affiliations: Vec<Affiliation>,
    #[serde(default)]
    pub match_decisions: Vec<MatchDecision>,
    #[serde(default)]
    pub verified_domains: Vec<VerifiedDomain>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProfileStatus {
    /// No profile file exists. A real state, not an empty profile.
    Unset,
    Set,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProfileState {
    pub status: ProfileStatus,
    pub profile: Option<UserProfile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AffiliationInput {
    /// Present when editing an existing affiliation, so its id, creation time and match decisions
    /// survive. The basis is never client-supplied.
    pub id: Option<String>,
    pub name: String,
    pub aliases: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProfileInput {
    pub display_name: Option<String>,
    pub affiliations: Vec<AffiliationInput>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProfileDeletionOutcome {
    Removed,
    /// There was no file to remove. The state after is the same: unset.
    NothingToRemove,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AffiliationMatchMethod {
    /// The normalized affiliation name equals the candidate.
    Exact,
    /// A normalized alias equals the candidate.
    Alias,
    /// Similar but not equal. Never applied without the user's confirmation.
    Fuzzy,
    /// Not found in the current sources; shown because the user decided it earlier.
    PreviouslyDecided,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CandidateDecision {
    Pending,
    Confirmed,
    Rejected,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AffiliationMatchCandidate {
    pub affiliation_id: String,
    pub kind: AffiliationMatchKind,
    pub display_value: String,
    pub normalized_name: String,
    pub method: AffiliationMatchMethod,
    /// The affiliation name or alias the candidate was compared against.
    pub matched_term: String,
    /// Extraction grade of the weakest mention. `None` for a registered portal, which was
    /// entered by the user or discovered by a connector, not extracted from text.
    pub validation_grade: Option<EntityValidationGrade>,
    pub document_count: Option<u64>,
    pub portal_key: Option<String>,
    pub agency_id: Option<i64>,
    pub entity_id: Option<String>,
    pub decision: CandidateDecision,
    pub currently_present: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AffiliationMatchStatus {
    /// No profile exists.
    Unset,
    /// A profile exists and declares no organization.
    NoAffiliationsDeclared,
    /// Sources were searched and nothing resembled a declared name. This says nothing about
    /// whether the organization appears in records the extractor could not recognize.
    NoMatchYet,
    HasCandidates,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AffiliationMatchList {
    pub status: AffiliationMatchStatus,
    pub candidates: Vec<AffiliationMatchCandidate>,
    pub searched_portal_count: u64,
    pub searched_agency_count: u64,
    pub searched_entity_count: u64,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AffiliationMatchDecisionInput {
    pub affiliation_id: String,
    pub kind: AffiliationMatchKind,
    /// The candidate's normalized name as listed. Re-normalized and bounded at the boundary.
    pub normalized_name: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DisclosureStatus {
    Unset,
    Declared,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DisclosedAffiliation {
    pub name: String,
    pub basis: AffiliationBasis,
}

/// What outgoing artifacts say about the author, and which subjects count as the author's own
/// organization right now. The subject sets are resolved against the current corpus, so they are
/// transient and never stored.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AffiliationDisclosure {
    pub status: DisclosureStatus,
    /// `None` when no profile exists or it declares no organization.
    pub sentence: Option<String>,
    pub affiliations: Vec<DisclosedAffiliation>,
    pub own_portal_keys: Vec<String>,
    pub own_agencies: Vec<OwnAgency>,
    pub own_entity_ids: Vec<String>,
}

/// An agency id is only unique within its portal.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub struct OwnAgency {
    pub portal_key: String,
    pub agency_id: i64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WorkEmailCodeStatus {
    Sent,
    InvalidEmail,
    /// The server's hourly send limit. `retry_after_seconds` says when to try again.
    RateLimited,
    NotSignedIn,
    /// The server could not deliver the message. No code can be entered.
    SendFailed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkEmailCodeReport {
    pub status: WorkEmailCodeStatus,
    pub expires_in_seconds: Option<u32>,
    pub retry_after_seconds: Option<u32>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WorkEmailConfirmStatus {
    /// The code matched. `domain` is recorded; a free-mail domain is recorded but is not proof.
    Verified,
    InvalidInput,
    NotSignedIn,
    /// No code was requested for this address in this session (or for a different address).
    NoPendingRequest,
    CodeInvalid,
    /// Expired, already used, or out of attempts: the server does not distinguish them.
    ExpiredOrExhausted,
    RateLimited,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkEmailConfirmReport {
    pub status: WorkEmailConfirmStatus,
    pub domain: Option<String>,
    pub is_free_mail: bool,
    pub attempts_remaining: Option<u32>,
    pub verified_at: Option<String>,
    pub retry_after_seconds: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AffiliationDomainLinkInput {
    pub affiliation_id: String,
    pub domain: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AffiliationLinkStatus {
    Linked,
    /// Neither the name nor an alias equals the domain's organization name. Stays self-declared.
    NotMatching,
    /// A consumer mailbox provider says nothing about an organization.
    FreeMailDomain,
    DomainNotVerified,
    AffiliationNotFound,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AffiliationLinkReport {
    pub status: AffiliationLinkStatus,
    /// Plain-language reason, shown as is.
    pub message: String,
    pub profile: ProfileState,
}
