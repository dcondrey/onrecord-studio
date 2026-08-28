//! Rebuildable, provenance-linked corpus index for cross-document pattern discovery.

use crate::error::AppError;
use crate::integrity::digest_hex;
use crate::types::{
    CorpusEntityKind, CorpusEvidenceReference, CorpusIndexStatus, CorpusPattern, CorpusPatternKind,
    CorpusPatternQuery,
};
use duckdb::{params, Connection, OptionalExt, Transaction};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{mpsc as std_mpsc, Arc};
use tokio::sync::{mpsc, oneshot, Semaphore};

const CORPUS_ALGORITHM_VERSION: &str = "deterministic-corpus-v3";
/// Shape of the tables in [`CORPUS_SCHEMA`]. Bump this whenever a column type, key, or constraint
/// changes; an index written under any other version is discarded and rebuilt rather than migrated.
/// 3: every integer column widened to `BIGINT`. DuckDB's `INTEGER` is 32-bit, and a real source
/// produced document id 4561106750446765, so every write of it failed with `Conversion Error:
/// Type UINT64 ... out of range for the destination type INT32` and the document never reached
/// the index. Same defect, same cause, as the note at the top of `0001_initial_schema.sql`.
/// 4: added `corpus_entity_truncations`, one row per document per entity kind recording whether
/// that kind's extraction hit its per-kind budget and had to stop early. Before this, every kind
/// shared one `MAXIMUM_ENTITIES_PER_DOCUMENT` pool, so a spreadsheet of dates could fill the whole
/// budget and leave every other kind at zero with nothing recording that extraction was cut short
/// rather than genuinely absent (see docs/local-analysis-architecture.md section 2).
/// 5: materialized distinct-document counts for entities avoid rebuilding concatenated document
/// keys for the common unscoped interactive pattern query. Scoped/time-bounded queries still
/// calculate within their requested slice.
/// 6: canonical entity kinds are schema-constrained and same-segment entity relationships retain
/// their source mention offsets for cross-format relationship patterns.
const CORPUS_SCHEMA_VERSION: i64 = 6;

const WRITER_QUEUE_CAPACITY: usize = 64;
const MAXIMUM_CONCURRENT_ANALYSES: usize = 4;
const SEGMENT_TARGET_CHARACTERS: usize = 2_048;
const SEGMENT_OVERLAP_CHARACTERS: usize = 192;
const MAXIMUM_SEGMENTS_PER_DOCUMENT: usize = 100_000;
const MAXIMUM_TERMS_PER_DOCUMENT: usize = 96;
/// Applied independently to each entity kind (see [`ENTITY_KINDS`]) rather than as one shared pool.
/// A single 20,000-entity pool let a spreadsheet of dates consume the entire budget and leave every
/// other kind at zero, silently, on document `6661148630021752`
/// (docs/local-analysis-architecture.md section 2). 4,000 per kind keeps the worst case
/// (`ENTITY_KINDS.len()` kinds all saturated) below the old shared ceiling while giving every kind
/// room to be genuinely present in a large document; a kind that still exhausts this is recorded as
/// truncated rather than silently starved.
const MAXIMUM_ENTITIES_PER_KIND: usize = 4_000;
/// Every `kind` value ever passed to `push_entity`/`push_entity_with_display`. Used to seed the
/// truncation map so a document's row set always states "not truncated" rather than omitting a kind
/// that simply had nothing in it.
const ENTITY_KINDS: [CorpusEntityKind; 6] = [
    CorpusEntityKind::Statute,
    CorpusEntityKind::Date,
    CorpusEntityKind::EmailDomain,
    CorpusEntityKind::UrlHost,
    CorpusEntityKind::Acronym,
    CorpusEntityKind::Organization,
];
const MAXIMUM_TOKEN_CHARACTERS: usize = 96;
const MAXIMUM_RELATIONSHIPS_PER_DOCUMENT: usize = 20_000;
const MAXIMUM_RELATIONSHIP_ENTITIES_PER_SEGMENT: usize = 64;

type WriterJob = Box<dyn FnOnce(&mut Connection) + Send + 'static>;

#[derive(Clone)]
pub struct CorpusIndex {
    writer: mpsc::Sender<WriterJob>,
    analysis_permits: Arc<Semaphore>,
}

// The million-document throughput harness in scripts/validate-release-performance.mjs measures
// the serialized writer and reports whether sharding is warranted for the release workload.

pub struct CorpusDocumentInput {
    pub portal_key: String,
    pub request_id: u64,
    pub document_id: u64,
    pub raw_file_sha256: String,
    pub extracted_text_sha256: String,
    pub text: String,
}

struct AnalyzedDocument {
    portal_key: String,
    request_id: u64,
    document_id: u64,
    raw_file_sha256: String,
    extracted_text_sha256: String,
    character_count: usize,
    document_simhash: u64,
    segments: Vec<AnalyzedSegment>,
    terms: Vec<(String, usize)>,
    entities: Vec<AnalyzedEntity>,
    relationships: Vec<AnalyzedRelationship>,
    relationships_truncated: bool,
    /// Per entity-kind: whether that kind's extraction hit [`MAXIMUM_ENTITIES_PER_KIND`] and had to
    /// stop before the text was fully scanned. Always has one entry per [`ENTITY_KINDS`] value, so a
    /// reader can tell "this document has no organizations" from "we stopped looking."
    entity_truncations: HashMap<CorpusEntityKind, bool>,
}

/// Tracks, per entity kind, how many entities have been accepted and whether that kind's budget was
/// exhausted. Enforcing the cap here rather than in every extraction loop keeps the truncation flag
/// and the push that trips it in one place.
struct EntityBudget {
    counts: HashMap<CorpusEntityKind, usize>,
    truncated: HashMap<CorpusEntityKind, bool>,
}

impl EntityBudget {
    fn new() -> Self {
        let mut counts = HashMap::new();
        let mut truncated = HashMap::new();
        for kind in ENTITY_KINDS {
            counts.insert(kind, 0);
            truncated.insert(kind, false);
        }
        Self { counts, truncated }
    }

    /// Whether `kind` still has room for another entity. Extraction loops use this to stop scanning
    /// for that kind early rather than continuing to burn CPU once the budget is spent, which is
    /// also why this is the one place that records truncation: every call site checks this *before*
    /// pushing another entity and stops as soon as it answers `true`, so `admit` below is never
    /// actually invoked while full and could not observe the cap being hit itself.
    fn is_full(&mut self, kind: CorpusEntityKind) -> bool {
        let full = self.counts.get(&kind).copied().unwrap_or(0) >= MAXIMUM_ENTITIES_PER_KIND;
        if full {
            self.truncated.insert(kind, true);
        }
        full
    }

    /// Consumes one slot of `kind`'s budget. Every caller checks `is_full` first and stops rather
    /// than calling this once it is spent, so the count here never exceeds the cap.
    fn admit(&mut self, kind: CorpusEntityKind) {
        *self.counts.entry(kind).or_insert(0) += 1;
    }
}

struct AnalyzedSegment {
    index: usize,
    character_start: usize,
    character_end: usize,
    content_sha256: String,
    simhash: u64,
}

struct AnalyzedEntity {
    kind: CorpusEntityKind,
    normalized_value: String,
    display_value: String,
    character_start: usize,
    character_end: usize,
    segment_index: usize,
    confidence_basis_points: u16,
}

struct AnalyzedRelationship {
    left_entity_id: String,
    right_entity_id: String,
    segment_index: usize,
    character_start: usize,
    character_end: usize,
    confidence_basis_points: u16,
}

#[derive(Clone)]
struct Token {
    normalized: String,
    // Byte offsets are for safe Rust slicing; character offsets are the stable IPC/UI coordinate.
    byte_start: usize,
    byte_end: usize,
    character_start: usize,
    character_end: usize,
}

#[derive(Clone)]
struct PatternDocument {
    portal_key: String,
    request_id: u64,
    document_id: u64,
    content_sha256: String,
    simhash: u64,
}

impl CorpusIndex {
    pub fn open<P: AsRef<Path>>(database_path: P) -> Result<Self, AppError> {
        let database_path = absolute_database_path(database_path.as_ref())?;
        let (writer, mut receiver) = mpsc::channel::<WriterJob>(WRITER_QUEUE_CAPACITY);
        let (ready_sender, ready_receiver) = std_mpsc::sync_channel(1);
        let writer_path = database_path.clone();
        std::thread::Builder::new()
            .name("onrecord-corpus-writer".to_string())
            .spawn(move || {
                let connection = open_writer_connection(&writer_path);
                match connection {
                    Ok(mut connection) => {
                        let _ = ready_sender.send(Ok::<(), String>(()));
                        while let Some(job) = receiver.blocking_recv() {
                            job(&mut connection);
                        }
                    }
                    Err(error) => {
                        let _ = ready_sender.send(Err(error.to_string()));
                    }
                }
            })
            .map_err(|error| {
                AppError::Storage(format!("Failed to start corpus writer thread: {error}"))
            })?;
        ready_receiver
            .recv()
            .map_err(|_| AppError::Storage("Corpus writer failed during startup".to_string()))?
            .map_err(AppError::Storage)?;
        Ok(Self {
            writer,
            analysis_permits: Arc::new(Semaphore::new(MAXIMUM_CONCURRENT_ANALYSES)),
        })
    }

    pub async fn index_document(&self, input: CorpusDocumentInput) -> Result<(), AppError> {
        validate_document_input(&input)?;
        let permit = self
            .analysis_permits
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| AppError::Internal("Corpus analyzer is unavailable".to_string()))?;
        let analyzed = tokio::task::spawn_blocking(move || analyze_document(input))
            .await
            .map_err(|error| AppError::Internal(format!("Corpus analyzer failed: {error}")))??;
        // The semaphore bounds CPU/memory-heavy analysis only. Commits are serialized separately
        // by the writer channel, so holding this permit while waiting to write would reduce throughput.
        drop(permit);
        self.write("Corpus document commit", move |connection| {
            persist_analyzed_document(connection, analyzed)
        })
        .await
    }

    pub async fn is_document_current(
        &self,
        portal_key: String,
        document_id: u64,
        extracted_text_sha256: String,
    ) -> Result<bool, AppError> {
        self.read("Corpus document version query", move |connection| {
            let mut statement = connection
                .prepare(
                    "SELECT 1
                     FROM corpus_documents
                     WHERE portal_key = ?1
                       AND document_id = ?2
                       AND extracted_text_sha256 = ?3
                       AND algorithm_version = ?4
                     LIMIT 1",
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to prepare corpus version query: {error}"))
                })?;
            let mut rows = statement
                .query(params![
                    portal_key,
                    document_id,
                    extracted_text_sha256,
                    CORPUS_ALGORITHM_VERSION
                ])
                .map_err(|error| {
                    AppError::Storage(format!("Failed to query corpus document version: {error}"))
                })?;
            rows.next().map(|row| row.is_some()).map_err(|error| {
                AppError::Storage(format!("Failed to read corpus document version: {error}"))
            })
        })
        .await
    }

    /// Entity kinds whose extraction hit its per-kind budget for this document, so a caller can
    /// state "we stopped looking" for those kinds rather than presenting a truncated result as a
    /// complete one. Empty means every kind's extraction ran to completion (or the document has no
    /// recorded truncation rows at all, which is the same fact for a document indexed after this
    /// table existed).
    pub async fn truncated_entity_kinds(
        &self,
        portal_key: String,
        document_id: u64,
    ) -> Result<Vec<String>, AppError> {
        self.read("Corpus entity truncation query", move |connection| {
            let mut statement = connection
                .prepare(
                    "SELECT entity_kind FROM corpus_entity_truncations
                     WHERE portal_key = ?1 AND document_id = ?2 AND truncated
                     ORDER BY entity_kind",
                )
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to prepare entity truncation query: {error}"
                    ))
                })?;
            statement
                .query_map(params![portal_key, document_id], |row| {
                    row.get::<_, String>(0)
                })
                .map_err(|error| {
                    AppError::Storage(format!("Failed to query entity truncations: {error}"))
                })?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| {
                    AppError::Storage(format!("Failed to read entity truncations: {error}"))
                })
        })
        .await
    }

    pub async fn query_patterns(
        &self,
        query: CorpusPatternQuery,
    ) -> Result<Vec<CorpusPattern>, AppError> {
        validate_pattern_query(&query)?;
        self.read("Corpus pattern query", move |connection| {
            load_patterns(connection, &query)
        })
        .await
    }

    pub async fn status(&self) -> Result<CorpusIndexStatus, AppError> {
        let available_write_capacity = self.writer.capacity();
        self.read("Corpus index status", move |connection| {
            let (documents, segments, terms, relationships) = connection
                .query_row(
                    "SELECT
                        (SELECT COUNT(*) FROM corpus_documents),
                        (SELECT COUNT(*) FROM corpus_segments),
                        (SELECT COUNT(*) FROM corpus_term_postings),
                        (SELECT COUNT(*) FROM corpus_entity_relationships)",
                    [],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, i64>(2)?,
                            row.get::<_, i64>(3)?,
                        ))
                    },
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to load corpus index status: {error}"))
                })?;
            Ok(CorpusIndexStatus {
                indexed_document_count: nonnegative_usize(documents, "document count")?,
                indexed_segment_count: nonnegative_usize(segments, "segment count")?,
                indexed_term_count: nonnegative_usize(terms, "term count")?,
                indexed_relationship_count: nonnegative_usize(relationships, "relationship count")?,
                available_write_capacity,
                algorithm_version: CORPUS_ALGORITHM_VERSION.to_string(),
            })
        })
        .await
    }

    async fn write<T, F>(&self, operation: &'static str, function: F) -> Result<T, AppError>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T, AppError> + Send + 'static,
    {
        let (result_sender, result_receiver) = oneshot::channel();
        self.writer
            .send(Box::new(move |connection| {
                let _ = result_sender.send(function(connection));
            }))
            .await
            .map_err(|_| AppError::Storage(format!("{operation}: corpus writer is unavailable")))?;
        result_receiver.await.map_err(|_| {
            AppError::Storage(format!(
                "{operation}: corpus writer terminated unexpectedly"
            ))
        })?
    }

    /// Runs a read on the writer's connection rather than opening one of its own.
    ///
    /// A second DuckDB connection to the same file is not a view of the writer's database. Opening
    /// one gives a separate instance that is stuck at whatever it first saw: measured in
    /// `a_write_is_visible_to_every_later_read`, where after two writes the writer's connection
    /// counts two documents and a freshly opened connection counts one. Every corpus read is
    /// evidence about the indexed corpus -- `status`, `query_patterns`, and the
    /// `is_document_current` check the reconciliation loop in `setup` uses to decide whether a
    /// document still needs indexing -- so a stale answer is not a slow refresh, it is a wrong
    /// one. Reads therefore queue behind writes on the single writer thread, which is the
    /// consistency this index was already serialized for.
    async fn read<T, F>(&self, operation: &'static str, function: F) -> Result<T, AppError>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> Result<T, AppError> + Send + 'static,
    {
        self.write(operation, move |connection| function(connection))
            .await
    }
}

fn absolute_database_path(path: &Path) -> Result<PathBuf, AppError> {
    if path.as_os_str().is_empty() || path == Path::new(":memory:") {
        return Err(AppError::Storage(
            "Corpus index requires a persistent database path".to_string(),
        ));
    }
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    std::env::current_dir()
        .map(|current| current.join(path))
        .map_err(|error| AppError::Storage(format!("Failed to resolve corpus path: {error}")))
}

fn open_writer_connection(path: &Path) -> Result<Connection, AppError> {
    replace_outdated_corpus_index(path)?;
    let connection = Connection::open(path).map_err(|error| {
        AppError::Storage(format!(
            "Failed to open corpus index '{}': {error}",
            path.display()
        ))
    })?;
    connection
        .execute_batch(
            "SET preserve_insertion_order = false;
             SET threads = 1;
             SET enable_progress_bar = false;",
        )
        .map_err(|error| AppError::Storage(format!("Failed to configure corpus index: {error}")))?;
    connection
        .execute_batch(CORPUS_SCHEMA)
        .map_err(|error| AppError::Storage(format!("Failed to migrate corpus index: {error}")))?;
    connection
        .execute(
            "INSERT INTO corpus_schema_version (singleton, version) VALUES (TRUE, ?1)
             ON CONFLICT (singleton) DO UPDATE SET version = excluded.version",
            params![CORPUS_SCHEMA_VERSION],
        )
        .map_err(|error| {
            AppError::Storage(format!(
                "Failed to stamp the corpus schema version: {error}"
            ))
        })?;
    Ok(connection)
}

/// Replaces an index written under a different schema version, before anything opens it.
///
/// The corpus is a derived projection of the canonical core, rebuilt by the reconciliation loop in
/// `setup`, so discarding it costs indexing time and no evidence. That is why the whole file goes
/// rather than being migrated.
///
/// The file, not the tables. A database created by an older DuckDB carries that storage version
/// with it -- one such file reports `storage_compatibility_version = v0.10.2` -- and under it a
/// column declared `INTEGER` materializes as `BIGINT`. The foreign keys in [`CORPUS_SCHEMA`] then
/// compare the child's declared `INTEGER` against the parent's stored `BIGINT` and refuse to bind,
/// which DuckDB does even under `CREATE TABLE IF NOT EXISTS`. Dropping every table does not help:
/// the widening is a property of the file, so the recreated parent is `BIGINT` again. The error
/// reaches the setup hook, which panics, and the application aborts at launch with no way back
/// short of deleting this file by hand -- which is exactly what this does.
fn replace_outdated_corpus_index(path: &Path) -> Result<(), AppError> {
    if !path.exists() {
        return Ok(());
    }
    if corpus_index_matches_current_schema(path) {
        return Ok(());
    }
    for stale in [path.to_path_buf(), wal_path(path)] {
        match std::fs::remove_file(&stale) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(AppError::Storage(format!(
                    "Failed to discard the outdated corpus index '{}': {error}",
                    stale.display()
                )))
            }
        }
    }
    Ok(())
}

