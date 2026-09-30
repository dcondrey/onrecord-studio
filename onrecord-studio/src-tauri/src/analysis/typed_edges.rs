//! Typed entity edges and the role and signer mentions they anchor on.
//!
//! An edge is emitted only when the words between two mentions carry an explicit cue such as
//! "pursuant to", "will", "paid" or "superseded by". Proximity alone stays `CoOccurrence`. Cues are
//! phrase lists matched over word tokens, so every rule errs toward no edge: a negation, a passive
//! construction whose actor is ambiguous, or a sentence break between the mentions voids the match.
//! Direction is part of the claim: `SignedBy` runs from the signing capacity to the signer, and the
//! `Supersedes`/`Amends` edges run from the newer instrument to the older.

use crate::analysis::phrase_match::{has_phrase, words, Phrases, Word};
use crate::analysis::reconstruction::ROLE;
use crate::types::{CorpusEntityKind, RelationshipKind};

/// Characters between two mentions beyond which no cue is read. Wide enough to cross a segment
/// boundary, narrow enough that the cue and both mentions are one thought.
const CUE_WINDOW_CHARACTERS: usize = 200;
const SIGNATURE_WINDOW_CHARACTERS: usize = 80;
const LOOKAHEAD_MENTIONS: usize = 24;
const MAXIMUM_TYPED_EDGES_PER_DOCUMENT: usize = 5_000;
const MAXIMUM_MENTIONS: usize = 4_000;
const MAXIMUM_SIGNER_WORDS: usize = 4;
/// A cue is a pattern match, not a verified fact, so no typed edge outranks this.
const CUE_CONFIDENCE_CEILING: u16 = 6_500;

const NEGATIONS: &[&str] = &[
    "not", "never", "unable", "cannot", "without", "neither", "nor",
];
const CITES: Phrases = &[
    &["cites"],
    &["cite"],
    &["citing"],
    &["cited"],
    &["pursuant", "to"],
    &["in", "accordance", "with"],
    &["as", "required", "by"],
    &["invokes"],
    &["invoking"],
    &["invoked"],
];
const PROMISES: Phrases = &[
    &["will"],
    &["shall"],
    &["must"],
    &["agrees", "to"],
    &["agree", "to"],
    &["commits", "to"],
    &["commit", "to"],
    &["promises"],
    &["promise"],
    &["undertakes"],
];
const DELIVERED: Phrases = &[
    &["provided"],
    &["delivered"],
    &["paid"],
    &["issued"],
    &["released"],
    &["sent"],
    &["completed"],
    &["remitted"],
    &["mailed"],
    &["produced"],
];
/// Words that put a delivery in the future or make its actor the recipient. With either present
/// the subject before the cue is not the one who acted, so no edge is written.
const NOT_ACTIVE_PAST: &[&str] = &[
    "will", "shall", "must", "unless", "be", "been", "being", "was", "were", "is", "are", "by",
];
/// Priority order: the "by" forms first, because they contain the same verb as the active forms
/// and point the other way.
const SUCCESSION_RULES: [(Phrases, RelationshipKind, bool); 4] = [
    (SUPERSEDED_BY, RelationshipKind::Supersedes, true),
    (SUPERSEDES, RelationshipKind::Supersedes, false),
    (AMENDED_BY, RelationshipKind::Amends, true),
    (AMENDS, RelationshipKind::Amends, false),
];
const SUPERSEDES: Phrases = &[
    &["supersedes"],
    &["replaces"],
    &["rescinds"],
    &["supplants"],
];
const SUPERSEDED_BY: Phrases = &[
    &["superseded", "by"],
    &["replaced", "by"],
    &["rescinded", "by"],
    &["supplanted", "by"],
];
const AMENDS: Phrases = &[&["amends"], &["amending"], &["modifies"], &["revises"]];
const AMENDED_BY: Phrases = &[&["amended", "by"], &["modified", "by"], &["revised", "by"]];
/// A period after one of these is not a sentence end.
const ABBREVIATIONS: &[&str] = &[
    "no", "nos", "inc", "co", "corp", "ltd", "sec", "secs", "dept", "st", "mr", "mrs", "ms", "dr",
    "esq", "vs", "u", "s", "c",
];
const SIGNATURE_LEAD_INS: &[&str] = &["sincerely", "respectfully", "respectfully submitted"];

