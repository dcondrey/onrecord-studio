//! Practice requirements a policy states and departures another document states.
//!
//! Everything here is sentence-level word-token matching over a closed topic table. A sentence is
//! read as a requirement or as a departure only when it carries the topic's object and action
//! words together with an obligation cue or a noncompliance cue, and never when it carries a
//! negation, a recommendation, a conditional or a statement of compliance. The extractors err on
//! the side of saying nothing: a sentence that cannot be read cleanly is dropped rather than
//! published as a weaker claim. Nothing here verifies that a stated count is true; it reports
//! what a document states.

use crate::analysis::phrase_match::{find_phrases, has_phrase, words, Phrases, Word};
use chrono::NaiveDate;

/// Longer spans are text with lost punctuation, where no sentence boundary can be cited.
const MAX_SENTENCE_CHARACTERS: usize = 700;
/// A count this many words from every noncompliance cue is not read as that cue's count.
const COUNT_CUE_WINDOW: usize = 12;
const HEAD_WORDS: usize = 40;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PracticeTopic {
    BodyWornCameraActivation,
    BodyWornCameraUpload,
    UseOfForceReport,
    UseOfForceSupervisorReview,
    PursuitAuthorization,
    PursuitReview,
    StopDataDocumentation,
    EvidenceHandling,
    ComplaintIntake,
    TrainingRequirement,
}

impl PracticeTopic {
    pub const fn label(self) -> &'static str {
        match self {
            Self::BodyWornCameraActivation => "body-worn camera activation",
            Self::BodyWornCameraUpload => "body-worn camera upload",
            Self::UseOfForceReport => "use-of-force reporting",
            Self::UseOfForceSupervisorReview => "use-of-force supervisor review",
            Self::PursuitAuthorization => "pursuit authorization",
            Self::PursuitReview => "pursuit review",
            Self::StopDataDocumentation => "stop and search documentation",
            Self::EvidenceHandling => "evidence handling",
            Self::ComplaintIntake => "complaint intake",
            Self::TrainingRequirement => "training and certification",
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BodyWornCameraActivation => "body_worn_camera_activation",
            Self::BodyWornCameraUpload => "body_worn_camera_upload",
            Self::UseOfForceReport => "use_of_force_report",
            Self::UseOfForceSupervisorReview => "use_of_force_supervisor_review",
            Self::PursuitAuthorization => "pursuit_authorization",
            Self::PursuitReview => "pursuit_review",
            Self::StopDataDocumentation => "stop_data_documentation",
            Self::EvidenceHandling => "evidence_handling",
            Self::ComplaintIntake => "complaint_intake",
            Self::TrainingRequirement => "training_requirement",
        }
    }
}

/// A topic matches a sentence when every `all_of` set has a phrase in it and no `none_of` phrase
/// does. `none_of` is what keeps one sentence from being two topics at once.
struct TopicRule {
    topic: PracticeTopic,
    all_of: &'static [Phrases],
    none_of: Phrases,
}