/// DuckDB writes its write-ahead log beside the database as `<file>.wal`. Leaving it behind next
/// to a new database of the same name would replay records written against the old shape.
fn wal_path(path: &Path) -> PathBuf {
    let mut wal = path.as_os_str().to_os_string();
    wal.push(".wal");
    PathBuf::from(wal)
}

/// Whether an existing index was stamped by this exact schema version.
///
/// Anything else is false: a missing marker table (an index written before the marker existed), a
/// different version, or a file that will not open or read at all. All of them describe an index
/// this build cannot use, and it is cheaper to rebuild one than to be wrong about it.
fn corpus_index_matches_current_schema(path: &Path) -> bool {
    let Ok(connection) = Connection::open(path) else {
        return false;
    };
    let marker_exists: Result<bool, _> = connection.query_row(
        "SELECT COUNT(*) > 0 FROM duckdb_tables() WHERE table_name = 'corpus_schema_version'",
        [],
        |row| row.get(0),
    );
    if !matches!(marker_exists, Ok(true)) {
        return false;
    }
    let recorded: Result<Option<i64>, _> = connection
        .query_row(
            "SELECT version FROM corpus_schema_version WHERE singleton",
            [],
            |row| row.get(0),
        )
        .optional();
    matches!(recorded, Ok(Some(version)) if version == CORPUS_SCHEMA_VERSION)
}

fn validate_document_input(input: &CorpusDocumentInput) -> Result<(), AppError> {
    if input.portal_key.trim().is_empty()
        || input.raw_file_sha256.len() != 64
        || input.extracted_text_sha256.len() != 64
    {
        return Err(AppError::DataParsing(
            "Corpus documents require a portal key and normalized SHA-256 digests".to_string(),
        ));
    }
    if input.text.is_empty() {
        return Err(AppError::DataParsing(
            "Corpus documents require extracted text".to_string(),
        ));
    }
    Ok(())
}

fn validate_pattern_query(query: &CorpusPatternQuery) -> Result<(), AppError> {
    if !(2..=100).contains(&query.minimum_evidence_count) {
        return Err(AppError::DataParsing(
            "Corpus patterns require between 2 and 100 evidence items".to_string(),
        ));
    }
    if !(1..=1_000).contains(&query.result_limit) {
        return Err(AppError::DataParsing(
            "Corpus pattern result limit must be between 1 and 1000".to_string(),
        ));
    }
    if query.minimum_confidence_basis_points > 10_000 {
        return Err(AppError::DataParsing(
            "Corpus confidence bound must be between 0 and 10000 basis points".to_string(),
        ));
    }
    let unique_kinds = query
        .pattern_kinds
        .iter()
        .map(|kind| kind.as_str())
        .collect::<BTreeSet<_>>();
    if query.pattern_kinds.len() > 7 || unique_kinds.len() != query.pattern_kinds.len() {
        return Err(AppError::DataParsing(
            "Corpus pattern-kind filters must be unique and contain at most seven values"
                .to_string(),
        ));
    }
    if query.required_terms.len() > 12
        || query.required_terms.iter().any(|term| {
            let trimmed = term.trim();
            !(2..=64).contains(&trimmed.chars().count()) || trimmed.chars().any(char::is_control)
        })
    {
        return Err(AppError::DataParsing(
            "Corpus recipe terms must contain 2 to 64 printable characters and at most 12 values"
                .to_string(),
        ));
    }
    if query
        .portal_key
        .as_deref()
        .is_some_and(|value| value.trim().is_empty())
    {
        return Err(AppError::DataParsing(
            "Corpus portal filter must not be empty".to_string(),
        ));
    }
    let indexed_from = query
        .indexed_from
        .as_deref()
        .map(chrono::DateTime::parse_from_rfc3339)
        .transpose()
        .map_err(|error| {
            AppError::DataParsing(format!("Invalid corpus index start time: {error}"))
        })?;
    let indexed_to = query
        .indexed_to
        .as_deref()
        .map(chrono::DateTime::parse_from_rfc3339)
        .transpose()
        .map_err(|error| {
            AppError::DataParsing(format!("Invalid corpus index end time: {error}"))
        })?;
    if matches!((indexed_from, indexed_to), (Some(from), Some(to)) if from > to) {
        return Err(AppError::DataParsing(
            "Corpus index start time must not follow the end time".to_string(),
        ));
    }
    Ok(())
}

fn analyze_document(input: CorpusDocumentInput) -> Result<AnalyzedDocument, AppError> {
    let tokens = tokenize(&input.text);
    let character_count = input.text.chars().count();
    let document_simhash = simhash(tokens.iter().map(|token| token.normalized.as_str()));
    let segments = analyze_segments(&input.text, &tokens, character_count)?;
    let terms = top_terms(&tokens);
    let (entities, entity_truncations) = extract_entities(&input.text, &tokens, &segments);
    let (relationships, relationships_truncated) = analyze_relationships(&entities);
    Ok(AnalyzedDocument {
        portal_key: input.portal_key,
        request_id: input.request_id,
        document_id: input.document_id,
        raw_file_sha256: input.raw_file_sha256,
        extracted_text_sha256: input.extracted_text_sha256,
        character_count,
        document_simhash,
        segments,
        terms,
        entities,
        relationships,
        relationships_truncated,
        entity_truncations,
    })
}

fn analyze_relationships(entities: &[AnalyzedEntity]) -> (Vec<AnalyzedRelationship>, bool) {
    let mut by_segment = BTreeMap::<usize, Vec<&AnalyzedEntity>>::new();
    for entity in entities {
        by_segment
            .entry(entity.segment_index)
            .or_default()
            .push(entity);
    }
    let mut relationships = Vec::new();
    let mut seen = BTreeSet::new();
    let mut truncated = false;
    'segments: for (segment_index, segment_entities) in by_segment {
        let mut unique = segment_entities
            .into_iter()
            .map(|entity| {
                (
                    stable_id(&format!(
                        "{}|{}",
                        entity.kind.as_str(),
                        entity.normalized_value
                    )),
                    entity,
                )
            })
            .collect::<Vec<_>>();
        unique.sort_by(|left, right| left.0.cmp(&right.0));
        unique.dedup_by(|left, right| left.0 == right.0);
        if unique.len() > MAXIMUM_RELATIONSHIP_ENTITIES_PER_SEGMENT {
            unique.truncate(MAXIMUM_RELATIONSHIP_ENTITIES_PER_SEGMENT);
            truncated = true;
        }
        for left_index in 0..unique.len() {
            for right_index in left_index + 1..unique.len() {
                if relationships.len() >= MAXIMUM_RELATIONSHIPS_PER_DOCUMENT {
                    truncated = true;
                    break 'segments;
                }
                let (left_id, left) = &unique[left_index];
                let (right_id, right) = &unique[right_index];
                let identity = (segment_index, left_id.clone(), right_id.clone());
                if !seen.insert(identity) {
                    continue;
                }
                relationships.push(AnalyzedRelationship {
                    left_entity_id: left_id.clone(),
                    right_entity_id: right_id.clone(),
                    segment_index,
                    character_start: left.character_start.min(right.character_start),
                    character_end: left.character_end.max(right.character_end),
                    confidence_basis_points: left
                        .confidence_basis_points
                        .min(right.confidence_basis_points),
                });
            }
        }
    }
    (relationships, truncated)
}

fn tokenize(text: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut token_byte_start = None;
    let mut token_character_start = 0_usize;
    let mut character_index = 0_usize;
    for (byte_index, character) in text.char_indices() {
        if character.is_alphanumeric() {
            if token_byte_start.is_none() {
                token_byte_start = Some(byte_index);
                token_character_start = character_index;
            }
        } else if let Some(byte_start) = token_byte_start.take() {
            push_token(
                text,
                &mut tokens,
                byte_start,
                byte_index,
                token_character_start,
                character_index,
            );
        }
        character_index += 1;
    }
    if let Some(byte_start) = token_byte_start {
        push_token(
            text,
            &mut tokens,
            byte_start,
            text.len(),
            token_character_start,
            character_index,
        );
    }
    tokens
}

fn push_token(
    text: &str,
    tokens: &mut Vec<Token>,
    byte_start: usize,
    byte_end: usize,
    character_start: usize,
    character_end: usize,
) {
    if character_end <= character_start
        || character_end - character_start > MAXIMUM_TOKEN_CHARACTERS
    {
        return;
    }
    let normalized = text[byte_start..byte_end].to_lowercase();
    tokens.push(Token {
        normalized,
        byte_start,
        byte_end,
        character_start,
        character_end,
    });
}

fn analyze_segments(
    text: &str,
    tokens: &[Token],
    character_count: usize,
) -> Result<Vec<AnalyzedSegment>, AppError> {
    if tokens.is_empty() {
        return Ok(Vec::new());
    }
    let mut segments = Vec::new();
    let mut character_start = 0_usize;
    while character_start < character_count && segments.len() < MAXIMUM_SEGMENTS_PER_DOCUMENT {
        let target_end = character_start
            .saturating_add(SEGMENT_TARGET_CHARACTERS)
            .min(character_count);
        let first_token = tokens.partition_point(|token| token.character_end <= character_start);
        let after_last = tokens.partition_point(|token| token.character_start < target_end);
        if first_token >= after_last {
            character_start = target_end;
            continue;
        }
        let segment_tokens = &tokens[first_token..after_last];
        let byte_start = segment_tokens[0].byte_start;
        let byte_end = segment_tokens
            .last()
            .map(|token| token.byte_end)
            .unwrap_or(byte_start);
        let actual_character_start = segment_tokens[0].character_start;
        let actual_character_end = segment_tokens
            .last()
            .map(|token| token.character_end)
            .unwrap_or(actual_character_start);
        let content = text.get(byte_start..byte_end).ok_or_else(|| {
            AppError::Internal("Corpus segment boundaries were invalid".to_string())
        })?;
        segments.push(AnalyzedSegment {
            index: segments.len(),
            character_start: actual_character_start,
            character_end: actual_character_end,
            content_sha256: digest_hex(Sha256::digest(content.as_bytes())),
            simhash: simhash(segment_tokens.iter().map(|token| token.normalized.as_str())),
        });
        if target_end == character_count {
            break;
        }
        character_start = target_end.saturating_sub(SEGMENT_OVERLAP_CHARACTERS);
    }
    if character_start < character_count && segments.len() == MAXIMUM_SEGMENTS_PER_DOCUMENT {
        return Err(AppError::DataParsing(format!(
            "Document requires more than {MAXIMUM_SEGMENTS_PER_DOCUMENT} corpus segments"
        )));
    }
    Ok(segments)
}

fn simhash<'a>(tokens: impl Iterator<Item = &'a str>) -> u64 {
    let mut weights = [0_i64; 64];
    for token in tokens.filter(|token| !is_stopword(token)) {
        let digest = Sha256::digest(token.as_bytes());
        let hash = u64::from_be_bytes(digest[..8].try_into().expect("SHA-256 prefix is 8 bytes"));
        for (bit, weight) in weights.iter_mut().enumerate() {
            if hash & (1_u64 << bit) == 0 {
                *weight -= 1;
            } else {
                *weight += 1;
            }
        }
    }
    weights
        .iter()
        .enumerate()
        .fold(0_u64, |fingerprint, (bit, weight)| {
            if *weight >= 0 {
                fingerprint | (1_u64 << bit)
            } else {
                fingerprint
            }
        })
}

fn top_terms(tokens: &[Token]) -> Vec<(String, usize)> {
    let mut frequencies = HashMap::<&str, usize>::new();
    for token in tokens {
        if token.normalized.len() >= 4 && !is_stopword(&token.normalized) {
            *frequencies.entry(&token.normalized).or_default() += 1;
        }
    }
    let mut terms = frequencies
        .into_iter()
        .filter(|(_, count)| *count >= 2)
        .map(|(term, count)| (term.to_string(), count))
        .collect::<Vec<_>>();
    terms.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    terms.truncate(MAXIMUM_TERMS_PER_DOCUMENT);
    terms
}