pub struct TextMention {
    pub kind: CorpusEntityKind,
    pub normalized: String,
    pub display: String,
    pub character_start: usize,
    pub character_end: usize,
}

pub struct EdgeInput {
    pub kind: CorpusEntityKind,
    pub character_start: usize,
    pub character_end: usize,
    pub confidence_basis_points: u16,
}

pub struct TypedEdge {
    pub left: usize,
    pub right: usize,
    pub kind: RelationshipKind,
    pub character_start: usize,
    pub character_end: usize,
    pub confidence_basis_points: u16,
}

/// Role titles and cue-anchored signer names, in document order. A signer is a name alone on the
/// line after `Sincerely,`, `Respectfully,`, or following `/s/`, `Signed:`, `Signature:`; a name
/// that is itself a role title is not a person.
pub fn role_and_signer_mentions(text: &str) -> Vec<TextMention> {
    let mut mentions = Vec::new();
    let mut cursor = ByteToCharacter::default();
    for found in ROLE.find_iter(text).take(MAXIMUM_MENTIONS) {
        mentions.push(TextMention {
            kind: CorpusEntityKind::Role,
            normalized: collapse(found.as_str()).to_lowercase(),
            display: collapse(found.as_str()),
            character_start: cursor.at(text, found.start()),
            character_end: cursor.at(text, found.end()),
        });
    }
    let mut cursor = ByteToCharacter::default();
    for (start, end) in signer_spans(text).into_iter().take(MAXIMUM_MENTIONS) {
        let name = &text[start..end];
        if ROLE.is_match(name) {
            continue;
        }
        mentions.push(TextMention {
            kind: CorpusEntityKind::Person,
            normalized: collapse(name).to_lowercase(),
            display: collapse(name),
            character_start: cursor.at(text, start),
            character_end: cursor.at(text, end),
        });
    }
    mentions
}

/// Byte spans of names sitting in a signature position.
fn signer_spans(text: &str) -> Vec<(usize, usize)> {
    let lines: Vec<(usize, &str)> = text
        .split_inclusive('\n')
        .scan(0_usize, |offset, line| {
            let start = *offset;
            *offset += line.len();
            Some((start, line))
        })
        .collect();
    let mut spans = Vec::new();
    for (index, (line_start, line)) in lines.iter().enumerate() {
        let lowered = line.trim().to_lowercase();
        let next_line = || {
            lines[index + 1..]
                .iter()
                .find(|(_, candidate)| !candidate.trim().is_empty())
        };
        let candidate = if let Some(position) = line.find("/s/") {
            let rest = position + 3;
            if line[rest..].trim().is_empty() {
                next_line().map(|(start, next)| (*start, *next))
            } else {
                Some((*line_start + rest, &line[rest..]))
            }
        } else if let Some(rest) = lowered
            .strip_prefix("signed:")
            .or_else(|| lowered.strip_prefix("signature:"))
        {
            if rest.trim().is_empty() {
                next_line().map(|(start, next)| (*start, *next))
            } else {
                let colon = line.find(':').map_or(0, |found| found + 1);
                Some((*line_start + colon, &line[colon..]))
            }
        } else if let Some(lead_in) = lowered.strip_suffix(',') {
            SIGNATURE_LEAD_INS
                .contains(&lead_in.trim())
                .then(|| next_line().map(|(start, next)| (*start, *next)))
                .flatten()
        } else {
            None
        };
        let Some((start, segment)) = candidate else {
            continue;
        };
        let trimmed = segment.trim();
        if is_person_name(trimmed) {
            let leading = segment.len() - segment.trim_start().len();
            spans.push((start + leading, start + leading + trimmed.len()));
        }
    }
    spans
}

/// Two to four words, each capitalized or a single initial, and nothing else on the line.
fn is_person_name(line: &str) -> bool {
    let words: Vec<&str> = line
        .split([' ', '\t'])
        .filter(|word| !word.is_empty())
        .collect();
    (2..=MAXIMUM_SIGNER_WORDS).contains(&words.len())
        && words.iter().all(|word| {
            let mut characters = word.chars();
            characters.next().is_some_and(char::is_uppercase)
                && characters.all(|character| {
                    character.is_alphabetic() || matches!(character, '\'' | '’' | '.' | '-')
                })
        })
}

