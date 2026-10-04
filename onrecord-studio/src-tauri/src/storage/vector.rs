//! Rebuildable local embedding projection.

use crate::blocking::spawn_with;
use crate::error::AppError;
use crate::integrity::sha256_hex;
use crate::storage::sqlite::DuckDbRepository;
use crate::types::{
    EmbeddingIndexReport, EmbeddingIndexStatus, EmbeddingModelStatus, HybridSearchFilter,
    HybridSearchResult,
};
use duckdb::{params, Connection, OptionalExt};
use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;

pub const EMBEDDING_MODEL_ID: &str = "Qdrant/bge-small-en-v1.5-onnx-Q";
pub const EMBEDDING_MODEL_REVISION: &str = "fastembed-6.1.0:BGESmallENV15Q";
pub const EMBEDDING_DIMENSION: usize = 384;
pub const EMBEDDING_DISTANCE_METRIC: &str = "COSINE";
pub const EMBEDDING_NORMALIZATION: &str = "L2_UNIT";
pub const EMBEDDING_CHUNKING_VERSION: &str = "onrecord-semantic-chunks-v1";

const VECTOR_SCHEMA_VERSION: i64 = 1;
const MAX_CHUNK_CHARS: usize = 1_600;
const CHUNK_OVERLAP_CHARS: usize = 160;
/// Passages per model call, taken consecutively from one document. Part of what a stored vector
/// means: the quantized model's output for a passage depends on the other passages in its batch
/// (measured on the live collection: 361 of 512 vectors change bits at batch size 1, 10 of 512 at
/// 32), so changing this or batching across documents re-embeds the corpus differently. One
/// batch's activations are all the model holds at a time.
const EMBED_BATCH_SIZE: usize = 16;
/// ONNX intra-op threads. Vectors are bit-identical at 2, 4 and 8 threads (measured); throughput
/// doubles from 2 to 4 and stops rising past the 4 performance cores it was measured on.
const MAX_INTRA_OP_THREADS: usize = 4;
/// Documents read per page during a rebuild; each carries its full extracted text.
const VECTOR_SOURCE_PAGE_SIZE: usize = 32;

#[derive(Debug, Clone)]
pub struct VectorSourceDocument {
    pub portal_key: String,
    pub request_id: u64,
    pub document_id: u64,
    pub file_name: String,
    pub title: String,
    pub agency_name: String,
    pub jurisdiction: String,
    pub document_type: Option<String>,
    pub publication_date: Option<String>,
    pub observation_date: Option<String>,
    pub effective_date: Option<String>,
    pub raw_file_sha256: String,
    pub extracted_text_sha256: String,
    pub text: String,
}

/// A document's best-matching chunk against a query: cosine of unit vectors, and where in the
/// document's extracted text that chunk lies.
#[derive(Debug, Clone)]
pub struct DocumentSimilarity {
    pub request_id: u64,
    pub document_id: u64,
    pub cosine: f32,
    pub character_start: i64,
    pub character_end: i64,
    pub excerpt: String,
}

struct SkipCheckFields {
    portal_key: String,
    document_id: u64,
    jurisdiction: String,
    agency_name: String,
    title: String,
    publication_date: Option<String>,
    observation_date: Option<String>,
    effective_date: Option<String>,
}

#[derive(Clone)]
pub struct VectorIndex {
    connection: Arc<Mutex<Connection>>,
    /// A second connection to the same database, used only by indexing. A document's chunks are
    /// written in one transaction on it while the model embeds the next batch, so readers on
    /// `connection` keep seeing the document's previous vectors until the replacement commits,
    /// and are never blocked behind the model.
    writer: Arc<Mutex<Connection>>,
    cache_dir: PathBuf,
    model: Arc<Mutex<Option<TextEmbedding>>>,
    running: Arc<AtomicBool>,
    cancelled: Arc<AtomicBool>,
    failed_documents: Arc<AtomicUsize>,
    last_error: Arc<Mutex<Option<String>>>,
}