fn extract_entities(
    text: &str,
    tokens: &[Token],
    segments: &[AnalyzedSegment],
) -> (Vec<AnalyzedEntity>, HashMap<CorpusEntityKind, bool>) {
    let mut entities = Vec::new();
    let mut budget = EntityBudget::new();
    for (index, token) in tokens.iter().enumerate() {
        if budget.is_full(CorpusEntityKind::Statute) {
            break;
        }
        if token.normalized == "section" {
            if let Some(value) = tokens.get(index + 1).filter(|value| {
                value
                    .normalized
                    .chars()
                    .any(|character| character.is_ascii_digit())
            }) {
                push_entity(
                    &mut entities,
                    &mut budget,
                    text,
                    segments,
                    CorpusEntityKind::Statute,
                    format!("section {}", value.normalized),
                    token.byte_start,
                    value.byte_end,
                    token.character_start,
                    value.character_end,
                    8_500,
                );
            }
        }
        if index + 4 < tokens.len()
            && token.normalized.chars().all(|value| value.is_ascii_digit())
            && tokens[index + 1].normalized == "u"
            && tokens[index + 2].normalized == "s"
            && tokens[index + 3].normalized == "c"
            && tokens[index + 4]
                .normalized
                .chars()
                .any(|value| value.is_ascii_digit())
        {
            let last = &tokens[index + 4];
            push_entity(
                &mut entities,
                &mut budget,
                text,
                segments,
                CorpusEntityKind::Statute,
                format!("{} u.s.c. {}", token.normalized, last.normalized),
                token.byte_start,
                last.byte_end,
                token.character_start,
                last.character_end,
                9_500,
            );
        }
    }
    extract_dates(text, segments, &mut entities, &mut budget);
    extract_email_domains(text, segments, &mut entities, &mut budget);
    extract_url_hosts(text, segments, &mut entities, &mut budget);
    extract_organizations_and_acronyms(text, tokens, segments, &mut entities, &mut budget);
    entities.sort_by(|left, right| {
        left.character_start
            .cmp(&right.character_start)
            .then_with(|| left.normalized_value.cmp(&right.normalized_value))
    });
    entities.dedup_by(|left, right| {
        left.kind == right.kind
            && left.normalized_value == right.normalized_value
            && left.character_start == right.character_start
    });
    (entities, budget.truncated)
}

#[allow(clippy::too_many_arguments)]
fn push_entity(
    entities: &mut Vec<AnalyzedEntity>,
    budget: &mut EntityBudget,
    text: &str,
    segments: &[AnalyzedSegment],
    kind: CorpusEntityKind,
    normalized_value: String,
    byte_start: usize,
    byte_end: usize,
    character_start: usize,
    character_end: usize,
    confidence_basis_points: u16,
) {
    let display_value = text
        .get(byte_start..byte_end)
        .unwrap_or(&normalized_value)
        .chars()
        .take(256)
        .collect();
    push_entity_with_display(
        entities,
        budget,
        segments,
        kind,
        normalized_value,
        display_value,
        character_start,
        character_end,
        confidence_basis_points,
    );
}

#[allow(clippy::too_many_arguments)]
fn push_entity_with_display(
    entities: &mut Vec<AnalyzedEntity>,
    budget: &mut EntityBudget,
    segments: &[AnalyzedSegment],
    kind: CorpusEntityKind,
    normalized_value: String,
    display_value: String,
    character_start: usize,
    character_end: usize,
    confidence_basis_points: u16,
) {
    if budget.is_full(kind) {
        return;
    }
    budget.admit(kind);
    // Segment coverage is not contiguous (see `analyze_segments`): a segment is bounded by its
    // first and last token, token-free windows are skipped outright, and neighbours share only
    // SEGMENT_OVERLAP_CHARACTERS. An entity can therefore sit inside no segment at all. Falling
    // straight back to segment 0 sent investigators to the top of the document; resolve by best
    // overlap and then by nearest segment so the index always names the closest real evidence.
    let segment_index = segments
        .iter()
        .position(|segment| {
            segment.character_start <= character_start && segment.character_end >= character_end
        })
        .or_else(|| nearest_segment_index(segments, character_start, character_end))
        .unwrap_or(0);
    entities.push(AnalyzedEntity {
        kind,
        normalized_value,
        display_value,
        character_start,
        character_end,
        segment_index,
        confidence_basis_points,
    });
}

/// Best-effort attribution for an entity no segment fully contains: the segment sharing the most
/// characters with it, and failing that the one whose range lies closest. Ties resolve to the
/// earlier segment. `None` only when the document has no segments.
fn nearest_segment_index(
    segments: &[AnalyzedSegment],
    character_start: usize,
    character_end: usize,
) -> Option<usize> {
    segments
        .iter()
        .enumerate()
        .min_by_key(|(_, segment)| {
            let overlap = character_end
                .min(segment.character_end)
                .saturating_sub(character_start.max(segment.character_start));
            let distance = segment
                .character_start
                .saturating_sub(character_end)
                .max(character_start.saturating_sub(segment.character_end));
            (std::cmp::Reverse(overlap), distance)
        })
        .map(|(index, _)| index)
}

const MONTH_NAMES: [(&str, u32); 12] = [
    ("january", 1),
    ("february", 2),
    ("march", 3),
    ("april", 4),
    ("may", 5),
    ("june", 6),
    ("july", 7),
    ("august", 8),
    ("september", 9),
    ("october", 10),
    ("november", 11),
    ("december", 12),
];

/// A date recognised in the text, together with what the parse had to assume to read it.
struct MatchedDate {
    /// Length of the surface form. Every form recognised here is ASCII, so this is both the byte
    /// length and the character length, which is what lets the scan advance both counters by one
    /// value.
    length: usize,
    date: chrono::NaiveDate,
    confidence_basis_points: u16,
}

/// Every date form that appears in released public records, not only ISO-8601.
///
/// Agencies write `3/17/2023` and `March 17, 2023`; they almost never write `2023-03-17`. An
/// ISO-only extractor found 32 dates in a corpus whose text layer carries over nine thousand, and
/// left 61 of 70 indexed documents with no date at all. The cross-document timeline had no input
/// for that reason alone.
///
/// Confidence records what the reading had to assume, per this codebase's rule that a stored
/// confidence derives from what the evaluation observed: an ISO date is unambiguous, a written
/// month name is unambiguous, a numeric form is read month-first by United States convention, and
/// a two-digit year has had its century inferred. A numeric date whose first field exceeds twelve
/// is rejected rather than re-read day-first, so day-first records are missed instead of being
/// silently transposed.
fn extract_dates(
    text: &str,
    segments: &[AnalyzedSegment],
    entities: &mut Vec<AnalyzedEntity>,
    budget: &mut EntityBudget,
) {
    let bytes = text.as_bytes();
    let mut byte_index = 0_usize;
    let mut character_index = 0_usize;
    while byte_index < bytes.len() && !budget.is_full(CorpusEntityKind::Date) {
        let at_boundary = byte_index == 0 || !bytes[byte_index - 1].is_ascii_alphanumeric();
        if at_boundary {
            if let Some(matched) = match_date(&text[byte_index..]) {
                let after = bytes.get(byte_index + matched.length);
                let ends_cleanly = match after {
                    None => true,
                    Some(value) => !value.is_ascii_alphanumeric() && !matches!(value, b'/' | b'-'),
                };
                if ends_cleanly {
                    push_entity(
                        entities,
                        budget,
                        text,
                        segments,
                        CorpusEntityKind::Date,
                        matched.date.format("%Y-%m-%d").to_string(),
                        byte_index,
                        byte_index + matched.length,
                        character_index,
                        character_index + matched.length,
                        matched.confidence_basis_points,
                    );
                    byte_index += matched.length;
                    character_index += matched.length;
                    continue;
                }
            }
        }
        let character_bytes = text[byte_index..]
            .chars()
            .next()
            .map(char::len_utf8)
            .unwrap_or(1);
        byte_index += character_bytes;
        character_index += 1;
    }
}

fn match_date(rest: &str) -> Option<MatchedDate> {
    match_iso_date(rest)
        .or_else(|| match_month_first_date(rest))
        .or_else(|| match_day_first_date(rest))
        .or_else(|| match_numeric_date(rest))
}

/// `2023-03-17`, and only in exactly that shape.
fn match_iso_date(rest: &str) -> Option<MatchedDate> {
    let bytes = rest.as_bytes();
    if bytes.len() < 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }
    if !bytes
        .iter()
        .take(10)
        .enumerate()
        .all(|(index, value)| matches!(index, 4 | 7) || value.is_ascii_digit())
    {
        return None;
    }
    let year = read_number(bytes, 0, 4)?.0;
    let month = read_number(bytes, 5, 2)?.0;
    let day = read_number(bytes, 8, 2)?.0;
    Some(MatchedDate {
        length: 10,
        date: chrono::NaiveDate::from_ymd_opt(year as i32, month, day)?,
        confidence_basis_points: 10_000,
    })
}

/// `March 17, 2023`, `Mar. 17 2023`, `September 3rd, 2019`.
fn match_month_first_date(rest: &str) -> Option<MatchedDate> {
    let bytes = rest.as_bytes();
    let word_end = alphabetic_run(bytes, 0);
    let month = month_from_word(rest.get(0..word_end)?)?;
    let mut cursor = word_end;
    if bytes.get(cursor) == Some(&b'.') {
        cursor += 1;
    }
    cursor = skip_separator(bytes, cursor, false)?;
    let (day, digits) = read_number(bytes, cursor, 2)?;
    cursor += digits;
    cursor = skip_ordinal_suffix(bytes, cursor);
    cursor = skip_separator(bytes, cursor, true)?;
    let (year, digits) = read_number(bytes, cursor, 4)?;
    if digits != 4 {
        return None;
    }
    Some(MatchedDate {
        length: cursor + digits,
        date: chrono::NaiveDate::from_ymd_opt(year as i32, month, day)?,
        confidence_basis_points: 9_800,
    })
}

/// `17 March 2023`, `3rd September, 2019`.
fn match_day_first_date(rest: &str) -> Option<MatchedDate> {
    let bytes = rest.as_bytes();
    let (day, digits) = read_number(bytes, 0, 2)?;
    let mut cursor = digits;
    cursor = skip_ordinal_suffix(bytes, cursor);
    cursor = skip_separator(bytes, cursor, false)?;
    let word_end = alphabetic_run(bytes, cursor);
    let month = month_from_word(rest.get(cursor..word_end)?)?;
    cursor = word_end;
    if bytes.get(cursor) == Some(&b'.') {
        cursor += 1;
    }
    cursor = skip_separator(bytes, cursor, true)?;
    let (year, digits) = read_number(bytes, cursor, 4)?;
    if digits != 4 {
        return None;
    }
    Some(MatchedDate {
        length: cursor + digits,
        date: chrono::NaiveDate::from_ymd_opt(year as i32, month, day)?,
        confidence_basis_points: 9_800,
    })
}

/// `3/17/2023`, `3-17-2023`, `3/17/23`. Read month-first; see the note on [`extract_dates`].
fn match_numeric_date(rest: &str) -> Option<MatchedDate> {
    let bytes = rest.as_bytes();
    let (month, month_digits) = read_number(bytes, 0, 2)?;
    let separator = *bytes.get(month_digits)?;
    if !matches!(separator, b'/' | b'-') {
        return None;
    }
    let (day, day_digits) = read_number(bytes, month_digits + 1, 2)?;
    if *bytes.get(month_digits + 1 + day_digits)? != separator {
        return None;
    }
    let year_start = month_digits + 2 + day_digits;
    let (year_value, year_digits) = read_number(bytes, year_start, 4)?;
    let (year, confidence_basis_points) = match year_digits {
        4 => (year_value, 9_000),
        2 if year_value <= 69 => (2_000 + year_value, 8_000),
        2 => (1_900 + year_value, 8_000),
        _ => return None,
    };
    // Without this, a hyphenated identifier such as `5-14-3191` parses as a valid calendar date in
    // the year 3191. A numeric form carries no other evidence that it is a date at all, so the
    // plausible-year window is the only thing separating it from a serial number.
    if !(1900..=2099).contains(&year) {
        return None;
    }
    Some(MatchedDate {
        length: year_start + year_digits,
        date: chrono::NaiveDate::from_ymd_opt(year as i32, month, day)?,
        confidence_basis_points,
    })
}

/// The whole alphabetic run is taken before matching, so `augment` cannot be read as an
/// abbreviation of `august`. An abbreviation must be at least three characters, which is enough to
/// keep every month distinct.
fn month_from_word(word: &str) -> Option<u32> {
    if word.len() < 3 {
        return None;
    }
    let lowered = word.to_ascii_lowercase();
    MONTH_NAMES
        .iter()
        .find(|(name, _)| *name == lowered || name.starts_with(&lowered))
        .map(|(_, month)| *month)
}

fn alphabetic_run(bytes: &[u8], from: usize) -> usize {
    let mut end = from;
    while end < bytes.len() && bytes[end].is_ascii_alphabetic() {
        end += 1;
    }
    end
}

fn read_number(bytes: &[u8], at: usize, maximum_digits: usize) -> Option<(u32, usize)> {
    let mut end = at;
    while end < bytes.len() && end - at < maximum_digits && bytes[end].is_ascii_digit() {
        end += 1;
    }
    if end == at {
        return None;
    }
    let value = std::str::from_utf8(&bytes[at..end]).ok()?.parse().ok()?;
    Some((value, end - at))
}

/// ASCII whitespace, optionally preceded by a comma when `allow_comma` is set. Bounded so a date
/// cannot be assembled across an arbitrary gap, and restricted to ASCII so the caller's byte and
/// character counters stay equal.
fn skip_separator(bytes: &[u8], from: usize, allow_comma: bool) -> Option<usize> {
    let mut cursor = from;
    if allow_comma && bytes.get(cursor) == Some(&b',') {
        cursor += 1;
    }
    let start = cursor;
    while cursor < bytes.len()
        && cursor - start < 3
        && matches!(bytes[cursor], b' ' | b'\t' | b'\r' | b'\n')
    {
        cursor += 1;
    }
    if cursor == start && cursor == from {
        return None;
    }
    Some(cursor)
}

fn skip_ordinal_suffix(bytes: &[u8], from: usize) -> usize {
    let suffix = bytes.get(from..from + 2);
    match suffix {
        Some(b"st") | Some(b"nd") | Some(b"rd") | Some(b"th") | Some(b"ST") | Some(b"ND")
        | Some(b"RD") | Some(b"TH") => from + 2,
        _ => from,
    }
}

fn extract_email_domains(
    text: &str,
    segments: &[AnalyzedSegment],
    entities: &mut Vec<AnalyzedEntity>,
    budget: &mut EntityBudget,
) {
    let bytes = text.as_bytes();
    let mut byte_index = 0_usize;
    let mut character_index = 0_usize;
    while byte_index < bytes.len() && !budget.is_full(CorpusEntityKind::EmailDomain) {
        if bytes[byte_index] == b'@' {
            let mut start = byte_index;
            while start > 0 && is_email_local_byte(bytes[start - 1]) {
                start -= 1;
            }
            let mut end = byte_index + 1;
            while end < bytes.len() && is_email_domain_byte(bytes[end]) {
                end += 1;
            }
            let domain = String::from_utf8_lossy(&bytes[byte_index + 1..end])
                .trim_matches('.')
                .to_ascii_lowercase();
            if start < byte_index
                && domain.contains('.')
                && !domain.starts_with('.')
                && !domain.ends_with('.')
            {
                let local_characters = text[start..byte_index].chars().count();
                let domain_characters = text[byte_index + 1..end].chars().count();
                push_entity_with_display(
                    entities,
                    budget,
                    segments,
                    CorpusEntityKind::EmailDomain,
                    domain.clone(),
                    format!("@{domain}"),
                    character_index.saturating_sub(local_characters),
                    character_index + 1 + domain_characters,
                    9_000,
                );
            }
        }
        let character_bytes = text[byte_index..]
            .chars()
            .next()
            .map(char::len_utf8)
            .unwrap_or(1);
        byte_index += character_bytes;
        character_index += 1;
    }
}