/// `mentions` must be sorted by `character_start`. Returns directed edges, left acting on right.
pub fn derive_typed_edges(text: &str, mentions: &[EdgeInput]) -> Vec<TypedEdge> {
    let mut edges = Vec::new();
    let mut characters: Option<Vec<char>> = None;
    for (left, from) in mentions.iter().enumerate() {
        for (offset, to) in mentions[left + 1..]
            .iter()
            .take(LOOKAHEAD_MENTIONS)
            .enumerate()
        {
            let right = left + 1 + offset;
            if to.character_start < from.character_end {
                continue;
            }
            if to.character_start - from.character_end > CUE_WINDOW_CHARACTERS {
                break;
            }
            if !could_carry_cue(from.kind, to.kind) {
                continue;
            }
            let characters = characters.get_or_insert_with(|| text.chars().collect());
            let connector: String = characters[from.character_end..to.character_start]
                .iter()
                .collect();
            let Some((kind, reversed)) = classify(from.kind, to.kind, &connector) else {
                continue;
            };
            if edges.len() >= MAXIMUM_TYPED_EDGES_PER_DOCUMENT {
                return edges;
            }
            let (left, right) = if reversed {
                (right, left)
            } else {
                (left, right)
            };
            edges.push(TypedEdge {
                left,
                right,
                kind,
                character_start: from.character_start,
                character_end: to.character_end,
                confidence_basis_points: from
                    .confidence_basis_points
                    .min(to.confidence_basis_points)
                    .min(CUE_CONFIDENCE_CEILING),
            });
        }
    }
    edges
}

fn is_actor(kind: CorpusEntityKind) -> bool {
    matches!(
        kind,
        CorpusEntityKind::Organization | CorpusEntityKind::Role | CorpusEntityKind::Person
    )
}

fn could_carry_cue(from: CorpusEntityKind, to: CorpusEntityKind) -> bool {
    use CorpusEntityKind::{Acronym, Date, Money, Organization, Person, Role, Statute};
    match (from, to) {
        (Person, Role | Organization) => true,
        (Statute, Statute) | (Acronym, Acronym) => true,
        (actor, Statute | Date | Money) => is_actor(actor),
        _ => false,
    }
}

/// Returns the edge kind and whether the edge runs from the later mention to the earlier one:
/// "A is superseded by B" means B supersedes A, and a signature block names the signer first.
fn classify(
    from: CorpusEntityKind,
    to: CorpusEntityKind,
    connector: &str,
) -> Option<(RelationshipKind, bool)> {
    use CorpusEntityKind::{Acronym, Date, Money, Person, Statute};
    if from == Person {
        let punctuation_only = connector
            .chars()
            .all(|character| character.is_whitespace() || ",;:|()-–—".contains(character));
        let short = connector.chars().count() <= SIGNATURE_WINDOW_CHARACTERS;
        let blank_line = connector.matches('\n').count() > 2;
        return (punctuation_only && short && !blank_line)
            .then_some((RelationshipKind::SignedBy, true));
    }
    if crosses_sentence(connector) {
        return None;
    }
    let words = words(connector);
    if from == to && matches!(from, Statute | Acronym) {
        return SUCCESSION_RULES
            .iter()
            .find(|(phrases, ..)| has_phrase(&words, phrases))
            .map(|&(_, kind, reversed)| (kind, reversed));
    }
    if words.iter().any(|word| {
        NEGATIONS.contains(&word.text.as_str())
            || word.text.ends_with("n't")
            || (word.text == "no" && !word.abbreviation_like())
    }) {
        return None;
    }
    match to {
        Statute if has_phrase(&words, CITES) => Some((RelationshipKind::Cites, false)),
        Date | Money if has_phrase(&words, PROMISES) && !is_passive(&words) => {
            Some((RelationshipKind::Promises, false))
        }
        Date | Money
            if has_phrase(&words, DELIVERED)
                && !words
                    .iter()
                    .any(|word| NOT_ACTIVE_PAST.contains(&word.text.as_str())) =>
        {
            Some((RelationshipKind::Delivered, false))
        }
        _ => None,
    }
}

/// "will be paid" names the actor as recipient; a promise needs the actor to be the one acting.
fn is_passive(words: &[Word]) -> bool {
    words.windows(2).any(|pair| {
        matches!(pair[0].text.as_str(), "will" | "shall" | "must") && pair[1].text == "be"
    })
}