impl VectorIndex {
    pub fn open(path: &Path, cache_dir: PathBuf) -> Result<Self, AppError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                AppError::Storage(format!(
                    "Failed to create vector index directory '{}': {error}",
                    parent.display()
                ))
            })?;
        }
        fs::create_dir_all(&cache_dir).map_err(|error| {
            AppError::Storage(format!(
                "Failed to create embedding model cache '{}': {error}",
                cache_dir.display()
            ))
        })?;
        let connection = super::open_duckdb(Some(path)).map_err(|error| {
            AppError::Storage(format!(
                "Failed to open vector index '{}': {error}",
                path.display()
            ))
        })?;
        connection
            .execute_batch(
                "SET preserve_insertion_order = false;
                 SET threads = 1;
                 SET enable_progress_bar = false;",
            )
            .map_err(|error| {
                AppError::Storage(format!("Failed to configure vector index: {error}"))
            })?;
        connection.execute_batch(VECTOR_SCHEMA).map_err(|error| {
            AppError::Storage(format!("Failed to migrate vector index: {error}"))
        })?;
        connection
            .execute(
                "INSERT INTO vector_schema_version (singleton, version) VALUES (TRUE, ?1)
                 ON CONFLICT (singleton) DO UPDATE SET version = excluded.version",
                params![VECTOR_SCHEMA_VERSION],
            )
            .map_err(|error| {
                AppError::Storage(format!("Failed to stamp vector schema: {error}"))
            })?;
        let writer = connection.try_clone().map_err(|error| {
            AppError::Storage(format!("Failed to open the vector index writer: {error}"))
        })?;
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
            writer: Arc::new(Mutex::new(writer)),
            cache_dir,
            model: Arc::new(Mutex::new(None)),
            running: Arc::new(AtomicBool::new(false)),
            cancelled: Arc::new(AtomicBool::new(false)),
            failed_documents: Arc::new(AtomicUsize::new(0)),
            last_error: Arc::new(Mutex::new(None)),
        })
    }

    /// Runs a synchronous DuckDB operation on a blocking worker rather than directly in the async
    /// body, mirroring `DuckDbRepository::query_connection` (`storage/sqlite/connection.rs`). Holding the
    /// connection lock while running a blocking DuckDB call straight on a tokio worker thread
    /// blocks that worker for the query's duration; this moves the call off it instead.
    async fn query_connection<T, F>(&self, operation: &'static str, query: F) -> Result<T, AppError>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T, AppError> + Send + 'static,
    {
        run_locked(Arc::clone(&self.connection), operation, query).await
    }

    /// [`Self::query_connection`] on the indexing writer. A purge runs here so it waits for a
    /// document replacement in progress instead of conflicting with its open transaction.
    async fn writer_connection<T, F>(
        &self,
        operation: &'static str,
        write: F,
    ) -> Result<T, AppError>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T, AppError> + Send + 'static,
    {
        run_locked(Arc::clone(&self.writer), operation, write).await
    }

    /// Whether a query can be embedded right now; the same condition [`Self::semantic_search`]
    /// refuses on, without `model_status`'s artifact directory scan.
    pub async fn model_ready(&self) -> bool {
        self.model.lock().await.is_some()
    }

    pub async fn model_status(&self) -> Result<EmbeddingModelStatus, AppError> {
        let (artifact_count, manifest_sha) = artifact_manifest(&self.cache_dir)?;
        let ready = self.model.lock().await.is_some();
        let last_error = self.last_error.lock().await.clone();
        Ok(EmbeddingModelStatus {
            model_id: EMBEDDING_MODEL_ID.to_string(),
            revision: EMBEDDING_MODEL_REVISION.to_string(),
            dimension: EMBEDDING_DIMENSION,
            distance_metric: EMBEDDING_DISTANCE_METRIC.to_string(),
            normalization: EMBEDDING_NORMALIZATION.to_string(),
            cache_dir: self.cache_dir.display().to_string(),
            installed: artifact_count > 0,
            ready,
            artifact_count,
            artifact_manifest_sha256: manifest_sha,
            last_error,
        })
    }

    pub async fn initialize_model(&self) -> Result<EmbeddingModelStatus, AppError> {
        let cache_dir = self.cache_dir.clone();
        let mut model = self.model.lock().await;
        if model.is_none() {
            let threads = std::thread::available_parallelism()
                .map_or(1, usize::from)
                .min(MAX_INTRA_OP_THREADS);
            // Only the CPU provider: vectors feed findings, so they must not depend on which
            // accelerator a machine happens to offer. The arena is ONNX Runtime's default.
            let options = TextInitOptions::new(EmbeddingModel::BGESmallENV15Q)
                .with_cache_dir(cache_dir)
                .with_show_download_progress(true)
                .with_max_length(512)
                .with_execution_providers(vec![ort::ep::CPU::default()
                    .with_arena_allocator(true)
                    .build()])
                .with_intra_threads(threads);
            match TextEmbedding::try_new(options) {
                Ok(initialized) => {
                    *model = Some(initialized);
                    *self.last_error.lock().await = None;
                }
                Err(error) => {
                    let message = format!("Embedding model initialization failed: {error}");
                    *self.last_error.lock().await = Some(message.clone());
                    return Err(AppError::Storage(message));
                }
            }
        }
        drop(model);
        self.record_model_manifest().await?;
        self.model_status().await
    }

    /// Unit-normalized embeddings for short texts, or `None` when the model is not installed.
    /// An installed model is loaded on first use; a missing one is never downloaded from here,
    /// because ingestion must not start a model download the operator did not ask for.
    pub async fn embed_short_texts(
        &self,
        texts: Vec<String>,
    ) -> Result<Option<Vec<Vec<f32>>>, AppError> {
        if texts.is_empty() {
            return Ok(Some(Vec::new()));
        }
        if !self.model_ready().await {
            let (artifact_count, _) = artifact_manifest(&self.cache_dir)?;
            if artifact_count == 0 {
                return Ok(None);
            }
            self.initialize_model().await?;
        }
        let mut guard = self.model.lock().await;
        let Some(model) = guard.as_mut() else {
            return Ok(None);
        };
        let embeddings = model
            .embed(texts, Some(64))
            .map_err(|error| AppError::Storage(format!("Failed to embed texts: {error}")))?;
        Ok(Some(embeddings.into_iter().map(normalize).collect()))
    }

    pub async fn index_documents(
        &self,
        repository: &DuckDbRepository,
        rebuild: bool,
    ) -> Result<EmbeddingIndexReport, AppError> {
        if self.running.swap(true, Ordering::SeqCst) {
            return Err(AppError::Storage(
                "Semantic indexing is already running".to_string(),
            ));
        }
        self.cancelled.store(false, Ordering::SeqCst);
        self.failed_documents.store(0, Ordering::SeqCst);
        let result = self.index_documents_inner(repository, rebuild).await;
        self.running.store(false, Ordering::SeqCst);
        result
    }

    pub fn cancel_indexing(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// Removes a purged document's embeddings. They hold excerpts of its text, so leaving them
    /// would keep a destroyed document searchable and citable as evidence.
    pub async fn purge_document(
        &self,
        portal_key: String,
        document_id: u64,
    ) -> Result<(), AppError> {
        let document_id = i64::try_from(document_id).map_err(|_| {
            AppError::DataParsing("Document identifier exceeds supported capacity".to_string())
        })?;
        self.writer_connection("Purge document embeddings", move |connection| {
            delete_embeddings(connection, &[(portal_key, document_id)])
        })
        .await
    }

    /// Removes every embedding of a purged source. See [`Self::purge_document`].
    pub async fn purge_portal(&self, portal_key: String) -> Result<(), AppError> {
        self.writer_connection("Purge source embeddings", move |connection| {
            let transaction = connection.transaction().map_err(|error| {
                AppError::Storage(format!("Failed to open an embedding purge: {error}"))
            })?;
            for table in ["embedding_chunks", "embedding_document_state"] {
                transaction
                    .execute(
                        &format!("DELETE FROM {table} WHERE portal_key = ?1"),
                        params![portal_key],
                    )
                    .map_err(|error| {
                        AppError::Storage(format!("Failed to purge {table}: {error}"))
                    })?;
            }
            transaction.commit().map_err(|error| {
                AppError::Storage(format!("Failed to commit an embedding purge: {error}"))
            })
        })
        .await
    }

    pub async fn index_status(
        &self,
        total_source_documents: usize,
    ) -> Result<EmbeddingIndexStatus, AppError> {
        let (documents, chunks) = self
            .query_connection("Read embedding status", |connection| {
                connection
                    .query_row(
                        "SELECT
                    (SELECT COUNT(DISTINCT portal_key || '|' || document_id) FROM embedding_chunks),
                    (SELECT COUNT(*) FROM embedding_chunks)",
                        [],
                        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
                    )
                    .map_err(|error| {
                        AppError::Storage(format!("Failed to read embedding status: {error}"))
                    })
            })
            .await?;
        let indexed_document_count = documents.max(0) as usize;
        Ok(EmbeddingIndexStatus {
            indexed_document_count,
            indexed_chunk_count: chunks.max(0) as usize,
            pending_document_count: total_source_documents.saturating_sub(indexed_document_count),
            failed_document_count: self.failed_documents.load(Ordering::SeqCst),
            running: self.running.load(Ordering::SeqCst),
            cancelled: self.cancelled.load(Ordering::SeqCst),
            last_error: self.last_error.lock().await.clone(),
        })
    }

    pub async fn semantic_search(
        &self,
        query: &str,
        filter: HybridSearchFilter,
        limit: usize,
    ) -> Result<Vec<HybridSearchResult>, AppError> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let query_embedding = {
            let mut guard = self.model.lock().await;
            if guard.is_none() {
                return Err(AppError::Storage("Initialize the embedding model to enable semantic retrieval. Document text search is available without it.".into()));
            }
            let model = guard.as_mut().ok_or_else(|| {
                AppError::Storage("Embedding model is not initialized".to_string())
            })?;
            let mut embeddings = model.embed(vec![query], Some(1)).map_err(|error| {
                AppError::Storage(format!("Failed to embed semantic query: {error}"))
            })?;
            let first = embeddings.pop().ok_or_else(|| {
                AppError::Storage("Embedding model returned no query vector".to_string())
            })?;
            normalize(first)
        };
        self.query_connection("Semantic search", move |connection| {
            in_one_snapshot(connection, |connection| {
                rank_passages(connection, &query_embedding, &filter, limit)
            })
        })
        .await
    }

    /// Each embedded document of `portal_key` with its single most similar chunk to `query`,
    /// most similar first. `None` when no embedding model is installed; an installed one is
    /// loaded, never downloaded. Unlike [`Self::semantic_search`] this keeps one row per document
    /// in one pass, so a large corpus costs a linear scan rather than a chunk-level dedupe.
    pub async fn nearest_documents(
        &self,
        query: &str,
        portal_key: &str,
        limit: usize,
    ) -> Result<Option<Vec<DocumentSimilarity>>, AppError> {
        let Some(mut embeddings) = self.embed_short_texts(vec![query.to_string()]).await? else {
            return Ok(None);
        };
        let query_embedding = embeddings.pop().ok_or_else(|| {
            AppError::Storage("Embedding model returned no query vector".to_string())
        })?;
        let portal_key = portal_key.to_string();
        self.query_connection("Nearest documents", move |connection| {
            in_one_snapshot(connection, |connection| {
                rank_documents(connection, &query_embedding, &portal_key, limit).map(Some)
            })
        })
        .await
    }

    async fn index_documents_inner(
        &self,
        repository: &DuckDbRepository,
        rebuild: bool,
    ) -> Result<EmbeddingIndexReport, AppError> {
        self.initialize_model().await?;
        if rebuild {
            self.query_connection("Clear vector projection", |connection| {
                connection
                    .execute("DELETE FROM embedding_chunks", [])
                    .map_err(|error| {
                        AppError::Storage(format!("Failed to clear vector projection: {error}"))
                    })?;
                connection
                    .execute("DELETE FROM embedding_document_state", [])
                    .map_err(|error| {
                        AppError::Storage(format!("Failed to clear vector state: {error}"))
                    })?;
                Ok(())
            })
            .await?;
        }
        let mut report = EmbeddingIndexReport {
            indexed_documents: 0,
            indexed_chunks: 0,
            skipped_documents: 0,
            failed_documents: 0,
            cancelled: false,
            first_error: None,
        };
        let mut after = None;
        let mut visited = std::collections::HashSet::new();
        'pages: loop {
            let page = repository
                .list_vector_source_documents_page(after.take(), VECTOR_SOURCE_PAGE_SIZE)
                .await?;
            let Some(last) = page.last() else {
                break;
            };
            after = Some((last.portal_key.clone(), last.document_id));
            for document in page {
                if self.cancelled.load(Ordering::SeqCst) {
                    report.cancelled = true;
                    break 'pages;
                }
                visited.insert((document.portal_key.clone(), document.document_id));
                match self.index_one_document(document).await {
                    Ok(IndexOneOutcome::Indexed(chunks)) => {
                        report.indexed_documents += 1;
                        report.indexed_chunks += chunks;
                    }
                    Ok(IndexOneOutcome::Skipped) => report.skipped_documents += 1,
                    Ok(IndexOneOutcome::Cancelled) => {
                        report.cancelled = true;
                        break 'pages;
                    }
                    Err(error) => {
                        report.failed_documents += 1;
                        self.failed_documents.fetch_add(1, Ordering::SeqCst);
                        let message = error.to_string();
                        report.first_error.get_or_insert_with(|| message.clone());
                        *self.last_error.lock().await = Some(message);
                    }
                }
                tokio::task::yield_now().await;
            }
        }
        if !report.cancelled {
            // A completed pass visited every document that still qualifies, so an indexed one it
            // did not visit was withdrawn from public view or lost its text, and its excerpts
            // must not stay searchable.
            self.query_connection("Prune withdrawn embeddings", move |connection| {
                let indexed = {
                    let mut statement = connection
                        .prepare(
                            "SELECT DISTINCT portal_key, document_id FROM embedding_document_state",
                        )
                        .map_err(|error| {
                            AppError::Storage(format!("Failed to read embedding state: {error}"))
                        })?;
                    statement
                        .query_map([], |row| {
                            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                        })
                        .and_then(|rows| rows.collect::<duckdb::Result<Vec<_>>>())
                        .map_err(|error| {
                            AppError::Storage(format!("Failed to read embedding state: {error}"))
                        })?
                };
                let withdrawn = indexed
                    .into_iter()
                    .filter(|(portal_key, document_id)| {
                        u64::try_from(*document_id)
                            .map_or(true, |id| !visited.contains(&(portal_key.clone(), id)))
                    })
                    .collect::<Vec<_>>();
                delete_embeddings(connection, &withdrawn)
            })
            .await?;
        }
        Ok(report)
    }

    async fn index_one_document(
        &self,
        document: VectorSourceDocument,
    ) -> Result<IndexOneOutcome, AppError> {
        let skip_check = SkipCheckFields {
            portal_key: document.portal_key.clone(),
            document_id: document.document_id,
            jurisdiction: document.jurisdiction.clone(),
            agency_name: document.agency_name.clone(),
            title: document.title.clone(),
            publication_date: document.publication_date.clone(),
            observation_date: document.observation_date.clone(),
            effective_date: document.effective_date.clone(),
        };
        let content_hash = document.extracted_text_sha256.clone();
        let skipped = self
            .query_connection("Inspect embedding state", move |connection| {
                let current = connection
                    .query_row(
                        "SELECT content_sha256 FROM embedding_document_state
                     WHERE portal_key = ?1 AND document_id = ?2 AND model_id = ?3
                       AND model_revision = ?4 AND chunking_version = ?5",
                        params![
                            skip_check.portal_key,
                            skip_check.document_id,
                            EMBEDDING_MODEL_ID,
                            EMBEDDING_MODEL_REVISION,
                            EMBEDDING_CHUNKING_VERSION
                        ],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()
                    .map_err(|error| {
                        AppError::Storage(format!("Failed to inspect embedding state: {error}"))
                    })?;
                if current.as_deref() == Some(&content_hash) {
                    // Source attribution may be corrected even when the extracted text is
                    // unchanged; rows already carrying it are left alone so an unchanged corpus
                    // re-indexes without rewriting every chunk.
                    connection.execute("UPDATE embedding_chunks SET jurisdiction = ?1, agency_name = ?2,
                    title = ?3, publication_date = ?4, observation_date = ?5, effective_date = ?6
                    WHERE portal_key = ?7 AND document_id = ?8 AND model_id = ?9 AND model_revision = ?10
                      AND (jurisdiction IS DISTINCT FROM ?1 OR agency_name IS DISTINCT FROM ?2
                        OR title IS DISTINCT FROM ?3 OR publication_date IS DISTINCT FROM ?4
                        OR observation_date IS DISTINCT FROM ?5 OR effective_date IS DISTINCT FROM ?6)",
                    params![skip_check.jurisdiction, skip_check.agency_name, skip_check.title,
                        skip_check.publication_date, skip_check.observation_date, skip_check.effective_date,
                        skip_check.portal_key, skip_check.document_id, EMBEDDING_MODEL_ID, EMBEDDING_MODEL_REVISION])
                    .map_err(|error| AppError::Storage(format!("Failed to refresh indexed source attribution: {error}")))?;
                    return Ok(true);
                }
                Ok(false)
            })
            .await?;
        if skipped {
            return Ok(IndexOneOutcome::Skipped);
        }

        let writer = Arc::clone(&self.writer);
        let model = Arc::clone(&self.model);
        let cancelled = Arc::clone(&self.cancelled);
        spawn_with("Embed document", AppError::Internal, move || {
            let mut writer = writer.blocking_lock();
            store_document(&mut writer, &document, &cancelled, |texts| {
                let mut model = model.blocking_lock();
                let model = model.as_mut().ok_or_else(|| {
                    AppError::Storage("Embedding model is not initialized".to_string())
                })?;
                model.embed(texts, Some(EMBED_BATCH_SIZE)).map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to embed document {}/{}: {error}",
                        document.portal_key, document.document_id
                    ))
                })
            })
        })
        .await?
    }

    async fn record_model_manifest(&self) -> Result<(), AppError> {
        let (count, hash) = artifact_manifest(&self.cache_dir)?;
        let cache_dir = self.cache_dir.display().to_string();
        self.query_connection("Record embedding model manifest", move |connection| {
            connection
                .execute(
                    "INSERT INTO embedding_model_manifests (
                    model_id, model_revision, dimension, distance_metric, normalization,
                    cache_dir, artifact_count, artifact_manifest_sha256, recorded_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8,
                           strftime(now(), '%Y-%m-%dT%H:%M:%S.%fZ'))
                 ON CONFLICT (model_id, model_revision)
                 DO UPDATE SET artifact_count = excluded.artifact_count,
                               artifact_manifest_sha256 = excluded.artifact_manifest_sha256,
                               recorded_at = excluded.recorded_at",
                    params![
                        EMBEDDING_MODEL_ID,
                        EMBEDDING_MODEL_REVISION,
                        EMBEDDING_DIMENSION as i64,
                        EMBEDDING_DISTANCE_METRIC,
                        EMBEDDING_NORMALIZATION,
                        cache_dir,
                        count as i64,
                        hash,
                    ],
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to record model manifest: {error}"))
                })?;
            Ok(())
        })
        .await
    }
}

