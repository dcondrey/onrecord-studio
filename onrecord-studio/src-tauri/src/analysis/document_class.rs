//! Coarse document-type classifier for policy-versus-practice comparison.
//!
//! Scores cue phrases in the file name, the opening of the text, and the body, weighting the places
//! a document names itself. `None` means no class won clearly: an unclassified document is not a
//! `Record`, and a comparison must not treat it as one.

use crate::analysis::assertion_index::extract_denial;
use crate::analysis::phrase_match::{count_phrases, words, Phrases};

const HEAD_CHARACTERS: usize = 600;
const BODY_CHARACTERS: usize = 20_000;
const NAME_WEIGHT: usize = 3;
const HEAD_WEIGHT: usize = 2;
const MINIMUM_SCORE: usize = 3;
const DENIAL_PHRASE_WEIGHT: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentClass {
    Policy,
    Record,
    Response,
    Minutes,
    Contract,
    Denial,
    Budget,
    /// A press release, official statement, remarks, or meeting notice: what an agency said, as
    /// opposed to a record of what it did. Agendas and minutes stay under `Minutes`; a budget book
    /// stays under `Budget`. Both are also statement-side sources for claim checks.
    PublicStatement,
}

impl DocumentClass {
    /// Whether a figure in this document is something the agency asserted rather than recorded.
    pub fn is_statement(self) -> bool {
        matches!(self, Self::PublicStatement | Self::Minutes)
    }
}

const CLASS_CUES: [(DocumentClass, Phrases); 8] = [
    (
        DocumentClass::Policy,
        &[
            &["policy"],
            &["procedure"],
            &["procedures"],
            &["handbook"],
            &["guidelines"],
            &["administrative", "regulation"],
            &["standard", "operating"],
        ],
    ),
    (
        DocumentClass::Record,
        &[
            &["invoice"],
            &["receipt"],
            &["report"],
            &["log"],
            &["memo"],
            &["memorandum"],
        ],
    ),
    (
        DocumentClass::Response,
        &[
            &["your", "request"],
            &["in", "response", "to"],
            &["response", "to"],
            &["public", "records", "request"],
            &["determination"],
        ],
    ),
    (
        DocumentClass::Minutes,
        &[
            &["minutes"],
            &["agenda"],
            &["roll", "call"],
            &["adjourned"],
            &["motion", "carried"],
        ],
    ),
    (
        DocumentClass::Contract,
        &[
            &["agreement"],
            &["contract"],
            &["whereas"],
            &["hereinafter"],
            &["memorandum", "of", "understanding"],
        ],
    ),
    (
        DocumentClass::Denial,
        &[
            &["exempt", "from", "disclosure"],
            &["we", "are", "denying"],
            &["request", "is", "denied"],
            &["withheld", "in", "full"],
        ],
    ),
    (
        DocumentClass::Budget,
        &[
            &["budget"],
            &["appropriation"],
            &["fiscal", "year"],
            &["expenditures"],
            &["revenue"],
        ],
    ),
    (
        DocumentClass::PublicStatement,
        &[
            &["for", "immediate", "release"],
            &["press", "release"],
            &["news", "release"],
            &["statement", "from"],
            &["statement", "by"],
            &["remarks"],
            &["regular", "meeting", "of"],
            &["special", "meeting", "of"],
            &["budget", "message"],
            &["state", "of", "the", "city"],
        ],
    ),
];

pub fn classify(file_name: Option<&str>, text: &str) -> Option<DocumentClass> {
    let name_words = words(file_name.unwrap_or_default());
    let body: String = text.chars().take(BODY_CHARACTERS).collect();
    let head_words = words(&body.chars().take(HEAD_CHARACTERS).collect::<String>());
    let body_words = words(&body);
    let mut scored: Vec<(usize, DocumentClass)> = CLASS_CUES
        .iter()
        .map(|&(class, cues)| {
            let denial_bonus = if class == DocumentClass::Denial && extract_denial(&body).is_some()
            {
                DENIAL_PHRASE_WEIGHT
            } else {
                0
            };
            (
                NAME_WEIGHT * count_phrases(&name_words, cues)
                    + HEAD_WEIGHT * count_phrases(&head_words, cues)
                    + count_phrases(&body_words, cues)
                    + denial_bonus,
                class,
            )
        })
        .filter(|(score, _)| *score >= MINIMUM_SCORE)
        .collect();
    // A denial letter is also a response; the specific class wins.
    if scored
        .iter()
        .any(|(_, class)| *class == DocumentClass::Denial)
    {
        scored.retain(|(_, class)| *class != DocumentClass::Response);
    }
    scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    match scored.as_slice() {
        [(_, class)] => Some(*class),
        [(top, class), (next, _), ..] if top > next => Some(*class),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_name_and_opening_outweigh_body_mentions() {
        let policy = "Records Request Policy\nThis policy governs how staff respond to a request.";
        assert_eq!(
            classify(Some("records-policy.pdf"), policy),
            Some(DocumentClass::Policy)
        );
        let minutes = "The council minutes note the agenda and a roll call. Adjourned.";
        assert_eq!(classify(None, minutes), Some(DocumentClass::Minutes));
    }

    #[test]
    fn no_clear_winner_is_unclassified_and_a_denial_outranks_a_response() {
        assert_eq!(classify(None, "Lunch is at noon."), None);
        let letter = "In response to your request, the records are exempt from disclosure. No responsive records exist.";
        assert_eq!(classify(None, letter), Some(DocumentClass::Denial));
    }

    #[test]
    fn press_releases_are_statements_and_agendas_stay_minutes() {
        let release = "FOR IMMEDIATE RELEASE\nStatement from the Mayor on the adopted budget.\nThe City closed the year with no deficit.";
        assert_eq!(
            classify(Some("press-release.pdf"), release),
            Some(DocumentClass::PublicStatement)
        );
        assert!(DocumentClass::PublicStatement.is_statement());
        let agenda =
            "Agenda: Regular Meeting of the City Council. Roll call. Minutes of the prior meeting.";
        assert_eq!(classify(None, agenda), Some(DocumentClass::Minutes));
        assert!(DocumentClass::Minutes.is_statement());
    }

    #[test]
    fn a_denial_that_mentions_a_statement_is_not_a_public_statement() {
        let letter = "In response to your request: this statement from the department confirms the records are exempt from disclosure and withheld in full.";
        assert_eq!(classify(None, letter), Some(DocumentClass::Denial));
        assert!(!DocumentClass::Denial.is_statement());
    }
}
