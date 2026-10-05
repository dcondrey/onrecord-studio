//! Item-level comparison of two copies of one agenda.
//!
//! An agenda is read as the list of its "Item N:" headings. Two copies are compared by item
//! number: an item in only one copy was added or removed; an item in both is compared by its
//! title and then by its whole text. Nothing is inferred about why a copy changed.
use super::agenda::ITEMS;
use crate::{
    integrity::sha256_hex,
    types::meetings::{AgendaItemChange, AgendaItemChangeKind},
};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) const TITLE_CHARACTERS: usize = 300;
const TITLE_END_MARKERS: [&str; 3] = ["Council District", "Proposed Actions", "Recommendation"];

fn collapsed(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The title of an item from the text after its heading, and whether a marker ended it. Without
/// a marker the title runs to the end of the text given.
pub(crate) fn item_title(rest: &str) -> (String, bool) {
    // Markers are sought in the collapsed text so a line break inside one does not hide it.
    let flat = collapsed(rest);
    let end = TITLE_END_MARKERS
        .iter()
        .filter_map(|marker| flat.find(marker))
        .min();
    let title = flat[..end.unwrap_or(flat.len())]
        .trim_end()
        .chars()
        .take(TITLE_CHARACTERS)
        .collect();
    (title, end.is_some())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OutlineItem {
    /// The item number, upper-cased, which identifies the item across copies.
    pub key: String,
    /// The heading as written ("Item 605").
    pub label: String,
    /// `None` when the item's number appears more than once in the copy, or it is the last item
    /// and no marker ends its title: only a following heading or a marker proves where a title
    /// stops.
    pub title: Option<String>,
    /// Digest of the item's whole text. `None` for the last item, whose end is not proven, and
    /// for a repeated number.
    pub text_sha256: Option<String>,
}

pub(crate) fn outline(text: &str) -> Vec<OutlineItem> {
    let headings = ITEMS.captures_iter(text).collect::<Vec<_>>();
    let mut items: Vec<OutlineItem> = Vec::new();
    let mut repeated = BTreeSet::new();
    for (index, heading) in headings.iter().enumerate() {
        let whole = heading.get(0).expect("whole match");
        let key = heading["number"].to_uppercase();
        if items.iter().any(|item| item.key == key) {
            repeated.insert(key);
            continue;
        }
        let next = headings
            .get(index + 1)
            .and_then(|next| next.get(0))
            .map(|next| next.start());
        let rest = &text[whole.end()..next.unwrap_or(text.len())];
        let (title, marked) = item_title(rest);
        items.push(OutlineItem {
            key,
            label: collapsed(&heading["label"]),
            title: (next.is_some() || marked).then_some(title),
            text_sha256: next.map(|_| sha256_hex(collapsed(rest).as_bytes())),
        });
    }
    for item in &mut items {
        if repeated.contains(&item.key) {
            item.title = None;
            item.text_sha256 = None;
        }
    }
    items
}

pub(crate) struct Comparison {
    pub changes: Vec<AgendaItemChange>,
    pub titles_not_compared: Vec<String>,
}

/// Removed items in the earlier copy's order, then added, reworded and text-changed items in the
/// later copy's order.
pub(crate) fn compare(earlier: &[OutlineItem], later: &[OutlineItem]) -> Comparison {
    let before = earlier
        .iter()
        .map(|item| (item.key.as_str(), item))
        .collect::<BTreeMap<_, _>>();
    let after = later
        .iter()
        .map(|item| item.key.as_str())
        .collect::<BTreeSet<_>>();
    let mut changes = earlier
        .iter()
        .filter(|item| !after.contains(item.key.as_str()))
        .map(|item| AgendaItemChange {
            kind: AgendaItemChangeKind::Removed,
            item: item.label.clone(),
            before: item.title.clone(),
            after: None,
        })
        .collect::<Vec<_>>();
    let mut titles_not_compared = Vec::new();
    for item in later {
        let Some(old) = before.get(item.key.as_str()) else {
            changes.push(AgendaItemChange {
                kind: AgendaItemChangeKind::Added,
                item: item.label.clone(),
                before: None,
                after: item.title.clone(),
            });
            continue;
        };
        match (&old.title, &item.title) {
            (Some(was), Some(is)) if was != is => changes.push(AgendaItemChange {
                kind: AgendaItemChangeKind::Reworded,
                item: item.label.clone(),
                before: Some(was.clone()),
                after: Some(is.clone()),
            }),
            (Some(_), Some(_)) => {
                if let (Some(was), Some(is)) = (&old.text_sha256, &item.text_sha256) {
                    if was != is {
                        changes.push(AgendaItemChange {
                            kind: AgendaItemChangeKind::TextChanged,
                            item: item.label.clone(),
                            before: None,
                            after: None,
                        });
                    }
                }
            }
            _ => titles_not_compared.push(item.label.clone()),
        }
    }
    Comparison {
        changes,
        titles_not_compared,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORIGINAL: &str = "CITY COUNCIL AGENDA \
        Item 601: Approval of Council Minutes. Proposed Actions: Approve. \
        Item 602: Honorary Street Name. Proposed Actions: Resolution. \
        Item 605: Shelter Operating Agreement with Provider A. Proposed Actions: Authorize $1,000,000. \
        Item 606: Library Hours. Proposed Actions: Informational. \
        NON-AGENDA PUBLIC COMMENT";

    fn kinds(comparison: &Comparison) -> Vec<(AgendaItemChangeKind, &str)> {
        comparison
            .changes
            .iter()
            .map(|change| (change.kind, change.item.as_str()))
            .collect()
    }

    #[test]
    fn one_added_one_removed_and_one_reworded_item_are_exactly_the_three_changes() {
        let revised = ORIGINAL
            .replace(
                "Item 602: Honorary Street Name. Proposed Actions: Resolution. ",
                "",
            )
            .replace("Provider A", "Provider B")
            .replace(
                "Item 606:",
                "Item 330: Emergency Repair Contract. Proposed Actions: Ratify. Item 606:",
            );
        let found = compare(&outline(ORIGINAL), &outline(&revised));
        assert_eq!(
            kinds(&found),
            vec![
                (AgendaItemChangeKind::Removed, "Item 602"),
                (AgendaItemChangeKind::Reworded, "Item 605"),
                (AgendaItemChangeKind::Added, "Item 330"),
            ]
        );
        let reworded = &found.changes[1];
        assert_eq!(
            reworded.before.as_deref(),
            Some("Shelter Operating Agreement with Provider A.")
        );
        assert_eq!(
            reworded.after.as_deref(),
            Some("Shelter Operating Agreement with Provider B.")
        );
        assert!(found.titles_not_compared.is_empty());
    }

    #[test]
    fn identical_copies_and_copies_differing_only_in_spacing_yield_no_changes() {
        let same = compare(&outline(ORIGINAL), &outline(ORIGINAL));
        assert!(same.changes.is_empty() && same.titles_not_compared.is_empty());
        let respaced = ORIGINAL.replace(' ', "\n  ");
        assert!(compare(&outline(ORIGINAL), &outline(&respaced))
            .changes
            .is_empty());
    }

    #[test]
    fn a_change_below_the_title_is_reported_without_a_claimed_rewording() {
        let revised = ORIGINAL.replace(
            "Authorize $1,000,000",
            "Authorize $1,250,000 (Rev. 10/5/26)",
        );
        assert_eq!(
            kinds(&compare(&outline(ORIGINAL), &outline(&revised))),
            vec![(AgendaItemChangeKind::TextChanged, "Item 605")]
        );
    }

    #[test]
    fn an_unbounded_last_title_or_a_repeated_number_is_not_compared() {
        let open_ended = "Item 1: Budget Item 2: Closing remarks and adjournment";
        let grown = "Item 1: Budget Item 2: Closing remarks and adjournment at noon";
        let found = compare(&outline(open_ended), &outline(grown));
        assert!(found.changes.is_empty());
        assert_eq!(found.titles_not_compared, vec!["Item 2"]);
        let repeated = "Item 1: Budget Item 1: Budget hearing continued Item 2: Other Item 3: End";
        let found = compare(
            &outline("Item 1: Budget Item 2: Other Item 3: End"),
            &outline(repeated),
        );
        assert_eq!(found.titles_not_compared, vec!["Item 1", "Item 3"]);
        assert!(found.changes.is_empty());
    }
}