async fn run_locked<T, F>(
    connection: Arc<Mutex<Connection>>,
    operation: &'static str,
    query: F,
) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce(&mut Connection) -> Result<T, AppError> + Send + 'static,
{
    spawn_with(operation, AppError::Internal, move || {
        let mut connection = connection.blocking_lock();
        query(&mut connection)
    })
    .await?
}

enum IndexOneOutcome {
    Indexed(usize),
    Skipped,
    /// Indexing was cancelled mid-document; its previous vectors, if any, are untouched.
    Cancelled,
}

/// Replaces one document's chunks in a single transaction, embedding `EMBED_BATCH_SIZE`
/// consecutive passages per `embed` call and appending them as they come, so neither the
/// passages, their vectors nor the model's activations are held for the whole document. The
/// transaction rolls back on error or cancellation, leaving any previous version in place.
fn store_document(
    connection: &mut Connection,
    document: &VectorSourceDocument,
    cancelled: &AtomicBool,
    mut embed: impl FnMut(&[&str]) -> Result<Vec<Vec<f32>>, AppError>,
) -> Result<IndexOneOutcome, AppError> {
    let storage = |action: &str| {
        let action = action.to_string();
        move |error: duckdb::Error| AppError::Storage(format!("Failed to {action}: {error}"))
    };
    // DuckDB's now() is the transaction start, so every row of a document shared one timestamp.
    let indexed_at = chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%S%.6fZ")
        .to_string();
    let transaction = connection
        .transaction()
        .map_err(storage("begin vector transaction"))?;
    transaction
        .execute(
            "DELETE FROM embedding_chunks
             WHERE portal_key = ?1 AND document_id = ?2
               AND model_id = ?3 AND model_revision = ?4",
            params![
                document.portal_key,
                document.document_id,
                EMBEDDING_MODEL_ID,
                EMBEDDING_MODEL_REVISION
            ],
        )
        .map_err(storage("replace old embeddings"))?;
    let mut appender = transaction
        .appender("embedding_chunks")
        .map_err(storage("open the embedding appender"))?;
    let mut chunks = chunk_text(&document.text);
    let mut batch = Vec::with_capacity(EMBED_BATCH_SIZE);
    let mut chunk_count = 0_usize;
    loop {
        if cancelled.load(Ordering::SeqCst) {
            return Ok(IndexOneOutcome::Cancelled);
        }
        batch.clear();
        batch.extend(chunks.by_ref().take(EMBED_BATCH_SIZE));
        if batch.is_empty() {
            break;
        }
        let texts = batch.iter().map(|chunk| chunk.text).collect::<Vec<_>>();
        let vectors = embed(&texts)?;
        if vectors.len() != batch.len() {
            return Err(AppError::Storage(format!(
                "Embedding model returned {} vectors for {} chunks",
                vectors.len(),
                batch.len()
            )));
        }
        for (chunk, vector) in batch.iter().zip(vectors) {
            appender
                .append_row(params![
                    format!(
                        "{}:{}:{}:{}",
                        document.portal_key,
                        document.document_id,
                        chunk.start,
                        EMBEDDING_MODEL_REVISION
                    ),
                    document.portal_key,
                    document.request_id,
                    document.document_id,
                    document.file_name,
                    document.title,
                    document.agency_name,
                    document.jurisdiction,
                    document.document_type,
                    document.publication_date,
                    document.observation_date,
                    document.effective_date,
                    chunk.start as i64,
                    chunk.end as i64,
                    chunk.text,
                    document.extracted_text_sha256,
                    sha256_hex(chunk.text.as_bytes()),
                    EMBEDDING_MODEL_ID,
                    EMBEDDING_MODEL_REVISION,
                    EMBEDDING_DIMENSION as i64,
                    EMBEDDING_DISTANCE_METRIC,
                    EMBEDDING_NORMALIZATION,
                    EMBEDDING_CHUNKING_VERSION,
                    encode_vector(&normalize(vector)),
                    indexed_at,
                ])
                .map_err(storage("store embedding chunk"))?;
        }
        chunk_count += batch.len();
    }
    appender
        .flush()
        .map_err(storage("store embedding chunks"))?;
    drop(appender);
    transaction
        .execute(
            "INSERT INTO embedding_document_state (
                portal_key, document_id, content_sha256, raw_file_sha256, model_id,
                model_revision, dimension, distance_metric, normalization, chunking_version,
                chunk_count, indexed_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT (portal_key, document_id, model_id, model_revision, chunking_version)
             DO UPDATE SET content_sha256 = excluded.content_sha256,
                           raw_file_sha256 = excluded.raw_file_sha256,
                           chunk_count = excluded.chunk_count,
                           indexed_at = excluded.indexed_at",
            params![
                document.portal_key,
                document.document_id,
                document.extracted_text_sha256,
                document.raw_file_sha256,
                EMBEDDING_MODEL_ID,
                EMBEDDING_MODEL_REVISION,
                EMBEDDING_DIMENSION as i64,
                EMBEDDING_DISTANCE_METRIC,
                EMBEDDING_NORMALIZATION,
                EMBEDDING_CHUNKING_VERSION,
                chunk_count as i64,
                indexed_at,
            ],
        )
        .map_err(storage("store embedding document state"))?;
    transaction
        .commit()
        .map_err(storage("commit vector transaction"))?;
    Ok(IndexOneOutcome::Indexed(chunk_count))
}

/// A passage of `EMBEDDING_CHUNKING_VERSION`: character (not byte) offsets into the document's
/// extracted text, and the trimmed text between them.
struct TextChunk<'a> {
    start: usize,
    end: usize,
    text: &'a str,
}

/// The passages of `text`, produced lazily. Only one chunk's window of character offsets is
/// held at a time, so chunking a document costs the same memory whatever its length. The
/// boundaries are `onrecord-semantic-chunks-v1`'s: changing them without bumping
/// `EMBEDDING_CHUNKING_VERSION` would leave stored offsets and `stable_id`s describing other text.
fn chunk_text(text: &str) -> Chunks<'_> {
    Chunks {
        text,
        start_char: 0,
        start_byte: 0,
        done: false,
        window: Vec::with_capacity(MAX_CHUNK_CHARS + 1),
    }
}

struct Chunks<'a> {
    text: &'a str,
    start_char: usize,
    start_byte: usize,
    done: bool,
    /// `window[i]` is the byte offset of character `start_char + i`.
    window: Vec<usize>,
}

