//! Candidate matches between declared affiliations and what the app already holds.
//!
//! Every candidate is a proposal. Exact and alias matches are listed as pending like fuzzy ones:
//! a confirmation is the user's statement, and nothing here applies one on its own.

use crate::types::profile::{
    AffiliationMatchCandidate, AffiliationMatchKind, AffiliationMatchMethod, CandidateDecision,
    MatchDecisionKind, UserProfile,
};
use crate::types::{CorpusEntityKind, CorpusEntityNode};
use std::collections::BTreeSet;

pub const ENTITY_SEARCH_LIMIT: usize = 200;
const MAXIMUM_COMPARED_CHARACTERS: usize = 300;

pub const LIMITATIONS: [&str; 3] = [
    "Organization names in documents are found by matching a word list of organization words \
     (Department, Office, Board, and similar). A name without such a word is not extracted, so a \
     missing candidate does not mean the organization is absent from the records.",
    "Only the 200 most-documented entities in the corpus are searched, ranked across every kind \
     (dates and amounts included), and of those only organizations and acronyms are compared. An \
     organization mentioned in few documents may not be listed.",
    "A match is a proposal. Nothing is applied until you confirm it, and a confirmation records \
     your statement; it does not verify that you belong to the organization.",
];

/// A registered portal or agency, reduced to what matching needs.
#[derive(Debug, Clone)]
pub struct PortalTarget {
    pub portal_key: String,
    pub display_name: String,
}

/// An agency recorded under a portal. Its id is unique only within that portal.
#[derive(Debug, Clone)]
pub struct AgencyTarget {
    pub portal_key: String,
    pub agency_id: i64,
    pub name: String,
}

/// What currently carries a confirmed decision. Resolved fresh each time because entity ids do
/// not survive a corpus rebuild.
#[derive(Debug, Default)]
pub struct ConfirmedSubjects {
    pub portal_keys: BTreeSet<String>,
    pub agencies: BTreeSet<(String, i64)>,
    pub entity_ids: BTreeSet<String>,
}

/// The entity resolver's normal form, from the resolver itself.
pub fn normalize_name(value: &str) -> String {
    crate::storage::corpus::normalized_name(value)
}

/// The resolver keeps a leading capitalized article inside an organization run ("The City
/// Clerk"), so it is dropped for comparison only. Stored decisions keep the resolver's form.
pub(super) fn without_leading_article(normalized: &str) -> &str {
    normalized.strip_prefix("the ").unwrap_or(normalized)
}

struct Term {
    text: String,
    normalized: String,
    is_alias: bool,
}

struct Source<'a> {
    kind: AffiliationMatchKind,
    display: &'a str,
    normalized: String,
    grade: Option<crate::types::EntityValidationGrade>,
    document_count: Option<u64>,
    portal_key: Option<&'a str>,
    agency_id: Option<i64>,
    entity_id: Option<&'a str>,
}

