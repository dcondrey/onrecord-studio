//! Replay: ledger entries and the entities currently in the corpus in, clusters out. Pure and
//! deterministic: the result depends on the set of entries and never on the order they arrived in.

use super::{LedgerEntry, ResolutionKey, Verdict};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// Corpus entities currently present, by stable key. A key can hold several ids in principle (the
/// shape a rebuild with different id derivation would produce); a cluster lists all of them.
pub type PresentEntities = BTreeMap<ResolutionKey, Vec<String>>;

type Pair = (ResolutionKey, ResolutionKey);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolutionCluster {
    pub members: Vec<ResolutionKey>,
    /// Ids of the members that exist in the corpus right now.
    pub entity_ids: Vec<String>,
    /// The latest SAME entries that joined the members, oldest first.
    pub entry_ids: Vec<String>,
}

/// SAME entries that, followed transitively, join two keys that a DIFFERENT entry separates.
/// Neither side is chosen: the component is left unmerged until a reviewer supersedes an entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Contradiction {
    pub keys: Vec<ResolutionKey>,
    pub same_entry_ids: Vec<String>,
    pub different_entry_ids: Vec<String>,
}

pub struct Replay {
    pub clusters: Vec<ResolutionCluster>,
    pub contradictions: Vec<Contradiction>,
    effective: BTreeMap<Pair, (Verdict, String)>,
    group: BTreeMap<ResolutionKey, ResolutionKey>,
    separated: BTreeSet<(ResolutionKey, ResolutionKey)>,
}

/// Latest entry per pair under the total order `(decided_at, id)`.
pub fn effective_entries(entries: &[LedgerEntry]) -> BTreeMap<Pair, &LedgerEntry> {
    let mut ordered: Vec<&LedgerEntry> = entries.iter().collect();
    ordered.sort_by(|a, b| (&a.decided_at, &a.id).cmp(&(&b.decided_at, &b.id)));
    let mut latest = BTreeMap::new();
    for entry in ordered {
        latest.insert((entry.left.clone(), entry.right.clone()), entry);
    }
    latest
}

struct DisjointSets {
    parent: Vec<usize>,
}

impl DisjointSets {
    fn find(&mut self, index: usize) -> usize {
        let mut root = index;
        while self.parent[root] != root {
            root = self.parent[root];
        }
        let mut cursor = index;
        while self.parent[cursor] != root {
            let next = self.parent[cursor];
            self.parent[cursor] = root;
            cursor = next;
        }
        root
    }

    fn union(&mut self, a: usize, b: usize) {
        let (a, b) = (self.find(a), self.find(b));
        if a != b {
            self.parent[a.max(b)] = a.min(b);
        }
    }
}

pub fn replay(entries: &[LedgerEntry], present: &PresentEntities) -> Replay {
    let latest = effective_entries(entries);
    let mut keys = BTreeSet::new();
    for (left, right) in latest.keys() {
        keys.insert(left.clone());
        keys.insert(right.clone());
    }
    let keys: Vec<ResolutionKey> = keys.into_iter().collect();
    let index: BTreeMap<&ResolutionKey, usize> =
        keys.iter().enumerate().map(|(i, key)| (key, i)).collect();

    let mut sets = DisjointSets {
        parent: (0..keys.len()).collect(),
    };
    for ((left, right), entry) in &latest {
        if entry.verdict == Verdict::Same {
            sets.union(index[left], index[right]);
        }
    }

    let mut same_by_root: BTreeMap<usize, Vec<&LedgerEntry>> = BTreeMap::new();
    let mut different_by_root: BTreeMap<usize, Vec<&LedgerEntry>> = BTreeMap::new();
    for ((left, right), entry) in &latest {
        let (a, b) = (sets.find(index[left]), sets.find(index[right]));
        match entry.verdict {
            Verdict::Same => same_by_root.entry(a).or_default().push(entry),
            Verdict::Different if a == b => different_by_root.entry(a).or_default().push(entry),
            _ => {}
        }
    }
    let mut members: BTreeMap<usize, Vec<ResolutionKey>> = BTreeMap::new();
    for (i, key) in keys.iter().enumerate() {
        members.entry(sets.find(i)).or_default().push(key.clone());
    }

    let mut clusters = Vec::new();
    let mut contradictions = Vec::new();
    let mut group: BTreeMap<ResolutionKey, ResolutionKey> = BTreeMap::new();
    for (root, mut component) in members {
        component.sort();
        let sames = same_by_root.remove(&root).unwrap_or_default();
        if sames.is_empty() {
            continue;
        }
        let ids = |entries: &[&LedgerEntry]| {
            let mut ordered = entries.to_vec();
            ordered.sort_by(|a, b| (&a.decided_at, &a.id).cmp(&(&b.decided_at, &b.id)));
            ordered.iter().map(|e| e.id.clone()).collect::<Vec<_>>()
        };
        if let Some(different) = different_by_root.remove(&root) {
            contradictions.push(Contradiction {
                keys: component,
                same_entry_ids: ids(&sames),
                different_entry_ids: ids(&different),
            });
            continue;
        }
        let mut entity_ids: Vec<String> = component
            .iter()
            .filter_map(|key| present.get(key))
            .flatten()
            .cloned()
            .collect();
        entity_ids.sort();
        entity_ids.dedup();
        for key in &component {
            group.insert(key.clone(), component[0].clone());
        }
        clusters.push(ResolutionCluster {
            members: component,
            entity_ids,
            entry_ids: ids(&sames),
        });
    }

    let group_of = |key: &ResolutionKey| group.get(key).unwrap_or(key).clone();
    let mut separated = BTreeSet::new();
    for ((left, right), entry) in &latest {
        if entry.verdict == Verdict::Different {
            let (a, b) = (group_of(left), group_of(right));
            separated.insert(if a <= b { (a, b) } else { (b, a) });
        }
    }

    Replay {
        clusters,
        contradictions,
        effective: latest
            .into_iter()
            .map(|(pair, entry)| (pair, (entry.verdict, entry.id.clone())))
            .collect(),
        group,
        separated,
    }
}

