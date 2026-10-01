//! Requests the user filed and follows, as the user recorded them.
//!
//! Nothing here is fetched from a portal and no credential is held. Nothing here is an input to
//! any finding, detector, confidence or ranking: the tracker is a per-device notebook whose only
//! computation is the statutory deadline for the dates the user confirmed.

use super::investigation::StatutoryExtensionReason;
use crate::analysis::statute::DeterminationBasis;
use crate::analysis::timeline::StatutoryComplianceAnalysis;
use serde::{Deserialize, Serialize};

pub const TRACKER_SCHEMA_VERSION: u32 = 1;
pub const MAXIMUM_CLAIMED_REQUESTS: usize = 200;
pub const MAXIMUM_EVENTS_PER_REQUEST: usize = 500;
pub const MAXIMUM_SUGGESTIONS_PER_REQUEST: usize = 500;
pub const MAXIMUM_IMPORTED_EMAILS_PER_REQUEST: usize = 500;
pub const MAXIMUM_REFERENCE_CHARACTERS: usize = 120;
pub const MAXIMUM_TITLE_CHARACTERS: usize = 200;
pub const MAXIMUM_JURISDICTION_CHARACTERS: usize = 80;
pub const MAXIMUM_SOURCE_URL_CHARACTERS: usize = 2048;
pub const MAXIMUM_PORTAL_KEY_CHARACTERS: usize = 120;
pub const MAXIMUM_SUBJECT_EXCERPT_CHARACTERS: usize = 120;
/// Bytes, not characters: offsets into the excerpt are byte offsets.
pub const MAXIMUM_MATCHED_SENTENCE_BYTES: usize = 500;
pub const MAXIMUM_EXCERPT_BYTES: usize = 900;
pub const MAXIMUM_IMPORT_PATHS: usize = 20;
/// The jurisdiction value the app has a deadline model for.
pub const CALIFORNIA_CPRA_JURISDICTION: &str = "CA_CPRA";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TimelineEventKind {
    Submitted,
    Acknowledged,
    ExtensionNotice,
    Determination,
    RecordsReleased,
    Closed,
    /// Recorded for the chronology only. The kinds below never move the statutory clock.
    ClarificationRequested,
    FeeNotice,
    PartialProduction,
    WithholdingNotice,
    FollowUpSent,
    AppealFiled,
    Other,
}

/// How a counted event is known. A suggestion is not a basis: it lives in
/// `ClaimedRequest::suggestions` and never reaches the deadline.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EventBasis {
    UserEntered,
    EmailConfirmed,
}

/// The parts of an email a suggestion or confirmed event cites. Never the whole body.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EmailEvidence {
    /// SHA-256 of the normalized Message-ID. Present only when the email carried one.
    pub message_id_hash: String,
    pub subject_excerpt: Option<String>,
    /// The email's Date header as an RFC 3339 instant, `.msg` files carry it already in UTC. Not
    /// necessarily the date of the event the sentence describes.
    pub email_date: Option<String>,
    /// A bounded window of the message text around the matched sentence, on one line.
    pub excerpt: String,
    /// `excerpt[sentence_start..sentence_end]` is exactly `matched_sentence`.
    pub sentence_start: usize,
    pub sentence_end: usize,
    pub matched_sentence: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TimelineEvent {
    pub id: String,
    pub kind: TimelineEventKind,
    /// RFC 3339, confirmed by the user.
    pub occurred_at: String,
    pub basis: EventBasis,
    pub evidence: Option<EmailEvidence>,
    /// Required for `ExtensionNotice`; the evaluator needs it and a guess would be invented.
    pub extension_reason: Option<StatutoryExtensionReason>,
    /// The date the agency's notice said it would respond by. Without it the extension is not
    /// applied, because the evaluator cannot validate an extension with no expected date.
    pub expected_determination_at: Option<String>,
    pub created_at: String,
}

