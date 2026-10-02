//! Deterministic parsing of an investigative utterance into a typed [`QueryPlan`].
//!
//! Matching is over lowercased word tokens through [`phrase_match`], never over raw text: a cue
//! matches only on word boundaries, and every cue list is data that can be read one entry at a
//! time. An utterance that fits no intent is reported as such with the list of what can be asked;
//! it is never guessed into the nearest intent, because an investigator acting on a wrong guess
//! would be reading an answer to a question they did not ask.
//!
//! Two rules keep the router honest. If more than one specific intent fits, the result is
//! `Ambiguous` and names the candidates. If an intent fits but its subject is a placeholder
//! ("this contractor"), the result asks for a name; it does not search for the word "contractor".

use crate::analysis::phrase_match::{words, Phrases, Word};
use crate::types::intent::{CrossCorpusSignal, IntentOption, QueryPlan};

/// Longest utterance the router reads. An investigative question is a sentence, not a document.
pub const MAX_UTTERANCE_CHARS: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntentKind {
    Contradictions,
    WhyMissedDeadline,
    Signatories,
    Appearance,
    SharedDenialLanguage,
    ReusedExtensionGrounds,
    BehaviorShift,
    OrphanTopic,
    Connection,
    Findings,
}

impl IntentKind {
    const ALL: [Self; 10] = [
        Self::Contradictions,
        Self::WhyMissedDeadline,
        Self::Signatories,
        Self::Appearance,
        Self::SharedDenialLanguage,
        Self::ReusedExtensionGrounds,
        Self::BehaviorShift,
        Self::OrphanTopic,
        Self::Connection,
        Self::Findings,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Contradictions => "Contradictions",
            Self::WhyMissedDeadline => "Why a request missed its deadline",
            Self::Signatories => "Who signed for someone",
            Self::Appearance => "Where a name appears across agencies",
            Self::SharedDenialLanguage => "Same denial language across agencies",
            Self::ReusedExtensionGrounds => "Same extension ground across agencies",
            Self::BehaviorShift => "Agencies whose behavior changed",
            Self::OrphanTopic => "Topics one agency has and another lacks",
            Self::Connection => "How two names are connected",
            Self::Findings => "Stored findings",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Contradictions => {
                "Stored findings where documents contradict each other or themselves, optionally for one agency."
            }
            Self::WhyMissedDeadline => {
                "The statutory evaluation of one request, stating whether the determination date was recorded or a stand-in."
            }
            Self::Signatories => {
                "Entities the corpus graph links to a name by a typed SIGNED_BY relationship."
            }
            Self::Appearance => {
                "Documents containing a name or an exact quoted phrase, counted per agency and portal."
            }
            Self::SharedDenialLanguage => {
                "Stored cross-agency findings of one denial paragraph reused by several agencies."
            }
            Self::ReusedExtensionGrounds => {
                "Stored findings of one statutory extension ground used across unrelated request topics."
            }
            Self::BehaviorShift => {
                "Stored findings of an agency's behavior stepping over a specific date."
            }
            Self::OrphanTopic => {
                "Stored findings of a topic prominent at one agency and absent at another."
            }
            Self::Connection => {
                "The shortest path between two names in the corpus graph, with each hop's relationship type."
            }
            Self::Findings => "The highest-ranked stored findings, optionally for one agency.",
        }
    }

    fn example(self) -> &'static str {
        match self {
            Self::Contradictions => "show contradictions in agency Public Works",
            Self::WhyMissedDeadline => "why did request 42 miss its deadline",
            Self::Signatories => "who signed for the City Attorney",
            Self::Appearance => "where does \"Acme Paving\" appear across agencies",
            Self::SharedDenialLanguage => "same denial language across agencies",
            Self::ReusedExtensionGrounds => "same extension reason across agencies",
            Self::BehaviorShift => "which agencies changed behavior",
            Self::OrphanTopic => "show orphan topics",
            Self::Connection => "how is \"Acme Paving\" connected to \"Jane Doe\"",
            Self::Findings => "show findings for agency Public Works",
        }
    }

    fn option(self) -> IntentOption {
        IntentOption {
            label: self.label().to_string(),
            description: self.description().to_string(),
            example: self.example().to_string(),
        }
    }
}