impl<'a> Iterator for Chunks<'a> {
    type Item = TextChunk<'a>;

    fn next(&mut self) -> Option<TextChunk<'a>> {
        let text = self.text;
        while !self.done && self.start_byte < text.len() {
            let start_byte = self.start_byte;
            self.window.clear();
            self.window.extend(
                text[start_byte..]
                    .char_indices()
                    .take(MAX_CHUNK_CHARS + 1)
                    .map(|(byte, _)| start_byte + byte),
            );
            let reaches_end = self.window.len() <= MAX_CHUNK_CHARS;
            if reaches_end {
                self.window.push(text.len());
            }
            let mut end = self.window.len() - 1;
            if !reaches_end {
                let window = &self.window;
                if let Some(boundary) = (MAX_CHUNK_CHARS * 2 / 3..end).rev().find(|index| {
                    matches!(
                        text[window[*index]..].chars().next(),
                        Some('.' | '\n' | ';')
                    )
                }) {
                    end = boundary + 1;
                }
            }
            let chunk = TextChunk {
                start: self.start_char,
                end: self.start_char + end,
                text: text[self.window[0]..self.window[end]].trim(),
            };
            if reaches_end {
                self.done = true;
            } else {
                let next = end - CHUNK_OVERLAP_CHARS;
                self.start_byte = self.window[next];
                self.start_char += next;
            }
            if !chunk.text.is_empty() {
                return Some(chunk);
            }
        }
        None
    }
}

fn normalize(mut vector: Vec<f32>) -> Vec<f32> {
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm > 0.0 {
        for value in &mut vector {
            *value /= norm;
        }
    }
    vector
}

fn encode_vector(vector: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(vector.len() * 4);
    for value in vector {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

/// Stored rows read per lookup of the passages a ranking kept.
const DETAIL_LOOKUP_BATCH: usize = 256;

/// Runs a ranking's scan and its follow-up lookups against one snapshot, so a document the
/// indexing writer replaces in between cannot make the lookup miss a row the scan ranked.
fn in_one_snapshot<T>(
    connection: &mut Connection,
    read: impl FnOnce(&Connection) -> Result<T, AppError>,
) -> Result<T, AppError> {
    let transaction = connection
        .transaction()
        .map_err(|error| AppError::Storage(format!("Failed to open a search snapshot: {error}")))?;
    let result = read(&transaction)?;
    transaction.commit().map_err(|error| {
        AppError::Storage(format!("Failed to close a search snapshot: {error}"))
    })?;
    Ok(result)
}

/// Dot product of `query` with a stored vector, read in place from its encoding.
fn dot_encoded(query: &[f32], bytes: &[u8]) -> Result<f32, AppError> {
    if bytes.len() != EMBEDDING_DIMENSION * 4 {
        return Err(AppError::Storage(format!(
            "Stored embedding has {} bytes; expected {}",
            bytes.len(),
            EMBEDDING_DIMENSION * 4
        )));
    }
    Ok(query
        .iter()
        .zip(bytes.chunks_exact(4))
        .map(|(a, chunk)| a * f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .sum())
}

/// The `limit` passages most similar to `query_embedding` that pass `filter`, best first, ties
/// by `stable_id`. A passage overlapping a better one of the same document is left out, and the
/// scan stops once `limit` survive, so only those are read in full.
fn rank_passages(
    connection: &Connection,
    query_embedding: &[f32],
    filter: &HybridSearchFilter,
    limit: usize,
) -> Result<Vec<HybridSearchResult>, AppError> {
    struct Candidate {
        score: f32,
        stable_id: String,
        portal_key: String,
        document_id: u64,
        start: usize,
        end: usize,
    }
    let storage = |action: &'static str| {
        move |error: duckdb::Error| AppError::Storage(format!("Failed to {action}: {error}"))
    };
    let present = |value: &Option<String>| value.clone().filter(|value| !value.trim().is_empty());
    let mut statement = connection
        .prepare(
            "SELECT stable_id, portal_key, document_id, character_start, character_end,
                    publication_date, observation_date, effective_date, vector
             FROM embedding_chunks
             WHERE model_id = ?1 AND model_revision = ?2
               AND (?3 IS NULL OR portal_key = ?3)
               AND (?4 IS NULL OR LOWER(agency_name) LIKE '%' || LOWER(?4) || '%')
               AND (?5 IS NULL OR LOWER(jurisdiction) = LOWER(?5))
               AND (?6 IS NULL OR LOWER(COALESCE(document_type, '')) = LOWER(?6))",
        )
        .map_err(storage("prepare semantic search"))?;
    let mut rows = statement
        .query(params![
            EMBEDDING_MODEL_ID,
            EMBEDDING_MODEL_REVISION,
            present(&filter.portal_key),
            present(&filter.agency_name),
            present(&filter.jurisdiction),
            present(&filter.document_type)
        ])
        .map_err(storage("execute semantic search"))?;
    let mut candidates = Vec::new();
    while let Some(row) = rows.next().map_err(storage("read semantic result"))? {
        let read = |row: &duckdb::Row<'_>| -> duckdb::Result<_> {
            Ok((
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<String>>(7)?,
            ))
        };
        let (publication, observation, effective) =
            read(row).map_err(storage("read semantic result"))?;
        if !date_filter_matches(&publication, &observation, &effective, filter) {
            continue;
        }
        let blob = row
            .get_ref(8)
            .and_then(|value| value.as_blob().map_err(Into::into))
            .map_err(storage("read semantic result"))?;
        let score = dot_encoded(query_embedding, blob)?;
        candidates.push(Candidate {
            score,
            stable_id: row.get(0).map_err(storage("read semantic result"))?,
            portal_key: row.get(1).map_err(storage("read semantic result"))?,
            document_id: row.get(2).map_err(storage("read semantic result"))?,
            start: row
                .get::<_, i64>(3)
                .map_err(storage("read semantic result"))?
                .max(0) as usize,
            end: row
                .get::<_, i64>(4)
                .map_err(storage("read semantic result"))?
                .max(0) as usize,
        });
    }
    drop(rows);
    candidates.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.stable_id.cmp(&right.stable_id))
    });
    let mut kept: Vec<Candidate> = Vec::new();
    for candidate in candidates {
        if kept.len() >= limit {
            break;
        }
        let overlaps = kept.iter().any(|prior| {
            prior.portal_key == candidate.portal_key
                && prior.document_id == candidate.document_id
                && ranges_overlap(prior.start, prior.end, candidate.start, candidate.end)
        });
        if !overlaps {
            kept.push(candidate);
        }
    }

    let mut details = std::collections::HashMap::new();
    for ids in kept.chunks(DETAIL_LOOKUP_BATCH) {
        let placeholders = vec!["?"; ids.len()].join(", ");
        let mut statement = connection
            .prepare(&format!(
                "SELECT stable_id, request_id, file_name, title, agency_name, jurisdiction,
                        document_type, publication_date, observation_date, effective_date, excerpt
                 FROM embedding_chunks WHERE stable_id IN ({placeholders})"
            ))
            .map_err(storage("prepare semantic result lookup"))?;
        let rows = statement
            .query_map(
                duckdb::params_from_iter(ids.iter().map(|candidate| &candidate.stable_id)),
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        (
                            row.get::<_, u64>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?,
                            row.get::<_, String>(5)?,
                            row.get::<_, Option<String>>(6)?,
                            row.get::<_, Option<String>>(7)?,
                            row.get::<_, Option<String>>(8)?,
                            row.get::<_, Option<String>>(9)?,
                            row.get::<_, String>(10)?,
                        ),
                    ))
                },
            )
            .map_err(storage("look up semantic results"))?;
        for row in rows {
            let (stable_id, detail) = row.map_err(storage("read semantic result"))?;
            details.insert(stable_id, detail);
        }
    }
    kept.into_iter()
        .map(|candidate| {
            let (
                request_id,
                file_name,
                title,
                agency_name,
                jurisdiction,
                document_type,
                publication_date,
                observation_date,
                effective_date,
                excerpt,
            ) = details.remove(&candidate.stable_id).ok_or_else(|| {
                AppError::Storage(format!(
                    "Semantic result {} disappeared during the search",
                    candidate.stable_id
                ))
            })?;
            Ok(HybridSearchResult {
                stable_id: candidate.stable_id,
                portal_key: candidate.portal_key,
                request_id,
                document_id: candidate.document_id,
                file_name,
                title,
                agency_name,
                jurisdiction,
                document_type,
                publication_date,
                observation_date,
                effective_date,
                character_start: candidate.start,
                character_end: candidate.end,
                excerpt,
                lexical_rank: None,
                semantic_score: Some(candidate.score as f64),
                fused_score: candidate.score as f64,
                retrieval_status: "SEMANTIC_READY".to_string(),
            })
        })
        .collect()
}