const BWC_OBJECT: Phrases = &[
    &["body", "worn", "camera"],
    &["body", "worn", "cameras"],
    &["body", "worn", "video"],
    &["body", "camera"],
    &["body", "cameras"],
    &["bodycam"],
    &["bodycams"],
    &["bwc"],
    &["bwcs"],
];
const BWC_ACTIVATE: Phrases = &[
    &["activate"],
    &["activated"],
    &["activates"],
    &["activating"],
    &["activation"],
    &["record"],
    &["recorded"],
    &["recording"],
    &["turn", "on"],
    &["turned", "on"],
];
const BWC_UPLOAD: Phrases = &[
    &["upload"],
    &["uploads"],
    &["uploaded"],
    &["uploading"],
    &["download"],
    &["downloaded"],
    &["downloading"],
    &["dock"],
    &["docked"],
    &["docking"],
    &["transfer"],
    &["transferred"],
    &["transferring"],
];
const FORCE_OBJECT: Phrases = &[
    &["use", "of", "force"],
    &["uses", "of", "force"],
    &["uof"],
    &["force", "incident"],
    &["force", "incidents"],
];
const FORCE_REPORT: Phrases = &[
    &["report"],
    &["reports"],
    &["reported"],
    &["reporting"],
    &["document"],
    &["documented"],
    &["documentation"],
];
const FORCE_REVIEW: Phrases = &[
    &["review"],
    &["reviews"],
    &["reviewed"],
    &["reviewing"],
    &["investigate"],
    &["investigated"],
    &["investigation"],
    &["evaluate"],
    &["evaluated"],
    &["evaluation"],
    &["approve"],
    &["approved"],
    &["approval"],
];
const SUPERVISION: Phrases = &[
    &["supervisor"],
    &["supervisors"],
    &["supervisory"],
    &["sergeant"],
    &["sergeants"],
    &["lieutenant"],
    &["lieutenants"],
    &["commander"],
    &["commanders"],
    &["chain", "of", "command"],
];
const PURSUIT_OBJECT: Phrases = &[&["pursuit"], &["pursuits"], &["chase"], &["chases"]];
const PURSUIT_AUTHORIZE: Phrases = &[
    &["authorize"],
    &["authorized"],
    &["authorization"],
    &["approve"],
    &["approved"],
    &["approval"],
    &["permission"],
];
const PURSUIT_REVIEW: Phrases = &[
    &["review"],
    &["reviews"],
    &["reviewed"],
    &["critique"],
    &["critiqued"],
    &["after", "action"],
    &["evaluate"],
    &["evaluated"],
    &["evaluation"],
];
const STOP_OBJECT: Phrases = &[
    &["traffic", "stop"],
    &["traffic", "stops"],
    &["pedestrian", "stop"],
    &["pedestrian", "stops"],
    &["vehicle", "stop"],
    &["vehicle", "stops"],
    &["stop", "data"],
    &["field", "interview"],
    &["field", "interviews"],
    &["consent", "search"],
    &["consent", "searches"],
    &["vehicle", "search"],
    &["vehicle", "searches"],
];
const STOP_DOCUMENT: Phrases = &[
    &["document"],
    &["documented"],
    &["documentation"],
    &["record"],
    &["recorded"],
    &["report"],
    &["reported"],
    &["collect"],
    &["collected"],
    &["complete"],
    &["completed"],
    &["log"],
    &["logged"],
    &["enter"],
    &["entered"],
];
const EVIDENCE_OBJECT: Phrases = &[
    &["evidence"],
    &["chain", "of", "custody"],
    &["seized", "property"],
    &["property", "room"],
];
const EVIDENCE_HANDLE: Phrases = &[
    &["log"],
    &["logged"],
    &["submit"],
    &["submitted"],
    &["book"],
    &["booked"],
    &["tag"],
    &["tagged"],
    &["inventory"],
    &["inventoried"],
    &["secure"],
    &["secured"],
    &["store"],
    &["stored"],
    &["transfer"],
    &["transferred"],
    &["chain", "of", "custody"],
];
const COMPLAINT_OBJECT: Phrases = &[
    &["complaint"],
    &["complaints"],
    &["misconduct", "allegation"],
    &["misconduct", "allegations"],
];
const COMPLAINT_HANDLE: Phrases = &[
    &["accept"],
    &["accepted"],
    &["receive"],
    &["received"],
    &["intake"],
    &["log"],
    &["logged"],
    &["forward"],
    &["forwarded"],
    &["assign"],
    &["assigned"],
    &["acknowledge"],
    &["acknowledged"],
    &["investigate"],
    &["investigated"],
    &["investigation"],
];
const TRAINING_OBJECT: Phrases = &[
    &["training"],
    &["trainings"],
    &["certification"],
    &["recertification"],
    &["certified"],
    &["qualification"],
    &["qualifications"],
    &["requalification"],
    &["in", "service"],
];
const TRAINING_ACTION: Phrases = &[
    &["complete"],
    &["completed"],
    &["receive"],
    &["received"],
    &["attend"],
    &["attended"],
    &["maintain"],
    &["maintained"],
    &["obtain"],
    &["obtained"],
    &["pass"],
    &["passed"],
];