impl Replay {
    fn group_of(&self, key: &ResolutionKey) -> ResolutionKey {
        self.group.get(key).unwrap_or(key).clone()
    }

    pub fn cluster_of(&self, key: &ResolutionKey) -> Option<&ResolutionCluster> {
        let head = self.group.get(key)?;
        self.clusters
            .iter()
            .find(|cluster| &cluster.members[0] == head)
    }

    pub fn latest_verdict(&self, left: &ResolutionKey, right: &ResolutionKey) -> Option<Verdict> {
        let pair = if left <= right {
            (left.clone(), right.clone())
        } else {
            (right.clone(), left.clone())
        };
        self.effective.get(&pair).map(|(verdict, _)| *verdict)
    }

    pub fn is_latest(&self, entry: &LedgerEntry) -> bool {
        self.effective
            .get(&(entry.left.clone(), entry.right.clone()))
            .is_some_and(|(_, id)| id == &entry.id)
    }

    /// Whether a pair needs no human decision: already judged, already one cluster, or its clusters
    /// are held apart by a DIFFERENT. `UNSURE` stays open unless `unsure_settles` says otherwise.
    pub fn is_settled(
        &self,
        left: &ResolutionKey,
        right: &ResolutionKey,
        unsure_settles: bool,
    ) -> bool {
        match self.latest_verdict(left, right) {
            Some(Verdict::Same | Verdict::Different) => return true,
            Some(Verdict::Unsure) if unsure_settles => return true,
            _ => {}
        }
        let (a, b) = (self.group_of(left), self.group_of(right));
        a == b
            || self
                .separated
                .contains(&if a <= b { (a, b) } else { (b, a) })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::resolution::{EntitySnapshot, EvidenceSnapshot, SuggestionBasis};

    pub(crate) fn key(value: &str) -> ResolutionKey {
        ResolutionKey::new("PERSON", value)
    }

    pub(crate) fn entry(
        id: &str,
        at: &str,
        left: &str,
        right: &str,
        verdict: Verdict,
    ) -> LedgerEntry {
        let snapshot = |name: &str| EntitySnapshot {
            key: key(name),
            display_value: name.to_string(),
            document_count: 1,
            examples: Vec::new(),
        };
        let (left, right) = if left <= right {
            (left, right)
        } else {
            (right, left)
        };
        LedgerEntry {
            id: id.to_string(),
            left: key(left),
            right: key(right),
            verdict,
            author: "reviewer".to_string(),
            decided_at: format!("2026-01-01T00:00:{at}Z"),
            reason: String::new(),
            supersedes: None,
            evidence: EvidenceSnapshot {
                basis: SuggestionBasis::NameOnly,
                left: snapshot(left),
                right: snapshot(right),
                shared_identifier: None,
                name_variants: Vec::new(),
            },
        }
    }

    fn present(pairs: &[(&str, &str)]) -> PresentEntities {
        pairs
            .iter()
            .map(|(value, id)| (key(value), vec![id.to_string()]))
            .collect()
    }

    fn permutations(items: &[LedgerEntry]) -> Vec<Vec<LedgerEntry>> {
        if items.len() <= 1 {
            return vec![items.to_vec()];
        }
        let mut all = Vec::new();
        for i in 0..items.len() {
            let mut rest = items.to_vec();
            let head = rest.remove(i);
            for mut tail in permutations(&rest) {
                tail.insert(0, head.clone());
                all.push(tail);
            }
        }
        all
    }

    #[test]
    fn a_different_blocks_a_merge_even_through_a_transitive_same_chain() {
        let entries = vec![
            entry("1", "01.000", "a", "b", Verdict::Same),
            entry("2", "02.000", "b", "c", Verdict::Same),
            entry("3", "03.000", "a", "c", Verdict::Different),
            entry("4", "04.000", "d", "e", Verdict::Same),
        ];
        let here = present(&[
            ("a", "ea"),
            ("b", "eb"),
            ("c", "ec"),
            ("d", "ed"),
            ("e", "ee"),
        ]);
        let reference = replay(&entries, &here);
        for ordering in permutations(&entries) {
            let result = replay(&ordering, &here);
            assert_eq!(result.clusters, reference.clusters);
            assert_eq!(result.contradictions, reference.contradictions);
        }
        assert_eq!(reference.clusters.len(), 1);
        assert_eq!(reference.clusters[0].members, vec![key("d"), key("e")]);
        assert_eq!(reference.contradictions.len(), 1);
        assert_eq!(
            reference.contradictions[0].keys,
            vec![key("a"), key("b"), key("c")]
        );
        assert!(reference.cluster_of(&key("a")).is_none());
        assert!(reference.is_settled(&key("a"), &key("c"), false));
    }

    #[test]
    fn the_latest_entry_wins_with_ties_broken_by_id_and_a_supersede_clears_a_contradiction() {
        let mut entries = vec![
            entry("x1", "01.000", "a", "b", Verdict::Same),
            entry("x2", "01.000", "a", "b", Verdict::Different),
        ];
        let here = present(&[("a", "ea"), ("b", "eb")]);
        let forward = replay(&entries, &here);
        entries.reverse();
        let reversed = replay(&entries, &here);
        assert!(forward.clusters.is_empty() && reversed.clusters.is_empty());
        assert_eq!(
            forward.latest_verdict(&key("a"), &key("b")),
            Some(Verdict::Different)
        );
        assert_eq!(
            reversed.latest_verdict(&key("b"), &key("a")),
            Some(Verdict::Different)
        );
        entries.push(entry("x3", "02.000", "a", "b", Verdict::Same));
        let resolved = replay(&entries, &here);
        assert_eq!(resolved.clusters[0].entity_ids, vec!["ea", "eb"]);
        assert_eq!(resolved.clusters[0].entry_ids, vec!["x3"]);
        assert!(resolved.is_latest(&entries[2]) && !resolved.is_latest(&entries[0]));
    }

    #[test]
    fn decisions_still_apply_when_the_entities_return_with_new_ids() {
        let entries = vec![
            entry("1", "01.000", "a", "b", Verdict::Same),
            entry("2", "02.000", "b", "c", Verdict::Different),
        ];
        let before = replay(
            &entries,
            &present(&[("a", "id-1"), ("b", "id-2"), ("c", "id-3")]),
        );
        let after = replay(
            &entries,
            &present(&[("a", "new-9"), ("b", "new-8"), ("c", "new-7")]),
        );
        assert_eq!(before.clusters[0].entity_ids, vec!["id-1", "id-2"]);
        assert_eq!(after.clusters[0].entity_ids, vec!["new-8", "new-9"]);
        assert_eq!(before.clusters[0].members, after.clusters[0].members);
        assert!(after.is_settled(&key("b"), &key("c"), false));
        assert!(
            after.is_settled(&key("a"), &key("c"), false),
            "the cluster is held apart from c"
        );
        assert!(!after.is_settled(&key("a"), &key("z"), false));
    }

    #[test]
    fn unsure_leaves_a_pair_open_unless_the_caller_hides_it() {
        let entries = vec![entry("1", "01.000", "a", "b", Verdict::Unsure)];
        let result = replay(&entries, &PresentEntities::new());
        assert!(!result.is_settled(&key("a"), &key("b"), false));
        assert!(result.is_settled(&key("a"), &key("b"), true));
        assert!(result.clusters.is_empty());
    }
}
