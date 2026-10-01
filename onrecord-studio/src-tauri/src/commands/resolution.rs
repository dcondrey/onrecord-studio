//! IPC commands for entity resolution: candidate pairs, the judgment ledger, clusters, and the
//! signed ledger package. Nothing here changes the corpus; a cluster is a reading of the ledger.

use crate::{
    blocking,
    error::AppError,
    integrity::{
        check_revocation, load_or_create_signing_key, load_revocations, sha256_hex, sign_package,
        verify_package_detailed, SignedPackage,
    },
    resolution::{
        self, apply_import, effective_entries, links_within_fanout, pair_lead, plan_import,
        rank_candidates, replay, CandidateGuards, Contradiction, EntitySnapshot, EvidenceSnapshot,
        IdentifierLink, ImportConflict, LedgerEntry, LedgerStore, NewJudgment, PairLead,
        PresentEntities, ResolutionEntity, ResolutionKey, ResolutionPackage, SuggestionBasis,
        Verdict, MAXIMUM_PAGE,
    },
    storage::{corpus::CorpusIndex, sqlite::DuckDbRepository},
};
use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::State;

/// The transport limit shared with case packets; checked before an untrusted payload is parsed.
const MAXIMUM_PACKAGE_BYTES: usize = 8 * 1024 * 1024;
const DEFAULT_PAGE: usize = 10;
const DEFAULT_LEDGER_PAGE: usize = 25;

async fn on_ledger<T, F>(
    store: &Arc<LedgerStore>,
    task: &'static str,
    operation: F,
) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce(&LedgerStore) -> Result<T, AppError> + Send + 'static,
{
    let store = Arc::clone(store);
    blocking::spawn(task, move || operation(&store)).await?
}

fn page_size(limit: Option<usize>, default: usize) -> Result<usize, AppError> {
    let limit = limit.unwrap_or(default);
    if limit == 0 || limit > MAXIMUM_PAGE {
        return Err(resolution::invalid(format!(
            "a page is between 1 and {MAXIMUM_PAGE} items"
        )));
    }
    Ok(limit)
}

fn snapshot(
    entity: &ResolutionEntity,
    examples: &BTreeMap<String, Vec<resolution::MentionExample>>,
) -> EntitySnapshot {
    EntitySnapshot {
        key: entity.key.clone(),
        display_value: entity.display_value.clone(),
        document_count: entity.document_count,
        examples: examples.get(&entity.entity_id).cloned().unwrap_or_default(),
    }
}