fn is_email_local_byte(value: u8) -> bool {
    value.is_ascii_alphanumeric() || matches!(value, b'.' | b'_' | b'%' | b'+' | b'-')
}

fn is_email_domain_byte(value: u8) -> bool {
    value.is_ascii_alphanumeric() || matches!(value, b'.' | b'-')
}

fn extract_url_hosts(
    text: &str,
    segments: &[AnalyzedSegment],
    entities: &mut Vec<AnalyzedEntity>,
    budget: &mut EntityBudget,
) {
    let bytes = text.as_bytes();
    let mut byte_index = 0_usize;
    let mut character_index = 0_usize;
    while byte_index < bytes.len() && !budget.is_full(CorpusEntityKind::UrlHost) {
        let tail = &bytes[byte_index..];
        if tail.starts_with(b"https://") || tail.starts_with(b"http://") {
            let mut end = byte_index;
            while end < bytes.len()
                && !bytes[end].is_ascii_whitespace()
                && !matches!(bytes[end], b'"' | b'\'' | b'<' | b'>' | b')' | b']')
            {
                end += 1;
            }
            while end > byte_index && matches!(bytes[end - 1], b'.' | b',' | b';' | b':') {
                end -= 1;
            }
            let candidate = &text[byte_index..end];
            if let Ok(url) = reqwest::Url::parse(candidate) {
                if let Some(host) = url.host_str().map(str::to_ascii_lowercase) {
                    let character_count = candidate.chars().count();
                    push_entity_with_display(
                        entities,
                        budget,
                        segments,
                        CorpusEntityKind::UrlHost,
                        host.clone(),
                        host,
                        character_index,
                        character_index + character_count,
                        9_500,
                    );
                    byte_index = end;
                    character_index += character_count;
                    continue;
                }
            }
        }
        let character_bytes = text[byte_index..]
            .chars()
            .next()
            .map(char::len_utf8)
            .unwrap_or(1);
        byte_index += character_bytes;
        character_index += 1;
    }
}

fn extract_organizations_and_acronyms(
    text: &str,
    tokens: &[Token],
    segments: &[AnalyzedSegment],
    entities: &mut Vec<AnalyzedEntity>,
    budget: &mut EntityBudget,
) {
    for token in tokens {
        if budget.is_full(CorpusEntityKind::Acronym) {
            break;
        }
        let raw = &text[token.byte_start..token.byte_end];
        let letter_count = raw.chars().filter(|value| value.is_alphabetic()).count();
        if (2..=12).contains(&letter_count)
            && raw
                .chars()
                .filter(|value| value.is_alphabetic())
                .all(char::is_uppercase)
            && !matches!(
                token.normalized.as_str(),
                "pdf" | "csv" | "json" | "html" | "xml" | "http" | "https" | "www"
            )
        {
            push_entity(
                entities,
                budget,
                text,
                segments,
                CorpusEntityKind::Acronym,
                token.normalized.clone(),
                token.byte_start,
                token.byte_end,
                token.character_start,
                token.character_end,
                7_500,
            );
        }
    }

    // A cap on ACRONYM must not stop ORGANIZATION extraction below: the two used to share one pool,
    // so a document saturated on one kind returned early and starved the other silently.
    let mut start = 0_usize;
    while start < tokens.len() && !budget.is_full(CorpusEntityKind::Organization) {
        if !is_title_token(text, &tokens[start]) {
            start += 1;
            continue;
        }
        let mut end = start + 1;
        let mut best_end = None;
        let mut has_organization_signal = is_organization_token(&tokens[start].normalized);
        while end < tokens.len() && end - start < 10 {
            let token = &tokens[end];
            if is_title_token(text, token) {
                has_organization_signal |= is_organization_token(&token.normalized);
            } else if !is_name_connector(&token.normalized) {
                break;
            }
            if has_organization_signal && end > start && !is_name_connector(&token.normalized) {
                best_end = Some(end + 1);
            }
            end += 1;
        }
        if let Some(end) = best_end {
            let first = &tokens[start];
            let last = &tokens[end - 1];
            let normalized = tokens[start..end]
                .iter()
                .map(|token| token.normalized.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            push_entity(
                entities,
                budget,
                text,
                segments,
                CorpusEntityKind::Organization,
                normalized,
                first.byte_start,
                last.byte_end,
                first.character_start,
                last.character_end,
                7_000,
            );
            start = end;
        } else {
            start += 1;
        }
    }
}

fn is_title_token(text: &str, token: &Token) -> bool {
    text[token.byte_start..token.byte_end]
        .chars()
        .next()
        .is_some_and(char::is_uppercase)
}

fn is_name_connector(value: &str) -> bool {
    matches!(value, "of" | "on" | "for" | "the" | "and" | "in")
}

fn is_organization_token(value: &str) -> bool {
    matches!(
        value,
        "agency"
            | "association"
            | "authority"
            | "bureau"
            | "city"
            | "coalition"
            | "commission"
            | "company"
            | "corporation"
            | "council"
            | "county"
            | "department"
            | "district"
            | "foundation"
            | "institute"
            | "office"
            | "police"
            | "services"
            | "task"
            | "university"
    )
}

fn persist_analyzed_document(
    connection: &mut Connection,
    document: AnalyzedDocument,
) -> Result<(), AppError> {
    let transaction = connection.transaction().map_err(|error| {
        AppError::Storage(format!("Failed to open corpus index transaction: {error}"))
    })?;
    let previous_entity_ids = {
        let mut statement = transaction
            .prepare(
                "SELECT DISTINCT entity_id FROM corpus_entity_mentions
                 WHERE portal_key = ?1 AND document_id = ?2",
            )
            .map_err(|error| {
                AppError::Storage(format!("Failed to prepare prior entity lookup: {error}"))
            })?;
        statement
            .query_map(params![document.portal_key, document.document_id], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|error| {
                AppError::Storage(format!("Failed to query prior entity mentions: {error}"))
            })?
            .collect::<duckdb::Result<Vec<_>>>()
            .map_err(|error| {
                AppError::Storage(format!("Failed to decode prior entity mentions: {error}"))
            })?
    };
    transaction
        .execute(
            "INSERT INTO corpus_documents (
                portal_key, request_id, document_id, raw_file_sha256, extracted_text_sha256,
                character_count, simhash_hex, relationships_truncated, algorithm_version, indexed_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, strftime(current_timestamp, '%Y-%m-%dT%H:%M:%fZ'))
             ON CONFLICT(portal_key, document_id) DO UPDATE SET
                request_id = excluded.request_id,
                raw_file_sha256 = excluded.raw_file_sha256,
                extracted_text_sha256 = excluded.extracted_text_sha256,
                character_count = excluded.character_count,
                simhash_hex = excluded.simhash_hex,
                relationships_truncated = excluded.relationships_truncated,
                algorithm_version = excluded.algorithm_version,
                indexed_at = excluded.indexed_at",
            params![
                document.portal_key,
                document.request_id,
                document.document_id,
                document.raw_file_sha256,
                document.extracted_text_sha256,
                document.character_count,
                format!("{:016x}", document.document_simhash),
                document.relationships_truncated,
                CORPUS_ALGORITHM_VERSION,
            ],
        )
        .map_err(|error| AppError::Storage(format!("Failed to upsert corpus document: {error}")))?;
    for table in [
        "corpus_segments",
        "corpus_term_postings",
        "corpus_entity_mentions",
        "corpus_entity_relationships",
        "corpus_entity_truncations",
        "corpus_lsh_bands",
    ] {
        transaction
            .execute(
                &format!("DELETE FROM {table} WHERE portal_key = ?1 AND document_id = ?2"),
                params![document.portal_key, document.document_id],
            )
            .map_err(|error| AppError::Storage(format!("Failed to refresh {table}: {error}")))?;
    }
    insert_segments(&transaction, &document)?;
    insert_terms(&transaction, &document)?;
    insert_entities(&transaction, &document)?;
    insert_relationships(&transaction, &document)?;
    refresh_entity_document_counts(&transaction, &document, previous_entity_ids)?;
    insert_entity_truncations(&transaction, &document)?;
    insert_lsh_bands(&transaction, &document)?;
    transaction
        .commit()
        .map_err(|error| AppError::Storage(format!("Failed to commit corpus index: {error}")))
}

fn refresh_entity_document_counts(
    transaction: &Transaction<'_>,
    document: &AnalyzedDocument,
    previous_entity_ids: Vec<String>,
) -> Result<(), AppError> {
    let mut affected = previous_entity_ids.into_iter().collect::<BTreeSet<_>>();
    affected.extend(document.entities.iter().map(|entity| {
        stable_id(&format!(
            "{}|{}",
            entity.kind.as_str(),
            entity.normalized_value
        ))
    }));
    for entity_id in affected {
        let document_count = transaction
            .query_row(
                "SELECT COUNT(*) FROM (
                    SELECT portal_key, document_id
                    FROM corpus_entity_mentions
                    WHERE entity_id = ?1
                    GROUP BY portal_key, document_id
                 ) distinct_documents",
                params![entity_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| {
                AppError::Storage(format!("Failed to count entity documents: {error}"))
            })?;
        if document_count == 0 {
            transaction
                .execute(
                    "DELETE FROM corpus_entity_document_counts WHERE entity_id = ?1",
                    params![entity_id],
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to remove an entity count: {error}"))
                })?;
        } else {
            transaction
                .execute(
                    "INSERT INTO corpus_entity_document_counts (entity_id, document_count)
                     VALUES (?1, ?2)
                     ON CONFLICT(entity_id) DO UPDATE SET document_count = excluded.document_count",
                    params![entity_id, document_count],
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to materialize an entity count: {error}"))
                })?;
        }
    }
    Ok(())
}

fn insert_segments(
    transaction: &Transaction<'_>,
    document: &AnalyzedDocument,
) -> Result<(), AppError> {
    let mut statement = transaction
        .prepare_cached(
            "INSERT INTO corpus_segments (
                portal_key, document_id, segment_index, character_start, character_end,
                content_sha256, simhash_hex
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )
        .map_err(|error| {
            AppError::Storage(format!("Failed to prepare corpus segments: {error}"))
        })?;
    for segment in &document.segments {
        statement
            .execute(params![
                document.portal_key,
                document.document_id,
                segment.index,
                segment.character_start,
                segment.character_end,
                segment.content_sha256,
                format!("{:016x}", segment.simhash),
            ])
            .map_err(|error| {
                AppError::Storage(format!("Failed to persist corpus segment: {error}"))
            })?;
    }
    Ok(())
}

fn insert_terms(
    transaction: &Transaction<'_>,
    document: &AnalyzedDocument,
) -> Result<(), AppError> {
    let mut statement = transaction
        .prepare_cached(
            "INSERT INTO corpus_term_postings (portal_key, document_id, term, frequency)
             VALUES (?1, ?2, ?3, ?4)",
        )
        .map_err(|error| AppError::Storage(format!("Failed to prepare corpus terms: {error}")))?;
    for (term, frequency) in &document.terms {
        statement
            .execute(params![
                document.portal_key,
                document.document_id,
                term,
                frequency
            ])
            .map_err(|error| {
                AppError::Storage(format!("Failed to persist corpus term: {error}"))
            })?;
    }
    Ok(())
}

fn insert_entities(
    transaction: &Transaction<'_>,
    document: &AnalyzedDocument,
) -> Result<(), AppError> {
    let mut entity_statement = transaction
        .prepare_cached(
            "INSERT INTO corpus_entities (entity_id, entity_kind, normalized_value, display_value)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(entity_id) DO UPDATE SET display_value = excluded.display_value",
        )
        .map_err(|error| {
            AppError::Storage(format!("Failed to prepare corpus entities: {error}"))
        })?;
    let mut mention_statement = transaction
        .prepare_cached(
            "INSERT INTO corpus_entity_mentions (
                portal_key, document_id, entity_id, segment_index, character_start,
                character_end, confidence_basis_points
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )
        .map_err(|error| {
            AppError::Storage(format!("Failed to prepare entity mentions: {error}"))
        })?;
    for entity in &document.entities {
        let entity_id = stable_id(&format!(
            "{}|{}",
            entity.kind.as_str(),
            entity.normalized_value
        ));
        entity_statement
            .execute(params![
                entity_id,
                entity.kind.as_str(),
                entity.normalized_value,
                entity.display_value,
            ])
            .map_err(|error| {
                AppError::Storage(format!("Failed to persist corpus entity: {error}"))
            })?;
        mention_statement
            .execute(params![
                document.portal_key,
                document.document_id,
                entity_id,
                entity.segment_index,
                entity.character_start,
                entity.character_end,
                entity.confidence_basis_points,
            ])
            .map_err(|error| {
                AppError::Storage(format!("Failed to persist entity mention: {error}"))
            })?;
    }
    Ok(())
}

fn insert_relationships(
    transaction: &Transaction<'_>,
    document: &AnalyzedDocument,
) -> Result<(), AppError> {
    let mut statement = transaction
        .prepare_cached(
            "INSERT INTO corpus_entity_relationships (
                portal_key, document_id, left_entity_id, right_entity_id, relationship_kind,
                segment_index, character_start, character_end, confidence_basis_points
             ) VALUES (?1, ?2, ?3, ?4, 'CO_OCCURS_IN_SEGMENT', ?5, ?6, ?7, ?8)",
        )
        .map_err(|error| {
            AppError::Storage(format!("Failed to prepare entity relationships: {error}"))
        })?;
    for relationship in &document.relationships {
        statement
            .execute(params![
                document.portal_key,
                document.document_id,
                relationship.left_entity_id,
                relationship.right_entity_id,
                relationship.segment_index,
                relationship.character_start,
                relationship.character_end,
                relationship.confidence_basis_points,
            ])
            .map_err(|error| {
                AppError::Storage(format!("Failed to persist entity relationship: {error}"))
            })?;
    }
    Ok(())
}

fn insert_entity_truncations(
    transaction: &Transaction<'_>,
    document: &AnalyzedDocument,
) -> Result<(), AppError> {
    let mut statement = transaction
        .prepare_cached(
            "INSERT INTO corpus_entity_truncations (portal_key, document_id, entity_kind, truncated)
             VALUES (?1, ?2, ?3, ?4)",
        )
        .map_err(|error| {
            AppError::Storage(format!("Failed to prepare corpus entity truncations: {error}"))
        })?;
    for kind in ENTITY_KINDS {
        let truncated = document
            .entity_truncations
            .get(&kind)
            .copied()
            .unwrap_or(false);
        statement
            .execute(params![
                document.portal_key,
                document.document_id,
                kind.as_str(),
                truncated
            ])
            .map_err(|error| {
                AppError::Storage(format!("Failed to persist entity truncation: {error}"))
            })?;
    }
    Ok(())
}

