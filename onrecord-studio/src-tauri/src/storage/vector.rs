//! Rebuildable local embedding projection.

use crate::blocking::spawn_with;
use crate::error::AppError;
use crate::integrity::sha256_hex;
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
const EMBED_BATCH_SIZE: usize = 16;

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
        let connection = Connection::open(path).map_err(|error| {
            AppError::Storage(format!(
                "Failed to open vector index '{}': {error}",
                path.display()
            ))
        })?;
        connection
            .execute_batch(
                "SET preserve_insertion_order = false;
                 SET threads = 1;
                 SET enable_progress_bar = false;
                 SET TimeZone = 'UTC';",
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
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
            cache_dir,
            model: Arc::new(Mutex::new(None)),
            running: Arc::new(AtomicBool::new(false)),
            cancelled: Arc::new(AtomicBool::new(false)),
            failed_documents: Arc::new(AtomicUsize::new(0)),
            last_error: Arc::new(Mutex::new(None)),
        })
    }

    /// Runs a synchronous DuckDB operation on a blocking worker rather than directly in the async
    /// body, mirroring `DuckDbRepository::query_connection` (`storage/sqlite.rs`). Holding the
    /// connection lock while running a blocking DuckDB call straight on a tokio worker thread
    /// blocks that worker for the query's duration; this moves the call off it instead.
    async fn query_connection<T, F>(&self, operation: &'static str, query: F) -> Result<T, AppError>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T, AppError> + Send + 'static,
    {
        let connection = Arc::clone(&self.connection);
        spawn_with(operation, AppError::Internal, move || {
            let mut connection = connection.blocking_lock();
            query(&mut connection)
        })
        .await?
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
            let options = TextInitOptions::new(EmbeddingModel::BGESmallENV15Q)
                .with_cache_dir(cache_dir)
                .with_show_download_progress(true)
                .with_max_length(512)
                .with_intra_threads(2);
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

    pub async fn index_documents(
        &self,
        documents: Vec<VectorSourceDocument>,
        rebuild: bool,
    ) -> Result<EmbeddingIndexReport, AppError> {
        if self.running.swap(true, Ordering::SeqCst) {
            return Err(AppError::Storage(
                "Semantic indexing is already running".to_string(),
            ));
        }
        self.cancelled.store(false, Ordering::SeqCst);
        self.failed_documents.store(0, Ordering::SeqCst);
        let result = self.index_documents_inner(documents, rebuild).await;
        self.running.store(false, Ordering::SeqCst);
        result
    }

    pub fn cancel_indexing(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
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
        let results = self
            .query_connection("Semantic search", move |connection| {
                let mut statement = connection
                    .prepare(
                        "SELECT stable_id, portal_key, request_id, document_id, file_name, title,
                        agency_name, jurisdiction, document_type, publication_date,
                        observation_date, effective_date, character_start, character_end,
                        excerpt, vector
                 FROM embedding_chunks
                 WHERE model_id = ?1 AND model_revision = ?2
                   AND (?3 IS NULL OR portal_key = ?3)
                   AND (?4 IS NULL OR LOWER(agency_name) LIKE '%' || LOWER(?4) || '%')
                   AND (?5 IS NULL OR LOWER(jurisdiction) = LOWER(?5))
                   AND (?6 IS NULL OR LOWER(COALESCE(document_type, '')) = LOWER(?6))
                 ORDER BY stable_id",
                    )
                    .map_err(|error| {
                        AppError::Storage(format!("Failed to prepare semantic search: {error}"))
                    })?;
                let portal = filter
                    .portal_key
                    .clone()
                    .filter(|value| !value.trim().is_empty());
                let agency = filter
                    .agency_name
                    .clone()
                    .filter(|value| !value.trim().is_empty());
                let jurisdiction = filter
                    .jurisdiction
                    .clone()
                    .filter(|value| !value.trim().is_empty());
                let doc_type = filter
                    .document_type
                    .clone()
                    .filter(|value| !value.trim().is_empty());
                let rows = statement
                    .query_map(
                        params![
                            EMBEDDING_MODEL_ID,
                            EMBEDDING_MODEL_REVISION,
                            portal,
                            agency,
                            jurisdiction,
                            doc_type
                        ],
                        |row| {
                            let blob: Vec<u8> = row.get(15)?;
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, u64>(2)?,
                                row.get::<_, u64>(3)?,
                                row.get::<_, String>(4)?,
                                row.get::<_, String>(5)?,
                                row.get::<_, String>(6)?,
                                row.get::<_, String>(7)?,
                                row.get::<_, Option<String>>(8)?,
                                row.get::<_, Option<String>>(9)?,
                                row.get::<_, Option<String>>(10)?,
                                row.get::<_, Option<String>>(11)?,
                                row.get::<_, i64>(12)?,
                                row.get::<_, i64>(13)?,
                                row.get::<_, String>(14)?,
                                blob,
                            ))
                        },
                    )
                    .map_err(|error| {
                        AppError::Storage(format!("Failed to execute semantic search: {error}"))
                    })?;
                let mut results = Vec::new();
                for row in rows {
                    let (
                        stable_id,
                        portal_key,
                        request_id,
                        document_id,
                        file_name,
                        title,
                        agency_name,
                        jurisdiction,
                        document_type,
                        publication_date,
                        observation_date,
                        effective_date,
                        start,
                        end,
                        excerpt,
                        blob,
                    ) = row.map_err(|error| {
                        AppError::Storage(format!("Failed to read semantic result: {error}"))
                    })?;
                    if !date_filter_matches(
                        &publication_date,
                        &observation_date,
                        &effective_date,
                        &filter,
                    ) {
                        continue;
                    }
                    let vector = decode_vector(&blob)?;
                    let score = dot(&query_embedding, &vector);
                    results.push(HybridSearchResult {
                        stable_id,
                        portal_key,
                        request_id,
                        document_id,
                        file_name,
                        title,
                        agency_name,
                        jurisdiction,
                        document_type,
                        publication_date,
                        observation_date,
                        effective_date,
                        character_start: start.max(0) as usize,
                        character_end: end.max(0) as usize,
                        excerpt,
                        lexical_rank: None,
                        semantic_score: Some(score as f64),
                        fused_score: score as f64,
                        retrieval_status: "SEMANTIC_READY".to_string(),
                    });
                }
                results.sort_by(|left, right| right.fused_score.total_cmp(&left.fused_score));
                dedupe_results(&mut results);
                results.truncate(limit);
                Ok(results)
            })
            .await?;
        Ok(results)
    }

    async fn index_documents_inner(
        &self,
        documents: Vec<VectorSourceDocument>,
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
        for document in documents {
            if self.cancelled.load(Ordering::SeqCst) {
                report.cancelled = true;
                break;
            }
            match self.index_one_document(document).await {
                Ok(IndexOneOutcome::Indexed(chunks)) => {
                    report.indexed_documents += 1;
                    report.indexed_chunks += chunks;
                }
                Ok(IndexOneOutcome::Skipped) => report.skipped_documents += 1,
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
        Ok(report)
    }

    async fn index_one_document(
        &self,
        document: VectorSourceDocument,
    ) -> Result<IndexOneOutcome, AppError> {
        let content_hash = document.extracted_text_sha256.clone();
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
        let content_hash_for_check = content_hash.clone();
        let skipped = self
            .query_connection("Inspect embedding state", move |connection| {
                let content_hash = content_hash_for_check;
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
                    // Source attribution may be corrected even when the extracted text is unchanged.
                    connection.execute("UPDATE embedding_chunks SET jurisdiction = ?1, agency_name = ?2,
                    title = ?3, publication_date = ?4, observation_date = ?5, effective_date = ?6
                    WHERE portal_key = ?7 AND document_id = ?8 AND model_id = ?9 AND model_revision = ?10",
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

        let chunks = chunk_text(&document.text);
        let texts = chunks
            .iter()
            .map(|chunk| chunk.text.clone())
            .collect::<Vec<_>>();
        let embeddings = {
            let mut model = self.model.lock().await;
            let model = model.as_mut().ok_or_else(|| {
                AppError::Storage("Embedding model is not initialized".to_string())
            })?;
            model
                .embed(texts, Some(EMBED_BATCH_SIZE))
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to embed document {}/{}: {error}",
                        document.portal_key, document.document_id
                    ))
                })?
        };
        if embeddings.len() != chunks.len() {
            return Err(AppError::Storage(format!(
                "Embedding model returned {} vectors for {} chunks",
                embeddings.len(),
                chunks.len()
            )));
        }

        let chunk_count = self
            .query_connection("Store document embeddings", move |connection| {
                let tx = connection.transaction().map_err(|error| {
                    AppError::Storage(format!("Failed to begin vector transaction: {error}"))
                })?;
                tx.execute(
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
                .map_err(|error| {
                    AppError::Storage(format!("Failed to replace old embeddings: {error}"))
                })?;
                for (chunk, vector) in chunks.iter().zip(embeddings) {
                    let vector = encode_vector(&normalize(vector));
                    let stable_id = format!(
                        "{}:{}:{}:{}",
                        document.portal_key,
                        document.document_id,
                        chunk.start,
                        EMBEDDING_MODEL_REVISION
                    );
                    let chunk_hash = sha256_hex(chunk.text.as_bytes());
                    tx.execute(
                        "INSERT INTO embedding_chunks (
                    stable_id, portal_key, request_id, document_id, file_name, title,
                    agency_name, jurisdiction, document_type, publication_date, observation_date,
                    effective_date, character_start, character_end, excerpt, content_sha256,
                    chunk_sha256, model_id, model_revision, dimension, distance_metric,
                    normalization, chunking_version, vector, indexed_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                           ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24,
                           strftime(now(), '%Y-%m-%dT%H:%M:%S.%fZ'))",
                        params![
                            stable_id,
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
                            content_hash,
                            chunk_hash,
                            EMBEDDING_MODEL_ID,
                            EMBEDDING_MODEL_REVISION,
                            EMBEDDING_DIMENSION as i64,
                            EMBEDDING_DISTANCE_METRIC,
                            EMBEDDING_NORMALIZATION,
                            EMBEDDING_CHUNKING_VERSION,
                            vector,
                        ],
                    )
                    .map_err(|error| {
                        AppError::Storage(format!("Failed to store embedding chunk: {error}"))
                    })?;
                }
                tx.execute(
                    "INSERT INTO embedding_document_state (
                portal_key, document_id, content_sha256, raw_file_sha256, model_id,
                model_revision, dimension, distance_metric, normalization, chunking_version,
                chunk_count, indexed_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11,
                       strftime(now(), '%Y-%m-%dT%H:%M:%S.%fZ'))
             ON CONFLICT (portal_key, document_id, model_id, model_revision, chunking_version)
             DO UPDATE SET content_sha256 = excluded.content_sha256,
                           raw_file_sha256 = excluded.raw_file_sha256,
                           chunk_count = excluded.chunk_count,
                           indexed_at = excluded.indexed_at",
                    params![
                        document.portal_key,
                        document.document_id,
                        content_hash,
                        document.raw_file_sha256,
                        EMBEDDING_MODEL_ID,
                        EMBEDDING_MODEL_REVISION,
                        EMBEDDING_DIMENSION as i64,
                        EMBEDDING_DISTANCE_METRIC,
                        EMBEDDING_NORMALIZATION,
                        EMBEDDING_CHUNKING_VERSION,
                        chunks.len() as i64,
                    ],
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to store embedding document state: {error}"))
                })?;
                tx.commit().map_err(|error| {
                    AppError::Storage(format!("Failed to commit vector transaction: {error}"))
                })?;
                Ok(chunks.len())
            })
            .await?;
        Ok(IndexOneOutcome::Indexed(chunk_count))
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

