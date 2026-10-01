//! Which pairs deserve a human look, in what order. Pure over entities already read from the
//! corpus. Nothing here decides anything: a shared validated identifier ranks above a shared name,
//! and both are only suggestions.

use super::{
    invalid, is_identifier_kind, is_name_kind, name_key, ordered_pair, Replay, ResolutionKey,
    SharedIdentifier, SuggestionBasis,
};
use crate::error::AppError;
use serde::Serialize;
use std::cmp::Reverse;
use std::collections::BTreeMap;

/// A validated identifier must sit in the same segment as an entity in at least this many
/// documents before the two are linked; one shared header line is not a pattern.
pub const MINIMUM_IDENTIFIER_DOCUMENTS: u64 = 2;
/// Resource guards. A group this large is a generic name or a shared mailbox, and pairing its
/// members is quadratic. Skipped groups are counted and reported, not hidden.
pub const MAXIMUM_NAME_GROUP: usize = 40;
pub const MAXIMUM_IDENTIFIER_FANOUT: usize = 12;
pub const MAXIMUM_PAGE: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolutionEntity {
    pub entity_id: String,
    pub key: ResolutionKey,
    pub display_value: String,
    pub document_count: u64,
}

/// A validated identifier seen in the same segment as `entity_id`, in `documents` documents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentifierLink {
    pub identifier_id: String,
    pub identifier_kind: String,
    pub identifier_display: String,
    pub entity_id: String,
    pub documents: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidateGuards {
    pub skipped_name_groups: u64,
    pub skipped_identifiers: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairLead {
    pub left: ResolutionEntity,
    pub right: ResolutionEntity,
    pub basis: SuggestionBasis,
    pub shared_identifier: Option<SharedIdentifier>,
    pub score: u64,
}

impl PairLead {
    pub fn token(&self) -> String {
        format!("{}||{}", self.left.key.token(), self.right.key.token())
    }

    fn tier(&self) -> u8 {
        u8::from(self.basis == SuggestionBasis::StrongIdentifier)
    }

    fn order(&self) -> (Reverse<u8>, Reverse<u64>, String) {
        (Reverse(self.tier()), Reverse(self.score), self.token())
    }
}

fn oriented(a: &ResolutionEntity, b: &ResolutionEntity) -> (ResolutionEntity, ResolutionEntity) {
    if a.key <= b.key {
        (a.clone(), b.clone())
    } else {
        (b.clone(), a.clone())
    }
}

pub fn pair_lead(
    a: &ResolutionEntity,
    b: &ResolutionEntity,
    links: &[IdentifierLink],
) -> Result<PairLead, AppError> {
    ordered_pair(a.key.clone(), b.key.clone())?;
    let (left, right) = oriented(a, b);
    let mut lead = PairLead {
        score: left.document_count.min(right.document_count),
        left,
        right,
        basis: SuggestionBasis::Manual,
        shared_identifier: None,
    };
    if left_and_right_share_name(&lead) {
        lead.basis = SuggestionBasis::NameOnly;
    }
    let strong = |entity: &ResolutionEntity| -> BTreeMap<&str, &IdentifierLink> {
        links
            .iter()
            .filter(|link| {
                link.documents >= MINIMUM_IDENTIFIER_DOCUMENTS && link.entity_id == entity.entity_id
            })
            .map(|link| (link.identifier_id.as_str(), link))
            .collect()
    };
    let (left_links, right_links) = (strong(&lead.left), strong(&lead.right));
    let best = left_links
        .iter()
        .filter_map(|(identifier, left)| Some((*left, *right_links.get(identifier)?)))
        .max_by_key(|(left, right)| {
            (
                left.documents.min(right.documents),
                left.identifier_id.clone(),
            )
        });
    if let Some((left, right)) = best {
        lead.basis = SuggestionBasis::StrongIdentifier;
        lead.score = left.documents.min(right.documents);
        lead.shared_identifier = Some(SharedIdentifier {
            kind: left.identifier_kind.clone(),
            display_value: left.identifier_display.clone(),
            left_documents: left.documents,
            right_documents: right.documents,
        });
    }
    Ok(lead)
}

fn left_and_right_share_name(lead: &PairLead) -> bool {
    let (left, right) = (&lead.left.key, &lead.right.key);
    left.kind == right.kind
        && is_name_kind(&left.kind)
        && !name_key(&left.value).is_empty()
        && name_key(&left.value) == name_key(&right.value)
}

/// Every pair worth a look that is not already settled by the ledger.
pub fn rank_candidates(
    entities: &[ResolutionEntity],
    links: &[IdentifierLink],
    replay: &Replay,
    include_unsure: bool,
) -> (Vec<PairLead>, CandidateGuards) {
    let mut guards = CandidateGuards::default();
    let mut found: BTreeMap<(ResolutionKey, ResolutionKey), PairLead> = BTreeMap::new();
    let mut consider = |lead: PairLead| {
        if replay.is_settled(&lead.left.key, &lead.right.key, !include_unsure) {
            return;
        }
        let slot = (lead.left.key.clone(), lead.right.key.clone());
        match found.get(&slot) {
            Some(held) if held.tier() >= lead.tier() => {}
            _ => {
                found.insert(slot, lead);
            }
        }
    };

    let mut by_name: BTreeMap<(&str, String), Vec<&ResolutionEntity>> = BTreeMap::new();
    for entity in entities.iter().filter(|e| is_name_kind(&e.key.kind)) {
        let name = name_key(&entity.key.value);
        if !name.is_empty() {
            by_name
                .entry((&entity.key.kind, name))
                .or_default()
                .push(entity);
        }
    }
    for group in by_name.values().filter(|group| group.len() > 1) {
        if group.len() > MAXIMUM_NAME_GROUP {
            guards.skipped_name_groups += 1;
            continue;
        }
        for (i, a) in group.iter().enumerate() {
            for b in &group[i + 1..] {
                if let Ok(lead) = pair_lead(a, b, &[]) {
                    consider(lead);
                }
            }
        }
    }

    let known: BTreeMap<&str, &ResolutionEntity> =
        entities.iter().map(|e| (e.entity_id.as_str(), e)).collect();
    let mut by_identifier: BTreeMap<&str, Vec<&IdentifierLink>> = BTreeMap::new();
    for link in links.iter().filter(|link| {
        link.documents >= MINIMUM_IDENTIFIER_DOCUMENTS
            && is_identifier_kind(&link.identifier_kind)
            && known.contains_key(link.entity_id.as_str())
    }) {
        by_identifier
            .entry(&link.identifier_id)
            .or_default()
            .push(link);
    }
    for group in by_identifier.values().filter(|group| group.len() > 1) {
        if group.len() > MAXIMUM_IDENTIFIER_FANOUT {
            guards.skipped_identifiers += 1;
            continue;
        }
        for (i, a) in group.iter().enumerate() {
            for b in &group[i + 1..] {
                let (left, right) = (known[a.entity_id.as_str()], known[b.entity_id.as_str()]);
                if left.key.kind != right.key.kind {
                    continue;
                }
                if let Ok(lead) = pair_lead(left, right, &[(*a).clone(), (*b).clone()]) {
                    consider(lead);
                }
            }
        }
    }

    let mut leads: Vec<PairLead> = found.into_values().collect();
    leads.sort_by_key(PairLead::order);
    (leads, guards)
}

fn encode_cursor(lead: &PairLead) -> String {
    format!("{}:{}:{}", lead.tier(), lead.score, lead.token())
}

fn decode_cursor(cursor: &str) -> Result<(Reverse<u8>, Reverse<u64>, String), AppError> {
    let mut parts = cursor.splitn(3, ':');
    let (Some(tier), Some(score), Some(token)) = (parts.next(), parts.next(), parts.next()) else {
        return Err(invalid("the page cursor is not valid"));
    };
    match (tier.parse::<u8>(), score.parse::<u64>()) {
        (Ok(tier), Ok(score)) => Ok((Reverse(tier), Reverse(score), token.to_string())),
        _ => Err(invalid("the page cursor is not valid")),
    }
}

pub struct CandidatePage {
    pub leads: Vec<PairLead>,
    pub next_cursor: Option<String>,
    /// Candidates from the cursor onward, this page included.
    pub remaining: usize,
}

/// A keyset page: everything strictly after the cursor's sort position. Judging a pair between
/// pages removes it from the list and cannot make the next page skip or repeat another.
pub fn page_after(
    leads: Vec<PairLead>,
    cursor: Option<&str>,
    limit: usize,
) -> Result<CandidatePage, AppError> {
    if limit == 0 || limit > MAXIMUM_PAGE {
        return Err(invalid(format!(
            "a page is between 1 and {MAXIMUM_PAGE} candidates"
        )));
    }
    let after = cursor.map(decode_cursor).transpose()?;
    let rest: Vec<PairLead> = leads
        .into_iter()
        .filter(|lead| after.as_ref().is_none_or(|after| &lead.order() > after))
        .collect();
    let remaining = rest.len();
    let leads: Vec<PairLead> = rest.into_iter().take(limit).collect();
    let next_cursor = (remaining > leads.len())
        .then(|| leads.last().map(encode_cursor))
        .flatten();
    Ok(CandidatePage {
        leads,
        next_cursor,
        remaining,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resolution::replay::tests::entry;
    use crate::resolution::{replay, PresentEntities, Verdict};

    fn entity(kind: &str, value: &str, documents: u64) -> ResolutionEntity {
        ResolutionEntity {
            entity_id: format!("id-{kind}-{value}"),
            key: ResolutionKey::new(kind, value),
            display_value: value.to_string(),
            document_count: documents,
        }
    }

    fn link(identifier: &str, entity: &ResolutionEntity, documents: u64) -> IdentifierLink {
        IdentifierLink {
            identifier_id: identifier.to_string(),
            identifier_kind: "EMAIL_ADDRESS".to_string(),
            identifier_display: format!("{identifier}@x.org"),
            entity_id: entity.entity_id.clone(),
            documents,
        }
    }

    #[test]
    fn candidates_rank_identifiers_first_and_skip_what_the_ledger_settled() {
        let (a, b) = (
            entity("PERSON", "john smith", 9),
            entity("PERSON", "smith john", 4),
        );
        let (c, d) = (
            entity("PERSON", "m garcia", 3),
            entity("PERSON", "maria garcia", 5),
        );
        let (e, f) = (
            entity("ORGANIZATION", "acme inc", 7),
            entity("ORGANIZATION", "acme llc", 2),
        );
        let (date_a, date_b) = (
            entity("DATE", "2024-01-02", 9),
            entity("DATE", "2024-02-01", 9),
        );
        let entities = vec![a, b, c.clone(), d.clone(), e, f, date_a, date_b];
        let links = vec![link("mg", &c, 3), link("mg", &d, 2), link("one", &c, 1)];

        let none = replay(&[], &PresentEntities::new());
        let (leads, guards) = rank_candidates(&entities, &links, &none, false);
        assert_eq!(guards, CandidateGuards::default());
        let bases: Vec<_> = leads.iter().map(|lead| lead.basis).collect();
        assert_eq!(
            bases,
            vec![
                SuggestionBasis::StrongIdentifier,
                SuggestionBasis::NameOnly,
                SuggestionBasis::NameOnly
            ]
        );
        assert_eq!(
            leads[0].shared_identifier.as_ref().unwrap().display_value,
            "mg@x.org"
        );
        assert_eq!((leads[0].score, leads[1].score, leads[2].score), (2, 4, 2));
        assert!(leads.iter().all(|lead| lead.left.key.kind != "DATE"));

        let judged = [
            entry(
                "1",
                "01.000",
                "m garcia",
                "maria garcia",
                Verdict::Different,
            ),
            entry("2", "02.000", "john smith", "smith john", Verdict::Unsure),
        ];
        let settled = replay(&judged, &PresentEntities::new());
        let (open, _) = rank_candidates(&entities, &links, &settled, false);
        assert!(open.iter().all(|lead| lead.left.key.value != "m garcia"));
        let (with_unsure, _) = rank_candidates(&entities, &links, &settled, true);
        assert_eq!(with_unsure.len(), open.len() + 1);
    }

    #[test]
    fn oversized_groups_are_skipped_and_counted_not_paired() {
        let crowd: Vec<_> = (0..=MAXIMUM_NAME_GROUP)
            .map(|i| entity("PERSON", &format!("john smith{}", " ".repeat(i)), 1))
            .collect();
        let none = replay(&[], &PresentEntities::new());
        let (leads, guards) = rank_candidates(&crowd, &[], &none, false);
        assert!(leads.is_empty());
        assert_eq!(guards.skipped_name_groups, 1);
    }

    #[test]
    fn a_keyset_cursor_neither_skips_nor_repeats_when_a_pair_is_judged_between_pages() {
        let people: Vec<_> = (0..6)
            .flat_map(|i| {
                let docs = 10 - i;
                [
                    entity("PERSON", &format!("pat{i} lee"), docs),
                    entity("PERSON", &format!("lee pat{i}"), docs),
                ]
            })
            .collect();
        let none = replay(&[], &PresentEntities::new());
        let (leads, _) = rank_candidates(&people, &[], &none, false);
        assert_eq!(leads.len(), 6);
        let first = page_after(leads.clone(), None, 2).unwrap();
        assert_eq!((first.remaining, first.leads.len()), (6, 2));
        let cursor = first.next_cursor.unwrap();

        let judged = [entry(
            "1",
            "01.000",
            "lee pat0",
            "pat0 lee",
            Verdict::Different,
        )];
        let settled = replay(&judged, &PresentEntities::new());
        let (after_judging, _) = rank_candidates(&people, &[], &settled, false);
        let second = page_after(after_judging, Some(&cursor), 2).unwrap();
        let tokens: Vec<_> = second.leads.iter().map(PairLead::token).collect();
        assert_eq!(tokens, vec![leads[2].token(), leads[3].token()]);
        assert!(page_after(leads, Some("garbage"), 2).is_err());
    }
}