fn insert_lsh_bands(
    transaction: &Transaction<'_>,
    document: &AnalyzedDocument,
) -> Result<(), AppError> {
    let mut statement = transaction
        .prepare_cached(
            "INSERT INTO corpus_lsh_bands (portal_key, document_id, band_index, band_value)
             VALUES (?1, ?2, ?3, ?4)",
        )
        .map_err(|error| AppError::Storage(format!("Failed to prepare LSH bands: {error}")))?;
    for band_index in 0..4_u8 {
        let band_value =
            ((document.document_simhash >> (u32::from(band_index) * 16)) & 0xffff) as u16;
        statement
            .execute(params![
                document.portal_key,
                document.document_id,
                band_index,
                band_value
            ])
            .map_err(|error| AppError::Storage(format!("Failed to persist LSH band: {error}")))?;
    }
    Ok(())
}

fn load_patterns(
    connection: &Connection,
    query: &CorpusPatternQuery,
) -> Result<Vec<CorpusPattern>, AppError> {
    let portal_key = query.portal_key.as_deref();
    let mut patterns = Vec::new();
    if query_requests_any(query, &[CorpusPatternKind::ExactDuplicate]) {
        load_exact_duplicate_patterns(connection, portal_key, query, &mut patterns)?;
    }
    if query_requests_any(
        query,
        &[
            CorpusPatternKind::SharedStatute,
            CorpusPatternKind::SharedEntity,
            CorpusPatternKind::TemporalCluster,
        ],
    ) {
        let mut category = Vec::new();
        load_shared_entity_patterns(connection, portal_key, query, &mut category)?;
        patterns.extend(category);
    }
    if query_requests_any(query, &[CorpusPatternKind::EntityRelationship]) {
        let mut category = Vec::new();
        load_shared_relationship_patterns(connection, portal_key, query, &mut category)?;
        patterns.extend(category);
    }
    if query_requests_any(query, &[CorpusPatternKind::NearDuplicate]) {
        let mut category = Vec::new();
        load_near_duplicate_patterns(connection, portal_key, query, &mut category)?;
        patterns.extend(category);
    }
    if query_requests_any(query, &[CorpusPatternKind::SharedTerm]) {
        let mut category = Vec::new();
        load_shared_term_patterns(connection, portal_key, query, &mut category)?;
        patterns.extend(category);
    }
    patterns.retain(|pattern| {
        let searchable = format!("{} {}", pattern.title, pattern.explanation).to_lowercase();
        pattern.confidence_basis_points >= query.minimum_confidence_basis_points
            && pattern.evidence.len() >= query.minimum_evidence_count
            && query
                .required_terms
                .iter()
                .all(|term| searchable.contains(&term.trim().to_lowercase()))
    });
    patterns.sort_by(|left, right| {
        right
            .confidence_basis_points
            .cmp(&left.confidence_basis_points)
            .then_with(|| right.evidence.len().cmp(&left.evidence.len()))
            .then_with(|| left.pattern_id.cmp(&right.pattern_id))
    });
    patterns.truncate(query.result_limit);
    Ok(patterns)
}

fn query_requests_any(query: &CorpusPatternQuery, kinds: &[CorpusPatternKind]) -> bool {
    query.pattern_kinds.is_empty()
        || query
            .pattern_kinds
            .iter()
            .any(|requested| kinds.contains(requested))
}

fn pattern_kind_requested(query: &CorpusPatternQuery, kind: CorpusPatternKind) -> bool {
    query.pattern_kinds.is_empty() || query.pattern_kinds.contains(&kind)
}

fn load_exact_duplicate_patterns(
    connection: &Connection,
    portal_key: Option<&str>,
    query: &CorpusPatternQuery,
    patterns: &mut Vec<CorpusPattern>,
) -> Result<(), AppError> {
    let mut statement = connection
        .prepare(
            "SELECT extracted_text_sha256
             FROM corpus_documents
             WHERE (?1 IS NULL OR portal_key = ?1)
               AND (?4 IS NULL OR indexed_at >= ?4)
               AND (?5 IS NULL OR indexed_at <= ?5)
             GROUP BY extracted_text_sha256
             HAVING COUNT(*) >= ?2
             ORDER BY COUNT(*) DESC
             LIMIT ?3",
        )
        .map_err(|error| {
            AppError::Storage(format!("Failed to prepare duplicate query: {error}"))
        })?;
    let hashes = statement
        .query_map(
            params![
                portal_key,
                query.minimum_evidence_count,
                query.result_limit,
                query.indexed_from,
                query.indexed_to
            ],
            |row| row.get::<_, String>(0),
        )
        .map_err(|error| AppError::Storage(format!("Failed to query duplicate hashes: {error}")))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| AppError::Storage(format!("Failed to decode duplicate hash: {error}")))?;
    for hash in hashes {
        let evidence = load_documents_for_value(
            connection,
            "extracted_text_sha256",
            &hash,
            query,
            "Exact extracted-text SHA-256 match",
        )?;
        patterns.push(CorpusPattern {
            pattern_id: stable_id(&format!("exact|{hash}")),
            kind: CorpusPatternKind::ExactDuplicate,
            title: format!(
                "{} documents contain identical extracted text",
                evidence.len()
            ),
            explanation:
                "The extracted-text SHA-256 digest is identical across every cited document."
                    .to_string(),
            confidence_basis_points: 10_000,
            evidence,
            algorithm_version: CORPUS_ALGORITHM_VERSION.to_string(),
        });
    }
    Ok(())
}

