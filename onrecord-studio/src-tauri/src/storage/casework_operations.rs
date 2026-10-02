//! The append-only operation log behind a case: what each saved revision added, changed, removed
//! or set, hash-chained in order. A packet carries the log so two desks that diverged from one
//! base can tell which incoming operations touch nothing the local desk touched since that base
//! (those commute and can be applied as they are) and which contend for the same item (those go
//! through the item picker). An author on an operation is the label the saving desk supplied;
//! the packet signature binds the log to a key, not to a person.

use crate::integrity::sha256_hex;
use crate::types::casework::*;
use serde::Serialize;
use std::collections::BTreeSet;

pub const OPERATION_ADD: &str = "ADD";
pub const OPERATION_CHANGE: &str = "CHANGE";
pub const OPERATION_REMOVE: &str = "REMOVE";
pub const OPERATION_SET: &str = "SET";
const FIELD_SECTION: &str = "case";

/// One operation before it is chained: everything but the hashes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingOperation {
    pub kind: &'static str,
    pub section: &'static str,
    pub target: String,
    pub summary: String,
    pub content_sha256: Option<String>,
}

/// A citation's verification rung is this desk's own measurement of whether the source is held
/// here, rewritten on every validate; it is not content the other desk authored, so it is left
/// out of the digest or every exchange would read as a change to every citation.
fn citation_digest(citation: &CaseworkCitation) -> Option<String> {
    let mut copy = citation.clone();
    copy.verification.clear();
    digest(&copy)
}

fn digest<T: Serialize>(value: &T) -> Option<String> {
    serde_json::to_vec(value)
        .ok()
        .map(|bytes| sha256_hex(&bytes))
}

fn diff_section<T: Serialize>(
    section: &'static str,
    label: impl Fn(&T) -> String,
    id: impl Fn(&T) -> &str,
    digest: impl Fn(&T) -> Option<String>,
    prior: &[T],
    next: &[T],
    out: &mut Vec<PendingOperation>,
) {
    for item in next {
        match prior.iter().find(|held| id(held) == id(item)) {
            None => out.push(PendingOperation {
                kind: OPERATION_ADD,
                section,
                target: id(item).to_string(),
                summary: label(item),
                content_sha256: digest(item),
            }),
            Some(held) if digest(held) != digest(item) => out.push(PendingOperation {
                kind: OPERATION_CHANGE,
                section,
                target: id(item).to_string(),
                summary: label(item),
                content_sha256: digest(item),
            }),
            Some(_) => {}
        }
    }
    for item in prior {
        if !next.iter().any(|held| id(held) == id(item)) {
            out.push(PendingOperation {
                kind: OPERATION_REMOVE,
                section,
                target: id(item).to_string(),
                summary: label(item),
                content_sha256: None,
            });
        }
    }
}

/// The operations that turn `prior` into `next`. A new case is every item added.
pub fn operations_between(
    prior: Option<&CaseworkCase>,
    next: &CaseworkCase,
) -> Vec<PendingOperation> {
    let empty = CaseworkCase::default();
    let prior = prior.unwrap_or(&empty);
    let mut out = Vec::new();
    for (field, before, after) in [
        ("title", &prior.title, &next.title),
        ("owner", &prior.owner, &next.owner),
        ("disposition", &prior.disposition, &next.disposition),
        ("notes", &prior.notes, &next.notes),
    ] {
        if before != after {
            out.push(PendingOperation {
                kind: OPERATION_SET,
                section: FIELD_SECTION,
                target: field.to_string(),
                summary: format!("Case {field}"),
                content_sha256: Some(sha256_hex(after.as_bytes())),
            });
        }
    }
    diff_section(
        "questions",
        |q: &CaseworkQuestion| q.question.clone(),
        |q| &q.id,
        digest,
        &prior.questions,
        &next.questions,
        &mut out,
    );
    diff_section(
        "requests",
        |r: &CaseworkRequest| r.title.clone(),
        |r| &r.id,
        digest,
        &prior.requests,
        &next.requests,
        &mut out,
    );
    diff_section(
        "decisions",
        |d: &CaseworkDecision| format!("{} · {}", d.body, d.agenda_item),
        |d| &d.id,
        digest,
        &prior.decisions,
        &next.decisions,
        &mut out,
    );
    diff_section(
        "commitments",
        |c: &CaseworkCommitment| c.promise.clone(),
        |c| &c.id,
        digest,
        &prior.commitments,
        &next.commitments,
        &mut out,
    );
    diff_section(
        "evidence",
        |e: &CaseworkCitation| e.label.clone(),
        |e| &e.id,
        citation_digest,
        &prior.evidence,
        &next.evidence,
        &mut out,
    );
    out
}

