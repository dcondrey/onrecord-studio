//! Wire types for the deterministic investigative intent router.
//!
//! An answer is either a set of rows, each stating the evidence basis it rests on and its
//! citations, or an explicit statement of why no rows are offered. `IntentStatus` keeps "nothing
//! matched" apart from "could not be asked" so a reader never renders an absence as a measurement.

use serde::{Deserialize, Serialize};

/// What an utterance was parsed into. Every variant is answered by SQL or a corpus graph read;
/// none of them involves a model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "SCREAMING_SNAKE_CASE",
    rename_all_fields = "camelCase"
)]
pub enum QueryPlan {
    /// Stored contradiction findings, optionally limited to one agency by name.
    ContradictionsInAgency { agency: Option<String> },
    /// Statutory evaluation of one request, with the determination basis stated.
    WhyRequestMissedDeadline {
        request_id: u64,
        portal_key: Option<String>,
    },
    /// Typed `SIGNED_BY` relationships around a named entity in the corpus graph.
    SignatoriesFor { subject: String },
    /// Per-agency aggregate of documents whose text contains a name or exact phrase.
    AppearanceAcrossAgencies { subject: String },
    /// Stored output of one Wave 8 cross-corpus detector.
    CrossCorpusSignal { signal: CrossCorpusSignal },
    /// Shortest graph path between two named entities, with each hop's relationship type.
    ConnectionBetween { from: String, to: String },
    /// Stored findings, optionally limited to one agency by name.
    FindingsForAgency { agency: Option<String> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CrossCorpusSignal {
    SharedDenialLanguage,
    ReusedExtensionGrounds,
    BehaviorShift,
    OrphanTopic,
}

/// What a row's claim rests on. The wording of a row must not exceed its basis: a co-occurrence
/// is proximity in text, never a stated relationship.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EvidenceBasis {
    StoredFinding,
    TypedRelationship,
    CoOccurrenceOnly,
    FullTextTermMatch,
    StatutorySourcedDetermination,
    StatutoryActivityProxy,
    StatutoryProvisionalObservation,
}

impl EvidenceBasis {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::StoredFinding => "STORED_FINDING",
            Self::TypedRelationship => "TYPED_RELATIONSHIP",
            Self::CoOccurrenceOnly => "CO_OCCURRENCE_ONLY",
            Self::FullTextTermMatch => "FULL_TEXT_TERM_MATCH",
            Self::StatutorySourcedDetermination => "STATUTORY_SOURCED_DETERMINATION",
            Self::StatutoryActivityProxy => "STATUTORY_ACTIVITY_PROXY",
            Self::StatutoryProvisionalObservation => "STATUTORY_PROVISIONAL_OBSERVATION",
        }
    }

    /// What this basis does and does not establish, in the words a reader is shown.
    pub const fn statement(self) -> &'static str {
        match self {
            Self::StoredFinding => {
                "A stored detector finding. It is a pattern signal for review, not a legal conclusion."
            }
            Self::TypedRelationship => {
                "A relationship type the document text itself carried, recorded in the corpus graph. The graph does not say which side is which."
            }
            Self::CoOccurrenceOnly => {
                "Proximity only: the names appeared near each other in the same text. This is not a stated relationship."
            }
            Self::FullTextTermMatch => {
                "Every search term is present in the document. This is a text match, not a verified mention of one entity."
            }
            Self::StatutorySourcedDetermination => {
                "Measured to a determination event the source recorded with its own notice."
            }
            Self::StatutoryActivityProxy => {
                "Measured to the earliest recorded portal activity, which stands in for the determination. The agency's actual determination date is not recorded."
            }
            Self::StatutoryProvisionalObservation => {
                "No activity was recorded after receipt, so the period was measured to today and is still running."
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum IntentStatus {
    /// Rows are offered.
    Answered,
    /// The query ran and matched nothing. This says nothing about records the query cannot see.
    NoMatch,
    /// The utterance matched no supported intent.
    NotUnderstood,
    /// The intent was recognized but a required name or number was missing.
    NeedsSubject,
    /// More than one intent, or more than one request, fits. The options say how to choose.
    Ambiguous,
    /// A source the intent needs is empty or could not be read. Not the same as no match.
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntentCitation {
    pub portal_key: String,
    pub document_id: Option<i64>,
    pub character_start: Option<i64>,
    pub character_end: Option<i64>,
    pub quote: Option<String>,
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntentRow {
    /// Stable within one answer; safe as a React key.
    pub id: String,
    pub headline: String,
    pub detail: Option<String>,
    pub basis: EvidenceBasis,
    /// The `EvidenceBasis` sentence, so the wording lives in one place.
    pub basis_statement: String,
    /// The relationship type for graph rows (`SIGNED_BY`, `CO_OCCURRENCE`, `MIXED`).
    pub relationship_kind: Option<String>,
    /// `None` means no confidence was assessed; it is never `0`.
    pub confidence_basis_points: Option<u16>,
    pub agency_name: Option<String>,
    pub portal_keys: Vec<String>,
    pub citations: Vec<IntentCitation>,
    pub omitted_citation_count: usize,
    /// Present exactly when `citations` is empty.
    pub citations_unavailable_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntentOption {
    pub label: String,
    pub description: String,
    pub example: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntentAnswer {
    pub utterance: String,
    pub status: IntentStatus,
    pub plan: Option<QueryPlan>,
    pub summary: String,
    pub rows: Vec<IntentRow>,
    /// More rows matched than `row_limit` allowed.
    pub truncated: bool,
    pub row_limit: usize,
    /// Limits and gaps in what was searched, one sentence each.
    pub notes: Vec<String>,
    /// What can be asked (not understood), the candidates (ambiguous), or the way to complete a
    /// question (needs subject).
    pub options: Vec<IntentOption>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `src/types/intent.ts` is hand-mirrored, so the wire spellings it relies on are pinned here.
    #[test]
    fn wire_spellings_match_the_typescript_mirror() {
        let plan = QueryPlan::WhyRequestMissedDeadline {
            request_id: 42,
            portal_key: None,
        };
        assert_eq!(
            serde_json::to_value(&plan).unwrap(),
            serde_json::json!({"kind": "WHY_REQUEST_MISSED_DEADLINE", "requestId": 42, "portalKey": null})
        );
        assert_eq!(
            serde_json::to_value(IntentStatus::NoMatch).unwrap(),
            serde_json::json!("NO_MATCH")
        );
        assert_eq!(
            serde_json::to_value(EvidenceBasis::CoOccurrenceOnly).unwrap(),
            serde_json::json!(EvidenceBasis::CoOccurrenceOnly.as_str())
        );
        assert_eq!(
            serde_json::to_value(CrossCorpusSignal::ReusedExtensionGrounds).unwrap(),
            serde_json::json!("REUSED_EXTENSION_GROUNDS")
        );
    }
}