enum IndexOneOutcome {
    Indexed(usize),
    Skipped,
}

struct TextChunk {
    start: usize,
    end: usize,
    text: String,
}

fn chunk_text(text: &str) -> Vec<TextChunk> {
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
        let start = chars[start_char];
        let end = chars[end_char];
        let excerpt = text[start..end].trim().to_string();
        if !excerpt.is_empty() {
            chunks.push(TextChunk {
                start: start_char,
                end: end_char,
                text: excerpt,
            });
        }
        if end_char == total {
            break;
        }
        start_char = end_char.saturating_sub(CHUNK_OVERLAP_CHARS);
    }
    chunks
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

fn dot(left: &[f32], right: &[f32]) -> f32 {
    left.iter().zip(right.iter()).map(|(a, b)| a * b).sum()
}

fn encode_vector(vector: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(vector.len() * 4);
    for value in vector {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

fn decode_vector(bytes: &[u8]) -> Result<Vec<f32>, AppError> {
    if bytes.len() != EMBEDDING_DIMENSION * 4 {
        return Err(AppError::Storage(format!(
            "Stored embedding has {} bytes; expected {}",
            bytes.len(),
            EMBEDDING_DIMENSION * 4
        )));
    }
    Ok(bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect())
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

fn dedupe_results(results: &mut Vec<HybridSearchResult>) {
    let mut retained: Vec<HybridSearchResult> = Vec::new();
    'outer: for result in results.drain(..) {
        for prior in &retained {
            if prior.portal_key == result.portal_key
                && prior.document_id == result.document_id
                && ranges_overlap(
                    prior.character_start,
                    prior.character_end,
                    result.character_start,
                    result.character_end,
                )
            {
                continue 'outer;
            }
        }
        retained.push(result);
    }
    *results = retained;
}

fn ranges_overlap(a_start: usize, a_end: usize, b_start: usize, b_end: usize) -> bool {
    a_start < b_end && b_start < a_end
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

    #[test]
    fn chunking_preserves_unicode_character_offsets() {
        let text = "Policy αβγ.\nSDPD café surveillance report.";
        let chunks = chunk_text(text);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].start, 0);
        assert_eq!(chunks[0].end, text.chars().count());
        assert!(chunks[0].text.contains("café"));
    }

    #[test]
    fn vector_codec_rejects_wrong_dimensions() {
        let vector = vec![0.0_f32; EMBEDDING_DIMENSION];
        let encoded = encode_vector(&vector);
        assert_eq!(decode_vector(&encoded).unwrap().len(), EMBEDDING_DIMENSION);
        assert!(decode_vector(&encoded[..encoded.len() - 4]).is_err());
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