/// Each document of `portal_key` with its single chunk most similar to `query_embedding`, most
/// similar first, ties by document id. Within a document, equal cosines keep the earliest chunk.
/// Excerpts are read only for the documents returned.
fn rank_documents(
    connection: &Connection,
    query_embedding: &[f32],
    portal_key: &str,
    limit: usize,
) -> Result<Vec<DocumentSimilarity>, AppError> {
    struct Best {
        stable_id: String,
        request_id: u64,
        cosine: f32,
        start: i64,
        end: i64,
    }
    let storage = |action: &'static str| {
        move |error: duckdb::Error| AppError::Storage(format!("Failed to {action}: {error}"))
    };
    let mut statement = connection
        .prepare(
            "SELECT stable_id, request_id, document_id, character_start, character_end, vector
             FROM embedding_chunks
             WHERE model_id = ?1 AND model_revision = ?2 AND portal_key = ?3",
        )
        .map_err(storage("prepare nearest documents"))?;
    let mut rows = statement
        .query(params![
            EMBEDDING_MODEL_ID,
            EMBEDDING_MODEL_REVISION,
            portal_key
        ])
        .map_err(storage("run nearest documents"))?;
    let mut best: std::collections::HashMap<u64, Best> = std::collections::HashMap::new();
    while let Some(row) = rows.next().map_err(storage("read nearest document"))? {
        let blob = row
            .get_ref(5)
            .and_then(|value| value.as_blob().map_err(Into::into))
            .map_err(storage("read nearest document"))?;
        let cosine = dot_encoded(query_embedding, blob)?;
        let document_id: u64 = row.get(2).map_err(storage("read nearest document"))?;
        let start: i64 = row.get(3).map_err(storage("read nearest document"))?;
        if best
            .get(&document_id)
            .is_some_and(|held| match held.cosine.total_cmp(&cosine) {
                std::cmp::Ordering::Greater => true,
                std::cmp::Ordering::Equal => held.start <= start,
                std::cmp::Ordering::Less => false,
            })
        {
            continue;
        }
        best.insert(
            document_id,
            Best {
                stable_id: row.get(0).map_err(storage("read nearest document"))?,
                request_id: row.get(1).map_err(storage("read nearest document"))?,
                cosine,
                start,
                end: row.get(4).map_err(storage("read nearest document"))?,
            },
        );
    }
    drop(rows);
    let mut ranked: Vec<(u64, Best)> = best.into_iter().collect();
    ranked.sort_by(|(left_id, left), (right_id, right)| {
        right
            .cosine
            .total_cmp(&left.cosine)
            .then(left_id.cmp(right_id))
    });
    ranked.truncate(limit);

    let mut excerpts = std::collections::HashMap::new();
    for ids in ranked.chunks(DETAIL_LOOKUP_BATCH) {
        let placeholders = vec!["?"; ids.len()].join(", ");
        let mut statement = connection
            .prepare(&format!(
                "SELECT stable_id, excerpt FROM embedding_chunks WHERE stable_id IN ({placeholders})"
            ))
            .map_err(storage("prepare excerpt lookup"))?;
        let rows = statement
            .query_map(
                duckdb::params_from_iter(ids.iter().map(|(_, best)| &best.stable_id)),
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .map_err(storage("look up excerpts"))?;
        for row in rows {
            let (stable_id, excerpt) = row.map_err(storage("read excerpt"))?;
            excerpts.insert(stable_id, excerpt);
        }
    }
    ranked
        .into_iter()
        .map(|(document_id, best)| {
            let excerpt = excerpts.remove(&best.stable_id).ok_or_else(|| {
                AppError::Storage(format!(
                    "Nearest chunk {} disappeared during the search",
                    best.stable_id
                ))
            })?;
            Ok(DocumentSimilarity {
                request_id: best.request_id,
                document_id,
                cosine: best.cosine,
                character_start: best.start,
                character_end: best.end,
                excerpt,
            })
        })
        .collect()
}

fn artifact_manifest(cache_dir: &Path) -> Result<(usize, Option<String>), AppError> {
    if !cache_dir.exists() {
        return Ok((0, None));
    }
    let mut entries = Vec::new();
    collect_artifacts(cache_dir, cache_dir, &mut entries)?;
    if entries.is_empty() {
        return Ok((0, None));
    }
    entries.sort();
    let mut hasher = Sha256::new();
    for entry in &entries {
        hasher.update(entry.as_bytes());
        hasher.update(b"\n");
    }
    let digest = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok((entries.len(), Some(digest)))
}

fn collect_artifacts(root: &Path, path: &Path, entries: &mut Vec<String>) -> Result<(), AppError> {
    for entry in fs::read_dir(path).map_err(|error| {
        AppError::Storage(format!(
            "Failed to read model cache '{}': {error}",
            path.display()
        ))
    })? {
        let entry = entry.map_err(|error| {
            AppError::Storage(format!("Failed to read model cache entry: {error}"))
        })?;
        let path = entry.path();
        let metadata = entry.metadata().map_err(|error| {
            AppError::Storage(format!("Failed to inspect '{}': {error}", path.display()))
        })?;
        if metadata.is_dir() {
            collect_artifacts(root, &path, entries)?;
        } else if metadata.is_file() {
            let bytes = fs::read(&path).map_err(|error| {
                AppError::Storage(format!("Failed to hash '{}': {error}", path.display()))
            })?;
            let hash = sha256_hex(&bytes);
            let relative = path.strip_prefix(root).unwrap_or(&path).display();
            entries.push(format!("{relative}\t{}\t{hash}", metadata.len()));
        }
    }
    Ok(())
}

fn date_filter_matches(
    publication: &Option<String>,
    observation: &Option<String>,
    effective: &Option<String>,
    filter: &HybridSearchFilter,
) -> bool {
    let value = publication
        .as_ref()
        .or(observation.as_ref())
        .or(effective.as_ref());
    if let Some(from) = &filter.date_from {
        if value.is_none_or(|date| date < from) {
            return false;
        }
    }
    if let Some(to) = &filter.date_to {
        if value.is_none_or(|date| date > to) {
            return false;
        }
    }
    true
}

fn ranges_overlap(a_start: usize, a_end: usize, b_start: usize, b_end: usize) -> bool {
    a_start < b_end && b_start < a_end
}

/// Deletes every embedding of the given documents in one transaction.
fn delete_embeddings(
    connection: &mut Connection,
    documents: &[(String, i64)],
) -> Result<(), AppError> {
    let transaction = connection.transaction().map_err(|error| {
        AppError::Storage(format!("Failed to open an embedding purge: {error}"))
    })?;
    for (portal_key, document_id) in documents {
        for table in ["embedding_chunks", "embedding_document_state"] {
            transaction
                .execute(
                    &format!("DELETE FROM {table} WHERE portal_key = ?1 AND document_id = ?2"),
                    params![portal_key, document_id],
                )
                .map_err(|error| AppError::Storage(format!("Failed to purge {table}: {error}")))?;
        }
    }
    transaction
        .commit()
        .map_err(|error| AppError::Storage(format!("Failed to commit an embedding purge: {error}")))
}