pub fn find_candidates(
    profile: &UserProfile,
    portals: &[PortalTarget],
    agencies: &[AgencyTarget],
    entities: &[CorpusEntityNode],
) -> Vec<AffiliationMatchCandidate> {
    let mut sources: Vec<Source<'_>> = portals
        .iter()
        .map(|portal| Source {
            kind: AffiliationMatchKind::Portal,
            display: &portal.display_name,
            normalized: normalize_name(&portal.display_name),
            grade: None,
            document_count: None,
            portal_key: Some(&portal.portal_key),
            agency_id: None,
            entity_id: None,
        })
        .collect();
    sources.extend(agencies.iter().map(|agency| Source {
        kind: AffiliationMatchKind::Agency,
        display: &agency.name,
        normalized: normalize_name(&agency.name),
        grade: None,
        document_count: None,
        portal_key: Some(&agency.portal_key),
        agency_id: Some(agency.agency_id),
        entity_id: None,
    }));
    sources.extend(
        entities
            .iter()
            .filter(|entity| {
                matches!(
                    entity.kind,
                    CorpusEntityKind::Organization | CorpusEntityKind::Acronym
                )
            })
            .map(|entity| Source {
                kind: AffiliationMatchKind::Organization,
                display: &entity.display_value,
                normalized: normalize_name(&entity.display_value),
                grade: Some(entity.validation_grade),
                document_count: Some(entity.document_count),
                portal_key: None,
                agency_id: None,
                entity_id: Some(&entity.entity_id),
            }),
    );

    let mut candidates = Vec::new();
    let mut seen = BTreeSet::new();
    for affiliation in &profile.affiliations {
        let terms = std::iter::once(Term {
            text: affiliation.name.clone(),
            normalized: without_leading_article(&normalize_name(&affiliation.name)).to_string(),
            is_alias: false,
        })
        .chain(affiliation.aliases.iter().map(|alias| Term {
            text: alias.clone(),
            normalized: without_leading_article(&normalize_name(alias)).to_string(),
            is_alias: true,
        }))
        .collect::<Vec<_>>();
        for source in &sources {
            if source.normalized.is_empty() {
                continue;
            }
            let Some((method, term)) =
                best_match(&terms, without_leading_article(&source.normalized))
            else {
                continue;
            };
            let key = (
                affiliation.id.clone(),
                source.kind,
                source.normalized.clone(),
                source.entity_id.map(str::to_string),
                source.portal_key.map(str::to_string),
                source.agency_id,
            );
            if !seen.insert(key) {
                continue;
            }
            candidates.push(AffiliationMatchCandidate {
                affiliation_id: affiliation.id.clone(),
                kind: source.kind,
                display_value: source.display.to_string(),
                normalized_name: source.normalized.clone(),
                method,
                matched_term: term.text.clone(),
                validation_grade: source.grade,
                document_count: source.document_count,
                portal_key: source.portal_key.map(str::to_string),
                agency_id: source.agency_id,
                entity_id: source.entity_id.map(str::to_string),
                decision: decision_for(profile, &affiliation.id, source.kind, &source.normalized),
                currently_present: true,
            });
        }
        // A decision the user made earlier stays visible when its source is gone, so it can be
        // revisited instead of lingering invisibly.
        for decision in profile
            .match_decisions
            .iter()
            .filter(|decision| decision.affiliation_id == affiliation.id)
        {
            let listed = candidates.iter().any(|candidate| {
                candidate.affiliation_id == affiliation.id
                    && candidate.kind == decision.kind
                    && candidate.normalized_name == decision.normalized_name
            });
            if !listed {
                candidates.push(AffiliationMatchCandidate {
                    affiliation_id: affiliation.id.clone(),
                    kind: decision.kind,
                    display_value: decision.normalized_name.clone(),
                    normalized_name: decision.normalized_name.clone(),
                    method: AffiliationMatchMethod::PreviouslyDecided,
                    matched_term: affiliation.name.clone(),
                    validation_grade: None,
                    document_count: None,
                    portal_key: None,
                    agency_id: None,
                    entity_id: None,
                    decision: candidate_decision(decision.decision),
                    currently_present: false,
                });
            }
        }
    }
    candidates.sort_by(|left, right| {
        method_rank(left.method)
            .cmp(&method_rank(right.method))
            .then_with(|| left.display_value.cmp(&right.display_value))
    });
    candidates
}

pub fn resolve_confirmed_subjects(
    profile: &UserProfile,
    portals: &[PortalTarget],
    agencies: &[AgencyTarget],
    entities: &[CorpusEntityNode],
) -> ConfirmedSubjects {
    let confirmed: BTreeSet<(AffiliationMatchKind, &str)> = profile
        .match_decisions
        .iter()
        .filter(|decision| decision.decision == MatchDecisionKind::Confirmed)
        .map(|decision| (decision.kind, decision.normalized_name.as_str()))
        .collect();
    let is_confirmed = |kind: AffiliationMatchKind, name: &str| {
        confirmed.contains(&(kind, normalize_name(name).as_str()))
    };
    ConfirmedSubjects {
        portal_keys: portals
            .iter()
            .filter(|portal| is_confirmed(AffiliationMatchKind::Portal, &portal.display_name))
            .map(|portal| portal.portal_key.clone())
            .collect(),
        agencies: agencies
            .iter()
            .filter(|agency| is_confirmed(AffiliationMatchKind::Agency, &agency.name))
            .map(|agency| (agency.portal_key.clone(), agency.agency_id))
            .collect(),
        entity_ids: entities
            .iter()
            .filter(|entity| {
                matches!(
                    entity.kind,
                    CorpusEntityKind::Organization | CorpusEntityKind::Acronym
                ) && is_confirmed(AffiliationMatchKind::Organization, &entity.display_value)
            })
            .map(|entity| entity.entity_id.clone())
            .collect(),
    }
}