const TOPIC_RULES: &[TopicRule] = &[
    TopicRule {
        topic: PracticeTopic::BodyWornCameraActivation,
        all_of: &[BWC_OBJECT, BWC_ACTIVATE],
        none_of: BWC_UPLOAD,
    },
    TopicRule {
        topic: PracticeTopic::BodyWornCameraUpload,
        all_of: &[BWC_OBJECT, BWC_UPLOAD],
        none_of: &[],
    },
    TopicRule {
        topic: PracticeTopic::UseOfForceReport,
        all_of: &[FORCE_OBJECT, FORCE_REPORT],
        none_of: SUPERVISION,
    },
    TopicRule {
        topic: PracticeTopic::UseOfForceSupervisorReview,
        all_of: &[FORCE_OBJECT, FORCE_REVIEW, SUPERVISION],
        none_of: &[],
    },
    TopicRule {
        topic: PracticeTopic::PursuitAuthorization,
        all_of: &[PURSUIT_OBJECT, PURSUIT_AUTHORIZE],
        none_of: &[],
    },
    TopicRule {
        topic: PracticeTopic::PursuitReview,
        all_of: &[PURSUIT_OBJECT, PURSUIT_REVIEW],
        none_of: &[],
    },
    TopicRule {
        topic: PracticeTopic::StopDataDocumentation,
        all_of: &[STOP_OBJECT, STOP_DOCUMENT],
        none_of: BWC_OBJECT,
    },
    TopicRule {
        topic: PracticeTopic::EvidenceHandling,
        all_of: &[EVIDENCE_OBJECT, EVIDENCE_HANDLE],
        none_of: BWC_OBJECT,
    },
    TopicRule {
        topic: PracticeTopic::ComplaintIntake,
        all_of: &[COMPLAINT_OBJECT, COMPLAINT_HANDLE],
        none_of: &[],
    },
    TopicRule {
        topic: PracticeTopic::TrainingRequirement,
        all_of: &[TRAINING_OBJECT, TRAINING_ACTION],
        none_of: &[],
    },
];

const OBLIGATION: Phrases = &[
    &["shall"],
    &["must"],
    &["required", "to"],
    &["is", "required"],
    &["are", "required"],
    &["will"],
];
/// A sentence that states no duty, or states the opposite of one.
const NOT_A_DUTY: Phrases = &[
    &["shall", "not"],
    &["must", "not"],
    &["will", "not"],
    &["won't"],
    &["may", "not"],
    &["need", "not"],
    &["not", "required"],
    &["not", "be", "required"],
    &["no", "requirement"],
    &["never"],
];
const SUPERSEDED: Phrases = &[
    &["superseded", "by"],
    &["has", "been", "rescinded"],
    &["was", "rescinded"],
    &["is", "rescinded"],
    &["no", "longer", "in", "effect"],
    &["is", "obsolete"],
];
/// Stamped across a retired policy's first lines.
const SUPERSEDED_IN_HEAD: Phrases = &[&["superseded"], &["rescinded"], &["obsolete"]];