const VECTOR_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS vector_schema_version (
    singleton BOOLEAN PRIMARY KEY CHECK (singleton),
    version BIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS embedding_model_manifests (
    model_id TEXT NOT NULL,
    model_revision TEXT NOT NULL,
    dimension BIGINT NOT NULL,
    distance_metric TEXT NOT NULL,
    normalization TEXT NOT NULL,
    cache_dir TEXT NOT NULL,
    artifact_count BIGINT NOT NULL,
    artifact_manifest_sha256 TEXT,
    recorded_at TEXT NOT NULL,
    PRIMARY KEY (model_id, model_revision)
);

CREATE TABLE IF NOT EXISTS embedding_document_state (
    portal_key TEXT NOT NULL,
    document_id BIGINT NOT NULL,
    content_sha256 TEXT NOT NULL,
    raw_file_sha256 TEXT NOT NULL,
    model_id TEXT NOT NULL,
    model_revision TEXT NOT NULL,
    dimension BIGINT NOT NULL,
    distance_metric TEXT NOT NULL,
    normalization TEXT NOT NULL,
    chunking_version TEXT NOT NULL,
    chunk_count BIGINT NOT NULL,
    indexed_at TEXT NOT NULL,
    PRIMARY KEY (portal_key, document_id, model_id, model_revision, chunking_version)
);

CREATE TABLE IF NOT EXISTS embedding_chunks (
    stable_id TEXT PRIMARY KEY,
    portal_key TEXT NOT NULL,
    request_id BIGINT NOT NULL,
    document_id BIGINT NOT NULL,
    file_name TEXT NOT NULL,
    title TEXT NOT NULL,
    agency_name TEXT NOT NULL,
    jurisdiction TEXT NOT NULL,
    document_type TEXT,
    publication_date TEXT,
    observation_date TEXT,
    effective_date TEXT,
    character_start BIGINT NOT NULL,
    character_end BIGINT NOT NULL,
    excerpt TEXT NOT NULL,
    content_sha256 TEXT NOT NULL,
    chunk_sha256 TEXT NOT NULL,
    model_id TEXT NOT NULL,
    model_revision TEXT NOT NULL,
    dimension BIGINT NOT NULL,
    distance_metric TEXT NOT NULL,
    normalization TEXT NOT NULL,
    chunking_version TEXT NOT NULL,
    vector BLOB NOT NULL,
    indexed_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_embedding_chunks_document ON embedding_chunks(portal_key, document_id);
CREATE INDEX IF NOT EXISTS idx_embedding_chunks_model ON embedding_chunks(model_id, model_revision);
"#;

#[cfg(test)]
mod tests {
    use super::*;

    /// Embeddings carry excerpts of the document's text, so purging a document or a source must
    /// remove exactly its rows from both tables and leave every other document's alone.
    #[tokio::test]
    async fn purging_removes_only_the_purged_documents_embeddings() {
        let temp = tempfile::tempdir().unwrap();
        let index =
            VectorIndex::open(&temp.path().join("vectors.db"), temp.path().join("cache")).unwrap();
        let rows = [("north", 1_i64), ("north", 2), ("south", 1)];
        {
            let connection = index.connection.lock().await;
            for (portal_key, document_id) in rows {
                connection
                    .execute(
                        "INSERT INTO embedding_chunks (
                            stable_id, portal_key, request_id, document_id, file_name, title,
                            agency_name, jurisdiction, character_start, character_end, excerpt,
                            content_sha256, chunk_sha256, model_id, model_revision, dimension,
                            distance_metric, normalization, chunking_version, vector, indexed_at
                         ) VALUES (?1, ?2, 1, ?3, 'f', 't', 'a', 'j', 0, 1, 'excerpt', 'c', 'h',
                            'm', 'r', 1, 'd', 'n', 'v', ''::BLOB, 'now')",
                        params![
                            format!("{portal_key}-{document_id}"),
                            portal_key,
                            document_id
                        ],
                    )
                    .unwrap();
                connection
                    .execute(
                        "INSERT INTO embedding_document_state (
                            portal_key, document_id, content_sha256, raw_file_sha256, model_id,
                            model_revision, dimension, distance_metric, normalization,
                            chunking_version, chunk_count, indexed_at
                         ) VALUES (?1, ?2, 'c', 'r', 'm', 'r', 1, 'd', 'n', 'v', 1, 'now')",
                        params![portal_key, document_id],
                    )
                    .unwrap();
            }
        }
        let remaining = |index: VectorIndex| async move {
            let connection = index.connection.lock().await;
            ["embedding_chunks", "embedding_document_state"].map(|table| {
                connection
                    .prepare(&format!(
                        "SELECT portal_key, document_id FROM {table} ORDER BY portal_key, document_id"
                    ))
                    .unwrap()
                    .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)))
                    .unwrap()
                    .collect::<duckdb::Result<Vec<_>>>()
                    .unwrap()
            })
        };

        index.purge_document("north".to_string(), 1).await.unwrap();
        let expected = vec![("north".to_string(), 2), ("south".to_string(), 1)];
        assert_eq!(remaining(index.clone()).await, [expected.clone(), expected]);

        index.purge_portal("north".to_string()).await.unwrap();
        let expected = vec![("south".to_string(), 1)];
        assert_eq!(remaining(index).await, [expected.clone(), expected]);
    }

    #[tokio::test]
    async fn searching_without_initialization_does_not_download_a_model() {
        let temp = tempfile::tempdir().unwrap();
        let index =
            VectorIndex::open(&temp.path().join("vectors.db"), temp.path().join("cache")).unwrap();
        let filter = serde_json::from_value(serde_json::json!({})).unwrap();
        assert!(index
            .semantic_search("shelter capacity", filter, 10)
            .await
            .is_err());
        let status = index.model_status().await.unwrap();
        assert!(!status.installed && !status.ready);
    }

    /// `onrecord-semantic-chunks-v1` as first shipped, materializing every character offset.
    /// Stored offsets and `stable_id`s were produced by it, so the streaming chunker must
    /// reproduce it exactly while `EMBEDDING_CHUNKING_VERSION` is unchanged.
    fn semantic_chunks_v1(text: &str) -> Vec<(usize, usize, String)> {
        if text.trim().is_empty() {
            return Vec::new();
        }
        let chars = text
            .char_indices()
            .map(|(byte, _)| byte)
            .chain(std::iter::once(text.len()))
            .collect::<Vec<_>>();
        let total = chars.len().saturating_sub(1);
        let mut chunks = Vec::new();
        let mut start_char = 0usize;
        while start_char < total {
            let mut end_char = (start_char + MAX_CHUNK_CHARS).min(total);
            if end_char < total {
                let search_start = start_char + (MAX_CHUNK_CHARS * 2 / 3);
                if let Some(boundary) = (search_start..end_char).rev().find(|index| {
                    text[chars[*index]..chars[*index + 1]]
                        .chars()
                        .next()
                        .is_some_and(|c| matches!(c, '.' | '\n' | ';'))
                }) {
                    end_char = (boundary + 1).min(total);
                }
            }
            let excerpt = text[chars[start_char]..chars[end_char]].trim().to_string();
            if !excerpt.is_empty() {
                chunks.push((start_char, end_char, excerpt));
            }
            if end_char == total {
                break;
            }
            start_char = end_char.saturating_sub(CHUNK_OVERLAP_CHARS);
        }
        chunks
    }

    #[test]
    fn streaming_chunks_keep_semantic_chunks_v1_boundaries() {
        let mut seed = 0x9e37_79b9_u32;
        let mut next = move |bound: u32| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed % bound
        };
        let pieces = [
            "a", "record", " ", " ", "café", "名", "αβγ", ".", "\n", ";", "  ", "\n\n", "x",
        ];
        let mut texts = vec![
            String::new(),
            "   \n  ".to_string(),
            "Policy αβγ.\nSDPD café surveillance report.".to_string(),
            "é".repeat(MAX_CHUNK_CHARS),
            "é".repeat(MAX_CHUNK_CHARS + 1),
            "é".repeat(5 * MAX_CHUNK_CHARS + 7),
            format!(
                "{}{}{}",
                "a".repeat(2_000),
                " ".repeat(3_000),
                "b".repeat(2_000)
            ),
        ];
        for _ in 0..40 {
            let length = next(12_000);
            let mut text = String::new();
            while text.chars().count() < length as usize {
                let piece = pieces[next(pieces.len() as u32) as usize];
                // Long boundary-free runs force cuts at the full window.
                let repeat = if next(10) == 0 { 400 } else { 1 };
                text.push_str(&piece.repeat(repeat));
            }
            texts.push(text);
        }
        for text in &texts {
            let streamed = chunk_text(text)
                .map(|chunk| (chunk.start, chunk.end, chunk.text.to_string()))
                .collect::<Vec<_>>();
            assert_eq!(streamed, semantic_chunks_v1(text));
        }
    }

    fn test_document(document_id: u64, text: &str) -> VectorSourceDocument {
        VectorSourceDocument {
            portal_key: "portal".to_string(),
            request_id: 7,
            document_id,
            file_name: "file.pdf".to_string(),
            title: "title".to_string(),
            agency_name: "agency".to_string(),
            jurisdiction: "jurisdiction".to_string(),
            document_type: None,
            publication_date: None,
            observation_date: None,
            effective_date: None,
            raw_file_sha256: "raw".to_string(),
            extracted_text_sha256: sha256_hex(text.as_bytes()),
            text: text.to_string(),
        }
    }

    /// A vector that is a function of the passage text alone.
    fn text_vector(text: &str) -> Vec<f32> {
        let digest = sha2::Sha256::digest(text.as_bytes());
        (0..EMBEDDING_DIMENSION)
            .map(|index| f32::from(digest[index % digest.len()]) - 127.5)
            .collect()
    }

    fn stored_chunks(
        connection: &Connection,
        document_id: u64,
    ) -> Vec<(i64, i64, String, Vec<u8>)> {
        connection
            .prepare(
                "SELECT character_start, character_end, excerpt, vector FROM embedding_chunks
                 WHERE document_id = ?1 ORDER BY character_start",
            )
            .unwrap()
            .query_map(params![document_id], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .unwrap()
            .collect::<duckdb::Result<Vec<_>>>()
            .unwrap()
    }

    /// The 11 GB build came from handing a whole document to one model call, which holds every
    /// batch's activations until the call returns. However large the document, each call must
    /// see at most one batch, and every passage must still be stored once, in order.
    #[tokio::test]
    async fn a_large_document_reaches_the_model_one_bounded_batch_at_a_time() {
        let temp = tempfile::tempdir().unwrap();
        let index =
            VectorIndex::open(&temp.path().join("vectors.db"), temp.path().join("cache")).unwrap();
        let paragraph = "Encampment abatement notice posted at the site; property stored.\n";
        let text = paragraph.repeat(3_000_000 / paragraph.len());
        let document = test_document(1, &text);
        let mut calls = Vec::new();
        let outcome = store_document(
            &mut *index.writer.lock().await,
            &document,
            &AtomicBool::new(false),
            |texts| {
                calls.push(texts.len());
                Ok(texts.iter().map(|text| text_vector(text)).collect())
            },
        )
        .unwrap();

        let expected = chunk_text(&text).collect::<Vec<_>>();
        assert!(expected.len() > 1_000);
        assert!(calls.iter().all(|&size| size <= EMBED_BATCH_SIZE));
        assert_eq!(calls.iter().sum::<usize>(), expected.len());
        assert!(matches!(outcome, IndexOneOutcome::Indexed(count) if count == expected.len()));
        let connection = index.connection.lock().await;
        let stored = stored_chunks(&connection, 1);
        assert_eq!(stored.len(), expected.len());
        for (row, chunk) in stored.iter().zip(&expected) {
            assert_eq!((row.0, row.1), (chunk.start as i64, chunk.end as i64));
            assert_eq!(row.2, chunk.text);
            assert_eq!(row.3, encode_vector(&normalize(text_vector(chunk.text))));
        }
        let state_count: i64 = connection
            .query_row(
                "SELECT chunk_count FROM embedding_document_state WHERE document_id = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(state_count as usize, expected.len());
    }

    /// Replacing a document is one transaction: an interrupted re-embed leaves the previous
    /// version exactly as it was, and a completed one replaces it, including chunks whose
    /// `stable_id` (same start offset) is reused by the new text.
    #[tokio::test]
    async fn an_interrupted_reembed_keeps_the_previous_version() {
        let temp = tempfile::tempdir().unwrap();
        let index =
            VectorIndex::open(&temp.path().join("vectors.db"), temp.path().join("cache")).unwrap();
        let embed = |texts: &[&str]| -> Result<Vec<Vec<f32>>, AppError> {
            Ok(texts.iter().map(|text| text_vector(text)).collect())
        };
        let original = test_document(1, &"First version of the release.\n".repeat(2_000));
        store_document(
            &mut *index.writer.lock().await,
            &original,
            &AtomicBool::new(false),
            embed,
        )
        .unwrap();
        let before = stored_chunks(&*index.connection.lock().await, 1);
        assert!(before.len() > 2 * EMBED_BATCH_SIZE);

        let revised = test_document(1, &"Second version, re-extracted text.\n".repeat(2_000));
        let cancelled = AtomicBool::new(false);
        let mut calls = 0;
        let outcome = store_document(
            &mut *index.writer.lock().await,
            &revised,
            &cancelled,
            |texts| {
                calls += 1;
                if calls == 2 {
                    cancelled.store(true, Ordering::SeqCst);
                }
                embed(texts)
            },
        )
        .unwrap();
        assert!(matches!(outcome, IndexOneOutcome::Cancelled));
        assert_eq!(stored_chunks(&*index.connection.lock().await, 1), before);

        let failing = store_document(
            &mut *index.writer.lock().await,
            &revised,
            &AtomicBool::new(false),
            |_| Err(AppError::Storage("model failed".to_string())),
        );
        assert!(failing.is_err());
        assert_eq!(stored_chunks(&*index.connection.lock().await, 1), before);

        store_document(
            &mut *index.writer.lock().await,
            &revised,
            &AtomicBool::new(false),
            embed,
        )
        .unwrap();
        let after = stored_chunks(&*index.connection.lock().await, 1);
        let expected = chunk_text(&revised.text)
            .map(|chunk| chunk.text.to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            after.iter().map(|row| row.2.clone()).collect::<Vec<_>>(),
            expected
        );
        assert_eq!(after[0].0, before[0].0);
    }

    /// A purge arriving while the document is being re-embedded must wait for that replacement
    /// and then remove it, rather than collide with its open transaction and fail, which would
    /// leave the destroyed document's new excerpts searchable once the replacement commits.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_purge_during_reembedding_waits_and_then_removes_it() {
        let temp = tempfile::tempdir().unwrap();
        let index =
            VectorIndex::open(&temp.path().join("vectors.db"), temp.path().join("cache")).unwrap();
        let embed = |texts: &[&str]| -> Result<Vec<Vec<f32>>, AppError> {
            Ok(texts.iter().map(|text| text_vector(text)).collect())
        };
        let original = test_document(1, &"Original release text.\n".repeat(200));
        store_document(
            &mut *index.writer.lock().await,
            &original,
            &AtomicBool::new(false),
            embed,
        )
        .unwrap();

        let (started, embedding) = std::sync::mpsc::channel();
        let (release, released) = std::sync::mpsc::channel::<()>();
        let writer = Arc::clone(&index.writer);
        let replacing = tokio::task::spawn_blocking(move || {
            let revised = test_document(1, &"Revised release text.\n".repeat(200));
            store_document(
                &mut writer.blocking_lock(),
                &revised,
                &AtomicBool::new(false),
                |texts| {
                    let _ = started.send(());
                    let _ = released.recv();
                    embed(texts)
                },
            )
        });
        tokio::task::spawn_blocking(move || embedding.recv())
            .await
            .unwrap()
            .unwrap();
        let purging = tokio::spawn({
            let index = index.clone();
            async move { index.purge_document("portal".to_string(), 1).await }
        });
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert!(!purging.is_finished());

        drop(release);
        replacing.await.unwrap().unwrap();
        purging.await.unwrap().unwrap();
        assert!(stored_chunks(&*index.connection.lock().await, 1).is_empty());
    }

    /// Re-indexing a document whose extracted text is unchanged embeds nothing (no model is
    /// loaded here, so any embedding attempt would fail) and keeps its vectors, while a corrected
    /// attribution still reaches its chunks.
    #[tokio::test]
    async fn reindexing_unchanged_text_embeds_nothing() {
        let temp = tempfile::tempdir().unwrap();
        let index =
            VectorIndex::open(&temp.path().join("vectors.db"), temp.path().join("cache")).unwrap();
        let document = test_document(1, &"Shelter capacity report.\n".repeat(300));
        store_document(
            &mut *index.writer.lock().await,
            &document,
            &AtomicBool::new(false),
            |texts| Ok(texts.iter().map(|text| text_vector(text)).collect()),
        )
        .unwrap();
        let before = stored_chunks(&*index.connection.lock().await, 1);

        let outcome = index.index_one_document(document.clone()).await.unwrap();
        assert!(matches!(outcome, IndexOneOutcome::Skipped));
        let corrected = VectorSourceDocument {
            title: "corrected title".to_string(),
            ..document
        };
        let outcome = index.index_one_document(corrected).await.unwrap();
        assert!(matches!(outcome, IndexOneOutcome::Skipped));

        let connection = index.connection.lock().await;
        assert_eq!(stored_chunks(&connection, 1), before);
        let titles: Vec<String> = connection
            .prepare("SELECT DISTINCT title FROM embedding_chunks")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<duckdb::Result<_>>()
            .unwrap();
        assert_eq!(titles, ["corrected title"]);
    }

    fn insert_chunk(
        connection: &Connection,
        document_id: u64,
        start: i64,
        end: i64,
        vector: &[f32],
    ) {
        connection
            .execute(
                "INSERT INTO embedding_chunks (
                    stable_id, portal_key, request_id, document_id, file_name, title,
                    agency_name, jurisdiction, character_start, character_end, excerpt,
                    content_sha256, chunk_sha256, model_id, model_revision, dimension,
                    distance_metric, normalization, chunking_version, vector, indexed_at
                 ) VALUES (?1, 'portal', 1, ?2, 'f', 't', 'a', 'j', ?3, ?4, ?5, 'c', 'h',
                    ?6, ?7, 384, 'd', 'n', 'v', ?8, 'now')",
                params![
                    format!("portal:{document_id}:{start}:{EMBEDDING_MODEL_REVISION}"),
                    document_id,
                    start,
                    end,
                    format!("excerpt {document_id}@{start}"),
                    EMBEDDING_MODEL_ID,
                    EMBEDDING_MODEL_REVISION,
                    encode_vector(vector),
                ],
            )
            .unwrap();
    }

    /// The ranking scans once and reads only what it returns; its results must equal ranking
    /// every passage, dropping overlaps, and then truncating, at every limit, including ties.
    #[tokio::test]
    async fn rankings_equal_the_exhaustive_definition() {
        let temp = tempfile::tempdir().unwrap();
        let index =
            VectorIndex::open(&temp.path().join("vectors.db"), temp.path().join("cache")).unwrap();
        let mut seed = 0x2545_f491_u32;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        };
        // Few distinct vectors, so equal scores are common within and across documents.
        let palette = (0..6)
            .map(|_| {
                (0..EMBEDDING_DIMENSION)
                    .map(|_| (next() % 7) as f32 - 3.0)
                    .collect::<Vec<f32>>()
            })
            .collect::<Vec<_>>();
        let mut rows = Vec::new();
        {
            let connection = index.connection.lock().await;
            for document_id in 1..=5_u64 {
                for step in 0..60_i64 {
                    let start = step * 1_440;
                    let end = start + 1_600;
                    let vector = &palette[(next() % palette.len() as u32) as usize];
                    insert_chunk(&connection, document_id, start, end, vector);
                    rows.push((document_id, start, end, vector.clone()));
                }
            }
        }
        let query = (0..EMBEDDING_DIMENSION)
            .map(|_| (next() % 5) as f32 - 2.0)
            .collect::<Vec<f32>>();
        let score = |vector: &[f32]| dot_encoded(&query, &encode_vector(vector)).unwrap();
        let stable_id = |document_id: u64, start: i64| {
            format!("portal:{document_id}:{start}:{EMBEDDING_MODEL_REVISION}")
        };

        let mut exhaustive = rows
            .iter()
            .map(|(document_id, start, end, vector)| {
                (
                    score(vector),
                    stable_id(*document_id, *start),
                    *document_id,
                    *start,
                    *end,
                )
            })
            .collect::<Vec<_>>();
        exhaustive.sort_by(|left, right| right.0.total_cmp(&left.0).then(left.1.cmp(&right.1)));
        let mut deduped: Vec<(f32, String, u64, i64, i64)> = Vec::new();
        for row in exhaustive {
            if !deduped
                .iter()
                .any(|prior| prior.2 == row.2 && prior.3 < row.4 && row.3 < prior.4)
            {
                deduped.push(row);
            }
        }
        let filter: HybridSearchFilter = serde_json::from_value(serde_json::json!({})).unwrap();
        for limit in [0, 1, 3, 7, deduped.len(), deduped.len() + 5] {
            let ranked = in_one_snapshot(&mut *index.connection.lock().await, |connection| {
                rank_passages(connection, &query, &filter, limit)
            })
            .unwrap();
            let expected = deduped.iter().take(limit).collect::<Vec<_>>();
            assert_eq!(ranked.len(), expected.len());
            for (result, row) in ranked.iter().zip(expected) {
                assert_eq!(result.stable_id, row.1);
                assert_eq!(result.semantic_score, Some(row.0 as f64));
                assert_eq!(result.excerpt, format!("excerpt {}@{}", row.2, row.3));
            }
        }

        let mut best: Vec<(u64, f32, i64)> = Vec::new();
        for (document_id, start, _, vector) in &rows {
            let cosine = score(vector);
            match best.iter_mut().find(|held| held.0 == *document_id) {
                Some(held) if cosine > held.1 || (cosine == held.1 && *start < held.2) => {
                    *held = (*document_id, cosine, *start)
                }
                Some(_) => {}
                None => best.push((*document_id, cosine, *start)),
            }
        }
        best.sort_by(|left, right| right.1.total_cmp(&left.1).then(left.0.cmp(&right.0)));
        for limit in [1, 3, usize::MAX] {
            let ranked = in_one_snapshot(&mut *index.connection.lock().await, |connection| {
                rank_documents(connection, &query, "portal", limit)
            })
            .unwrap();
            let expected = best.iter().take(limit).collect::<Vec<_>>();
            assert_eq!(
                ranked
                    .iter()
                    .map(|document| (
                        document.document_id,
                        document.cosine,
                        document.character_start
                    ))
                    .collect::<Vec<_>>(),
                expected.into_iter().copied().collect::<Vec<_>>()
            );
            for document in &ranked {
                assert_eq!(
                    document.excerpt,
                    format!(
                        "excerpt {}@{}",
                        document.document_id, document.character_start
                    )
                );
            }
        }
    }

    #[test]
    fn vector_codec_rejects_wrong_dimensions() {
        let vector = vec![0.5_f32; EMBEDDING_DIMENSION];
        let encoded = encode_vector(&vector);
        assert_eq!(
            dot_encoded(&vector, &encoded).unwrap(),
            0.25 * EMBEDDING_DIMENSION as f32
        );
        assert!(dot_encoded(&vector, &encoded[..encoded.len() - 4]).is_err());
    }

    #[tokio::test]
    #[ignore = "Downloads the pinned FastEmbed ONNX model into a temporary cache"]
    async fn real_model_initializes_and_records_artifacts() {
        let temporary = tempfile::tempdir().unwrap();
        let index = VectorIndex::open(
            &temporary.path().join("vectors.db"),
            temporary.path().join("models"),
        )
        .unwrap();
        let status = index.initialize_model().await.unwrap();
        assert!(status.ready);
        assert!(status.installed);
        assert_eq!(status.dimension, EMBEDDING_DIMENSION);
        assert!(status.artifact_count > 0);
        assert!(status.artifact_manifest_sha256.is_some());
    }
}

