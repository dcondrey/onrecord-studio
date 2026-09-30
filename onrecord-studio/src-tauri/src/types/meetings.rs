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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingDocument {
    pub role: MeetingDocumentRole,
    pub portal_key: String,
    pub document_id: i64,
    pub label: String,
    pub source_url: Option<String>,
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
    pub documents: Vec<MeetingDocument>,
    pub comment_deadlines: Vec<FindingCitation>,
    pub comment_instructions: Vec<FindingCitation>,
    pub suggestions: Vec<MeetingSuggestion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingCalendar {
    pub today: String,
    pub meetings: Vec<CalendarMeeting>,
    pub limitations: Vec<String>,
}