fn load_shared_entity_patterns(
    connection: &Connection,
    portal_key: Option<&str>,
    query: &CorpusPatternQuery,
    patterns: &mut Vec<CorpusPattern>,
) -> Result<(), AppError> {
    let remaining = query.result_limit.saturating_sub(patterns.len());
    let include_statutes = pattern_kind_requested(query, CorpusPatternKind::SharedStatute);
    let include_dates = pattern_kind_requested(query, CorpusPatternKind::TemporalCluster);
    let include_other_entities = pattern_kind_requested(query, CorpusPatternKind::SharedEntity);
    let entities = if portal_key.is_none()
        && query.indexed_from.is_none()
        && query.indexed_to.is_none()
    {
        let mut statement = connection
            .prepare(
                "SELECT entities.entity_id, entities.entity_kind, entities.display_value
                 FROM corpus_entities AS entities
                 INNER JOIN corpus_entity_document_counts AS counts
                   ON counts.entity_id = entities.entity_id
                 WHERE counts.document_count >= ?1
                   AND ((?3 AND entities.entity_kind = 'STATUTE')
                     OR (?4 AND entities.entity_kind = 'DATE')
                     OR (?5 AND entities.entity_kind NOT IN ('STATUTE', 'DATE')))
                 ORDER BY counts.document_count DESC, entities.entity_id
                 LIMIT ?2",
            )
            .map_err(|error| {
                AppError::Storage(format!(
                    "Failed to prepare materialized entity query: {error}"
                ))
            })?;
        statement
            .query_map(
                params![
                    query.minimum_evidence_count,
                    remaining,
                    include_statutes,
                    include_dates,
                    include_other_entities
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .map_err(|error| {
                AppError::Storage(format!("Failed to query materialized entities: {error}"))
            })?
            .collect::<duckdb::Result<Vec<_>>>()
            .map_err(|error| {
                AppError::Storage(format!("Failed to decode materialized entities: {error}"))
            })?
    } else {
        let mut statement = connection
            .prepare(
            "SELECT entities.entity_id, entities.entity_kind, entities.display_value
             FROM corpus_entities AS entities
             INNER JOIN corpus_entity_mentions AS mentions ON mentions.entity_id = entities.entity_id
             INNER JOIN corpus_documents AS documents
               ON documents.portal_key = mentions.portal_key AND documents.document_id = mentions.document_id
             WHERE (?1 IS NULL OR mentions.portal_key = ?1)
               AND (?4 IS NULL OR documents.indexed_at >= ?4)
               AND (?5 IS NULL OR documents.indexed_at <= ?5)
               AND ((?6 AND entities.entity_kind = 'STATUTE')
                 OR (?7 AND entities.entity_kind = 'DATE')
                 OR (?8 AND entities.entity_kind NOT IN ('STATUTE', 'DATE')))
             GROUP BY entities.entity_id, entities.entity_kind, entities.display_value
             HAVING COUNT(DISTINCT mentions.portal_key || ':' || mentions.document_id) >= ?2
             ORDER BY COUNT(DISTINCT mentions.portal_key || ':' || mentions.document_id) DESC
             LIMIT ?3",
            )
            .map_err(|error| AppError::Storage(format!("Failed to prepare entity query: {error}")))?;
        statement
            .query_map(
                params![
                    portal_key,
                    query.minimum_evidence_count,
                    remaining,
                    query.indexed_from,
                    query.indexed_to,
                    include_statutes,
                    include_dates,
                    include_other_entities
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .map_err(|error| {
                AppError::Storage(format!("Failed to query shared entities: {error}"))
            })?
            .collect::<duckdb::Result<Vec<_>>>()
            .map_err(|error| {
                AppError::Storage(format!("Failed to decode shared entity: {error}"))
            })?
    };
    for (entity_id, entity_kind, display_value) in entities {
        let evidence = load_entity_evidence(connection, &entity_id, query)?;
        let kind = if entity_kind == "STATUTE" {
            CorpusPatternKind::SharedStatute
        } else if entity_kind == "DATE" {
            CorpusPatternKind::TemporalCluster
        } else {
            CorpusPatternKind::SharedEntity
        };
        patterns.push(CorpusPattern {
            pattern_id: stable_id(&format!("entity|{entity_id}")),
            kind,
            title: format!("Shared {entity_kind}: {display_value}"),
            explanation: format!(
                "The normalized {entity_kind} appears in {} independently indexed documents.",
                evidence.len()
            ),
            confidence_basis_points: if matches!(kind, CorpusPatternKind::SharedEntity) {
                8_000
            } else {
                9_000
            },
            evidence,
            algorithm_version: CORPUS_ALGORITHM_VERSION.to_string(),
        });
    }
    Ok(())
}

fn load_shared_relationship_patterns(
    connection: &Connection,
    portal_key: Option<&str>,
    query: &CorpusPatternQuery,
    patterns: &mut Vec<CorpusPattern>,
) -> Result<(), AppError> {
    let mut statement = connection
        .prepare(
            "SELECT relationships.left_entity_id, relationships.right_entity_id,
                    left_entity.display_value, right_entity.display_value,
                    MIN(relationships.confidence_basis_points)
             FROM corpus_entity_relationships AS relationships
             INNER JOIN corpus_documents AS documents
               ON documents.portal_key = relationships.portal_key
              AND documents.document_id = relationships.document_id
             INNER JOIN corpus_entities AS left_entity
               ON left_entity.entity_id = relationships.left_entity_id
             INNER JOIN corpus_entities AS right_entity
               ON right_entity.entity_id = relationships.right_entity_id
             WHERE (?1 IS NULL OR relationships.portal_key = ?1)
               AND (?4 IS NULL OR documents.indexed_at >= ?4)
               AND (?5 IS NULL OR documents.indexed_at <= ?5)
             GROUP BY relationships.left_entity_id, relationships.right_entity_id,
                      left_entity.display_value, right_entity.display_value
             HAVING COUNT(DISTINCT relationships.portal_key || ':' || relationships.document_id) >= ?2
             ORDER BY COUNT(DISTINCT relationships.portal_key || ':' || relationships.document_id) DESC,
                      relationships.left_entity_id, relationships.right_entity_id
             LIMIT ?3",
        )
        .map_err(|error| {
            AppError::Storage(format!("Failed to prepare entity-relationship query: {error}"))
        })?;
    let relationships = statement
        .query_map(
            params![
                portal_key,
                query.minimum_evidence_count,
                query.result_limit,
                query.indexed_from,
                query.indexed_to
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            },
        )
        .map_err(|error| {
            AppError::Storage(format!("Failed to query entity relationships: {error}"))
        })?
        .collect::<duckdb::Result<Vec<_>>>()
        .map_err(|error| {
            AppError::Storage(format!("Failed to decode entity relationship: {error}"))
        })?;
    for (left_id, right_id, left_display, right_display, confidence) in relationships {
        let evidence = load_relationship_evidence(
            connection,
            &left_id,
            &right_id,
            &left_display,
            &right_display,
            query,
        )?;
        patterns.push(CorpusPattern {
            pattern_id: stable_id(&format!("relationship|{left_id}|{right_id}")),
            kind: CorpusPatternKind::EntityRelationship,
            title: format!("Recurring relationship: {left_display} + {right_display}"),
            explanation: format!(
                "The two normalized entities occur in the same bounded source segment in {} independently indexed documents. Co-occurrence is not evidence of causation.",
                evidence.len()
            ),
            confidence_basis_points: u16::try_from(confidence).map_err(|_| {
                AppError::Storage(format!(
                    "Invalid relationship confidence loaded from the corpus: {confidence}"
                ))
            })?,
            evidence,
            algorithm_version: CORPUS_ALGORITHM_VERSION.to_string(),
        });
    }
    Ok(())
}

fn load_relationship_evidence(
    connection: &Connection,
    left_entity_id: &str,
    right_entity_id: &str,
    left_display: &str,
    right_display: &str,
    query: &CorpusPatternQuery,
) -> Result<Vec<CorpusEvidenceReference>, AppError> {
    let mut statement = connection
        .prepare(
            "SELECT relationships.portal_key, documents.request_id, relationships.document_id,
                    MIN(relationships.segment_index), MIN(relationships.character_start),
                    MAX(relationships.character_end), documents.extracted_text_sha256
             FROM corpus_entity_relationships AS relationships
             INNER JOIN corpus_documents AS documents
               ON documents.portal_key = relationships.portal_key
              AND documents.document_id = relationships.document_id
             WHERE relationships.left_entity_id = ?1
               AND relationships.right_entity_id = ?2
               AND (?3 IS NULL OR relationships.portal_key = ?3)
               AND (?5 IS NULL OR documents.indexed_at >= ?5)
               AND (?6 IS NULL OR documents.indexed_at <= ?6)
             GROUP BY relationships.portal_key, documents.request_id,
                      relationships.document_id, documents.extracted_text_sha256
             ORDER BY relationships.portal_key, relationships.document_id
             LIMIT ?4",
        )
        .map_err(|error| {
            AppError::Storage(format!("Failed to prepare relationship evidence: {error}"))
        })?;
    statement
        .query_map(
            params![
                left_entity_id,
                right_entity_id,
                query.portal_key,
                query.result_limit,
                query.indexed_from,
                query.indexed_to
            ],
            |row| {
                Ok(CorpusEvidenceReference {
                    portal_key: row.get(0)?,
                    request_id: row.get(1)?,
                    document_id: row.get(2)?,
                    segment_index: Some(row.get(3)?),
                    character_start: Some(row.get(4)?),
                    character_end: Some(row.get(5)?),
                    evidence_label: format!(
                        "Same-segment co-occurrence: {left_display} + {right_display}"
                    ),
                    content_sha256: row.get(6)?,
                })
            },
        )
        .map_err(|error| {
            AppError::Storage(format!("Failed to query relationship evidence: {error}"))
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            AppError::Storage(format!("Failed to decode relationship evidence: {error}"))
        })
}

fn load_near_duplicate_patterns(
    connection: &Connection,
    portal_key: Option<&str>,
    query: &CorpusPatternQuery,
    patterns: &mut Vec<CorpusPattern>,
) -> Result<(), AppError> {
    let remaining = query.result_limit.saturating_sub(patterns.len());
    let mut statement = connection
        .prepare(
            "SELECT DISTINCT
                left_doc.portal_key, left_doc.request_id, left_doc.document_id,
                left_doc.extracted_text_sha256, left_doc.simhash_hex,
                right_doc.portal_key, right_doc.request_id, right_doc.document_id,
                right_doc.extracted_text_sha256, right_doc.simhash_hex
             FROM corpus_lsh_bands AS left_band
             INNER JOIN corpus_lsh_bands AS right_band
               ON right_band.band_index = left_band.band_index
              AND right_band.band_value = left_band.band_value
              AND (right_band.portal_key > left_band.portal_key OR
                   (right_band.portal_key = left_band.portal_key AND right_band.document_id > left_band.document_id))
             INNER JOIN corpus_documents AS left_doc
               ON left_doc.portal_key = left_band.portal_key AND left_doc.document_id = left_band.document_id
             INNER JOIN corpus_documents AS right_doc
               ON right_doc.portal_key = right_band.portal_key AND right_doc.document_id = right_band.document_id
             WHERE (?1 IS NULL OR (left_doc.portal_key = ?1 AND right_doc.portal_key = ?1))
               AND left_doc.extracted_text_sha256 <> right_doc.extracted_text_sha256
               AND (?3 IS NULL OR (left_doc.indexed_at >= ?3 AND right_doc.indexed_at >= ?3))
               AND (?4 IS NULL OR (left_doc.indexed_at <= ?4 AND right_doc.indexed_at <= ?4))
             GROUP BY left_doc.portal_key, left_doc.request_id, left_doc.document_id,
                      left_doc.extracted_text_sha256, left_doc.simhash_hex,
                      right_doc.portal_key, right_doc.request_id, right_doc.document_id,
                      right_doc.extracted_text_sha256, right_doc.simhash_hex
             HAVING COUNT(*) >= 2
             LIMIT ?2",
        )
        .map_err(|error| AppError::Storage(format!("Failed to prepare near-duplicate query: {error}")))?;
    let rows = statement
        .query_map(
            params![
                portal_key,
                remaining.saturating_mul(8),
                query.indexed_from,
                query.indexed_to
            ],
            |row| {
                Ok((
                    PatternDocument {
                        portal_key: row.get(0)?,
                        request_id: row.get(1)?,
                        document_id: row.get(2)?,
                        content_sha256: row.get(3)?,
                        simhash: parse_simhash(row.get::<_, String>(4)?)?,
                    },
                    PatternDocument {
                        portal_key: row.get(5)?,
                        request_id: row.get(6)?,
                        document_id: row.get(7)?,
                        content_sha256: row.get(8)?,
                        simhash: parse_simhash(row.get::<_, String>(9)?)?,
                    },
                ))
            },
        )
        .map_err(|error| AppError::Storage(format!("Failed to query near duplicates: {error}")))?;
    for row in rows {
        let (left, right) = row.map_err(|error| {
            AppError::Storage(format!("Failed to decode near duplicate: {error}"))
        })?;
        let distance = (left.simhash ^ right.simhash).count_ones();
        if distance > 8 {
            continue;
        }
        let confidence = 9_500_u16.saturating_sub((distance as u16).saturating_mul(500));
        if confidence < query.minimum_confidence_basis_points {
            continue;
        }
        let identity = format!(
            "near|{}|{}|{}|{}",
            left.portal_key, left.document_id, right.portal_key, right.document_id
        );
        patterns.push(CorpusPattern {
            pattern_id: stable_id(&identity),
            kind: CorpusPatternKind::NearDuplicate,
            title: "Documents contain strongly similar language".to_string(),
            explanation: format!(
                "Locality-sensitive hashing produced a {distance}-bit SimHash distance (0 is identical; the review threshold is 8)."
            ),
            confidence_basis_points: confidence,
            evidence: vec![
                document_evidence(&left, "Near-duplicate fingerprint candidate"),
                document_evidence(&right, "Near-duplicate fingerprint candidate"),
            ],
            algorithm_version: CORPUS_ALGORITHM_VERSION.to_string(),
        });
        if patterns.len() >= query.result_limit {
            break;
        }
    }
    Ok(())
}

fn load_shared_term_patterns(
    connection: &Connection,
    portal_key: Option<&str>,
    query: &CorpusPatternQuery,
    patterns: &mut Vec<CorpusPattern>,
) -> Result<(), AppError> {
    let remaining = query.result_limit.saturating_sub(patterns.len());
    let mut statement = connection
        .prepare(
            "SELECT term
             FROM corpus_term_postings AS postings
             INNER JOIN corpus_documents AS documents
               ON documents.portal_key = postings.portal_key AND documents.document_id = postings.document_id
             WHERE (?1 IS NULL OR postings.portal_key = ?1)
               AND (?4 IS NULL OR documents.indexed_at >= ?4)
               AND (?5 IS NULL OR documents.indexed_at <= ?5)
             GROUP BY term
             HAVING COUNT(DISTINCT postings.portal_key || ':' || postings.document_id) >= ?2
             ORDER BY COUNT(DISTINCT postings.portal_key || ':' || postings.document_id) DESC, SUM(frequency) DESC
             LIMIT ?3",
        )
        .map_err(|error| {
            AppError::Storage(format!("Failed to prepare shared-term query: {error}"))
        })?;
    let terms = statement
        .query_map(
            params![
                portal_key,
                query.minimum_evidence_count,
                remaining,
                query.indexed_from,
                query.indexed_to
            ],
            |row| row.get::<_, String>(0),
        )
        .map_err(|error| AppError::Storage(format!("Failed to query shared terms: {error}")))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| AppError::Storage(format!("Failed to decode shared term: {error}")))?;
    for term in terms {
        let evidence = load_term_evidence(connection, &term, query)?;
        patterns.push(CorpusPattern {
            pattern_id: stable_id(&format!("term|{term}")),
            kind: CorpusPatternKind::SharedTerm,
            title: format!("Recurring corpus term: {term}"),
            explanation: format!(
                "The normalized term occurs repeatedly in {} independently indexed documents.",
                evidence.len()
            ),
            confidence_basis_points: 7_000,
            evidence,
            algorithm_version: CORPUS_ALGORITHM_VERSION.to_string(),
        });
    }
    Ok(())
}

fn load_documents_for_value(
    connection: &Connection,
    column: &str,
    value: &str,
    query: &CorpusPatternQuery,
    label: &str,
) -> Result<Vec<CorpusEvidenceReference>, AppError> {
    if column != "extracted_text_sha256" {
        return Err(AppError::Internal(
            "Unsupported corpus evidence column".to_string(),
        ));
    }
    let mut statement = connection
        .prepare(
            "SELECT portal_key, request_id, document_id, extracted_text_sha256
             FROM corpus_documents
             WHERE extracted_text_sha256 = ?1 AND (?2 IS NULL OR portal_key = ?2)
               AND (?4 IS NULL OR indexed_at >= ?4)
               AND (?5 IS NULL OR indexed_at <= ?5)
             ORDER BY portal_key, document_id
             LIMIT ?3",
        )
        .map_err(|error| {
            AppError::Storage(format!("Failed to prepare corpus evidence: {error}"))
        })?;
    let evidence = statement
        .query_map(
            params![
                value,
                query.portal_key,
                query.result_limit,
                query.indexed_from,
                query.indexed_to
            ],
            |row| {
                Ok(CorpusEvidenceReference {
                    portal_key: row.get(0)?,
                    request_id: row.get(1)?,
                    document_id: row.get(2)?,
                    segment_index: None,
                    character_start: None,
                    character_end: None,
                    evidence_label: label.to_string(),
                    content_sha256: row.get(3)?,
                })
            },
        )
        .map_err(|error| AppError::Storage(format!("Failed to query corpus evidence: {error}")))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| AppError::Storage(format!("Failed to decode corpus evidence: {error}")))?;
    Ok(evidence)
}

fn load_entity_evidence(
    connection: &Connection,
    entity_id: &str,
    query: &CorpusPatternQuery,
) -> Result<Vec<CorpusEvidenceReference>, AppError> {
    let mut statement = connection
        .prepare(
            "SELECT mentions.portal_key, documents.request_id, mentions.document_id,
                    mentions.segment_index, mentions.character_start, mentions.character_end,
                    documents.extracted_text_sha256, entities.display_value
             FROM corpus_entity_mentions AS mentions
             INNER JOIN corpus_documents AS documents
               ON documents.portal_key = mentions.portal_key AND documents.document_id = mentions.document_id
             INNER JOIN corpus_entities AS entities ON entities.entity_id = mentions.entity_id
             WHERE mentions.entity_id = ?1 AND (?2 IS NULL OR mentions.portal_key = ?2)
               AND (?4 IS NULL OR documents.indexed_at >= ?4)
               AND (?5 IS NULL OR documents.indexed_at <= ?5)
             GROUP BY mentions.portal_key, documents.request_id, mentions.document_id,
                      mentions.segment_index, mentions.character_start, mentions.character_end,
                      documents.extracted_text_sha256, entities.display_value
             ORDER BY mentions.portal_key, mentions.document_id
             LIMIT ?3",
        )
        .map_err(|error| AppError::Storage(format!("Failed to prepare entity evidence: {error}")))?;
    let evidence = statement
        .query_map(
            params![
                entity_id,
                query.portal_key,
                query.result_limit,
                query.indexed_from,
                query.indexed_to
            ],
            |row| {
                Ok(CorpusEvidenceReference {
                    portal_key: row.get(0)?,
                    request_id: row.get(1)?,
                    document_id: row.get(2)?,
                    segment_index: Some(row.get(3)?),
                    character_start: Some(row.get(4)?),
                    character_end: Some(row.get(5)?),
                    content_sha256: row.get(6)?,
                    evidence_label: format!(
                        "Shared normalized entity: {}",
                        row.get::<_, String>(7)?
                    ),
                })
            },
        )
        .map_err(|error| AppError::Storage(format!("Failed to query entity evidence: {error}")))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| AppError::Storage(format!("Failed to decode entity evidence: {error}")))?;
    Ok(evidence)
}

fn load_term_evidence(
    connection: &Connection,
    term: &str,
    query: &CorpusPatternQuery,
) -> Result<Vec<CorpusEvidenceReference>, AppError> {
    let mut statement = connection
        .prepare(
            "SELECT postings.portal_key, documents.request_id, postings.document_id,
                    documents.extracted_text_sha256, postings.frequency
             FROM corpus_term_postings AS postings
             INNER JOIN corpus_documents AS documents
               ON documents.portal_key = postings.portal_key AND documents.document_id = postings.document_id
             WHERE postings.term = ?1 AND (?2 IS NULL OR postings.portal_key = ?2)
               AND (?4 IS NULL OR documents.indexed_at >= ?4)
               AND (?5 IS NULL OR documents.indexed_at <= ?5)
             ORDER BY postings.frequency DESC, postings.portal_key, postings.document_id
             LIMIT ?3",
        )
        .map_err(|error| AppError::Storage(format!("Failed to prepare term evidence: {error}")))?;
    let evidence = statement
        .query_map(
            params![
                term,
                query.portal_key,
                query.result_limit,
                query.indexed_from,
                query.indexed_to
            ],
            |row| {
                let frequency = row.get::<_, usize>(4)?;
                Ok(CorpusEvidenceReference {
                    portal_key: row.get(0)?,
                    request_id: row.get(1)?,
                    document_id: row.get(2)?,
                    segment_index: None,
                    character_start: None,
                    character_end: None,
                    content_sha256: row.get(3)?,
                    evidence_label: format!("Term '{term}' occurs {frequency} times"),
                })
            },
        )
        .map_err(|error| AppError::Storage(format!("Failed to query term evidence: {error}")))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| AppError::Storage(format!("Failed to decode term evidence: {error}")))?;
    Ok(evidence)
}

fn document_evidence(document: &PatternDocument, label: &str) -> CorpusEvidenceReference {
    CorpusEvidenceReference {
        portal_key: document.portal_key.clone(),
        request_id: document.request_id,
        document_id: document.document_id,
        segment_index: None,
        character_start: None,
        character_end: None,
        evidence_label: label.to_string(),
        content_sha256: document.content_sha256.clone(),
    }
}

fn parse_simhash(value: String) -> duckdb::Result<u64> {
    u64::from_str_radix(&value, 16).map_err(|error| {
        duckdb::Error::FromSqlConversionFailure(0, duckdb::types::Type::Text, Box::new(error))
    })
}

fn stable_id(value: &str) -> String {
    digest_hex(Sha256::digest(value.as_bytes()))
}

fn nonnegative_usize(value: i64, label: &str) -> Result<usize, AppError> {
    usize::try_from(value)
        .map_err(|_| AppError::Storage(format!("Corpus {label} was negative or too large")))
}

fn is_stopword(value: &str) -> bool {
    matches!(
        value,
        "about"
            | "after"
            | "again"
            | "also"
            | "been"
            | "before"
            | "being"
            | "between"
            | "both"
            | "could"
            | "does"
            | "each"
            | "from"
            | "have"
            | "into"
            | "more"
            | "most"
            | "other"
            | "over"
            | "shall"
            | "should"
            | "such"
            | "than"
            | "that"
            | "their"
            | "there"
            | "these"
            | "they"
            | "this"
            | "those"
            | "through"
            | "under"
            | "upon"
            | "were"
            | "what"
            | "when"
            | "where"
            | "which"
            | "while"
            | "will"
            | "with"
            | "would"
            | "your"
    )
}