/// Every intent the router can answer, with a runnable example each.
pub fn capabilities() -> Vec<IntentOption> {
    IntentKind::ALL.iter().map(|kind| kind.option()).collect()
}

/// What the router did with an utterance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedIntent {
    Plan(QueryPlan),
    /// The intent is clear; a name or number it needs is missing or a placeholder.
    NeedsSubject {
        kind: IntentKind,
        hint: &'static str,
    },
    Ambiguous(Vec<IntentKind>),
    NotUnderstood,
}

impl ParsedIntent {
    pub fn options(&self) -> Vec<IntentOption> {
        match self {
            Self::NotUnderstood => capabilities(),
            Self::Ambiguous(kinds) => kinds.iter().map(|kind| kind.option()).collect(),
            Self::NeedsSubject { kind, .. } => vec![kind.option()],
            Self::Plan(_) => Vec::new(),
        }
    }
}

const WHY: Phrases = &[&["why"], &["explain"], &["how", "late"]];
const DEADLINE: Phrases = &[
    &["deadline"],
    &["deadlines"],
    &["late"],
    &["overdue"],
    &["past", "due"],
    &["missed"],
    &["miss"],
    &["misses"],
    &["exceeded"],
    &["overran"],
    &["violated"],
];
const REQUEST_LABEL: Phrases = &[&["request"], &["requests"], &["case"]];
const REQUEST_NUMBER_FILLER: &[&str] = &["no", "number", "id", "nr"];

const SIGNED: Phrases = &[
    &["who", "signed"],
    &["who", "signs"],
    &["who", "sign"],
    &["signatory"],
    &["signatories"],
    &["signer"],
    &["signers"],
];

const CONTRADICTION: Phrases = &[
    &["contradiction"],
    &["contradictions"],
    &["contradict"],
    &["contradicts"],
    &["contradicting"],
    &["contradictory"],
    &["inconsistent"],
    &["inconsistency"],
    &["inconsistencies"],
    &["conflicting", "statements"],
];

const FINDING: Phrases = &[
    &["findings"],
    &["finding"],
    &["red", "flags"],
    &["flags"],
    &["concerns"],
    &["anomalies"],
    &["patterns"],
    &["issues"],
];

const WHERE: Phrases = &[&["where"]];
const APPEAR: Phrases = &[
    &["appear"],
    &["appears"],
    &["appearing"],
    &["mentioned"],
    &["show", "up"],
    &["shows", "up"],
    &["occur"],
    &["occurs"],
    &["listed"],
    &["used"],
    &["named"],
];
const WHICH_AGENCIES: Phrases = &[
    &["which", "agencies"],
    &["which", "portals"],
    &["which", "departments"],
    &["what", "agencies"],
    &["what", "portals"],
];
const AGENCY_VERBS: Phrases = &[
    &["mention"],
    &["mentions"],
    &["cite"],
    &["cites"],
    &["use"],
    &["uses"],
    &["contract", "with"],
    &["contracts", "with"],
    &["hire"],
    &["hired"],
    &["paid"],
    &["name"],
    &["names"],
];
const SCOPE_CUTS: Phrases = &[
    &["across"],
    &["in", "other"],
    &["in", "multiple"],
    &["in", "several"],
    &["between"],
];

const BETWEEN: Phrases = &[
    &["connection", "between"],
    &["connections", "between"],
    &["path", "between"],
    &["path", "from"],
    &["link", "between"],
    &["links", "between"],
    &["relationship", "between"],
    &["relationships", "between"],
];
const FORWARD: Phrases = &[
    &["connected", "to"],
    &["connected", "with"],
    &["linked", "to"],
    &["linked", "with"],
    &["related", "to"],
    &["tied", "to"],
];
const PAIR_SPLIT: Phrases = &[&["and"], &["to"], &["with"]];