fn evidence_for(
    lead: &PairLead,
    examples: &BTreeMap<String, Vec<resolution::MentionExample>>,
) -> EvidenceSnapshot {
    let mut variants = vec![
        lead.left.display_value.clone(),
        lead.right.display_value.clone(),
    ];
    variants.sort();
    variants.dedup();
    EvidenceSnapshot {
        basis: lead.basis,
        left: snapshot(&lead.left, examples),
        right: snapshot(&lead.right, examples),
        shared_identifier: lead.shared_identifier.clone(),
        name_variants: variants,
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolutionCandidate {
    pub pair_id: String,
    pub left_entity_id: String,
    pub right_entity_id: String,
    pub evidence: EvidenceSnapshot,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolutionCandidatePage {
    pub candidates: Vec<ResolutionCandidate>,
    pub next_cursor: Option<String>,
    /// Candidates from this page's start onward.
    pub remaining: u64,
    /// The entity scan hit its resource guard; candidates beyond it were not considered.
    pub scan_truncated: bool,
    pub guards: CandidateGuards,
    pub ledger_head: String,
}

/// Pairs worth a human look, strongest evidence first. `include_unsure` brings back pairs the
/// reviewer previously marked unsure; every other judged pair stays out.
#[tauri::command]
pub async fn list_resolution_candidates(
    cursor: Option<String>,
    limit: Option<usize>,
    include_unsure: Option<bool>,
    index: State<'_, CorpusIndex>,
    store: State<'_, Arc<LedgerStore>>,
) -> Result<ResolutionCandidatePage, AppError> {
    let limit = page_size(limit, DEFAULT_PAGE)?;
    let document = on_ledger(&store, "Resolution ledger read", LedgerStore::load).await?;
    let ledger_head = document.head().to_string();
    let settled = replay(&document.entries(), &PresentEntities::new());
    let scan = index.resolution_scan().await?;
    let (leads, guards) = rank_candidates(
        &scan.entities,
        &scan.links,
        &settled,
        include_unsure.unwrap_or(false),
    );
    let page = resolution::page_after(leads, cursor.as_deref(), limit)?;
    let ids: Vec<String> = page
        .leads
        .iter()
        .flat_map(|lead| [lead.left.entity_id.clone(), lead.right.entity_id.clone()])
        .collect();
    let examples = index.resolution_examples(ids).await?;
    let candidates = page
        .leads
        .iter()
        .map(|lead| ResolutionCandidate {
            pair_id: lead.token(),
            left_entity_id: lead.left.entity_id.clone(),
            right_entity_id: lead.right.entity_id.clone(),
            evidence: evidence_for(lead, &examples),
        })
        .collect();
    Ok(ResolutionCandidatePage {
        candidates,
        next_cursor: page.next_cursor,
        remaining: page.remaining as u64,
        scan_truncated: scan.scan_truncated,
        guards,
        ledger_head,
    })
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordResolutionInput {
    pub left_entity_id: String,
    pub right_entity_id: String,
    pub verdict: Verdict,
    pub author: String,
    pub reason: String,
    /// The entry this one replaces, for an undo or a change of mind.
    pub supersedes: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolutionLedgerItem {
    pub entry: LedgerEntry,
    pub imported_from: Option<String>,
    pub hash: String,
    /// The entry that currently decides its pair; earlier ones are history.
    pub is_latest: bool,
}

/// The pair's identifier links, minus any identifier the candidate queue would have skipped for
/// fanout. Links for just two entities cannot see the fanout, so the identifiers they share are
/// re-read across every entity they link before the queue's own cut is applied.
async fn fanout_checked_links(
    index: &CorpusIndex,
    pair: &[String],
) -> Result<Vec<IdentifierLink>, AppError> {
    let own = index.resolution_links_for(pair.to_vec()).await?;
    let shared: Vec<String> = own
        .iter()
        .filter(|link| link.entity_id == pair[0])
        .filter(|link| {
            own.iter().any(|other| {
                other.entity_id == pair[1] && other.identifier_id == link.identifier_id
            })
        })
        .map(|link| link.identifier_id.clone())
        .collect();
    let all = index.resolution_links_of_identifiers(shared).await?;
    let linked = index
        .resolution_entities_by_id(all.iter().map(|link| link.entity_id.clone()).collect())
        .await?;
    let known = linked.iter().map(|e| (e.entity_id.as_str(), e)).collect();
    let (within, _) = links_within_fanout(&all, &known);
    Ok(within
        .into_values()
        .flatten()
        .filter(|link| pair.contains(&link.entity_id))
        .cloned()
        .collect())
}

/// Records one judgment. The evidence snapshot is rebuilt here from the corpus, never taken from
/// the caller, so an entry can only claim what the index showed.
#[tauri::command]
pub async fn record_resolution_judgment(
    input: RecordResolutionInput,
    index: State<'_, CorpusIndex>,
    store: State<'_, Arc<LedgerStore>>,
) -> Result<ResolutionLedgerItem, AppError> {
    let ids = vec![input.left_entity_id.clone(), input.right_entity_id.clone()];
    let entities = index.resolution_entities_by_id(ids.clone()).await?;
    let find = |id: &str| {
        entities
            .iter()
            .find(|entity| entity.entity_id == id)
            .ok_or_else(|| {
                resolution::invalid(
                    "one of these entities is no longer in the index; reload the queue",
                )
            })
    };
    let (left, right) = (find(&input.left_entity_id)?, find(&input.right_entity_id)?);
    let links = fanout_checked_links(&index, &ids).await?;
    let examples = index.resolution_examples(ids).await?;
    let lead = pair_lead(left, right, &links)?;
    let evidence = evidence_for(&lead, &examples);
    let judgment = NewJudgment {
        left: lead.left.key.clone(),
        right: lead.right.key.clone(),
        verdict: input.verdict,
        author: input.author,
        reason: input.reason,
        supersedes: input.supersedes,
        evidence,
    };
    on_ledger(&store, "Resolution ledger write", move |store| {
        let link =
            store.update(|document| resolution::record_judgment(document, judgment, Utc::now()))?;
        Ok(ResolutionLedgerItem {
            entry: link.entry,
            imported_from: link.imported_from,
            hash: link.hash,
            is_latest: true,
        })
    })
    .await
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BasisDecisions {
    pub basis: SuggestionBasis,
    pub same: u64,
    pub different: u64,
    pub unsure: u64,
}

/// Counts of this device's own current judgments, by what prompted them. A count of what the
/// reviewer decided, not a measure of how right either they or the suggestions were.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DecisionSummary {
    pub by_basis: Vec<BasisDecisions>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolutionLedgerPage {
    pub items: Vec<ResolutionLedgerItem>,
    pub next_cursor: Option<String>,
    /// Entries matching the filter, this page included.
    pub total: u64,
    pub entry_count: u64,
    pub head: String,
    pub summary: DecisionSummary,
    pub contradictions: Vec<Contradiction>,
}

fn summarize(document: &resolution::LedgerDocument) -> DecisionSummary {
    let entries = document.entries();
    let latest = effective_entries(&entries);
    let mut counts: BTreeMap<u8, BasisDecisions> = BTreeMap::new();
    for link in document
        .links
        .iter()
        .filter(|link| link.imported_from.is_none())
    {
        if latest
            .get(&(link.entry.left.clone(), link.entry.right.clone()))
            .map(|e| &e.id)
            != Some(&link.entry.id)
        {
            continue;
        }
        let basis = link.entry.evidence.basis;
        let slot = counts.entry(basis as u8).or_insert(BasisDecisions {
            basis,
            same: 0,
            different: 0,
            unsure: 0,
        });
        match link.entry.verdict {
            Verdict::Same => slot.same += 1,
            Verdict::Different => slot.different += 1,
            Verdict::Unsure => slot.unsure += 1,
        }
    }
    DecisionSummary {
        by_basis: counts.into_values().collect(),
    }
}

/// The ledger newest first, with every superseded entry still present. `pair` narrows it to the
/// history of one pair (a `pairId` from a candidate or an entry).
#[tauri::command]
pub async fn list_resolution_ledger(
    cursor: Option<String>,
    limit: Option<usize>,
    pair: Option<String>,
    store: State<'_, Arc<LedgerStore>>,
) -> Result<ResolutionLedgerPage, AppError> {
    let limit = page_size(limit, DEFAULT_LEDGER_PAGE)?;
    let after = cursor
        .map(|cursor| {
            cursor
                .split_once('|')
                .map(|(time, id)| (time.to_string(), id.to_string()))
                .ok_or_else(|| resolution::invalid("the page cursor is not valid"))
        })
        .transpose()?;
    on_ledger(&store, "Resolution ledger read", move |store| {
        let document = store.load()?;
        let entries = document.entries();
        let current = replay(&entries, &PresentEntities::new());
        let mut matching: Vec<_> = document
            .links
            .iter()
            .filter(|link| {
                pair.as_ref().is_none_or(|pair| {
                    format!("{}||{}", link.entry.left.token(), link.entry.right.token()) == *pair
                })
            })
            .collect();
        matching.sort_by(|a, b| {
            (&b.entry.decided_at, &b.entry.id).cmp(&(&a.entry.decided_at, &a.entry.id))
        });
        let rest: Vec<_> = matching
            .into_iter()
            .filter(|link| {
                after
                    .as_ref()
                    .is_none_or(|(time, id)| (&link.entry.decided_at, &link.entry.id) < (time, id))
            })
            .collect();
        let total = rest.len() as u64;
        let taken: Vec<_> = rest.iter().take(limit).collect();
        let next_cursor = (rest.len() > taken.len())
            .then(|| {
                taken
                    .last()
                    .map(|link| format!("{}|{}", link.entry.decided_at, link.entry.id))
            })
            .flatten();
        Ok(ResolutionLedgerPage {
            items: taken
                .iter()
                .map(|link| ResolutionLedgerItem {
                    entry: link.entry.clone(),
                    imported_from: link.imported_from.clone(),
                    hash: link.hash.clone(),
                    is_latest: current.is_latest(&link.entry),
                })
                .collect(),
            next_cursor,
            total,
            entry_count: document.links.len() as u64,
            head: document.head().to_string(),
            summary: summarize(&document),
            contradictions: current.contradictions.clone(),
        })
    })
    .await
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClusterMember {
    pub entity_id: String,
    pub key: ResolutionKey,
    pub display_value: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClusterReason {
    pub entry_id: String,
    pub left_display_value: String,
    pub right_display_value: String,
    pub author: String,
    pub decided_at: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolutionClusterView {
    pub entity_id: String,
    pub key: ResolutionKey,
    /// The other entities a reviewer judged to be the same as this one. Empty when none were.
    pub members: Vec<ClusterMember>,
    pub reasons: Vec<ClusterReason>,
    /// Unresolved contradictions that involve this entity; its cluster is left unmerged.
    pub contradictions: Vec<Contradiction>,
    /// Cites this reading: the ledger head together with the corpus state it was applied to.
    pub resolution_id: String,
    pub ledger_head: String,
    pub corpus_state_digest: String,
}

/// The cluster of an entity under the current ledger. `None` when the id names no entity, which a
/// rebuild causes; callers hold ids from before it and are told rather than shown an empty cluster.
#[tauri::command]
pub async fn get_resolution_clusters(
    entity_id: String,
    index: State<'_, CorpusIndex>,
    repository: State<'_, DuckDbRepository>,
    store: State<'_, Arc<LedgerStore>>,
) -> Result<Option<ResolutionClusterView>, AppError> {
    let Some(subject) = index
        .resolution_entities_by_id(vec![entity_id.clone()])
        .await?
        .into_iter()
        .next()
    else {
        return Ok(None);
    };
    let document = on_ledger(&store, "Resolution ledger read", LedgerStore::load).await?;
    let entries = document.entries();
    let mut keys: Vec<ResolutionKey> = entries
        .iter()
        .flat_map(|entry| [entry.left.clone(), entry.right.clone()])
        .collect();
    keys.sort();
    keys.dedup();
    let mut present = PresentEntities::new();
    for entity in index.resolution_entities_by_key(keys).await? {
        present
            .entry(entity.key)
            .or_default()
            .push(entity.entity_id);
    }
    let reading = replay(&entries, &present);
    let (corpus_state_digest, _) = repository.corpus_state_identity().await?;
    let cluster = reading.cluster_of(&subject.key);
    let others: Vec<String> = cluster
        .map(|cluster| {
            cluster
                .entity_ids
                .iter()
                .filter(|id| **id != subject.entity_id)
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    let members = index
        .resolution_entities_by_id(others)
        .await?
        .into_iter()
        .map(|entity| ClusterMember {
            entity_id: entity.entity_id,
            key: entity.key,
            display_value: entity.display_value,
        })
        .collect();
    let reasons = cluster
        .map(|cluster| {
            cluster
                .entry_ids
                .iter()
                .filter_map(|id| entries.iter().find(|entry| &entry.id == id))
                .map(|entry| ClusterReason {
                    entry_id: entry.id.clone(),
                    left_display_value: entry.evidence.left.display_value.clone(),
                    right_display_value: entry.evidence.right.display_value.clone(),
                    author: entry.author.clone(),
                    decided_at: entry.decided_at.clone(),
                    reason: entry.reason.clone(),
                })
                .collect()
        })
        .unwrap_or_default();
    let contradictions = reading
        .contradictions
        .iter()
        .filter(|contradiction| contradiction.keys.contains(&subject.key))
        .cloned()
        .collect();
    Ok(Some(ResolutionClusterView {
        entity_id: subject.entity_id,
        key: subject.key,
        members,
        reasons,
        contradictions,
        resolution_id: resolution::resolution_id(document.head(), &corpus_state_digest),
        ledger_head: document.head().to_string(),
        corpus_state_digest,
    }))
}

/// The whole ledger as a signed package. Entries keep their original author and time; the chain is
/// local and is rebuilt by whoever imports it.
#[tauri::command]
pub async fn export_resolution_ledger(
    store: State<'_, Arc<LedgerStore>>,
    key_directory: State<'_, crate::integrity::SigningKeyDirectory>,
) -> Result<String, AppError> {
    let document = on_ledger(&store, "Resolution ledger read", LedgerStore::load).await?;
    let package = ResolutionPackage {
        schema: resolution::PACKAGE_SCHEMA.to_string(),
        exported_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        entries: document.entries(),
    };
    let signing_key = load_or_create_signing_key(&key_directory.0).await?;
    let signed = sign_package(package, &signing_key).map_err(AppError::Export)?;
    let envelope = serde_json::to_string_pretty(&signed).map_err(|error| {
        AppError::Export(format!("Failed to encode the signed ledger: {error}"))
    })?;
    if envelope.len() > MAXIMUM_PACKAGE_BYTES {
        return Err(AppError::Export(format!(
            "The signed ledger exceeds the {MAXIMUM_PACKAGE_BYTES}-byte transport limit."
        )));
    }
    Ok(envelope)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolutionImportPreview {
    pub signer_key_id: String,
    /// Binds the import to exactly the bytes that were previewed.
    pub packet_sha256: String,
    pub exported_at: String,
    pub entry_count: u64,
    pub additions: u64,
    pub already_present: u64,
    pub conflicts: Vec<ImportConflict>,
    pub contradictions_before: u64,
    pub contradictions_after: u64,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolutionImportResult {
    pub added: u64,
    pub already_present: u64,
    pub conflicts: Vec<ImportConflict>,
    pub head: String,
}

/// Mirrors the case-packet check: the signature must verify, and a key this installation marked
/// compromised is refused. A valid signature proves the file is unchanged, not who holds the key.
async fn verify_package(
    payload: &str,
    report_directory: &std::path::Path,
) -> Result<(ResolutionPackage, String), AppError> {
    if payload.len() > MAXIMUM_PACKAGE_BYTES {
        return Err(resolution::invalid(format!(
            "this package exceeds the {MAXIMUM_PACKAGE_BYTES}-byte transport limit"
        )));
    }
    let package: SignedPackage<ResolutionPackage> =
        serde_json::from_str(payload).map_err(|error| {
            resolution::invalid(format!("this is not a signed resolution ledger ({error})"))
        })?;
    let checks = verify_package_detailed(&package);
    if !checks.all_passed() {
        return Err(resolution::invalid(format!(
            "the signature does not verify; the bytes changed since signing. It was not previewed or imported. {}",
            checks.detail.unwrap_or_default()
        )));
    }
    if package.manifest.schema != resolution::PACKAGE_SCHEMA {
        return Err(resolution::invalid(
            "this package is not a resolution ledger of a known version",
        ));
    }
    let signer = package.signature.signer_key_id.clone();
    let revocations = load_revocations(report_directory).await?;
    if let Some(revoked) = check_revocation(&signer, &revocations) {
        if revoked.status != crate::integrity::REVOCATION_STATUS_RETIRED {
            return Err(resolution::invalid(format!(
                "this package was signed by key {signer}, which this installation revoked as compromised on {}. It was not previewed or imported.",
                revoked.revoked_at
            )));
        }
    }
    Ok((package.manifest, signer))
}

#[tauri::command]
pub async fn preview_resolution_import(
    payload: String,
    store: State<'_, Arc<LedgerStore>>,
    report_directory: State<'_, PathBuf>,
) -> Result<ResolutionImportPreview, AppError> {
    let (package, signer) = verify_package(&payload, &report_directory).await?;
    let packet_sha256 = sha256_hex(payload.as_bytes());
    on_ledger(&store, "Resolution import preview", move |store| {
        let document = store.load()?;
        let plan = plan_import(&document, &package.entries)?;
        let before = replay(&document.entries(), &PresentEntities::new());
        let mut merged = document.clone();
        apply_import(&mut merged, &plan, &signer)?;
        let after = replay(&merged.entries(), &PresentEntities::new());
        Ok(ResolutionImportPreview {
            signer_key_id: signer,
            packet_sha256,
            exported_at: package.exported_at,
            entry_count: package.entries.len() as u64,
            additions: plan.additions.len() as u64,
            already_present: plan.already_present as u64,
            conflicts: plan.conflicts,
            contradictions_before: before.contradictions.len() as u64,
            contradictions_after: after.contradictions.len() as u64,
            warnings: vec![
                "Signature verified: the file has not changed since that key signed it. That proves origin and integrity, not who holds the key.".to_string(),
            ],
        })
    })
    .await
}

/// Adds the entries this ledger does not have. An entry whose id is already here is never
/// replaced, even when the content differs; it is reported as a conflict instead.
#[tauri::command]
pub async fn import_resolution_ledger(
    payload: String,
    packet_sha256: String,
    store: State<'_, Arc<LedgerStore>>,
    report_directory: State<'_, PathBuf>,
) -> Result<ResolutionImportResult, AppError> {
    if sha256_hex(payload.as_bytes()) != packet_sha256 {
        return Err(resolution::invalid(
            "the package changed since it was previewed; preview it again",
        ));
    }
    let (package, signer) = verify_package(&payload, &report_directory).await?;
    on_ledger(&store, "Resolution import", move |store| {
        store.update(|document| {
            let plan = plan_import(document, &package.entries)?;
            apply_import(document, &plan, &signer)?;
            Ok(ResolutionImportResult {
                added: plan.additions.len() as u64,
                already_present: plan.already_present as u64,
                conflicts: plan.conflicts,
                head: document.head().to_string(),
            })
        })
    })
    .await
}