/// An event an email seems to describe, waiting for the user. Counts toward nothing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SuggestedEvent {
    pub id: String,
    pub kind: TimelineEventKind,
    /// The email's Date header when it had one; the candidate `occurredAt`, not a finding.
    pub candidate_occurred_at: Option<String>,
    pub evidence: EmailEvidence,
    pub suggested_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ImportedEmail {
    /// Deduplication key. Empty is never stored: an email with no Message-ID is not recorded here.
    pub message_id_hash: String,
    pub subject_excerpt: Option<String>,
    pub email_date: Option<String>,
    pub imported_at: String,
    pub suggestion_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ClaimedRequest {
    pub id: String,
    pub portal_key: Option<String>,
    pub reference: String,
    pub title: Option<String>,
    pub source_url: Option<String>,
    pub jurisdiction: Option<String>,
    pub claimed_at: String,
    pub events: Vec<TimelineEvent>,
    pub suggestions: Vec<SuggestedEvent>,
    pub imported_emails: Vec<ImportedEmail>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TrackerDocument {
    pub schema_version: u32,
    pub claims: Vec<ClaimedRequest>,
}

impl Default for TrackerDocument {
    fn default() -> Self {
        Self {
            schema_version: TRACKER_SCHEMA_VERSION,
            claims: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ClaimRequestInput {
    pub portal_key: Option<String>,
    pub reference: String,
    pub title: Option<String>,
    pub source_url: Option<String>,
    pub jurisdiction: Option<String>,
}

/// Replaces every editable field; the claim's events, suggestions and identity are untouched.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateClaimedRequestInput {
    pub claim_id: String,
    pub portal_key: Option<String>,
    pub reference: String,
    pub title: Option<String>,
    pub source_url: Option<String>,
    pub jurisdiction: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AddTimelineEventInput {
    pub claim_id: String,
    pub kind: TimelineEventKind,
    pub occurred_at: String,
    pub extension_reason: Option<StatutoryExtensionReason>,
    pub expected_determination_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmSuggestedEventInput {
    pub claim_id: String,
    pub suggestion_id: String,
    /// The user may correct what the email suggested.
    pub kind: TimelineEventKind,
    pub occurred_at: String,
    pub extension_reason: Option<StatutoryExtensionReason>,
    pub expected_determination_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RejectSuggestedEventInput {
    pub claim_id: String,
    pub suggestion_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RemoveTimelineEventInput {
    pub claim_id: String,
    pub event_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ImportRequestEmailsInput {
    pub claim_id: String,
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EmailImportOutcome {
    /// Read, and its suggestions (possibly none) were added.
    Imported,
    /// Its Message-ID was already imported for this request.
    AlreadyImported,
    /// Read, but it has no Message-ID, so it cannot be cited or deduplicated. No suggestion was
    /// made from it.
    NoMessageId,
    /// The file could not be read as an email; `detail` says why.
    Rejected,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EmailImportFileResult {
    pub file_name: String,
    pub outcome: EmailImportOutcome,
    pub suggestions_added: u32,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EmailImportReport {
    pub claim: ClaimedRequest,
    pub files: Vec<EmailImportFileResult>,
}

/// Whether the app can compute a deadline at all for this request.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DeadlineModel {
    CaliforniaCpra,
    NoModel,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CountdownKind {
    /// `days` is calendar days from the observation date to the effective deadline, never negative.
    DaysRemaining,
    /// The deadline passed with no confirmed determination; `days` is how many days ago.
    DeadlinePassed,
    DeterminationRecorded,
    NoSubmissionDate,
    NoDeadlineModel,
    /// The recorded dates cannot be evaluated together; `statement` says which contradict.
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Countdown {
    pub kind: CountdownKind,
    pub days: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RequestDeadline {
    pub claim_id: String,
    pub model: DeadlineModel,
    /// The statutory evaluator's own result. Absent when there is no model or no submission date.
    pub analysis: Option<StatutoryComplianceAnalysis>,
    pub countdown: Countdown,
    /// Absent whenever there is no analysis.
    pub determination_basis: Option<DeterminationBasis>,
    /// True only when the evaluator counted a violation against a sourced determination event.
    pub violation_asserted: bool,
    /// What the deadline rests on, in words. Always present.
    pub basis_statement: String,
    pub notes: Vec<String>,
    /// Suggestions awaiting the user. They are excluded from everything above.
    pub pending_suggestion_count: u32,
}