const CORPUS_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS corpus_schema_version (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    version BIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS corpus_documents (
    portal_key TEXT NOT NULL,
    request_id BIGINT NOT NULL,
    document_id BIGINT NOT NULL,
    raw_file_sha256 TEXT NOT NULL CHECK (length(raw_file_sha256) = 64),
    extracted_text_sha256 TEXT NOT NULL CHECK (length(extracted_text_sha256) = 64),
    character_count BIGINT NOT NULL CHECK (character_count >= 0),
    simhash_hex TEXT NOT NULL CHECK (length(simhash_hex) = 16),
    relationships_truncated BOOLEAN NOT NULL,
    algorithm_version TEXT NOT NULL,
    indexed_at TEXT NOT NULL,
    PRIMARY KEY (portal_key, document_id)
);

CREATE INDEX IF NOT EXISTS idx_corpus_documents_content_hash
ON corpus_documents(extracted_text_sha256, portal_key, document_id);

CREATE TABLE IF NOT EXISTS corpus_segments (
    portal_key TEXT NOT NULL,
    document_id BIGINT NOT NULL,
    segment_index BIGINT NOT NULL,
    character_start BIGINT NOT NULL,
    character_end BIGINT NOT NULL,
    content_sha256 TEXT NOT NULL CHECK (length(content_sha256) = 64),
    simhash_hex TEXT NOT NULL CHECK (length(simhash_hex) = 16),
    PRIMARY KEY (portal_key, document_id, segment_index),
    FOREIGN KEY (portal_key, document_id)
        REFERENCES corpus_documents(portal_key, document_id)
);

CREATE INDEX IF NOT EXISTS idx_corpus_segments_content_hash
ON corpus_segments(content_sha256, portal_key, document_id);

CREATE TABLE IF NOT EXISTS corpus_term_postings (
    portal_key TEXT NOT NULL,
    document_id BIGINT NOT NULL,
    term TEXT NOT NULL,
    frequency BIGINT NOT NULL CHECK (frequency > 0),
    PRIMARY KEY (portal_key, document_id, term),
    FOREIGN KEY (portal_key, document_id)
        REFERENCES corpus_documents(portal_key, document_id)
);

CREATE INDEX IF NOT EXISTS idx_corpus_terms_lookup
ON corpus_term_postings(term, portal_key, document_id);

CREATE TABLE IF NOT EXISTS corpus_entities (
    entity_id TEXT PRIMARY KEY,
    entity_kind TEXT NOT NULL CHECK (
        entity_kind IN ('STATUTE', 'DATE', 'EMAIL_DOMAIN', 'URL_HOST', 'ACRONYM', 'ORGANIZATION')
    ),
    normalized_value TEXT NOT NULL,
    display_value TEXT NOT NULL,
    UNIQUE (entity_kind, normalized_value)
);

CREATE TABLE IF NOT EXISTS corpus_entity_mentions (
    portal_key TEXT NOT NULL,
    document_id BIGINT NOT NULL,
    entity_id TEXT NOT NULL,
    segment_index BIGINT NOT NULL,
    character_start BIGINT NOT NULL,
    character_end BIGINT NOT NULL,
    confidence_basis_points BIGINT NOT NULL,
    PRIMARY KEY (portal_key, document_id, entity_id, character_start),
    FOREIGN KEY (portal_key, document_id)
        REFERENCES corpus_documents(portal_key, document_id),
    FOREIGN KEY (entity_id) REFERENCES corpus_entities(entity_id)
);

CREATE TABLE IF NOT EXISTS corpus_entity_document_counts (
    entity_id TEXT PRIMARY KEY,
    document_count BIGINT NOT NULL CHECK (document_count > 0),
    FOREIGN KEY (entity_id) REFERENCES corpus_entities(entity_id)
);

CREATE INDEX IF NOT EXISTS idx_corpus_mentions_entity
ON corpus_entity_mentions(entity_id, portal_key, document_id);

CREATE TABLE IF NOT EXISTS corpus_entity_relationships (
    portal_key TEXT NOT NULL,
    document_id BIGINT NOT NULL,
    left_entity_id TEXT NOT NULL,
    right_entity_id TEXT NOT NULL,
    relationship_kind TEXT NOT NULL CHECK (relationship_kind = 'CO_OCCURS_IN_SEGMENT'),
    segment_index BIGINT NOT NULL,
    character_start BIGINT NOT NULL,
    character_end BIGINT NOT NULL,
    confidence_basis_points BIGINT NOT NULL CHECK (confidence_basis_points BETWEEN 0 AND 10000),
    PRIMARY KEY (
        portal_key, document_id, left_entity_id, right_entity_id, relationship_kind, segment_index
    ),
    CHECK (left_entity_id < right_entity_id),
    FOREIGN KEY (portal_key, document_id)
        REFERENCES corpus_documents(portal_key, document_id),
    FOREIGN KEY (left_entity_id) REFERENCES corpus_entities(entity_id),
    FOREIGN KEY (right_entity_id) REFERENCES corpus_entities(entity_id)
);

CREATE INDEX IF NOT EXISTS idx_corpus_relationships_pair
ON corpus_entity_relationships(left_entity_id, right_entity_id, portal_key, document_id);

-- One row per document per entity kind that was ever emitted (see ENTITY_KINDS), stating whether
-- that kind's extraction hit MAXIMUM_ENTITIES_PER_KIND and stopped early. A reader must be able to
-- tell "this document has no organizations" from "we stopped looking" (docs/local-analysis-
-- architecture.md section 2), which a missing row cannot express, so every kind gets a row.
CREATE TABLE IF NOT EXISTS corpus_entity_truncations (
    portal_key TEXT NOT NULL,
    document_id BIGINT NOT NULL,
    entity_kind TEXT NOT NULL,
    truncated BOOLEAN NOT NULL,
    PRIMARY KEY (portal_key, document_id, entity_kind),
    FOREIGN KEY (portal_key, document_id)
        REFERENCES corpus_documents(portal_key, document_id)
);

CREATE TABLE IF NOT EXISTS corpus_lsh_bands (
    portal_key TEXT NOT NULL,
    document_id BIGINT NOT NULL,
    band_index BIGINT NOT NULL CHECK (band_index BETWEEN 0 AND 3),
    band_value BIGINT NOT NULL CHECK (band_value BETWEEN 0 AND 65535),
    PRIMARY KEY (portal_key, document_id, band_index),
    FOREIGN KEY (portal_key, document_id)
        REFERENCES corpus_documents(portal_key, document_id)
);

CREATE INDEX IF NOT EXISTS idx_corpus_lsh_lookup
ON corpus_lsh_bands(band_index, band_value, portal_key, document_id);
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn input(document_id: u64, text: &str) -> CorpusDocumentInput {
        CorpusDocumentInput {
            portal_key: "records".to_string(),
            request_id: document_id,
            document_id,
            raw_file_sha256: format!("{:064x}", document_id),
            extracted_text_sha256: digest_hex(Sha256::digest(text.as_bytes())),
            text: text.to_string(),
        }
    }

    fn dates_in(text: &str) -> Vec<(String, u16)> {
        let mut found = analyze_document(input(1, text))
            .unwrap()
            .entities
            .into_iter()
            .filter(|entity| entity.kind == CorpusEntityKind::Date)
            .map(|entity| (entity.normalized_value, entity.confidence_basis_points))
            .collect::<Vec<_>>();
        found.sort();
        found.dedup();
        found
    }

    /// The surface forms here are taken verbatim from released records in the corpus this parser
    /// was written against; the ISO-only predecessor read none of them, which left 61 of 70 indexed
    /// documents with no date at all. The negative cases matter as much as the positive ones: a
    /// numeric date carries no evidence that it is a date rather than a serial number, so the
    /// plausible-year window and the refusal to transpose a day-first form are what keep this from
    /// manufacturing dates that were never written.
    #[test]
    fn dates_are_read_in_the_forms_agencies_actually_write() {
        assert_eq!(
            dates_in("Sent: Tuesday, February 7, 2023 11:34 PM"),
            vec![("2023-02-07".to_string(), 9_800)]
        );
        assert_eq!(
            dates_in("On or about 4/22/2022, City issued the notice."),
            vec![("2022-04-22".to_string(), 9_000)]
        );
        assert_eq!(
            dates_in("Fri 5/10/2024 11:42 AM"),
            vec![("2024-05-10".to_string(), 9_000)]
        );
        assert_eq!(
            dates_in("emailed to you on March 3, 2024 and again Mar. 4 2024"),
            vec![
                ("2024-03-03".to_string(), 9_800),
                ("2024-03-04".to_string(), 9_800),
            ]
        );
        assert_eq!(
            dates_in("filed 17 March 2023 and served 3rd September, 2019"),
            vec![
                ("2019-09-03".to_string(), 9_800),
                ("2023-03-17".to_string(), 9_800),
            ]
        );
        assert_eq!(
            dates_in("request logged 2026-08-24"),
            vec![("2026-08-24".to_string(), 10_000)]
        );

        // A two-digit year has had its century inferred, and says so.
        assert_eq!(
            dates_in("invoice dated 5/25/22"),
            vec![("2022-05-25".to_string(), 8_000)]
        );

        // A phone number, a hyphenated identifier, and a fiscal-year range are not dates.
        assert!(dates_in("call 858-514-3191 or 5-14-3191 about FY 2023-2024").is_empty());

        // The whole alphabetic run is matched, so this is not an abbreviation of August.
        assert!(dates_in("augment 5, 2023 with the attached exhibit").is_empty());

        // A day-first numeric form is refused rather than silently transposed into 3 May.
        assert!(dates_in("dated 17/3/2023").is_empty());
    }

    #[test]
    fn analysis_is_deterministic_and_retains_source_offsets() {
        let text = "Under 42 U.S.C. 1983, the Regional Task Force on Homelessness (RTFH) requested review from analyst@rtfhsd.org at https://www.rtfhsd.org/data under section 6254 on 2026-08-24.";
        let first = analyze_document(input(1, text)).unwrap();
        let second = analyze_document(input(1, text)).unwrap();
        assert_eq!(first.document_simhash, second.document_simhash);
        assert_eq!(first.segments.len(), 1);
        assert!(first.entities.iter().any(|entity| {
            entity.kind == CorpusEntityKind::Statute && entity.normalized_value == "42 u.s.c. 1983"
        }));
        assert!(first.entities.iter().any(|entity| {
            entity.kind == CorpusEntityKind::Date && entity.normalized_value == "2026-08-24"
        }));
        assert!(first.entities.iter().any(|entity| {
            entity.kind == CorpusEntityKind::Organization
                && entity
                    .normalized_value
                    .contains("regional task force on homelessness")
        }));
        assert!(first.entities.iter().any(|entity| {
            entity.kind == CorpusEntityKind::EmailDomain && entity.display_value == "@rtfhsd.org"
        }));
        assert!(first.entities.iter().any(|entity| {
            entity.kind == CorpusEntityKind::UrlHost && entity.normalized_value == "www.rtfhsd.org"
        }));
        assert!(first.entities.iter().any(|entity| {
            entity.kind == CorpusEntityKind::Acronym && entity.normalized_value == "rtfh"
        }));
        for entity in first.entities {
            assert!(entity.character_start < entity.character_end);
            assert!(entity.character_end <= text.chars().count());
        }
    }

    /// An index written under an older schema must not make the application unlaunchable.
    ///
    /// DuckDB binds a foreign key against the catalog it already holds, even under
    /// `CREATE TABLE IF NOT EXISTS` on a table that already exists, so a legacy `BIGINT`
    /// `document_id` fails the create of every table referencing it. That error is raised from the
    /// setup hook, which panics: the shipped app aborted at launch, before drawing a window, with
    /// no recovery short of deleting the file by hand. Nothing caught it because every other test
    /// here opens a fresh database.
    #[tokio::test]
    async fn a_corpus_index_written_under_an_older_schema_is_rebuilt_rather_than_fatal() {
        let directory = tempdir().expect("temp dir must be created");
        let path = directory.path().join("corpus.db");
        {
            let legacy = Connection::open(&path).expect("the legacy fixture must open");
            legacy
                .execute_batch(
                    "CREATE TABLE corpus_documents (
                        portal_key TEXT NOT NULL,
                        request_id BIGINT NOT NULL,
                        document_id BIGINT NOT NULL,
                        raw_file_sha256 TEXT NOT NULL,
                        extracted_text_sha256 TEXT NOT NULL,
                        character_count BIGINT NOT NULL,
                        simhash_hex TEXT NOT NULL,
                        algorithm_version TEXT NOT NULL,
                        indexed_at TEXT NOT NULL,
                        PRIMARY KEY (portal_key, document_id)
                    );",
                )
                .expect("the legacy schema must apply");
            legacy
                .execute(
                    "INSERT INTO corpus_documents VALUES
                     ('records', 1, 1, ?1, ?2, 12, '0123456789abcdef', 'legacy-v1', '2026-08-01T00:00:00Z')",
                    params![format!("{:064x}", 1), format!("{:064x}", 2)],
                )
                .expect("the legacy row must insert");
            // A sentinel that belongs to no schema version. If the whole file is replaced it
            // disappears; if only the known tables were dropped it would survive, and the
            // storage-version widening that caused the real crash would survive with it.
            legacy
                .execute_batch("CREATE TABLE legacy_sentinel (marker TEXT);")
                .expect("the sentinel must be created");
        }

        let index = CorpusIndex::open(&path)
            .expect("an index written under an older schema must be rebuilt, not abort startup");
        let status = index.status().await.expect("status must read");
        // Discarded rather than migrated: the core is canonical and reconciliation refills this.
        assert_eq!(status.indexed_document_count, 0);
        let sentinel_survived: bool = Connection::open(&path)
            .expect("the rebuilt index must open")
            .query_row(
                "SELECT COUNT(*) > 0 FROM duckdb_tables() WHERE table_name = 'legacy_sentinel'",
                [],
                |row| row.get(0),
            )
            .expect("the sentinel probe must run");
        assert!(
            !sentinel_survived,
            "the outdated index file must be replaced, not have its known tables dropped"
        );

        // And the rebuilt index accepts writes, not merely opens.
        index
            .index_document(input(1, "Section 6254 requires records review."))
            .await
            .expect("the rebuilt index must accept writes");
    }

    /// A write must be visible to every read that follows it.
    ///
    /// It was not. `read` used to open its own `Connection` to the corpus file, and a second
    /// DuckDB connection to a file this process already holds is a separate instance frozen at
    /// what it first saw. The first read returned the right count and every read after it
    /// returned that same count forever, so `status` under-reported the index, `query_patterns`
    /// returned stale investigative output, and the reconciliation loop in `setup` decided
    /// whether to re-index from an answer that stopped changing.
    ///
    /// The second assertion is the one that failed; the first passed even with the defect present,
    /// which is why nothing caught it.
    #[tokio::test]
    async fn a_write_is_visible_to_every_later_read() {
        let directory = tempdir().expect("temp dir must be created");
        let index = CorpusIndex::open(directory.path().join("corpus.db")).unwrap();

        index
            .index_document(input(1, "Section 6254 requires records review."))
            .await
            .unwrap();
        assert_eq!(
            index.status().await.unwrap().indexed_document_count,
            1,
            "no read had happened before this write"
        );

        index
            .index_document(input(2, "A second responsive record was released."))
            .await
            .unwrap();
        assert_eq!(
            index.status().await.unwrap().indexed_document_count,
            2,
            "a read happened before this write, and the write must still be visible"
        );
    }

    /// A portal document identifier does not fit in 32 bits, and DuckDB's `INTEGER` is 32 bits.
    ///
    /// A real source produced document id 4561106750446765, and every write of it failed with
    /// `Conversion Error: Type UINT64 with value 4561106750446765 can't be cast because the value
    /// is out of range for the destination type INT32`, so no document from that source ever
    /// reached the index. Every other test here uses identifiers small enough to fit.
    #[tokio::test]
    async fn a_document_identifier_wider_than_thirty_two_bits_is_indexed() {
        const WIDE_DOCUMENT_ID: u64 = 4_561_106_750_446_765;
        assert!(WIDE_DOCUMENT_ID > i64::from(i32::MAX) as u64);

        let directory = tempdir().expect("temp dir must be created");
        let index = CorpusIndex::open(directory.path().join("corpus.db")).unwrap();
        index
            .index_document(input(
                WIDE_DOCUMENT_ID,
                "Section 6254 requires records review of the encampment sweep notice.",
            ))
            .await
            .expect("a 53-bit document identifier must be indexable");
        assert_eq!(index.status().await.unwrap().indexed_document_count, 1);
    }

    #[tokio::test]
    async fn indexes_and_explains_cross_document_patterns() {
        let directory = tempdir().unwrap();
        let index = CorpusIndex::open(directory.path().join("corpus.db")).unwrap();
        let shared =
            "Section 6254 requires records review on 2026-08-24. recurringterm recurringterm.";
        index.index_document(input(1, shared)).await.unwrap();
        index.index_document(input(2, shared)).await.unwrap();
        index
            .index_document(input(
                3,
                "Section 6254 requires a public records review. recurringterm recurringterm.",
            ))
            .await
            .unwrap();

        let patterns = index
            .query_patterns(CorpusPatternQuery::default())
            .await
            .unwrap();
        assert!(patterns.iter().any(|pattern| {
            pattern.kind == CorpusPatternKind::ExactDuplicate && pattern.evidence.len() == 2
        }));
        assert!(patterns.iter().any(|pattern| {
            pattern.kind == CorpusPatternKind::SharedStatute && pattern.evidence.len() >= 2
        }));
        assert!(patterns.iter().any(|pattern| {
            pattern.kind == CorpusPatternKind::EntityRelationship
                && pattern.evidence.len() >= 2
                && pattern
                    .explanation
                    .contains("Co-occurrence is not evidence of causation")
        }));
        assert!(patterns.iter().all(|pattern| {
            !pattern.pattern_id.is_empty()
                && pattern
                    .evidence
                    .iter()
                    .all(|evidence| evidence.content_sha256.len() == 64)
        }));

        let high_confidence = index
            .query_patterns(CorpusPatternQuery {
                minimum_confidence_basis_points: 9_500,
                ..CorpusPatternQuery::default()
            })
            .await
            .unwrap();
        assert!(high_confidence
            .iter()
            .all(|pattern| pattern.confidence_basis_points >= 9_500));

        let future_only = index
            .query_patterns(CorpusPatternQuery {
                indexed_from: Some("2999-01-01T00:00:00Z".to_string()),
                ..CorpusPatternQuery::default()
            })
            .await
            .unwrap();
        assert!(future_only.is_empty());

        let relationship_recipe = index
            .query_patterns(CorpusPatternQuery {
                pattern_kinds: vec![CorpusPatternKind::EntityRelationship],
                required_terms: vec!["causation".to_string()],
                ..CorpusPatternQuery::default()
            })
            .await
            .unwrap();
        assert!(!relationship_recipe.is_empty());
        assert!(relationship_recipe
            .iter()
            .all(|pattern| pattern.kind == CorpusPatternKind::EntityRelationship));

        let invalid_recipe = index
            .query_patterns(CorpusPatternQuery {
                pattern_kinds: vec![CorpusPatternKind::SharedTerm, CorpusPatternKind::SharedTerm],
                ..CorpusPatternQuery::default()
            })
            .await;
        assert!(invalid_recipe.is_err());
    }
}