/// The hash of one chained operation. Every field that gives the operation its meaning is bound,
/// so a log entry cannot be re-attributed, re-ordered or re-targeted without breaking the chain.
pub fn operation_hash(operation: &CaseOperation) -> String {
    sha256_hex(
        format!(
            "{}|{}|{}|{}|{}|{}|{}|{}|{}",
            operation.previous_hash.as_deref().unwrap_or(""),
            operation.revision,
            operation.sequence,
            operation.author,
            operation.at,
            operation.kind,
            operation.section,
            operation.target,
            operation.content_sha256.as_deref().unwrap_or(""),
        )
        .as_bytes(),
    )
}

/// Chains `pending` after `previous_hash` as revision `revision`.
pub fn chain(
    pending: Vec<PendingOperation>,
    revision: u64,
    author: &str,
    at: &str,
    mut previous_hash: Option<String>,
) -> Vec<CaseOperation> {
    let mut out = Vec::with_capacity(pending.len());
    for (sequence, item) in pending.into_iter().enumerate() {
        let mut operation = CaseOperation {
            revision,
            sequence: sequence as u64,
            author: author.to_string(),
            at: at.to_string(),
            kind: item.kind.to_string(),
            section: item.section.to_string(),
            target: item.target,
            summary: item.summary,
            content_sha256: item.content_sha256,
            previous_hash: previous_hash.take(),
            hash: String::new(),
        };
        operation.hash = operation_hash(&operation);
        previous_hash = Some(operation.hash.clone());
        out.push(operation);
    }
    out
}

/// True when every operation's hash is its own recomputation and each names its predecessor.
/// The first operation's predecessor is not checked: a packet carries the log from the case's
/// beginning, so an unbroken chain from the first entry is what an exchanged log can prove.
pub fn chain_is_intact(operations: &[CaseOperation]) -> bool {
    operations.iter().enumerate().all(|(index, operation)| {
        operation.hash == operation_hash(operation)
            && (index == 0
                || operation.previous_hash.as_deref() == Some(operations[index - 1].hash.as_str()))
    })
}

/// Splits the incoming operations into those that commute with the local operations recorded
/// since the last exchange and those that contend with them.
///
/// Revision numbers are desk-local, so "since the base" is decided per side: the caller supplies
/// the local operations after its last sync point, and an incoming operation is judged by its
/// content against the local case rather than by its number. An operation commutes when it adds
/// or changes an item that no local operation since the sync touched, the incoming case still
/// holds that item, and the local case does not already hold it with the same content. A removal
/// never commutes: an item the other desk lacks may only be older there, so a removal is a
/// picker decision. A field set never commutes: two desks' case notes are one value with no
/// merge. A broken incoming chain makes every operation contested, because the log then proves
/// nothing about what was touched.
pub fn partition_operations(
    incoming: &[CaseOperation],
    incoming_case: &CaseworkCase,
    local_case: &CaseworkCase,
    local_since_sync: &[CaseOperation],
) -> (Vec<CaseOperation>, Vec<CaseOperation>) {
    if !chain_is_intact(incoming) {
        return (Vec::new(), incoming.to_vec());
    }
    let touched: BTreeSet<(&str, &str)> = local_since_sync
        .iter()
        .map(|operation| (operation.section.as_str(), operation.target.as_str()))
        .collect();
    let present = |section: &str, target: &str| match section {
        "questions" => incoming_case.questions.iter().any(|q| q.id == target),
        "requests" => incoming_case.requests.iter().any(|r| r.id == target),
        "decisions" => incoming_case.decisions.iter().any(|d| d.id == target),
        "commitments" => incoming_case.commitments.iter().any(|c| c.id == target),
        "evidence" => incoming_case.evidence.iter().any(|e| e.id == target),
        _ => false,
    };
    let local_digest = |section: &str, target: &str| match section {
        "questions" => local_case
            .questions
            .iter()
            .find(|q| q.id == target)
            .and_then(digest),
        "requests" => local_case
            .requests
            .iter()
            .find(|r| r.id == target)
            .and_then(digest),
        "decisions" => local_case
            .decisions
            .iter()
            .find(|d| d.id == target)
            .and_then(digest),
        "commitments" => local_case
            .commitments
            .iter()
            .find(|c| c.id == target)
            .and_then(digest),
        "evidence" => local_case
            .evidence
            .iter()
            .find(|e| e.id == target)
            .and_then(citation_digest),
        _ => None,
    };
    let mut commuting: Vec<CaseOperation> = Vec::new();
    let mut contested = Vec::new();
    for operation in incoming {
        let additive = operation.kind == OPERATION_ADD || operation.kind == OPERATION_CHANGE;
        let key = (operation.section.as_str(), operation.target.as_str());
        if additive && local_digest(key.0, key.1) == operation.content_sha256 {
            continue;
        }
        if additive && !touched.contains(&key) && present(key.0, key.1) {
            // One entry per item: the latest operation is the one whose content the case holds.
            commuting.retain(|held| (held.section.as_str(), held.target.as_str()) != key);
            commuting.push(operation.clone());
        } else {
            contested.push(operation.clone());
        }
    }
    (commuting, contested)
}