const DEPARTURE: Phrases = &[
    &["did", "not"],
    &["didn't"],
    &["failed", "to"],
    &["failure", "to"],
    &["failing", "to"],
    &["not", "activated"],
    &["not", "reviewed"],
    &["not", "documented"],
    &["not", "reported"],
    &["not", "uploaded"],
    &["not", "authorized"],
    &["not", "approved"],
    &["not", "completed"],
    &["not", "recorded"],
    &["not", "submitted"],
    &["not", "logged"],
    &["not", "investigated"],
    &["not", "forwarded"],
    &["not", "accepted"],
    &["not", "received"],
    &["not", "followed"],
    &["not", "complied"],
    &["noncompliance"],
    &["non", "compliance"],
    &["not", "in", "compliance"],
    &["out", "of", "compliance"],
    &["lacked"],
    &["lacking"],
    &["was", "missing"],
    &["were", "missing"],
];
/// A sentence that only looks like a departure: a recommendation, a hypothetical, a statement
/// that the rule does not apply, a statement that the agency had no rule, or a finding of
/// compliance.
const NOT_A_DEPARTURE: Phrases = &[
    &["recommend"],
    &["recommends"],
    &["recommended"],
    &["recommendation"],
    &["should"],
    &["shall"],
    &["must"],
    &["if"],
    &["whether"],
    &["would"],
    &["could"],
    &["who", "fail"],
    &["not", "required"],
    &["no", "requirement"],
    &["no", "policy"],
    &["not", "have", "a", "policy"],
    &["without", "a", "policy"],
    &["no", "instances"],
    &["no", "instance"],
    &["no", "evidence"],
    &["no", "cases"],
    &["no", "violations"],
    &["found", "no"],
    // Consequence clauses and hedges in policy text and in cautious findings.
    &["will"],
    &["may"],
    &["might"],
    &["discipline"],
    &["disciplined"],
    &["disciplinary"],
    &["did", "not", "fail"],
    &["never", "failed"],
    &["fully", "compliant"],
    &["in", "full", "compliance"],
];