/// Offset-coordinate regression tests.
///
/// The corpus index carries two coordinate systems: byte offsets for safe Rust slicing, and
/// character offsets as the stable IPC/UI coordinate. An ASCII fixture cannot tell them apart,
/// so every case here uses multi-byte text on purpose.
#[cfg(test)]
mod offset_tests {
    use super::*;

    fn input(document_id: u64, text: &str) -> CorpusDocumentInput {
        CorpusDocumentInput {
            portal_key: "records".to_string(),
            request_id: document_id,
            document_id,
            raw_file_sha256: format!("{:064x}", document_id),
            extracted_text_sha256: digest_hex(Sha256::digest(text.as_bytes())),
            text: text.to_string(),
        }
    }

    fn slice_by_characters(text: &str, start: usize, end: usize) -> String {
        text.chars()
            .skip(start)
            .take(end.saturating_sub(start))
            .collect()
    }

    /// Lowercased alphanumeric words, so a comparison ignores punctuation that entity
    /// normalization drops while still catching a wrong or shifted range.
    fn alphanumeric_words(value: &str) -> Vec<String> {
        value
            .split(|character: char| !character.is_alphanumeric())
            .filter(|word| !word.is_empty())
            .map(str::to_lowercase)
            .collect()
    }

    // INVARIANT(corpus): `character_start`/`character_end` are Unicode scalar offsets into the
    // source text. Indexing the source by those offsets must return the entity itself. This is
    // the assertion the pre-existing test's name promises but does not make, and it is what would
    // catch a byte offset being written into a character field.
    #[test]
    fn entity_character_offsets_address_the_entity_in_multibyte_text() {
        // The accented prefix makes every byte offset larger than its character offset, so a
        // swapped pair cannot pass silently.
        let text = "Résumé données café — Under 42 U.S.C. 1983 the Regional Task Force on \
                    Homelessness (RTFH) contacted analyst@rtfhsd.org about 2026-08-24.";
        let analyzed = analyze_document(input(1, text)).expect("analysis should succeed");
        let character_count = text.chars().count();

        assert!(
            !analyzed.entities.is_empty(),
            "fixture produced no entities"
        );
        for entity in &analyzed.entities {
            assert!(
                entity.character_start < entity.character_end,
                "{} has an empty range",
                entity.kind,
            );
            assert!(
                entity.character_end <= character_count,
                "{} runs past the end of the document: {} > {character_count}",
                entity.kind,
                entity.character_end,
            );

            let addressed = slice_by_characters(text, entity.character_start, entity.character_end);
            let expected_length = entity.character_end - entity.character_start;
            assert_eq!(
                addressed.chars().count(),
                expected_length,
                "{} range does not resolve to {expected_length} characters",
                entity.kind,
            );

            // For the kinds whose recorded range is the value itself, the addressed text must
            // contain that value. URL_HOST and EMAIL_DOMAIN deliberately span more than their
            // value (the whole URL, and `local@domain`), so they are checked separately below.
            // Compared on alphanumeric content only: a range legitimately includes punctuation
            // that normalization strips, so raw equality would fail on a correct offset.
            if matches!(
                entity.kind,
                CorpusEntityKind::Statute
                    | CorpusEntityKind::Date
                    | CorpusEntityKind::Acronym
                    | CorpusEntityKind::Organization
            ) {
                assert_eq!(
                    alphanumeric_words(&addressed),
                    alphanumeric_words(&entity.normalized_value),
                    "{} range does not address its own value (addressed {addressed:?})",
                    entity.kind,
                );
            }
        }
    }

    // LIMIT(corpus): URL_HOST and EMAIL_DOMAIN record a range spanning more than the value they
    // report, so a UI highlighting the range highlights more text than the label describes.
    // Pinned as current behavior so narrowing it later is a deliberate change.
    #[test]
    fn url_and_email_ranges_deliberately_span_more_than_their_value() {
        let text = "Contact analyst@rtfhsd.org or visit https://www.rtfhsd.org/data/reports today.";
        let analyzed = analyze_document(input(2, text)).expect("analysis should succeed");

        let email = analyzed
            .entities
            .iter()
            .find(|entity| entity.kind == CorpusEntityKind::EmailDomain)
            .expect("an email domain entity");
        let addressed = slice_by_characters(text, email.character_start, email.character_end);
        assert!(addressed.contains("@rtfhsd.org"), "{addressed:?}");
        assert!(
            addressed.starts_with("analyst"),
            "range should include the local part"
        );

        let host = analyzed
            .entities
            .iter()
            .find(|entity| entity.kind == CorpusEntityKind::UrlHost)
            .expect("a url host entity");
        let addressed = slice_by_characters(text, host.character_start, host.character_end);
        assert!(
            addressed.starts_with("https://"),
            "range spans the whole URL: {addressed:?}"
        );
        assert!(addressed.contains(&host.normalized_value));
    }

    // Analysis must not panic on text whose byte and character lengths diverge sharply, and the
    // scanners must never produce an offset that fails to resolve.
    #[test]
    fn analysis_survives_non_latin_and_emoji_text() {
        for text in [
            "政府記録の要求 2026-08-24 について 42 U.S.C. 1983 を参照してください。",
            "Записи 2026-08-24 согласно 42 U.S.C. 1983 для analyst@rtfhsd.org.",
            "Records 📄 for 2026-08-24 🗂 under 42 U.S.C. 1983 via https://www.rtfhsd.org/x.",
        ] {
            let analyzed = analyze_document(input(3, text)).expect("analysis should succeed");
            let character_count = text.chars().count();
            for entity in &analyzed.entities {
                assert!(
                    entity.character_end <= character_count,
                    "{} overran {text:?}",
                    entity.kind,
                );
                let addressed =
                    slice_by_characters(text, entity.character_start, entity.character_end);
                assert_eq!(
                    addressed.chars().count(),
                    entity.character_end - entity.character_start,
                );
            }
        }
    }

    // INVARIANT(corpus): segment ranges are character offsets too, and must stay ordered and
    // within the document.
    #[test]
    fn segment_ranges_stay_ordered_and_within_the_document() {
        let mut text = String::from("Café ");
        for index in 0..400 {
            text.push_str(&format!(
                "Paragraph {index} concerns the municipal records request and its disposition. "
            ));
        }
        let analyzed = analyze_document(input(4, &text)).expect("analysis should succeed");
        let character_count = text.chars().count();

        assert!(
            analyzed.segments.len() > 1,
            "fixture should span several segments"
        );
        for segment in &analyzed.segments {
            assert!(segment.character_start < segment.character_end);
            assert!(
                segment.character_end <= character_count,
                "segment overran the document: {} > {character_count}",
                segment.character_end,
            );
        }
        for pair in analyzed.segments.windows(2) {
            assert!(
                pair[0].character_start < pair[1].character_start,
                "segments must advance",
            );
        }
    }

    // REGRESSION(corpus): segment coverage is NOT contiguous. A segment is bounded by its first and
    // last token, token-free windows are skipped outright, and neighbours share only
    // SEGMENT_OVERLAP_CHARACTERS, so an entity can sit inside no segment. That case used to report
    // segment 0 -- the top of the document -- pointing an investigator at the wrong evidence.
    // `push_entity_with_display` now resolves by best overlap, then by nearest segment.
    //
    // This replaces a test that fed 400 repetitions of "Paragraph N concerns the municipal records
    // request and its disposition." through `analyze_document` and asserted inside a loop over the
    // entities that came back. That prose contains no date, email, URL, statute, acronym, or
    // organization phrase, so it yields ZERO entities: the loop body never executed and the test
    // asserted nothing at all. Building the segments directly is what makes the gap reachable.
    #[test]
    fn an_entity_no_segment_contains_resolves_to_the_nearest_segment_not_segment_zero() {
        let segment =
            |index: usize, character_start: usize, character_end: usize| AnalyzedSegment {
                index,
                character_start,
                character_end,
                content_sha256: String::new(),
                simhash: 0,
            };
        // Deliberate gaps at 2_000..4_000 and 6_000..8_000, the shape `analyze_segments` produces
        // when it skips a token-free window.
        let segments = vec![
            segment(0, 0, 2_000),
            segment(1, 4_000, 6_000),
            segment(2, 8_000, 10_000),
        ];

        let resolve = |start: usize, end: usize| {
            let mut entities = Vec::new();
            let mut budget = EntityBudget::new();
            push_entity_with_display(
                &mut entities,
                &mut budget,
                &segments,
                CorpusEntityKind::Organization,
                "value".to_string(),
                "value".to_string(),
                start,
                end,
                1,
            );
            entities[0].segment_index
        };

        // Fully contained: the exact-containment path, unchanged.
        assert_eq!(resolve(4_100, 4_200), 1);
        // Straddling the end of segment 1 by more than the overlap: most shared characters wins.
        assert_eq!(resolve(5_900, 6_400), 1);
        // Inside a gap, closer to the segment that follows it than the one before.
        assert_eq!(resolve(3_800, 3_900), 1);
        // Inside a gap, closer to the segment that precedes it.
        assert_eq!(resolve(2_100, 2_200), 0);
        // Inside the second gap. The old fallback answered 0 here, four thousand characters and
        // two segments away from the entity it was describing.
        assert_eq!(resolve(7_000, 7_100), 2);
        // No segments at all is the only remaining route to 0.
        let mut entities = Vec::new();
        let mut budget = EntityBudget::new();
        push_entity_with_display(
            &mut entities,
            &mut budget,
            &[],
            CorpusEntityKind::Organization,
            "value".to_string(),
            "value".to_string(),
            10,
            20,
            1,
        );
        assert_eq!(entities[0].segment_index, 0);
    }

    // REGRESSION(corpus): a single shared MAXIMUM_ENTITIES_PER_DOCUMENT pool let one kind's
    // extraction consume the entire budget and leave every other kind at zero, silently. Document
    // 6661148630021752 (docs/local-analysis-architecture.md section 2) is a real corpus instance:
    // 20,000 DATE entities and zero of anything else, with nothing recording that extraction had
    // been cut short rather than the document genuinely containing no organizations.
    //
    // This test already caught a real defect once: an earlier version of `EntityBudget` recorded
    // truncation inside `admit`'s over-budget branch, but every extraction loop checks `is_full`
    // and stops *before* calling `admit` again, so that branch could never run and the flag was
    // always `false` no matter how saturated a kind was. This test failed against that version
    // (`entity_truncations.get(&CorpusEntityKind::Date)` read `Some(false)`), which is what moved
    // the truncation
    // record into `is_full` itself.
    #[test]
    fn one_saturated_entity_kind_does_not_starve_or_hide_the_others() {
        let mut text = String::from("City of San Diego contact@example.com ");
        for _ in 0..5_000 {
            text.push_str("1/1/2001 ");
        }
        let analyzed = analyze_document(input(5, &text)).expect("analysis should succeed");

        assert_eq!(
            analyzed.entity_truncations.get(&CorpusEntityKind::Date),
            Some(&true),
            "flooding DATE past its per-kind budget must record that kind as truncated"
        );
        for kind in [
            CorpusEntityKind::Organization,
            CorpusEntityKind::EmailDomain,
        ] {
            assert_eq!(
                analyzed.entity_truncations.get(&kind),
                Some(&false),
                "kind {kind} was not saturated and must not be reported as truncated"
            );
        }
        assert!(
            analyzed
                .entities
                .iter()
                .any(|entity| entity.kind == CorpusEntityKind::Organization),
            "the flood of DATE entities must not have starved ORGANIZATION extraction"
        );
        assert!(
            analyzed
                .entities
                .iter()
                .any(|entity| entity.kind == CorpusEntityKind::EmailDomain),
            "the flood of DATE entities must not have starved EMAIL_DOMAIN extraction"
        );
    }
}