const DENIAL_TEXT: Phrases = &[
    &["denial"],
    &["denials"],
    &["boilerplate"],
    &["template"],
    &["templates"],
    &["language"],
    &["wording"],
    &["phrasing"],
    &["paragraph"],
    &["paragraphs"],
];
const EXTENSION_GROUND: Phrases = &[
    &["extension"],
    &["extensions"],
    &["ground"],
    &["grounds"],
    &["reason"],
    &["reasons"],
];
const SAME: Phrases = &[
    &["same"],
    &["shared"],
    &["identical"],
    &["reused"],
    &["copied"],
    &["common"],
    &["repeated"],
    &["matching"],
    &["similar"],
    &["recycled"],
];
const ACROSS_AGENCIES: Phrases = &[
    &["across", "agencies"],
    &["across", "jurisdictions"],
    &["across", "portals"],
    &["across", "departments"],
    &["between", "agencies"],
    &["multiple", "agencies"],
    &["several", "agencies"],
    &["other", "agencies"],
    &["different", "agencies"],
    &["agencies"],
    &["portals"],
    &["jurisdictions"],
];

const BEHAVIOR_SHIFT: Phrases = &[
    &["behavior", "change"],
    &["behaviour", "change"],
    &["changed", "behavior"],
    &["changed", "behaviour"],
    &["behavior", "shift"],
    &["behaviour", "shift"],
    &["temporal", "shift"],
    &["change", "point"],
    &["change", "points"],
    &["step", "change"],
    &["shifted"],
];
const ORPHAN_TOPIC: Phrases = &[
    &["orphan", "topic"],
    &["orphan", "topics"],
    &["topics", "missing"],
    &["topic", "missing"],
    &["topics", "absent"],
    &["topic", "absent"],
    &["missing", "topics"],
    &["missing", "topic"],
    &["absent", "topics"],
];

/// Words that can open a subject without being part of it ("in agency X", "for the X").
const LEADING_FILLER: &[&str] = &[
    "the", "a", "an", "of", "for", "in", "at", "by", "on", "to", "with", "from", "about", "behalf",
    "is", "are", "was", "were", "does", "do", "did", "has", "have", "how", "what", "whats", "show",
    "me", "phrase", "name", "term",
];
const TRAILING_FILLER: &[&str] = &["please", "now", "today", "again", "here", "there"];
/// An agency subject may also lead with the noun that introduced it.
const AGENCY_LEAD: &[&str] = &[
    "agency",
    "agencies",
    "department",
    "dept",
    "office",
    "findings",
    "finding",
];
/// A subject made only of these names a scope, not an agency.
const SCOPE_WORDS: &[&str] = &[
    "across",
    "all",
    "every",
    "any",
    "agencies",
    "portals",
    "corpus",
    "everywhere",
    "anywhere",
    "jurisdictions",
    "everything",
    "everyone",
    "departments",
    "each",
];
/// A subject made only of these is a placeholder for a name the utterance did not give.
const PLACEHOLDER_WORDS: &[&str] = &[
    "this",
    "that",
    "these",
    "those",
    "it",
    "them",
    "they",
    "one",
    "contractor",
    "contractors",
    "vendor",
    "vendors",
    "company",
    "companies",
    "entity",
    "entities",
    "person",
    "people",
    "funding",
    "funder",
    "source",
    "sources",
    "organization",
    "organisation",
    "grant",
    "agency",
    "department",
    "phrase",
    "language",
    "paragraph",
    "text",
    "same",
    "denial",
    "who",
    "someone",
    "something",
];

struct Utterance {
    words: Vec<Word>,
    /// Text between double quotes, verbatim. The way to name something the token matcher would
    /// otherwise split, such as a hyphenated portal key or a multi-word company.
    quoted: Vec<String>,
}

