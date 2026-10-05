use super::investigation::ClaimCheckView;
use crate::analysis::finding::FindingCitation;
use serde::{Deserialize, Serialize};

/// `DateNotRecorded` is not a date: a meeting whose documents carry no unambiguous dated
/// heading is shown as such, never placed on a guessed day.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MeetingTiming {
    Upcoming,
    Past,
    DateNotRecorded,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MeetingDocumentRole {
    Agenda,
    Minutes,
    Video,
}

/// Why a document shows no posting time. Each is a different absence, never a time.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PostingAbsence {
    /// No fetch record exists for the bytes the app holds (for example a hand-added file).
    NoFetchRecord,
    /// The server sent no `Last-Modified` header.
    NoHeader,
    /// The header is not a readable HTTP date.
    UnreadableHeader,
    /// The host was measured to report the request time as `Last-Modified`.
    RequestTimeHost,
    /// The header equals the server's own `Date` for the response.
    HeaderIsRequestTime,
    /// The response `Date` was not kept for this fetch and the header is too close to the fetch
    /// to tell it from the request time.
    RequestTimeNotRuledOut,
}

/// The server's `Last-Modified` statement about a file, or the reason there is none. It is what
/// the server said about the file, not proof of when the public could first read it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PostingTime {
    #[serde(rename_all = "camelCase")]
    Recorded {
        /// RFC 3339, UTC.
        last_modified: String,
    },
    #[serde(rename_all = "camelCase")]
    NotRecorded { reason: PostingAbsence },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingDocument {
    pub role: MeetingDocumentRole,
    pub portal_key: String,
    pub document_id: i64,
    pub label: String,
    pub source_url: Option<String>,
    pub posting: PostingTime,
    /// Whole minutes from `Last-Modified` to the meeting start, negative when the header is
    /// later than the start. `None` unless this is an agenda with a recorded posting time and
    /// the meeting's date and start time are both recorded.
    pub minutes_before_meeting: Option<i64>,
    /// Fetches of this document's address that returned bytes different from the copy held at
    /// the time. The earlier text is not retained, so those copies cannot be compared.
    pub differing_fetches: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AgendaItemChangeKind {
    Added,
    Removed,
    /// The item's title differs between the copies.
    Reworded,
    /// The title is the same and the item's text below it differs.
    TextChanged,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgendaItemChange {
    pub kind: AgendaItemChangeKind,
    pub item: String,
    /// The item's title in the earlier copy, when the item is there and its title is bounded.
    pub before: Option<String>,
    /// The item's title in the later copy, when the item is there and its title is bounded.
    pub after: Option<String>,
}

/// How two copies of an agenda were put in order.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RevisionOrderBasis {
    /// Every copy carries a recorded `Last-Modified`; earlier header first.
    LastModified,
    /// At least one copy has no recorded posting time; ordered by when the app collected them.
    CollectionOrder,
}

/// Differences between two consecutive copies of one meeting's agenda.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgendaRevision {
    pub earlier_document_id: i64,
    pub earlier_label: String,
    pub later_document_id: i64,
    pub later_label: String,
    pub order_basis: RevisionOrderBasis,
    pub changes: Vec<AgendaItemChange>,
    /// Items present in both copies whose title could not be bounded in one of them, so their
    /// wording was not compared.
    pub titles_not_compared: Vec<String>,
}

/// Counts over agenda files whose posting time and meeting start are both recorded. A count of
/// header times, not a finding about any notice requirement.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PostingCounts {
    pub agenda_files_with_both_times: u32,
    pub under_72_hours_before_start: u32,
    pub after_meeting_start: u32,
}

/// What a suggestion reason rests on. `Counted` is a computed comparison, `Stated` is text a
/// document states, `Pattern` is a statistical observation over released documents.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SuggestionBasis {
    Counted,
    Stated,
    Pattern,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SuggestionReasonKind {
    InvestigationTopic,
    WatchedAgency,
    WatchedEntity,
    FindingSubject,
    EvidenceGap,
    CheckableFigure,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SuggestionReason {
    pub kind: SuggestionReasonKind,
    pub basis: SuggestionBasis,
    pub reason: String,
    pub matched: String,
    /// The agenda passage the reason rests on.
    pub citation: FindingCitation,
    pub finding_id: Option<String>,
    pub gap_key: Option<String>,
    pub claim_check: Option<ClaimCheckView>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicCommentDraft {
    pub text: String,
    pub bases: Vec<SuggestionBasis>,
    /// `None` means no document in the corpus records a comment deadline for this meeting.
    pub deadline: Option<FindingCitation>,
    pub instructions: Vec<FindingCitation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingSuggestion {
    pub id: String,
    pub agenda_item: String,
    pub title: String,
    pub item_citation: FindingCitation,
    pub reasons: Vec<SuggestionReason>,
    pub comment: PublicCommentDraft,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarMeeting {
    pub id: String,
    pub jurisdiction: String,
    pub body: String,
    pub body_citation: FindingCitation,
    pub date: Option<String>,
    pub date_citation: Option<FindingCitation>,
    /// Why the date is not recorded; `None` whenever `date` is present.
    pub date_note: Option<String>,
    pub timing: MeetingTiming,
    /// The start time the agenda heading states beside its date, as written (24-hour `HH:MM`,
    /// local to the publisher). `None` when no agenda states one or agendas disagree.
    pub start_time: Option<String>,
    pub documents: Vec<MeetingDocument>,
    pub revisions: Vec<AgendaRevision>,
    pub comment_deadlines: Vec<FindingCitation>,
    pub comment_instructions: Vec<FindingCitation>,
    pub suggestions: Vec<MeetingSuggestion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingCalendar {
    pub today: String,
    pub meetings: Vec<CalendarMeeting>,
    pub posting_counts: PostingCounts,
    pub limitations: Vec<String>,
}