fn decision_for(
    profile: &UserProfile,
    affiliation_id: &str,
    kind: AffiliationMatchKind,
    normalized: &str,
) -> CandidateDecision {
    profile
        .match_decisions
        .iter()
        .find(|decision| {
            decision.affiliation_id == affiliation_id
                && decision.kind == kind
                && decision.normalized_name == normalized
        })
        .map_or(CandidateDecision::Pending, |decision| {
            candidate_decision(decision.decision)
        })
}

fn candidate_decision(decision: MatchDecisionKind) -> CandidateDecision {
    match decision {
        MatchDecisionKind::Confirmed => CandidateDecision::Confirmed,
        MatchDecisionKind::Rejected => CandidateDecision::Rejected,
    }
}

fn method_rank(method: AffiliationMatchMethod) -> u8 {
    match method {
        AffiliationMatchMethod::Exact => 0,
        AffiliationMatchMethod::Alias => 1,
        AffiliationMatchMethod::Fuzzy => 2,
        AffiliationMatchMethod::PreviouslyDecided => 3,
    }
}

fn best_match<'a>(
    terms: &'a [Term],
    candidate: &str,
) -> Option<(AffiliationMatchMethod, &'a Term)> {
    if let Some(term) = terms
        .iter()
        .find(|term| !term.is_alias && term.normalized == candidate)
    {
        return Some((AffiliationMatchMethod::Exact, term));
    }
    if let Some(term) = terms
        .iter()
        .find(|term| term.is_alias && term.normalized == candidate)
    {
        return Some((AffiliationMatchMethod::Alias, term));
    }
    terms
        .iter()
        .find(|term| is_fuzzy_match(&term.normalized, candidate))
        .map(|term| (AffiliationMatchMethod::Fuzzy, term))
}

/// Similar without being equal: the shorter name's words (at least two) all appear in the longer,
/// or the two differ by a small edit distance. Deliberately conservative; a wrong proposal costs
/// the user a click, but a hidden one is invisible.
fn is_fuzzy_match(term: &str, candidate: &str) -> bool {
    if term.is_empty() || candidate.is_empty() || term == candidate {
        return false;
    }
    let term_words: BTreeSet<&str> = term.split(' ').collect();
    let candidate_words: BTreeSet<&str> = candidate.split(' ').collect();
    let (short, long) = if term_words.len() <= candidate_words.len() {
        (&term_words, &candidate_words)
    } else {
        (&candidate_words, &term_words)
    };
    if short.len() >= 2 && short.is_subset(long) {
        return true;
    }
    let shorter = term.chars().count().min(candidate.chars().count());
    if shorter < 6
        || term.chars().count() > MAXIMUM_COMPARED_CHARACTERS
        || candidate.chars().count() > MAXIMUM_COMPARED_CHARACTERS
    {
        return false;
    }
    edit_distance(term, candidate) <= (shorter / 8).max(1)
}

fn edit_distance(left: &str, right: &str) -> usize {
    let right: Vec<char> = right.chars().collect();
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    for (row, left_character) in left.chars().enumerate() {
        let mut current = vec![row + 1];
        for (column, right_character) in right.iter().enumerate() {
            let substitution = previous[column] + usize::from(left_character != *right_character);
            current.push(
                substitution
                    .min(previous[column + 1] + 1)
                    .min(current[column] + 1),
            );
        }
        previous = current;
    }
    previous[right.len()]
}
