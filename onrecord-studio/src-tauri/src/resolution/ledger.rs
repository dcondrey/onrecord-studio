//! The ledger file: an append-only hash chain of judgments, one JSON document per device.
//!
//! Entries are never edited or removed; a later entry for the same pair supersedes an earlier one
//! at replay time and the earlier one stays in the file. Every link commits to the one before it,
//! so an edited, dropped or reordered entry makes the whole file fail to load. A file that cannot
//! be read or verified is an error and is never replaced by an empty ledger on the next write.

use super::{invalid, ordered_pair, EvidenceSnapshot, ResolutionKey, Verdict};
use crate::atomic_write::write_atomic;
use crate::collaboration::validate_key_id;
use crate::error::AppError;
use crate::integrity::sha256_hex;
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use uuid::Uuid;

pub const LEDGER_FILE_NAME: &str = "resolution_ledger.json";
pub const LEDGER_VERSION: u32 = 1;
pub const PACKAGE_SCHEMA: &str = "onrecord-resolution-ledger-v1";
pub const GENESIS_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";
pub const MAXIMUM_REASON_CHARACTERS: usize = 1000;
pub const MAXIMUM_AUTHOR_CHARACTERS: usize = 120;
/// Resource guards, not policy: a ledger past these is refused rather than loaded unbounded.
pub const MAXIMUM_LEDGER_ENTRIES: usize = 100_000;
const MAXIMUM_FILE_BYTES: u64 = 256 * 1024 * 1024;
const MAXIMUM_DISPLAY_CHARACTERS: usize = 500;
const MAXIMUM_EVIDENCE_LIST: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LedgerEntry {
    pub id: String,
    pub left: ResolutionKey,
    pub right: ResolutionKey,
    pub verdict: Verdict,
    pub author: String,
    /// RFC 3339 UTC with milliseconds, so string order is time order.
    pub decided_at: String,
    pub reason: String,
    /// The entry this one deliberately replaces (an undo or a changed mind); history stays.
    pub supersedes: Option<String>,
    pub evidence: EvidenceSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LedgerLink {
    pub entry: LedgerEntry,
    /// Key id of the signer when this entry arrived by import; absent for a local judgment.
    pub imported_from: Option<String>,
    pub prev_hash: String,
    pub hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LedgerDocument {
    pub version: u32,
    pub links: Vec<LedgerLink>,
}

impl Default for LedgerDocument {
    fn default() -> Self {
        Self {
            version: LEDGER_VERSION,
            links: Vec::new(),
        }
    }
}

impl LedgerDocument {
    pub fn head(&self) -> &str {
        self.links
            .last()
            .map_or(GENESIS_HASH, |link| link.hash.as_str())
    }

    pub fn entries(&self) -> Vec<LedgerEntry> {
        self.links.iter().map(|link| link.entry.clone()).collect()
    }
}

fn link_hash(
    prev_hash: &str,
    entry: &LedgerEntry,
    imported_from: Option<&str>,
) -> Result<String, AppError> {
    let mut bytes = prev_hash.as_bytes().to_vec();
    bytes.push(0);
    bytes.extend(
        serde_json::to_vec(entry).map_err(|error| {
            AppError::Storage(format!("Failed to encode a ledger entry: {error}"))
        })?,
    );
    match imported_from {
        Some(signer) => {
            bytes.push(1);
            bytes.extend(signer.as_bytes());
        }
        None => bytes.push(0),
    }
    Ok(sha256_hex(&bytes))
}

fn canonical_time(value: &str) -> bool {
    DateTime::parse_from_rfc3339(value).is_ok_and(|parsed| {
        parsed
            .with_timezone(&Utc)
            .to_rfc3339_opts(SecondsFormat::Millis, true)
            == value
    })
}

fn bounded(field: &str, value: &str, maximum: usize, multiline: bool) -> Result<(), AppError> {
    if value
        .chars()
        .any(|c| c.is_control() && !(multiline && matches!(c, '\n' | '\t')))
    {
        return Err(invalid(format!(
            "{field} must not contain control characters"
        )));
    }
    if value.chars().count() > maximum {
        return Err(invalid(format!(
            "{field} must be at most {maximum} characters"
        )));
    }
    Ok(())
}

fn validate_snapshot(snapshot: &super::EntitySnapshot) -> Result<(), AppError> {
    bounded(
        "a display value",
        &snapshot.display_value,
        MAXIMUM_DISPLAY_CHARACTERS,
        false,
    )?;
    bounded(
        "an entity kind",
        &snapshot.key.kind,
        MAXIMUM_DISPLAY_CHARACTERS,
        false,
    )?;
    bounded(
        "an entity value",
        &snapshot.key.value,
        MAXIMUM_DISPLAY_CHARACTERS,
        false,
    )?;
    if snapshot.examples.len() > MAXIMUM_EVIDENCE_LIST {
        return Err(invalid("too many example mentions in the evidence"));
    }
    Ok(())
}

/// Shape and bounds of one entry. Applied to local judgments and to every imported entry.
pub fn validate_entry(entry: &LedgerEntry) -> Result<(), AppError> {
    let plain_id = |id: &str| {
        !id.is_empty()
            && id.len() <= 64
            && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    };
    if !plain_id(&entry.id) || entry.supersedes.as_deref().is_some_and(|id| !plain_id(id)) {
        return Err(invalid(
            "an entry id is up to 64 letters, digits or hyphens",
        ));
    }
    if entry.left >= entry.right {
        return Err(invalid(
            "an entry names two different entities in canonical order",
        ));
    }
    for key in [&entry.left, &entry.right] {
        bounded(
            "an entity kind",
            &key.kind,
            MAXIMUM_DISPLAY_CHARACTERS,
            false,
        )?;
        bounded(
            "an entity value",
            &key.value,
            MAXIMUM_DISPLAY_CHARACTERS,
            false,
        )?;
        if key.kind.is_empty() || key.value.is_empty() {
            return Err(invalid("an entity key needs a kind and a value"));
        }
    }
    bounded(
        "the reviewer name",
        &entry.author,
        MAXIMUM_AUTHOR_CHARACTERS,
        false,
    )?;
    if entry.author.trim().is_empty() {
        return Err(invalid("say who made this judgment"));
    }
    bounded("the reason", &entry.reason, MAXIMUM_REASON_CHARACTERS, true)?;
    if !canonical_time(&entry.decided_at) {
        return Err(invalid("a judgment time is UTC with milliseconds"));
    }
    validate_snapshot(&entry.evidence.left)?;
    validate_snapshot(&entry.evidence.right)?;
    if entry.evidence.name_variants.len() > MAXIMUM_EVIDENCE_LIST {
        return Err(invalid("too many name variants in the evidence"));
    }
    for variant in &entry.evidence.name_variants {
        bounded("a name variant", variant, MAXIMUM_DISPLAY_CHARACTERS, false)?;
    }
    if let Some(shared) = &entry.evidence.shared_identifier {
        bounded(
            "an identifier",
            &shared.display_value,
            MAXIMUM_DISPLAY_CHARACTERS,
            false,
        )?;
    }
    Ok(())
}

/// Recomputes the whole chain. Any mismatch is an error naming the first bad link.
pub fn verify_document(document: &LedgerDocument) -> Result<(), AppError> {
    if document.version != LEDGER_VERSION {
        return Err(AppError::Storage(format!(
            "The resolution ledger is version {}, this build reads version {LEDGER_VERSION}. It was not modified.",
            document.version
        )));
    }
    if document.links.len() > MAXIMUM_LEDGER_ENTRIES {
        return Err(AppError::Storage(format!(
            "The resolution ledger holds more than {MAXIMUM_LEDGER_ENTRIES} entries. It was not modified."
        )));
    }
    let mut previous = GENESIS_HASH.to_string();
    let mut ids = BTreeSet::new();
    for (position, link) in document.links.iter().enumerate() {
        let broken = |why: &str| {
            AppError::Storage(format!(
                "The resolution ledger failed verification at entry {} ({why}). It was not modified.",
                position + 1
            ))
        };
        validate_entry(&link.entry).map_err(|_| broken("entry is malformed"))?;
        if let Some(signer) = &link.imported_from {
            if validate_key_id(signer).is_err() {
                return Err(broken("signer is malformed"));
            }
        }
        if !ids.insert(link.entry.id.as_str()) {
            return Err(broken("duplicate entry id"));
        }
        if link.prev_hash != previous {
            return Err(broken("chain link does not follow the previous entry"));
        }
        if link_hash(&previous, &link.entry, link.imported_from.as_deref())? != link.hash {
            return Err(broken("entry does not match its recorded hash"));
        }
        previous = link.hash.clone();
    }
    Ok(())
}

fn append(
    document: &mut LedgerDocument,
    entry: LedgerEntry,
    imported_from: Option<String>,
) -> Result<(), AppError> {
    let prev_hash = document.head().to_string();
    let hash = link_hash(&prev_hash, &entry, imported_from.as_deref())?;
    document.links.push(LedgerLink {
        entry,
        imported_from,
        prev_hash,
        hash,
    });
    Ok(())
}

/// Strictly after every existing entry, so a judgment made right after another always sorts last
/// (an undo within the same millisecond would otherwise tie and be ordered by random id).
fn next_decision_time(document: &LedgerDocument, now: DateTime<Utc>) -> String {
    let latest = document
        .links
        .iter()
        .filter_map(|link| DateTime::parse_from_rfc3339(&link.entry.decided_at).ok())
        .map(|time| time.with_timezone(&Utc))
        .max();
    let time = match latest {
        Some(latest) if latest >= now => latest + Duration::milliseconds(1),
        _ => now,
    };
    time.to_rfc3339_opts(SecondsFormat::Millis, true)
}

#[derive(Debug, Clone)]
pub struct NewJudgment {
    pub left: ResolutionKey,
    pub right: ResolutionKey,
    pub verdict: Verdict,
    pub author: String,
    pub reason: String,
    pub supersedes: Option<String>,
    pub evidence: EvidenceSnapshot,
}

pub fn record_judgment(
    document: &mut LedgerDocument,
    judgment: NewJudgment,
    now: DateTime<Utc>,
) -> Result<LedgerLink, AppError> {
    if document.links.len() >= MAXIMUM_LEDGER_ENTRIES {
        return Err(invalid(format!(
            "the ledger holds {MAXIMUM_LEDGER_ENTRIES} entries, its limit"
        )));
    }
    let (left, right) = ordered_pair(judgment.left, judgment.right)?;
    if let Some(target) = &judgment.supersedes {
        let existing = document
            .links
            .iter()
            .find(|link| &link.entry.id == target)
            .ok_or_else(|| invalid("the entry being superseded is not in this ledger"))?;
        if existing.entry.left != left || existing.entry.right != right {
            return Err(invalid("a superseding entry must judge the same pair"));
        }
    }
    let entry = LedgerEntry {
        id: Uuid::new_v4().to_string(),
        left,
        right,
        verdict: judgment.verdict,
        author: judgment.author.trim().to_string(),
        decided_at: next_decision_time(document, now),
        reason: judgment.reason.trim().to_string(),
        supersedes: judgment.supersedes,
        evidence: judgment.evidence,
    };
    validate_entry(&entry)?;
    append(document, entry, None)?;
    Ok(document.links.last().expect("just appended").clone())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportConflict {
    pub entry_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportPlan {
    pub additions: Vec<LedgerEntry>,
    pub already_present: usize,
    pub conflicts: Vec<ImportConflict>,
}

/// Matches incoming entries to the local ledger by id. Identical content is already present; the
/// same id with different content is a conflict and the local entry is kept untouched.
pub fn plan_import(
    document: &LedgerDocument,
    incoming: &[LedgerEntry],
) -> Result<ImportPlan, AppError> {
    let local: BTreeMap<&str, &LedgerEntry> = document
        .links
        .iter()
        .map(|link| (link.entry.id.as_str(), &link.entry))
        .collect();
    let mut seen = BTreeSet::new();
    let mut plan = ImportPlan {
        additions: Vec::new(),
        already_present: 0,
        conflicts: Vec::new(),
    };
    for entry in incoming {
        validate_entry(entry)?;
        if !seen.insert(entry.id.as_str()) {
            return Err(invalid("the package repeats an entry id"));
        }
        match local.get(entry.id.as_str()) {
            Some(held) if *held == entry => plan.already_present += 1,
            Some(_) => plan.conflicts.push(ImportConflict {
                entry_id: entry.id.clone(),
                reason: "an entry with this id already exists here with different content; the local one was kept".to_string(),
            }),
            None => plan.additions.push(entry.clone()),
        }
    }
    plan.additions
        .sort_by(|a, b| (&a.decided_at, &a.id).cmp(&(&b.decided_at, &b.id)));
    Ok(plan)
}

pub fn apply_import(
    document: &mut LedgerDocument,
    plan: &ImportPlan,
    signer_key_id: &str,
) -> Result<(), AppError> {
    if document.links.len() + plan.additions.len() > MAXIMUM_LEDGER_ENTRIES {
        return Err(invalid(format!(
            "importing would pass the {MAXIMUM_LEDGER_ENTRIES}-entry limit"
        )));
    }
    for entry in &plan.additions {
        append(document, entry.clone(), Some(signer_key_id.to_string()))?;
    }
    Ok(())
}

/// The portable, signed form: entries only. The chain is local to a device and is rebuilt on import.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolutionPackage {
    pub schema: String,
    pub exported_at: String,
    pub entries: Vec<LedgerEntry>,
}

pub struct LedgerStore {
    path: PathBuf,
    gate: Mutex<()>,
}

impl LedgerStore {
    pub fn new(directory: &Path) -> Self {
        Self {
            path: directory.join(LEDGER_FILE_NAME),
            gate: Mutex::new(()),
        }
    }

    pub fn load(&self) -> Result<LedgerDocument, AppError> {
        let _guard = self.lock()?;
        self.read()
    }

    /// Read, change, verify and write under one lock; an error from `change` writes nothing.
    pub fn update<T>(
        &self,
        change: impl FnOnce(&mut LedgerDocument) -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        let _guard = self.lock()?;
        let mut document = self.read()?;
        let outcome = change(&mut document)?;
        verify_document(&document)?;
        let bytes = serde_json::to_vec(&document).map_err(|error| {
            AppError::Storage(format!("Failed to encode the resolution ledger: {error}"))
        })?;
        write_atomic(&self.path, "the resolution ledger", &bytes)?;
        Ok(outcome)
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, ()>, AppError> {
        self.gate
            .lock()
            .map_err(|_| AppError::Internal("The resolution ledger lock was poisoned".to_string()))
    }

    /// A missing file is an empty ledger. Anything unreadable, over-size, of another version or
    /// failing chain verification is an error: reading it as empty would let the next judgment
    /// overwrite every earlier one.
    fn read(&self) -> Result<LedgerDocument, AppError> {
        let file = match fs::File::open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(LedgerDocument::default())
            }
            Err(error) => {
                return Err(AppError::Storage(format!(
                    "Failed to read the resolution ledger: {error}"
                )))
            }
        };
        let mut bytes = Vec::new();
        file.take(MAXIMUM_FILE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| {
                AppError::Storage(format!("Failed to read the resolution ledger: {error}"))
            })?;
        if bytes.len() as u64 > MAXIMUM_FILE_BYTES {
            return Err(AppError::Storage(
                "The resolution ledger file is larger than its size limit. It was not modified."
                    .to_string(),
            ));
        }
        let document: LedgerDocument = serde_json::from_slice(&bytes).map_err(|error| {
            AppError::Storage(format!(
                "The resolution ledger is not valid ({error}). It was not modified."
            ))
        })?;
        verify_document(&document)?;
        Ok(document)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resolution::replay::tests::{entry, key};
    use crate::resolution::{EntitySnapshot, SuggestionBasis};

    fn judgment(left: &str, right: &str, verdict: Verdict) -> NewJudgment {
        let snapshot = |name: &str| EntitySnapshot {
            key: key(name),
            display_value: name.to_string(),
            document_count: 2,
            examples: Vec::new(),
        };
        NewJudgment {
            left: key(left),
            right: key(right),
            verdict,
            author: "Maria".to_string(),
            reason: "same signature block".to_string(),
            supersedes: None,
            evidence: EvidenceSnapshot {
                basis: SuggestionBasis::NameOnly,
                left: snapshot(left.min(right)),
                right: snapshot(left.max(right)),
                shared_identifier: None,
                name_variants: Vec::new(),
            },
        }
    }

    fn built(count: usize) -> LedgerDocument {
        let mut document = LedgerDocument::default();
        let now = Utc::now();
        for index in 0..count {
            record_judgment(
                &mut document,
                judgment(&format!("a{index}"), "z", Verdict::Same),
                now,
            )
            .unwrap();
        }
        document
    }

    #[test]
    fn the_chain_detects_an_edit_a_removal_and_a_reordering() {
        let document = built(3);
        verify_document(&document).unwrap();

        let mut edited = document.clone();
        edited.links[1].entry.verdict = Verdict::Different;
        assert!(verify_document(&edited).is_err());

        let mut removed = document.clone();
        removed.links.remove(1);
        assert!(verify_document(&removed).is_err());

        let mut reordered = document.clone();
        reordered.links.swap(0, 1);
        assert!(verify_document(&reordered).is_err());

        let mut truncated_tail = document.clone();
        truncated_tail.links.pop();
        verify_document(&truncated_tail).unwrap();
        assert_ne!(truncated_tail.head(), document.head());
    }

    #[test]
    fn a_corrupt_or_other_version_file_is_an_error_and_is_never_overwritten() {
        let directory = tempfile::tempdir().unwrap();
        let store = LedgerStore::new(directory.path());
        assert!(store.load().unwrap().links.is_empty());
        store
            .update(|doc| record_judgment(doc, judgment("a", "b", Verdict::Same), Utc::now()))
            .unwrap();
        let path = directory.path().join(LEDGER_FILE_NAME);
        let good = std::fs::read(&path).unwrap();

        let mut other: LedgerDocument = serde_json::from_slice(&good).unwrap();
        other.version = 2;
        for damaged in [b"{ no".to_vec(), serde_json::to_vec(&other).unwrap()] {
            std::fs::write(&path, &damaged).unwrap();
            assert!(store.load().is_err());
            assert!(store
                .update(|doc| record_judgment(doc, judgment("c", "d", Verdict::Same), Utc::now()))
                .is_err());
            assert_eq!(std::fs::read(&path).unwrap(), damaged);
        }
    }

    #[test]
    fn a_judgment_made_right_after_another_sorts_after_it_so_an_undo_wins() {
        let mut document = LedgerDocument::default();
        let now = Utc::now();
        let first = record_judgment(&mut document, judgment("a", "b", Verdict::Same), now).unwrap();
        let mut undo = judgment("b", "a", Verdict::Unsure);
        undo.supersedes = Some(first.entry.id.clone());
        let second = record_judgment(&mut document, undo, now - Duration::seconds(5)).unwrap();
        assert!(second.entry.decided_at > first.entry.decided_at);
        let mut wrong_pair = judgment("a", "c", Verdict::Unsure);
        wrong_pair.supersedes = Some(first.entry.id);
        assert!(record_judgment(&mut document, wrong_pair, now).is_err());
        assert!(record_judgment(&mut document, judgment("a", "a", Verdict::Same), now).is_err());
    }

    #[test]
    fn importing_adds_new_entries_and_never_overwrites_a_local_one() {
        let signer = "ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12";
        let mut local = LedgerDocument::default();
        append(
            &mut local,
            entry("same-id", "01.000", "a", "b", Verdict::Same),
            None,
        )
        .unwrap();
        let before = local.clone();

        let mut changed = entry("same-id", "01.000", "a", "b", Verdict::Different);
        changed.reason = "tampered".to_string();
        let incoming = vec![
            entry("same-id", "01.000", "a", "b", Verdict::Same),
            changed.clone(),
        ];
        assert!(
            plan_import(&local, &incoming).is_err(),
            "a package repeating an id is refused"
        );

        let incoming = vec![
            changed,
            entry("new-id", "02.000", "c", "d", Verdict::Different),
        ];
        let plan = plan_import(&local, &incoming).unwrap();
        assert_eq!(plan.conflicts.len(), 1);
        assert_eq!(plan.additions.len(), 1);
        apply_import(&mut local, &plan, signer).unwrap();
        verify_document(&local).unwrap();
        assert_eq!(local.links[0], before.links[0]);
        assert_eq!(local.links.len(), 2);
        assert_eq!(local.links[1].imported_from.as_deref(), Some(signer));

        let again = plan_import(
            &local,
            &[entry("new-id", "02.000", "c", "d", Verdict::Different)],
        )
        .unwrap();
        assert_eq!((again.already_present, again.additions.len()), (1, 0));
    }
}
