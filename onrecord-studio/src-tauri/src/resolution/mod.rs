//! Persistent entity resolution: a per-device, append-only ledger of human judgments about whether
//! two corpus entities are the same thing, and a pure replay that turns the ledger into clusters.
//!
//! A judgment is keyed by `(kind, normalized value)`, never by `entity_id`, so it survives a corpus
//! rebuild that discards every id. Nothing here changes the corpus: a cluster is a reading of the
//! ledger, and a name that merely looks alike is a lead for a human to judge, never an automatic
//! merge. The only fact a SAME entry asserts is that a named reviewer said so, on the evidence the
//! entry snapshots.

mod candidates;
mod identifiers;
mod ledger;
mod replay;

pub use candidates::*;
pub use identifiers::*;
pub use ledger::*;
pub use replay::*;

use crate::error::AppError;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

pub(crate) fn invalid(message: impl Into<String>) -> AppError {
    AppError::DataParsing(format!("RESOLUTION_INVALID: {}", message.into()))
}

/// Kinds whose spelling can vary while naming one party; only these are proposed by name.
pub fn is_name_kind(kind: &str) -> bool {
    matches!(kind, "PERSON" | "ORGANIZATION")
}

/// Identity of an entity that does not depend on its corpus `entity_id`: the entity kind and the
/// index's own normalized value (which is what `entity_id` is derived from, so two ids with one key
/// cannot exist, and the same key reappears after a rebuild).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolutionKey {
    pub kind: String,
    pub value: String,
}

impl ResolutionKey {
    pub fn new(kind: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            value: value.into(),
        }
    }

    pub fn token(&self) -> String {
        format!("{}|{}", self.kind, self.value)
    }
}

/// Two keys in canonical order; a pair of identical keys is not a judgment.
pub fn ordered_pair(
    a: ResolutionKey,
    b: ResolutionKey,
) -> Result<(ResolutionKey, ResolutionKey), AppError> {
    match a.cmp(&b) {
        std::cmp::Ordering::Less => Ok((a, b)),
        std::cmp::Ordering::Greater => Ok((b, a)),
        std::cmp::Ordering::Equal => Err(invalid("an entity cannot be judged against itself")),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Verdict {
    Same,
    Different,
    Unsure,
}

/// What prompted the pair. Recorded with each judgment so agreement can later be counted per kind
/// of suggestion; `Manual` means the reviewer paired them without a suggestion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SuggestionBasis {
    StrongIdentifier,
    NameOnly,
    Manual,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MentionExample {
    pub portal_key: String,
    pub document_id: u64,
    pub character_start: u64,
    pub character_end: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntitySnapshot {
    pub key: ResolutionKey,
    pub display_value: String,
    pub document_count: u64,
    /// A bounded sample of where the entity was read, not all of them; compare with `document_count`.
    pub examples: Vec<MentionExample>,
}

/// A validated identifier that appears in the same text segment as both entities.
/// Co-occurrence is not attribution: it is a reason to look, not a statement of ownership.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedIdentifier {
    pub kind: String,
    pub display_value: String,
    pub left_documents: u64,
    pub right_documents: u64,
}

/// What the reviewer saw when deciding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceSnapshot {
    pub basis: SuggestionBasis,
    pub left: EntitySnapshot,
    pub right: EntitySnapshot,
    pub shared_identifier: Option<SharedIdentifier>,
    pub name_variants: Vec<String>,
}

const NAME_SUFFIXES: [&str; 8] = [
    "inc",
    "llc",
    "corp",
    "corporation",
    "co",
    "ltd",
    "company",
    "the",
];

/// Everything that is not a letter, a number, or JavaScript `\s` whitespace becomes a space. The
/// whitespace set is spelled out because Rust's `\s` and JavaScript's differ (U+0085, U+FEFF).
fn name_noise() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(
            r"[^\p{L}\p{N}\x{9}-\x{d}\x{20}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}\x{feff}]",
        )
        .expect("static pattern")
    })
}

fn is_script_space(character: char) -> bool {
    matches!(character,
        '\u{9}'..='\u{d}' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}'
        | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}')
}

/// Order-insensitive, punctuation-free, corporate-suffix-free form of a name. Mirrors `nameKey` in
/// `src/utils/entityEdges.ts` exactly; `src/utils/nameKeyFixture.json` is checked against both.
/// Only for proposing candidates: two names sharing a key are worth a human look, never one entity.
pub fn name_key(value: &str) -> String {
    let lowered = value.to_lowercase();
    let cleaned = name_noise().replace_all(&lowered, " ");
    let mut tokens: Vec<&str> = cleaned
        .split(is_script_space)
        .filter(|token| !token.is_empty() && !NAME_SUFFIXES.contains(token))
        .collect();
    // JavaScript sorts strings by UTF-16 code unit, which differs from Rust's code point order
    // for characters outside the Basic Multilingual Plane.
    tokens.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
    tokens.join(" ")
}

/// `sha256(head | corpus digest)`: the handle a caller cites to say "this reading of the ledger
/// against this corpus state". Either input changing changes it.
pub fn resolution_id(ledger_head: &str, corpus_state_digest: &str) -> String {
    crate::integrity::sha256_hex(
        format!("onrecord-resolution-v1\n{ledger_head}\n{corpus_state_digest}").as_bytes(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Deserialize)]
    struct Case {
        input: String,
        key: String,
    }

    #[test]
    fn name_key_matches_the_typescript_fixture() {
        let cases: Vec<Case> =
            serde_json::from_str(include_str!("../../../src/utils/nameKeyFixture.json")).unwrap();
        assert!(cases.len() >= 12);
        for case in cases {
            assert_eq!(name_key(&case.input), case.key, "input {:?}", case.input);
        }
    }

    #[test]
    fn a_pair_is_ordered_and_never_with_itself() {
        let a = ResolutionKey::new("PERSON", "ann lee");
        let b = ResolutionKey::new("PERSON", "zed");
        assert_eq!(ordered_pair(b.clone(), a.clone()).unwrap(), (a.clone(), b));
        assert!(ordered_pair(a.clone(), a).is_err());
    }
}