/// A blank line, or a terminator that ends a sentence rather than an abbreviation or a decimal.
fn crosses_sentence(connector: &str) -> bool {
    let lines: Vec<&str> = connector.split('\n').collect();
    if lines.len() > 2
        && lines[1..lines.len() - 1]
            .iter()
            .any(|line| line.trim().is_empty())
    {
        return true;
    }
    let characters: Vec<char> = connector.chars().collect();
    characters.iter().enumerate().any(|(index, character)| {
        if !matches!(character, '.' | '!' | '?' | ';') {
            return false;
        }
        if characters
            .get(index + 1)
            .is_some_and(|next| !next.is_whitespace())
        {
            return false;
        }
        if *character != '.' {
            return true;
        }
        let word_before: String = characters[..index]
            .iter()
            .rev()
            .take_while(|previous| previous.is_alphabetic())
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<String>()
            .to_lowercase();
        !ABBREVIATIONS.contains(&word_before.as_str())
    })
}

fn collapse(raw: &str) -> String {
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Converts ascending byte offsets to character offsets without rescanning from the start.
#[derive(Default)]
struct ByteToCharacter {
    byte: usize,
    character: usize,
}

impl ByteToCharacter {
    fn at(&mut self, text: &str, byte: usize) -> usize {
        self.character += text[self.byte..byte].chars().count();
        self.byte = byte;
        self.character
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use CorpusEntityKind::{Date, Money, Organization, Person, Role, Statute};

    fn edges(
        text: &str,
        mentions: &[(CorpusEntityKind, &str)],
    ) -> Vec<(usize, usize, RelationshipKind)> {
        let inputs: Vec<EdgeInput> = mentions
            .iter()
            .map(|(kind, needle)| {
                let byte = text.find(needle).expect("needle in text");
                let start = text[..byte].chars().count();
                EdgeInput {
                    kind: *kind,
                    character_start: start,
                    character_end: start + needle.chars().count(),
                    confidence_basis_points: 9_000,
                }
            })
            .collect();
        derive_typed_edges(text, &inputs)
            .into_iter()
            .map(|edge| (edge.left, edge.right, edge.kind))
            .collect()
    }

    #[test]
    fn negation_passive_and_direction_change_what_the_edge_claims() {
        let agency = [(Organization, "City Agency"), (Date, "2026-01-05")];
        assert_eq!(
            edges("The City Agency will respond by 2026-01-05", &agency),
            [(0, 1, RelationshipKind::Promises)]
        );
        assert!(edges("The City Agency will not respond by 2026-01-05", &agency).is_empty());
        let vendor = [(Organization, "Acme Vendor"), (Money, "$500.00")];
        assert_eq!(
            edges("Acme Vendor paid $500.00", &vendor),
            [(0, 1, RelationshipKind::Delivered)]
        );
        assert!(edges("Acme Vendor was paid $500.00", &vendor).is_empty());
        assert_eq!(
            edges(
                "Section 12 is superseded by Section 40",
                &[(Statute, "Section 12"), (Statute, "Section 40")]
            ),
            [(1, 0, RelationshipKind::Supersedes)]
        );
    }

    #[test]
    fn abbreviations_do_not_end_or_negate_a_sentence() {
        let mentions = [(Organization, "City Agency"), (Statute, "Section 12")];
        assert_eq!(
            edges("City Agency, Contract No. 5, cites Section 12", &mentions),
            [(0, 1, RelationshipKind::Cites)]
        );
        assert!(edges("City Agency cites nothing. Section 12 applies", &mentions).is_empty());
    }

    #[test]
    fn signer_needs_a_signature_position_and_runs_from_role_to_signer() {
        let text = "Sincerely,\nJane Q. Doe\nCity Attorney";
        let found = role_and_signer_mentions(text);
        assert_eq!(
            found
                .iter()
                .map(|mention| (mention.kind, mention.normalized.as_str()))
                .collect::<Vec<_>>(),
            [(Role, "city attorney"), (Person, "jane q. doe")]
        );
        assert_eq!(
            edges(text, &[(Person, "Jane Q. Doe"), (Role, "City Attorney")]),
            [(1, 0, RelationshipKind::SignedBy)]
        );
        assert!(role_and_signer_mentions("Jane Doe met City Attorney staff")
            .iter()
            .all(|mention| mention.kind != Person));
    }
}