#[cfg(test)]
mod live_collection_measurement {
    use super::*;
    use std::io::Write;
    use std::time::Instant;

    const QUERIES: &[&str] = &[
        "encampment abatement schedule",
        "shelter bed capacity and occupancy",
        "COVID-19 swab testing for city employees",
        "Caltrans delegated maintenance agreement",
        "police citations for illegal lodging",
        "body worn camera footage request",
        "safe sleeping site operations",
        "homeless outreach team emails",
        "fire at a homeless encampment",
        "protest arrests at UC San Diego",
        "unsafe camping ordinance enforcement",
        "sidewalk cleanup and property storage notices",
    ];

    /// Builds the semantic index over a collected live database and records what it cost and
    /// what it returns, so an indexing change is judged on real records: wall time and
    /// passages/s for the build, then per-query latency and the top 20 of `semantic_search` and
    /// `nearest_documents` for a fixed query set, written as JSON lines to `ONRECORD_TOPK_OUT`.
    /// `ONRECORD_EVAL_DIR` holds `core.db`; the index is built beside it as `vectors.db` from the
    /// installed model in `ONRECORD_EMBEDDING_CACHE`. Peak memory is read from the process, e.g.
    /// `/usr/bin/time -l <test binary> --exact
    /// storage::vector::live_collection_measurement::index_and_query_a_live_collection
    /// --ignored --nocapture`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "needs a collected live database and an installed embedding model"]
    async fn index_and_query_a_live_collection() {
        let root = PathBuf::from(std::env::var("ONRECORD_EVAL_DIR").expect("ONRECORD_EVAL_DIR"));
        let cache = PathBuf::from(
            std::env::var("ONRECORD_EMBEDDING_CACHE").expect("ONRECORD_EMBEDDING_CACHE"),
        );
        let out = PathBuf::from(std::env::var("ONRECORD_TOPK_OUT").expect("ONRECORD_TOPK_OUT"));
        let repository = DuckDbRepository::new(root.join("core.db")).unwrap();
        let index = VectorIndex::open(&root.join("vectors.db"), cache).unwrap();
        index.initialize_model().await.unwrap();

        let started = Instant::now();
        let report = index.index_documents(&repository, false).await.unwrap();
        let seconds = started.elapsed().as_secs_f64();
        eprintln!(
            "index: {report:?} in {seconds:.1}s = {:.1} passages/s",
            report.indexed_chunks as f64 / seconds
        );

        let portal_key = index
            .connection
            .lock()
            .await
            .query_row("SELECT MIN(portal_key) FROM embedding_chunks", [], |row| {
                row.get::<_, String>(0)
            })
            .unwrap();
        let mut file = fs::File::create(&out).unwrap();
        for query in QUERIES {
            let filter = serde_json::from_value(serde_json::json!({})).unwrap();
            let started = Instant::now();
            let passages = index.semantic_search(query, filter, 20).await.unwrap();
            let passage_ms = started.elapsed().as_secs_f64() * 1000.0;
            let started = Instant::now();
            let documents = index
                .nearest_documents(query, &portal_key, 20)
                .await
                .unwrap()
                .unwrap();
            let document_ms = started.elapsed().as_secs_f64() * 1000.0;
            eprintln!("{query}: semantic_search {passage_ms:.0} ms, nearest_documents {document_ms:.0} ms");
            let line = serde_json::json!({
                "query": query,
                "semantic_search": passages.iter().map(|result| serde_json::json!([
                    result.stable_id, result.character_start, result.character_end,
                    result.semantic_score.map(|score| (score as f32).to_bits()),
                ])).collect::<Vec<_>>(),
                "nearest_documents": documents.iter().map(|document| serde_json::json!([
                    document.document_id, document.character_start, document.character_end,
                    document.cosine.to_bits(),
                ])).collect::<Vec<_>>(),
            });
            writeln!(file, "{line}").unwrap();
        }
    }