enum Outcome {
    Plan(QueryPlan),
    NeedsSubject(&'static str),
}

struct Rule {
    kind: IntentKind,
    /// A generic rule fires only when no specific rule did, so "show contradiction findings in X"
    /// is a contradiction question and not an ambiguity with the findings list.
    generic: bool,
    apply: fn(&Utterance) -> Option<Outcome>,
}

const RULES: &[Rule] = &[
    Rule {
        kind: IntentKind::WhyMissedDeadline,
        generic: false,
        apply: why_missed_deadline,
    },
    Rule {
        kind: IntentKind::Signatories,
        generic: false,
        apply: signatories,
    },
    Rule {
        kind: IntentKind::Connection,
        generic: false,
        apply: connection,
    },
    Rule {
        kind: IntentKind::Appearance,
        generic: false,
        apply: appearance,
    },
    Rule {
        kind: IntentKind::SharedDenialLanguage,
        generic: false,
        apply: shared_denial_language,
    },
    Rule {
        kind: IntentKind::ReusedExtensionGrounds,
        generic: false,
        apply: reused_extension_grounds,
    },
    Rule {
        kind: IntentKind::BehaviorShift,
        generic: false,
        apply: behavior_shift,
    },
    Rule {
        kind: IntentKind::OrphanTopic,
        generic: false,
        apply: orphan_topic,
    },
    Rule {
        kind: IntentKind::Contradictions,
        generic: false,
        apply: contradictions,
    },
    Rule {
        kind: IntentKind::Findings,
        generic: true,
        apply: findings,
    },
];

/// Parses one utterance. Never guesses: see the module documentation.
pub fn parse(utterance: &str) -> ParsedIntent {
    let utterance = Utterance {
        words: words(utterance),
        quoted: quoted_spans(utterance),
    };
    if utterance.words.is_empty() {
        return ParsedIntent::NotUnderstood;
    }
    let mut matched: Vec<(IntentKind, bool, Outcome)> = RULES
        .iter()
        .filter_map(|rule| {
            (rule.apply)(&utterance).map(|outcome| (rule.kind, rule.generic, outcome))
        })
        .collect();
    // A rule that could not find its subject must not veto one that could.
    if matched
        .iter()
        .any(|(_, _, outcome)| matches!(outcome, Outcome::Plan(_)))
    {
        matched.retain(|(_, _, outcome)| matches!(outcome, Outcome::Plan(_)));
    }
    if matched.iter().any(|(_, generic, _)| !generic) {
        matched.retain(|(_, generic, _)| !generic);
    }
    match matched.len() {
        0 => ParsedIntent::NotUnderstood,
        1 => match matched.remove(0) {
            (_, _, Outcome::Plan(plan)) => ParsedIntent::Plan(plan),
            (kind, _, Outcome::NeedsSubject(hint)) => ParsedIntent::NeedsSubject { kind, hint },
        },
        _ => ParsedIntent::Ambiguous(matched.into_iter().map(|(kind, _, _)| kind).collect()),
    }
}

fn why_missed_deadline(utterance: &Utterance) -> Option<Outcome> {
    if !has(&utterance.words, WHY) || !has(&utterance.words, DEADLINE) {
        return None;
    }
    let Some(request_id) = request_number(&utterance.words) else {
        return Some(Outcome::NeedsSubject(
            "Give the request number, for example: why did request 42 miss its deadline",
        ));
    };
    Some(Outcome::Plan(QueryPlan::WhyRequestMissedDeadline {
        request_id,
        portal_key: utterance.quoted.first().cloned(),
    }))
}

fn signatories(utterance: &Utterance) -> Option<Outcome> {
    let (_, cue_end) = find_phrase(&utterance.words, SIGNED, 0)?;
    let subject = subject_of(utterance, cue_end, utterance.words.len(), &[]);
    match subject.filter(|value| !is_placeholder(value)) {
        Some(subject) => Some(Outcome::Plan(QueryPlan::SignatoriesFor { subject })),
        None => Some(Outcome::NeedsSubject(
            "Name the office or person, for example: who signed for the City Attorney",
        )),
    }
}

fn connection(utterance: &Utterance) -> Option<Outcome> {
    let words = &utterance.words;
    let between = find_phrase(words, BETWEEN, 0);
    let forward = find_phrase(words, FORWARD, 0);
    if between.is_none() && forward.is_none() {
        return None;
    }
    let pair = if utterance.quoted.len() >= 2 {
        Some((utterance.quoted[0].clone(), utterance.quoted[1].clone()))
    } else if let Some((_, cue_end)) = between {
        let split = find_phrase(words, PAIR_SPLIT, cue_end);
        split.and_then(|(split_start, split_end)| {
            Some((
                clean_subject(words, cue_end, split_start, &[])?,
                clean_subject(words, split_end, words.len(), &[])?,
            ))
        })
    } else {
        forward.and_then(|(cue_start, cue_end)| {
            Some((
                clean_subject(words, 0, cue_start, &[])?,
                clean_subject(words, cue_end, words.len(), &[])?,
            ))
        })
    };
    match pair.filter(|(from, to)| !is_placeholder(from) && !is_placeholder(to)) {
        Some((from, to)) => Some(Outcome::Plan(QueryPlan::ConnectionBetween { from, to })),
        None => Some(Outcome::NeedsSubject(
            "Name both, for example: how is \"Acme Paving\" connected to \"Jane Doe\"",
        )),
    }
}

fn appearance(utterance: &Utterance) -> Option<Outcome> {
    let words = &utterance.words;
    let subject = if let Some((_, where_end)) = find_phrase(words, WHERE, 0) {
        let (appear_start, _) = find_phrase(words, APPEAR, where_end)?;
        subject_of(utterance, where_end, appear_start, &[])
    } else {
        let (_, which_end) = find_phrase(words, WHICH_AGENCIES, 0)?;
        let (_, verb_end) = find_phrase(words, AGENCY_VERBS, which_end)?;
        let end = find_phrase(words, SCOPE_CUTS, verb_end).map_or(words.len(), |(start, _)| start);
        subject_of(utterance, verb_end, end, &[])
    };
    match subject.filter(|value| !is_placeholder(value)) {
        Some(subject) => Some(Outcome::Plan(QueryPlan::AppearanceAcrossAgencies {
            subject,
        })),
        None => Some(Outcome::NeedsSubject(
            "Name it or quote it, for example: where does \"Acme Paving\" appear across agencies",
        )),
    }
}

fn shared_denial_language(utterance: &Utterance) -> Option<Outcome> {
    let words = &utterance.words;
    (has(words, DENIAL_TEXT) && has(words, SAME) && has(words, ACROSS_AGENCIES)).then_some(
        Outcome::Plan(QueryPlan::CrossCorpusSignal {
            signal: CrossCorpusSignal::SharedDenialLanguage,
        }),
    )
}

fn reused_extension_grounds(utterance: &Utterance) -> Option<Outcome> {
    let words = &utterance.words;
    (has(words, EXTENSION_GROUND) && has(words, SAME) && has(words, ACROSS_AGENCIES)).then_some(
        Outcome::Plan(QueryPlan::CrossCorpusSignal {
            signal: CrossCorpusSignal::ReusedExtensionGrounds,
        }),
    )
}

fn behavior_shift(utterance: &Utterance) -> Option<Outcome> {
    has(&utterance.words, BEHAVIOR_SHIFT).then_some(Outcome::Plan(QueryPlan::CrossCorpusSignal {
        signal: CrossCorpusSignal::BehaviorShift,
    }))
}

fn orphan_topic(utterance: &Utterance) -> Option<Outcome> {
    has(&utterance.words, ORPHAN_TOPIC).then_some(Outcome::Plan(QueryPlan::CrossCorpusSignal {
        signal: CrossCorpusSignal::OrphanTopic,
    }))
}

fn contradictions(utterance: &Utterance) -> Option<Outcome> {
    let (_, cue_end) = find_phrase(&utterance.words, CONTRADICTION, 0)?;
    Some(Outcome::Plan(QueryPlan::ContradictionsInAgency {
        agency: agency_of(utterance, cue_end),
    }))
}

fn findings(utterance: &Utterance) -> Option<Outcome> {
    let (_, cue_end) = find_phrase(&utterance.words, FINDING, 0)?;
    Some(Outcome::Plan(QueryPlan::FindingsForAgency {
        agency: agency_of(utterance, cue_end),
    }))
}

/// The agency named after a cue, or `None` for "all agencies". A scope word ("across agencies")
/// is not an agency name.
fn agency_of(utterance: &Utterance, from: usize) -> Option<String> {
    subject_of(utterance, from, utterance.words.len(), AGENCY_LEAD)
        .filter(|subject| !is_scope(subject))
}

fn subject_of(
    utterance: &Utterance,
    start: usize,
    end: usize,
    extra_leading: &[&str],
) -> Option<String> {
    if let Some(quoted) = utterance.quoted.first() {
        return Some(quoted.clone());
    }
    clean_subject(&utterance.words, start, end, extra_leading)
}

fn clean_subject(
    words: &[Word],
    start: usize,
    end: usize,
    extra_leading: &[&str],
) -> Option<String> {
    let mut start = start.min(words.len());
    let mut end = end.min(words.len());
    while start < end
        && (LEADING_FILLER.contains(&words[start].text.as_str())
            || extra_leading.contains(&words[start].text.as_str()))
    {
        start += 1;
    }
    while start < end && TRAILING_FILLER.contains(&words[end - 1].text.as_str()) {
        end -= 1;
    }
    (start < end).then(|| {
        words[start..end]
            .iter()
            .map(|word| word.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    })
}

fn is_scope(subject: &str) -> bool {
    subject
        .split(' ')
        .all(|word| SCOPE_WORDS.contains(&word) || LEADING_FILLER.contains(&word))
}

fn is_placeholder(subject: &str) -> bool {
    subject
        .split(' ')
        .all(|word| PLACEHOLDER_WORDS.contains(&word) || LEADING_FILLER.contains(&word))
}

/// The number after `request`, tolerating `request #42`, `request no. 42` and `request number 42`.
fn request_number(words: &[Word]) -> Option<u64> {
    let (_, label_end) = find_phrase(words, REQUEST_LABEL, 0)?;
    let mut index = label_end;
    while words
        .get(index)
        .is_some_and(|word| REQUEST_NUMBER_FILLER.contains(&word.text.as_str()))
    {
        index += 1;
    }
    let candidate = words.get(index)?;
    if !candidate
        .text
        .chars()
        .all(|character| character.is_ascii_digit())
    {
        return None;
    }
    // `26-0412`, `4.2` and `12/2026` tokenize as a number joined to more digits; the leading part
    // alone names a different request.
    let joined_to_more = matches!(candidate.followed_by, Some('-' | '.' | '/'))
        && words.get(index + 1).is_some_and(|next| {
            next.text
                .chars()
                .next()
                .is_some_and(|character| character.is_ascii_digit())
        });
    if joined_to_more {
        return None;
    }
    candidate.text.parse().ok()
}

/// Spans between straight or curly double quotes, verbatim and trimmed. An unclosed quote yields
/// nothing rather than swallowing the rest of the sentence.
fn quoted_spans(text: &str) -> Vec<String> {
    let mut spans = Vec::new();
    let mut open: Option<String> = None;
    for character in text.chars() {
        match (&mut open, character) {
            (None, '"' | '“') => open = Some(String::new()),
            (Some(buffer), '"' | '”') => {
                let value = buffer.trim().to_string();
                if !value.is_empty() {
                    spans.push(value);
                }
                open = None;
            }
            (Some(buffer), other) => buffer.push(other),
            (None, _) => {}
        }
    }
    spans
}

fn has(words: &[Word], phrases: Phrases) -> bool {
    find_phrase(words, phrases, 0).is_some()
}

/// The earliest phrase occurrence starting at or after `from`, as `(start, end_exclusive)`. On a
/// tie the longer phrase wins, so `who signed` is not cut short by a shorter cue at the same word.
fn find_phrase(words: &[Word], phrases: Phrases, from: usize) -> Option<(usize, usize)> {
    let mut best: Option<(usize, usize)> = None;
    for phrase in phrases {
        if phrase.is_empty() || words.len() < phrase.len() {
            continue;
        }
        for start in from..=words.len() - phrase.len() {
            let window = &words[start..start + phrase.len()];
            if window
                .iter()
                .zip(*phrase)
                .all(|(word, cue)| word.text == *cue)
            {
                let candidate = (start, start + phrase.len());
                if best.is_none_or(|(best_start, best_end)| {
                    start < best_start || (start == best_start && candidate.1 > best_end)
                }) {
                    best = Some(candidate);
                }
                break;
            }
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(text: &str) -> QueryPlan {
        match parse(text) {
            ParsedIntent::Plan(plan) => plan,
            other => panic!("{text:?} did not parse to a plan: {other:?}"),
        }
    }

    #[test]
    fn the_five_investigator_phrasings_parse_to_their_plans() {
        assert_eq!(
            plan("show contradictions in agency Public Works"),
            QueryPlan::ContradictionsInAgency {
                agency: Some("public works".into())
            }
        );
        assert_eq!(
            plan("why did request 42 miss its deadline"),
            QueryPlan::WhyRequestMissedDeadline {
                request_id: 42,
                portal_key: None
            }
        );
        assert_eq!(
            plan("who signed for the City Attorney"),
            QueryPlan::SignatoriesFor {
                subject: "city attorney".into()
            }
        );
        assert_eq!(
            plan("where does Acme Paving appear across agencies"),
            QueryPlan::AppearanceAcrossAgencies {
                subject: "acme paving".into()
            }
        );
        assert_eq!(
            plan("same denial language across agencies"),
            QueryPlan::CrossCorpusSignal {
                signal: CrossCorpusSignal::SharedDenialLanguage
            }
        );
    }

    /// The listed examples are what the not-understood answer tells an investigator to type, so
    /// each must actually parse to the intent it is listed under.
    #[test]
    fn every_advertised_example_parses_to_its_own_intent() {
        for kind in IntentKind::ALL {
            let parsed = parse(kind.example());
            let ParsedIntent::Plan(plan) = parsed else {
                panic!("example for {kind:?} did not parse: {parsed:?}");
            };
            let matches_kind = match (kind, &plan) {
                (IntentKind::Contradictions, QueryPlan::ContradictionsInAgency { .. })
                | (IntentKind::WhyMissedDeadline, QueryPlan::WhyRequestMissedDeadline { .. })
                | (IntentKind::Signatories, QueryPlan::SignatoriesFor { .. })
                | (IntentKind::Appearance, QueryPlan::AppearanceAcrossAgencies { .. })
                | (IntentKind::Connection, QueryPlan::ConnectionBetween { .. })
                | (IntentKind::Findings, QueryPlan::FindingsForAgency { .. }) => true,
                (IntentKind::SharedDenialLanguage, QueryPlan::CrossCorpusSignal { signal }) => {
                    *signal == CrossCorpusSignal::SharedDenialLanguage
                }
                (IntentKind::ReusedExtensionGrounds, QueryPlan::CrossCorpusSignal { signal }) => {
                    *signal == CrossCorpusSignal::ReusedExtensionGrounds
                }
                (IntentKind::BehaviorShift, QueryPlan::CrossCorpusSignal { signal }) => {
                    *signal == CrossCorpusSignal::BehaviorShift
                }
                (IntentKind::OrphanTopic, QueryPlan::CrossCorpusSignal { signal }) => {
                    *signal == CrossCorpusSignal::OrphanTopic
                }
                _ => false,
            };
            assert!(matches_kind, "{kind:?} example parsed to {plan:?}");
        }
    }

    #[test]
    fn request_numbers_tolerate_the_common_spellings_and_reject_the_rest() {
        for text in [
            "why did request #42 miss its deadline",
            "why did request no. 42 miss its deadline",
            "why did request number 42 miss its deadline",
            "Why was Request 42 late?",
        ] {
            assert!(
                matches!(
                    plan(text),
                    QueryPlan::WhyRequestMissedDeadline { request_id: 42, .. }
                ),
                "{text}"
            );
        }
        for text in [
            "why did my request miss its deadline",
            "why did request 99999999999999999999999 miss its deadline",
            "why did request 4x2 miss its deadline",
            "why did request 26-0412 miss its deadline",
            "why did request 4.2 miss its deadline",
            "why did request 12/2026 miss its deadline",
        ] {
            assert!(
                matches!(parse(text), ParsedIntent::NeedsSubject { .. }),
                "{text}"
            );
        }
    }

    #[test]
    fn a_quoted_portal_key_scopes_the_request_and_survives_its_hyphen() {
        assert_eq!(
            plan("why did request 42 miss its deadline \"sd-city\""),
            QueryPlan::WhyRequestMissedDeadline {
                request_id: 42,
                portal_key: Some("sd-city".into())
            }
        );
    }

    #[test]
    fn a_placeholder_subject_asks_for_a_name_instead_of_searching_the_placeholder() {
        for text in [
            "where does this contractor appear across agencies",
            "who signed for them",
            "how is it connected to that vendor",
        ] {
            assert!(
                matches!(parse(text), ParsedIntent::NeedsSubject { .. }),
                "{text}: {:?}",
                parse(text)
            );
        }
    }

    #[test]
    fn quoting_names_what_a_placeholder_could_not() {
        assert_eq!(
            plan("where does the contractor \"Acme Paving\" appear"),
            QueryPlan::AppearanceAcrossAgencies {
                subject: "Acme Paving".into()
            }
        );
    }

    #[test]
    fn unsupported_or_empty_input_is_not_understood_and_lists_what_can_be_asked() {
        for text in [
            "",
            "   ",
            "tell me a joke",
            "the weather in san diego",
            "42",
        ] {
            let parsed = parse(text);
            assert_eq!(parsed, ParsedIntent::NotUnderstood, "{text:?}");
            assert_eq!(parsed.options(), capabilities());
        }
        assert!(capabilities().len() >= 5);
    }

    #[test]
    fn two_specific_intents_are_reported_as_ambiguous_not_ranked() {
        // "denial grounds" fits both the denial-paragraph signal and the extension-ground signal.
        match parse("same denial grounds across agencies") {
            ParsedIntent::Ambiguous(kinds) => {
                assert!(kinds.contains(&IntentKind::SharedDenialLanguage));
                assert!(kinds.contains(&IntentKind::ReusedExtensionGrounds));
            }
            other => panic!("expected ambiguity, got {other:?}"),
        }
    }

    #[test]
    fn a_generic_rule_yields_to_a_specific_one() {
        assert_eq!(
            plan("show contradiction findings in agency Parks"),
            QueryPlan::ContradictionsInAgency {
                agency: Some("parks".into())
            }
        );
    }

    #[test]
    fn a_scope_word_is_not_an_agency_name() {
        for text in [
            "show contradictions",
            "show contradictions across agencies",
            "contradictions in all agencies",
        ] {
            assert_eq!(
                plan(text),
                QueryPlan::ContradictionsInAgency { agency: None },
                "{text}"
            );
        }
    }

    #[test]
    fn connection_questions_split_the_pair_in_either_phrasing() {
        let expected = QueryPlan::ConnectionBetween {
            from: "acme paving".into(),
            to: "jane doe".into(),
        };
        assert_eq!(plan("how is Acme Paving connected to Jane Doe"), expected);
        assert_eq!(
            plan("show the connection between Acme Paving and Jane Doe"),
            expected
        );
        assert!(matches!(
            parse("how is Acme Paving connected to"),
            ParsedIntent::NeedsSubject { .. }
        ));
    }

    #[test]
    fn cues_match_on_word_boundaries_only() {
        // "latest" contains "late" and "signed" is inside "designed"; neither is a cue.
        assert_eq!(
            parse("why is the latest report designed this way"),
            ParsedIntent::NotUnderstood
        );
    }

    #[test]
    fn an_unclosed_quote_does_not_swallow_the_sentence() {
        assert!(quoted_spans("where does \"Acme Paving appear").is_empty());
        assert_eq!(quoted_spans("a “b c” d \"e\""), vec!["b c", "e"]);
    }
}
