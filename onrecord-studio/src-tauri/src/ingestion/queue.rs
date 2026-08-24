//! Multi-threaded scheduling queue and worker coordinator for NextRequest processing.

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Semaphore, Mutex};
use tokio::task::JoinHandle;
use crate::error::AppError;
use crate::ingestion::client::ApiClient;
use crate::ingestion::limiter::RateLimiter; // Implements your token-bucket threshold checks
use crate::types::{PublicRequest, DocumentMetadata};

/// Live ingestion telemetry dispatched upstream to update the Tauri UI panel.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "type", content = "payload", rename_all = "camelCase")]
pub enum IngestionProgress {
    PageStarted { subdomain: String, page: usize },
    PageCompleted { subdomain: String, page: usize, records_found: usize },
    DocumentStarted { document_id: u64, file_name: String },
    DocumentCompleted { document_id: u64, bytes_saved: u64 },
    TaskFailed { target_id: String, error_msg: String },
}

/// Task definitions queued inside the internal channel pipeline.
#[derive(Debug, Clone)]
pub enum IngestionTask {
    FetchMetadataPage { subdomain: String, page: usize, per_page: usize },
    DownloadBinaryPayload { subdomain: String, document: DocumentMetadata, output_directory: String },
}

pub struct QueueManager {
    client: ApiClient,
    limiter: Arc<RateLimiter>,
    task_sender: mpsc::Sender<IngestionTask>,
    task_receiver: Arc<Mutex<mpsc::Receiver<IngestionTask>>>,
    progress_sender: mpsc::Sender<IngestionProgress>,
    concurrency_semaphore: Arc<Semaphore>,
    worker_handles: Mutex<Vec<JoinHandle<()>>>,
}

impl QueueManager {
    /// Instantiates a thread-safe scheduler engine.
    pub fn new(
        client: ApiClient,
        limiter: RateLimiter,
        progress_sender: mpsc::Sender<IngestionProgress>,
        max_concurrent_workers: usize,
    ) -> Self {
        // Buffer channel handles up to 10,000 tasks simultaneously
        let (task_sender, task_receiver) = mpsc::channel(10000);
        
        Self {
            client,
            limiter: Arc::new(limiter),
            task_sender,
            task_receiver: Arc::new(Mutex::new(task_receiver)),
            progress_sender,
            concurrency_semaphore: Arc::new(Semaphore::new(max_concurrent_workers)),
            worker_handles: Mutex::new(Vec::new()),
        }
    }

    /// Provides structural access to dispatch execution commands directly onto the queue.
    pub fn get_dispatcher(&self) -> mpsc::Sender<IngestionTask> {
        self.task_sender.clone()
    }

    /// Boots up the parallel worker pool to process inbound tasks asynchronously.
    pub async fn start_workers(&self, num_workers: usize) {
        let mut handles = self.worker_handles.lock().await;
        
        for worker_id in 0..num_workers {
            let rx = Arc::clone(&self.task_receiver);
            let client = self.client.clone();
            let limiter = Arc::clone(&self.limiter);
            let progress = self.progress_sender.clone();
            let semaphore = Arc::clone(&self.concurrency_semaphore);
            let dispatcher = self.task_sender.clone();

            let handle = tokio::spawn(async move {
                loop {
                    // Lock the channel receiver to pull the next execution block safely
                    let task = {
                        let mut receiver = rx.lock().await;
                        receiver.recv().await
                    };

                    match task {
                        Some(IngestionTask::FetchMetadataPage { subdomain, page, per_page }) => {
                            let _permit = semaphore.acquire().await.unwrap();
                            limiter.throttle(&subdomain).await; // Block worker if hitting target rates

                            let _ = progress.send(IngestionProgress::PageStarted { 
                                subdomain: subdomain.clone(), 
                                page 
                            }).await;

                            match client.fetch_requests(&subdomain, page, per_page).await {
                                Ok(response) => {
                                    let records_count = response.requests.len();
                                    
                                    // Process nested requests and queue downstream tasks
                                    for mut record in response.requests {
                                        record.compute_timeline_anomalies();
                                        
                                        // TODO: Pass record metadata envelope directly to your SQLite repository here
                                        
                                        // Automatically schedule binary asset extraction tasks
                                        for doc in record.documents {
                                            let _ = dispatcher.send(IngestionTask::DownloadBinaryPayload {
                                                subdomain: subdomain.clone(),
                                                document: doc,
                                                output_directory: format!("./downloads/{}_documents", subdomain),
                                            }).await;
                                        }
                                    }

                                    let _ = progress.send(IngestionProgress::PageCompleted { 
                                        subdomain: subdomain.clone(), 
                                        page, 
                                        records_found: records_count 
                                    }).await;

                                    // Auto-advance paginations if additional content streams remain
                                    if let Some(meta) = response.meta {
                                        if meta.next_page.is_some() && page < meta.total_pages {
                                            let _ = dispatcher.send(IngestionTask::FetchMetadataPage {
                                                subdomain: subdomain.clone(),
                                                page: page + 1,
                                                per_page,
                                            }).await;
                                        }
                                    }
                                }
                                Err(e) => {
                                    let _ = progress.send(IngestionProgress::TaskFailed {
                                        target_id: format!("{}-page-{}", subdomain, page),
                                        error_msg: e.to_string(),
                                    }).await;
                                }
                            }
                        }
                        Some(IngestionTask::DownloadBinaryPayload { subdomain, document, output_directory }) => {
                            let _permit = semaphore.acquire().await.unwrap();
                            limiter.throttle(&subdomain).await;

                            let doc_id = document.id;
                            let filename = document.file_name.unwrap_or_else(|| format!("document_{}.bin", doc_id));

                            let _ = progress.send(IngestionProgress::DocumentStarted { 
                                document_id: doc_id, 
                                file_name: filename.clone() 
                            }).await;

                            match client.download_document(&subdomain, doc_id).await {
                                Ok(bytes) => {
                                    let bytes_len = bytes.len() as u64;
                                    
                                    // Async file layout persistence block
                                    let file_path = std::path::Path::new(&output_directory).join(format!("{}_{}", doc_id, filename));
                                    if let Some(parent) = file_path.parent() {
                                        let _ = tokio::fs::create_dir_all(parent).await;
                                    }
                                    
                                    if let Ok(mut file) = tokio::fs::File::create(&file_path).await {
                                        use tokio::io::AsyncWriteExt;
                                        let _ = file.write_all(&bytes).await;
                                    }

                                    let _ = progress.send(IngestionProgress::DocumentCompleted { 
                                        document_id: doc_id, 
                                        bytes_saved: bytes_len 
                                    }).await;
                                }
                                Err(e) => {
                                    let _ = progress.send(IngestionProgress::TaskFailed {
                                        target_id: format!("{}-doc-{}", subdomain, doc_id),
                                        error_msg: e.to_string(),
                                    }).await;
                                }
                            }
                        }
                        None => break, // Channel closed completely, shut down thread worker cleanly
                    }
                }
            });

            handles.push(handle);
        }
    }
}