/// Copies the items the commuting operations name from `incoming` into `local`, replacing an
/// item of the same id or appending it, and copies any evidence those items cite that the local
/// case lacks so no citation is left pointing at nothing.
pub fn apply_operations(
    local: &mut CaseworkCase,
    incoming: &CaseworkCase,
    commuting: &[CaseOperation],
) {
    fn upsert<T: Clone>(items: &mut Vec<T>, item: &T, same: impl Fn(&T, &T) -> bool) {
        match items.iter_mut().find(|held| same(held, item)) {
            Some(held) => *held = item.clone(),
            None => items.push(item.clone()),
        }
    }
    let mut cited: BTreeSet<String> = BTreeSet::new();
    for operation in commuting {
        let target = operation.target.as_str();
        match operation.section.as_str() {
            "questions" => {
                if let Some(item) = incoming.questions.iter().find(|q| q.id == target) {
                    cited.extend(item.citation_ids.iter().cloned());
                    cited.extend(item.counter_citation_ids.iter().cloned());
                    upsert(&mut local.questions, item, |a, b| a.id == b.id);
                }
            }
            "requests" => {
                if let Some(item) = incoming.requests.iter().find(|r| r.id == target) {
                    cited.extend(item.citation_ids.iter().cloned());
                    cited.extend(
                        item.productions
                            .iter()
                            .flat_map(|p| p.citation_ids.iter().cloned()),
                    );
                    upsert(&mut local.requests, item, |a, b| a.id == b.id);
                }
            }
            "decisions" => {
                if let Some(item) = incoming.decisions.iter().find(|d| d.id == target) {
                    cited.extend(item.citation_ids.iter().cloned());
                    cited.extend(item.outcome_citation_ids.iter().cloned());
                    upsert(&mut local.decisions, item, |a, b| a.id == b.id);
                }
            }
            "commitments" => {
                if let Some(item) = incoming.commitments.iter().find(|c| c.id == target) {
                    cited.extend(item.citation_ids.iter().cloned());
                    cited.extend(item.completion_citation_ids.iter().cloned());
                    upsert(&mut local.commitments, item, |a, b| a.id == b.id);
                }
            }
            "evidence" => {
                if let Some(item) = incoming.evidence.iter().find(|e| e.id == target) {
                    upsert(&mut local.evidence, item, |a, b| a.id == b.id);
                }
            }
            _ => {}
        }
    }
    for id in cited {
        if !local.evidence.iter().any(|e| e.id == id) {
            if let Some(item) = incoming.evidence.iter().find(|e| e.id == id) {
                local.evidence.push(item.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn question(id: &str, text: &str) -> CaseworkQuestion {
        CaseworkQuestion {
            id: id.into(),
            question: text.into(),
            ..CaseworkQuestion::default()
        }
    }

    fn case(questions: Vec<CaseworkQuestion>) -> CaseworkCase {
        CaseworkCase {
            id: "c".into(),
            questions,
            ..CaseworkCase::default()
        }
    }

    /// The whole point of the log: an item the other desk added commutes with local work on
    /// other items, an item the local desk removed since the base does not come back, and a
    /// tampered log proves nothing.
    #[test]
    fn only_untouched_additive_operations_commute_and_a_broken_chain_contests_everything() {
        let base = case(vec![question("q1", "kept"), question("q2", "to drop")]);
        let theirs = case(vec![
            question("q1", "kept"),
            question("q2", "their edit of q2"),
            question("q3", "their new question"),
        ]);
        let mine = case(vec![question("q1", "my edit of q1")]);
        let their_log = chain(
            operations_between(Some(&base), &theirs),
            2,
            "Partner",
            "2026-10-01T00:00:00Z",
            None,
        );
        let my_log = chain(
            operations_between(Some(&base), &mine),
            2,
            "Me",
            "2026-10-01T00:00:00Z",
            None,
        );
        assert!(chain_is_intact(&their_log));

        let (commuting, contested) = partition_operations(&their_log, &theirs, &mine, &my_log);
        assert_eq!(
            commuting
                .iter()
                .map(|o| o.target.as_str())
                .collect::<Vec<_>>(),
            vec!["q3"]
        );
        assert_eq!(
            contested
                .iter()
                .map(|o| o.target.as_str())
                .collect::<Vec<_>>(),
            vec!["q2"],
            "an item removed locally since the base is never re-added by the other desk's edit"
        );

        let mut merged = mine.clone();
        apply_operations(&mut merged, &theirs, &commuting);
        assert_eq!(
            merged
                .questions
                .iter()
                .map(|q| q.id.as_str())
                .collect::<Vec<_>>(),
            vec!["q1", "q3"]
        );
        assert_eq!(merged.questions[0].question, "my edit of q1");

        let mut forged = their_log.clone();
        forged[1].author = "Someone else".into();
        assert!(!chain_is_intact(&forged));
        let (commuting, contested) = partition_operations(&forged, &theirs, &mine, &my_log);
        assert!(commuting.is_empty());
        assert_eq!(contested.len(), 2);
    }
}