const COUNT_FILLERS: &[&str] = &[
    "the", "all", "those", "these", "our", "total", "reviewed", "sampled",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sentence {
    /// Character offsets into the document text, end exclusive.
    pub character_start: usize,
    pub character_end: usize,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParameterUnit {
    Minutes,
    Hours,
    Days,
    Percent,
}

/// A measurable quantity a sentence states, such as "within 24 hours".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Parameter {
    pub value: u64,
    pub unit: ParameterUnit,
}

impl Parameter {
    pub fn display(self) -> String {
        let unit = match self.unit {
            ParameterUnit::Minutes => "minutes",
            ParameterUnit::Hours => "hours",
            ParameterUnit::Days => "days",
            ParameterUnit::Percent => "percent",
        };
        format!("{} {unit}", self.value)
    }

    /// Two parameters conflict only when they measure the same unit and state different values.
    pub fn conflicts_with(self, other: Self) -> bool {
        self.unit == other.unit && self.value != other.value
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Requirement {
    pub topic: PracticeTopic,
    pub sentence: Sentence,
    pub parameter: Option<Parameter>,
}

/// What a departure sentence says about how many cases failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatedCount {
    /// No count attached to the noncompliance.
    Unstated,
    /// `noncompliant` of `total` cases, with `0 < noncompliant <= total`.
    Stated { noncompliant: u64, total: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Departure {
    pub topic: PracticeTopic,
    pub sentence: Sentence,
    pub count: StatedCount,
    pub parameter: Option<Parameter>,
    /// Dates the sentence states, earliest first. Empty when it dates nothing.
    pub dates: Vec<NaiveDate>,
}

pub fn sentences(text: &str) -> Vec<Sentence> {
    let chars: Vec<char> = text.chars().collect();
    let mut found = Vec::new();
    let mut start = 0;
    let mut push = |start: usize, end: usize| {
        let first = (start..end).find(|&index| !chars[index].is_whitespace());
        let last = (start..end).rfind(|&index| !chars[index].is_whitespace());
        if let (Some(first), Some(last)) = (first, last) {
            if last + 1 - first <= MAX_SENTENCE_CHARACTERS {
                found.push(Sentence {
                    character_start: first,
                    character_end: last + 1,
                    text: chars[first..=last].iter().collect(),
                });
            }
        }
    };
    for index in 0..chars.len() {
        let character = chars[index];
        let ends_sentence = matches!(character, '.' | '!' | '?')
            && chars.get(index + 1).is_none_or(|next| next.is_whitespace())
            && !(character == '.' && abbreviation_before(&chars, index, start));
        let blank_line = character == '\n'
            && chars[index + 1..]
                .iter()
                .find(|next| **next != ' ' && **next != '\t' && **next != '\r')
                == Some(&'\n');
        if ends_sentence {
            push(start, index + 1);
            start = index + 1;
        } else if blank_line {
            push(start, index);
            start = index + 1;
        }
    }
    push(start, chars.len());
    found
}

/// A period after a list number opening a sentence, an initial, or a common abbreviation does not
/// end a sentence. A number elsewhere ("14 of 50.") does.
fn abbreviation_before(chars: &[char], period: usize, sentence_start: usize) -> bool {
    const ABBREVIATIONS: &[&str] = &[
        "no", "nos", "sec", "secs", "dr", "mr", "mrs", "ms", "st", "vs", "inc", "dept", "gen",
        "lt", "sgt", "cpl", "capt", "det", "ofc", "art", "para", "fig", "approx",
    ];
    let start = chars[..period]
        .iter()
        .rposition(|character| !character.is_alphanumeric())
        .map_or(0, |position| position + 1);
    let token: String = chars[start..period]
        .iter()
        .flat_map(|character| character.to_lowercase())
        .collect();
    let opens_sentence = chars[sentence_start.min(start)..start]
        .iter()
        .all(|character| character.is_whitespace());
    !token.is_empty()
        && ((opens_sentence && token.chars().all(|character| character.is_ascii_digit()))
            || token.chars().count() == 1
            || ABBREVIATIONS.contains(&token.as_str()))
}

fn topics_in(words: &[Word]) -> Vec<PracticeTopic> {
    TOPIC_RULES
        .iter()
        .filter(|rule| {
            rule.all_of.iter().all(|set| has_phrase(words, set)) && !has_phrase(words, rule.none_of)
        })
        .map(|rule| rule.topic)
        .collect()
}

/// Affirmative duties a policy states about a practice topic.
pub fn requirements(text: &str) -> Vec<Requirement> {
    sentences(text)
        .into_iter()
        .flat_map(|sentence| {
            let tokens = words(&sentence.text);
            if !has_phrase(&tokens, OBLIGATION) || has_phrase(&tokens, NOT_A_DUTY) {
                return Vec::new();
            }
            let parameter = parameter_in(&tokens);
            topics_in(&tokens)
                .into_iter()
                .map(|topic| Requirement {
                    topic,
                    sentence: sentence.clone(),
                    parameter,
                })
                .collect()
        })
        .collect()
}

/// Sentences that state a practice was not followed.
pub fn departures(text: &str) -> Vec<Departure> {
    sentences(text)
        .into_iter()
        .flat_map(|sentence| {
            let tokens = words(&sentence.text);
            let cues = find_phrases(&tokens, DEPARTURE);
            if cues.is_empty() || has_phrase(&tokens, NOT_A_DEPARTURE) {
                return Vec::new();
            }
            let Some(count) = stated_count(&tokens, &cues) else {
                return Vec::new();
            };
            let parameter = parameter_in(&tokens);
            let dates = dates_in(&tokens);
            topics_in(&tokens)
                .into_iter()
                .map(|topic| Departure {
                    topic,
                    sentence: sentence.clone(),
                    count,
                    parameter,
                    dates: dates.clone(),
                })
                .collect()
        })
        .collect()
}

/// A policy whose own text says it was replaced or withdrawn.
pub fn states_it_is_superseded(text: &str) -> bool {
    let tokens = words(text);
    has_phrase(&tokens, SUPERSEDED)
        || has_phrase(&tokens[..tokens.len().min(HEAD_WORDS)], SUPERSEDED_IN_HEAD)
}

fn number_at(tokens: &[Word], index: usize) -> Option<(u64, usize)> {
    let first = tokens.get(index)?;
    let mut value: u64 = first.text.parse().ok()?;
    let mut consumed = 1;
    // "1,200" arrives as "1" then "200".
    while tokens[index + consumed - 1].followed_by == Some(',') {
        let Some(group) = tokens.get(index + consumed) else {
            break;
        };
        if group.text.len() != 3 || !group.text.chars().all(|c| c.is_ascii_digit()) {
            break;
        }
        value = value
            .checked_mul(1000)?
            .checked_add(group.text.parse().ok()?)?;
        consumed += 1;
    }
    Some((value, consumed))
}

/// `None` rejects the sentence: a count of zero states compliance and an impossible count
/// (more failures than cases) cannot be read. Two counts in one sentence are ambiguous about which
/// is the failure count, so the sentence is kept with no count rather than with a guess.
fn stated_count(tokens: &[Word], cues: &[usize]) -> Option<StatedCount> {
    let mut readings = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        let Some((noncompliant, used)) = number_at(tokens, index) else {
            index += 1;
            continue;
        };
        let mut next = index + used;
        if tokens.get(next).is_some_and(|word| word.text == "out") {
            next += 1;
        }
        if tokens.get(next).is_some_and(|word| word.text == "of") {
            next += 1;
            if tokens
                .get(next)
                .is_some_and(|word| COUNT_FILLERS.contains(&word.text.as_str()))
            {
                next += 1;
            }
            if let Some((total, _)) = number_at(tokens, next) {
                readings.push((index, noncompliant, total));
            }
        }
        index += used;
    }
    match readings.as_slice() {
        [] => Some(StatedCount::Unstated),
        [(position, noncompliant, total)] => {
            if *noncompliant == 0 || *noncompliant > *total {
                return None;
            }
            let near_a_cue = cues
                .iter()
                .any(|cue| cue.abs_diff(*position) <= COUNT_CUE_WINDOW);
            Some(if near_a_cue {
                StatedCount::Stated {
                    noncompliant: *noncompliant,
                    total: *total,
                }
            } else {
                StatedCount::Unstated
            })
        }
        _ => Some(StatedCount::Unstated),
    }
}

fn parameter_in(tokens: &[Word]) -> Option<Parameter> {
    (0..tokens.len()).find_map(|index| {
        let (value, used) = number_at(tokens, index)?;
        let last = &tokens[index + used - 1];
        let unit = if last.followed_by == Some('%') {
            ParameterUnit::Percent
        } else {
            match tokens.get(index + used)?.text.as_str() {
                "minute" | "minutes" => ParameterUnit::Minutes,
                "hour" | "hours" => ParameterUnit::Hours,
                "day" | "days" => ParameterUnit::Days,
                "percent" => ParameterUnit::Percent,
                _ => return None,
            }
        };
        Some(Parameter { value, unit })
    })
}

fn month_number(word: &str) -> Option<u32> {
    const MONTHS: [(&str, &str); 12] = [
        ("january", "jan"),
        ("february", "feb"),
        ("march", "mar"),
        ("april", "apr"),
        ("may", "may"),
        ("june", "jun"),
        ("july", "jul"),
        ("august", "aug"),
        ("september", "sep"),
        ("october", "oct"),
        ("november", "nov"),
        ("december", "dec"),
    ];
    MONTHS
        .iter()
        .position(|(full, short)| {
            word == *full || word == *short || (*short == "sep" && word == "sept")
        })
        .map(|position| position as u32 + 1)
}

fn year(word: &Word) -> Option<i32> {
    let value: i32 = word.text.parse().ok()?;
    (word.text.len() == 4 && (1990..=2100).contains(&value)).then_some(value)
}

/// `March 12, 2026`, `March 2026`, `2026-03-12` and `3/12/2026`. A bare year dates nothing.
fn dates_in(tokens: &[Word]) -> Vec<NaiveDate> {
    let mut dates = Vec::new();
    for index in 0..tokens.len() {
        let word = &tokens[index];
        let date = if let Some(month) = month_number(&word.text) {
            let day = tokens
                .get(index + 1)
                .and_then(|next| next.text.parse::<u32>().ok());
            match (
                day,
                tokens.get(index + 2).and_then(year),
                tokens.get(index + 1).and_then(year),
            ) {
                (Some(day), Some(year), _) if day <= 31 && word.followed_by != Some('-') => {
                    NaiveDate::from_ymd_opt(year, month, day)
                }
                (_, _, Some(year)) => NaiveDate::from_ymd_opt(year, month, 1),
                _ => None,
            }
        } else if let Some(year) = year(word).filter(|_| word.followed_by == Some('-')) {
            let month = tokens
                .get(index + 1)
                .and_then(|next| next.text.parse().ok());
            let day = tokens
                .get(index + 2)
                .and_then(|next| next.text.parse().ok());
            month
                .zip(day)
                .and_then(|(month, day)| NaiveDate::from_ymd_opt(year, month, day))
        } else if word.followed_by == Some('/') {
            let month = word.text.parse().ok();
            let day = tokens
                .get(index + 1)
                .filter(|next| next.followed_by == Some('/'))
                .and_then(|next| next.text.parse().ok());
            let year = tokens.get(index + 2).and_then(year);
            month
                .zip(day)
                .zip(year)
                .and_then(|((month, day), year)| NaiveDate::from_ymd_opt(year, month, day))
        } else {
            None
        };
        dates.extend(date);
    }
    dates.sort_unstable();
    dates.dedup();
    dates
}

#[cfg(test)]
mod tests {
    use super::*;

    fn topics_of_requirements(text: &str) -> Vec<PracticeTopic> {
        requirements(text).into_iter().map(|r| r.topic).collect()
    }

    #[test]
    fn a_sentence_split_keeps_abbreviations_list_numbers_and_decimals_whole() {
        let text = "1. Officers shall follow Sec. 4.02 and General Order No. 7 at 9 a.m. daily. Second sentence!";
        let found = sentences(text);
        assert_eq!(found.len(), 2, "{found:?}");
        assert_eq!(
            found[0].text,
            "1. Officers shall follow Sec. 4.02 and General Order No. 7 at 9 a.m. daily."
        );
        let tail = sentences("Failed to activate in 14 of 50. Next sentence.");
        assert_eq!(tail.len(), 2, "a trailing number ends its sentence");
        let offsets: Vec<_> = found
            .iter()
            .map(|s| {
                text.chars()
                    .skip(s.character_start)
                    .take(s.character_end - s.character_start)
                    .collect::<String>()
            })
            .collect();
        assert_eq!(offsets[1], "Second sentence!");
    }

    #[test]
    fn character_offsets_count_characters_not_bytes() {
        let text = "Caf\u{e9} notes \u{2014} fine. Officers shall activate the BWC.";
        let found = sentences(text);
        let second = &found[1];
        let slice: String = text
            .chars()
            .skip(second.character_start)
            .take(second.character_end - second.character_start)
            .collect();
        assert_eq!(slice, second.text);
        assert_ne!(second.character_start, text.find("Officers").unwrap());
    }

    #[test]
    fn an_obligation_on_a_topic_is_a_requirement_with_its_parameter() {
        let found = requirements(
            "Officers shall activate their body-worn cameras before arriving at any call for service. \
             Recordings from body-worn cameras must be uploaded within 24 hours of the end of the shift.",
        );
        let topics: Vec<_> = found.iter().map(|r| r.topic).collect();
        assert_eq!(
            topics,
            [
                PracticeTopic::BodyWornCameraActivation,
                PracticeTopic::BodyWornCameraUpload
            ]
        );
        assert_eq!(found[0].parameter, None);
        assert_eq!(
            found[1].parameter,
            Some(Parameter {
                value: 24,
                unit: ParameterUnit::Hours
            })
        );
    }

    #[test]
    fn a_prohibition_an_exemption_or_a_bare_topic_is_not_a_requirement() {
        for text in [
            "Officers shall not review a use of force report they authored.",
            "Activation of the body-worn camera is not required during a restroom break.",
            "Body-worn cameras are issued to patrol officers.",
            "Supervisors shall be trained.",
        ] {
            assert!(topics_of_requirements(text).is_empty(), "{text}");
        }
    }

    #[test]
    fn a_use_of_force_review_sentence_is_not_also_a_reporting_sentence() {
        assert_eq!(
            topics_of_requirements(
                "A supervisor shall review each use of force report within 72 hours."
            ),
            [PracticeTopic::UseOfForceSupervisorReview]
        );
        assert_eq!(
            topics_of_requirements(
                "Officers must submit a use of force report before the end of shift."
            ),
            [PracticeTopic::UseOfForceReport]
        );
    }

    #[test]
    fn a_count_boundary_is_zero_none_one_total_and_above_total() {
        let departure = |n: &str| {
            departures(&format!(
                "Body-worn cameras were not activated in {n} of 40 incidents."
            ))
        };
        assert!(departure("0").is_empty(), "0 failures states compliance");
        for n in ["1", "39", "40"] {
            let found = departure(n);
            let count = found.first().map(|d| d.count);
            assert_eq!(
                count,
                Some(StatedCount::Stated {
                    noncompliant: n.parse().unwrap(),
                    total: 40
                }),
                "{n}"
            );
        }
        assert!(
            departure("41").is_empty(),
            "more failures than cases is unreadable"
        );
    }

    #[test]
    fn a_thousands_separator_and_a_distant_or_doubled_count_are_read_conservatively() {
        let found =
            departures("Officers failed to activate body-worn cameras in 1,200 of 4,000 stops.");
        assert_eq!(
            found[0].count,
            StatedCount::Stated {
                noncompliant: 1200,
                total: 4000
            }
        );
        let doubled = departures(
            "Officers failed to activate body-worn cameras in 12 of 40 stops and 3 of 9 calls.",
        );
        assert_eq!(doubled[0].count, StatedCount::Unstated);
        let distant = departures(
            "In 36 of 50 incidents, as the sampling plan described in the appendix of the annual \
             review explains at length, officers failed to activate body-worn cameras.",
        );
        assert_eq!(distant[0].count, StatedCount::Unstated);
    }

    #[test]
    fn negated_hypothetical_recommended_and_compliant_sentences_are_not_departures() {
        for text in [
            "Activation of body-worn cameras was not required during the transport.",
            "The audit found no instances in which officers failed to activate body-worn cameras.",
            "If an officer fails to activate a body-worn camera, the supervisor should be notified.",
            "We recommend that officers who failed to activate body-worn cameras be retrained.",
            "The department did not have a policy on body-worn camera activation.",
            "Officers must not fail to activate body-worn cameras.",
            "The audit asked whether body-worn cameras were not activated.",
        ] {
            assert!(departures(text).is_empty(), "{text}");
        }
    }

    #[test]
    fn a_departure_carries_its_topic_parameter_and_earliest_date() {
        let found = departures(
            "Between March 3, 2026 and 2026-06-30 supervisors did not review 9 of 30 use of force \
             incidents within 72 hours.",
        );
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].topic, PracticeTopic::UseOfForceSupervisorReview);
        assert_eq!(
            found[0].parameter,
            Some(Parameter {
                value: 72,
                unit: ParameterUnit::Hours
            })
        );
        assert_eq!(
            found[0].dates[0],
            NaiveDate::from_ymd_opt(2026, 3, 3).unwrap()
        );
        assert_eq!(found[0].dates.len(), 2);
    }

    #[test]
    fn dates_read_four_written_forms_and_ignore_a_bare_year() {
        let dates = dates_in(&words("Filed 3/12/2025, due March 2026, reviewed in 2024."));
        assert_eq!(
            dates,
            [
                NaiveDate::from_ymd_opt(2025, 3, 12).unwrap(),
                NaiveDate::from_ymd_opt(2026, 3, 1).unwrap(),
            ]
        );
    }

    #[test]
    fn only_an_explicit_statement_makes_a_policy_superseded() {
        assert!(states_it_is_superseded(
            "This policy has been superseded by General Order 12."
        ));
        assert!(states_it_is_superseded(
            "SUPERSEDED\nBody-worn camera policy"
        ));
        assert!(!states_it_is_superseded(
            "This policy supersedes General Order 12. Officers shall activate body-worn cameras."
        ));
    }
}