    /// Embeds the first 512 passages of a live collection under different intra-op thread
    /// counts and batch shapes and reports, per configuration, whether every vector is
    /// bit-identical to the indexing configuration (2 threads, batches of 16) and its speed.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "needs a collected live database and an installed embedding model"]
    async fn embedding_reproducibility_across_threads_and_batches() {
        let root = PathBuf::from(std::env::var("ONRECORD_EVAL_DIR").expect("ONRECORD_EVAL_DIR"));
        let cache = PathBuf::from(
            std::env::var("ONRECORD_EMBEDDING_CACHE").expect("ONRECORD_EMBEDDING_CACHE"),
        );
        let repository = DuckDbRepository::new(root.join("core.db")).unwrap();
        let mut passages = Vec::new();
        let mut after = None;
        while passages.len() < 512 {
            let page = repository
                .list_vector_source_documents_page(after.take(), 8)
                .await
                .unwrap();
            let Some(last) = page.last() else { break };
            after = Some((last.portal_key.clone(), last.document_id));
            for document in page {
                passages.extend(chunk_text(&document.text).map(|chunk| chunk.text.to_string()));
            }
        }
        passages.truncate(512);
        let embed = |threads: usize, batch: usize| {
            let mut model = TextEmbedding::try_new(
                TextInitOptions::new(EmbeddingModel::BGESmallENV15Q)
                    .with_cache_dir(cache.clone())
                    .with_max_length(512)
                    .with_intra_threads(threads),
            )
            .unwrap();
            let started = Instant::now();
            let mut vectors = Vec::new();
            for group in passages.chunks(batch) {
                vectors.extend(model.embed(group, Some(batch)).unwrap());
            }
            (vectors, started.elapsed().as_secs_f64())
        };
        let (reference, seconds) = embed(2, 16);
        eprintln!(
            "threads 2 batch 16: {:.1} passages/s",
            passages.len() as f64 / seconds
        );
        for (threads, batch) in [(2, 16), (4, 16), (8, 16), (4, 32), (4, 1)] {
            let (vectors, seconds) = embed(threads, batch);
            let identical = vectors
                .iter()
                .zip(&reference)
                .filter(|(left, right)| left == right)
                .count();
            let max_difference = vectors
                .iter()
                .zip(&reference)
                .flat_map(|(left, right)| left.iter().zip(right).map(|(a, b)| (a - b).abs()))
                .fold(0.0_f32, f32::max);
            eprintln!(
                "threads {threads} batch {batch}: {:.1} passages/s, {identical}/{} bit-identical, max |diff| {max_difference:e}",
                passages.len() as f64 / seconds,
                passages.len()
            );
        }
    }
}

#[cfg(test)]
mod subject_relevance_calibration {
    use super::*;

    /// Prints bge-small cosine similarity for labelled real titles, to set the subject threshold.
    /// Needs the installed model: `ONRECORD_MODEL_CACHE=<app data>/models cargo test --lib
    /// subject_relevance_calibration -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore = "needs the installed embedding model"]
    async fn similarity_of_labelled_titles() {
        let cache = std::env::var("ONRECORD_MODEL_CACHE").expect("ONRECORD_MODEL_CACHE");
        let temp = tempfile::tempdir().unwrap();
        let index = VectorIndex::open(&temp.path().join("v.db"), PathBuf::from(cache)).unwrap();
        let cases: &[(&str, &[(&str, bool)])] = &[
            (
                "homelessness",
                &[
                    ("Homelessness Hub at UC San Diego", true),
                    ("Eviction Study UCSD Panel 2024", true),
                    ("Unsheltered point in time count 2024 report", true),
                    ("Encampment abatement schedule", true),
                    ("Safe sleeping site operations agreement", true),
                    ("RV safe parking program expansion", true),
                    ("Housing", true),
                    ("Parking revenue trends", false),
                    ("911 Calls", false),
                    ("Budget visualizations", false),
                    ("Transit Stops GTFS", false),
                    ("get it done requests closed 2025", false),
                    ("Local Businesses", false),
                    ("Indian Housing Block Grant Program", false),
                    ("FY27 Estimate Cover Letter", false),
                    ("cpp minutes 260318", false),
                    ("Opportunity Zones News", false),
                ],
            ),
            (
                "police use of force",
                &[
                    ("RIPA police stop data", true),
                    ("Use of force incidents dataset", true),
                    ("Officer involved shootings 2023", true),
                    ("Commission on Police Practices minutes", true),
                    ("Body worn camera policy", true),
                    ("Homelessness Hub at UC San Diego", false),
                    ("Parking revenue trends", false),
                    ("Budget visualizations", false),
                    ("Indian Housing Block Grant Program", false),
                    ("Transit Stops GTFS", false),
                    ("Local Businesses", false),
                ],
            ),
        ];
        for (subject, titles) in cases {
            let mut texts = vec![format!(
                "Represent this sentence for searching relevant passages: {subject}"
            )];
            texts.extend(titles.iter().map(|(title, _)| title.to_string()));
            let vectors = index
                .embed_short_texts(texts)
                .await
                .unwrap()
                .expect("model installed");
            let mut scored: Vec<(f32, &str, bool)> = titles
                .iter()
                .zip(&vectors[1..])
                .map(|((title, relevant), vector)| {
                    (
                        vector.iter().zip(&vectors[0]).map(|(a, b)| a * b).sum(),
                        *title,
                        *relevant,
                    )
                })
                .collect();
            scored.sort_by(|a, b| b.0.total_cmp(&a.0));
            for (score, title, relevant) in scored {
                eprintln!(
                    "{subject}\t{score:.3}\t{}\t{title}",
                    if relevant { "REL" } else { "---" }
                );
            }
        }
    }
}
