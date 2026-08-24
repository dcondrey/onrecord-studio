//! Tauri command IPC bridges connecting frontend events to the background ingestion queue.

use tauri::{State, AppHandle, Emitter};
use tokio::sync::mpsc;
use crate::error::AppError;
use crate::ingestion::client::{ApiClient, HttpClientConfig};
use crate::ingestion::limiter::RateLimiter;
use crate::ingestion::queue::{QueueManager, IngestionTask, IngestionProgress};

pub struct AppQueueState {
    pub dispatcher: mpsc::Sender<IngestionTask>,
}

/// Dispatches a background execution command to start scraping a target NextRequest subdomain.
#[tauri::command]
pub async fn start_subdomain_sync(
    subdomain: String,
    start_page: usize,
    per_page: usize,
    state: State<'_, AppQueueState>,
) -> Result<String, AppError> {
    let task = IngestionTask::FetchMetadataPage {
        subdomain: subdomain.clone(),
        page: start_page,
        per_page,
    };

    // Push the command into the thread channel buffer instantly without halting UI runtime threads
    state.dispatcher.send(task).await
        .map_err(|_| AppError::Internal("Ingestion core queue is currently unreachable".to_string()))?;

    Ok(format!("Sync initialized for portal: {}.nextrequest.com", subdomain))
}

/// Natively initializes the application's global workspace queue supervisor.
pub async fn initialize_queue_runtime(app_handle: AppHandle) -> mpsc::Sender<IngestionTask> {
    let client_config = HttpClientConfig::default();
    let client = ApiClient::new(client_config).unwrap();
    
    // Set token bucket parameters to max burst of 5 requests, refilling 1 token/sec
    let rate_limiter = RateLimiter::new(5.0, 1.0);
    
    // Wire up an internal MPSC feedback channel loop for live telemetry reporting
    let (progress_tx, mut progress_rx) = mpsc::channel::<IngestionProgress>(1000);

    // Boot up the QueueManager with 4 concurrent worker threads to split the network load
    let queue_manager = QueueManager::new(client, rate_limiter, progress_tx, 4);
    queue_manager.start_workers(4).await;
    
    let dispatcher = queue_manager.get_dispatcher();
    
    // Spawn a dedicated listener task to stream structural pipeline progress logs over to the UI
    tokio::spawn(async move {
        while let Some(progress_event) = progress_rx.recv().await {
            // Emits JSON-serialized state modifications over to the React context engine
            let _ = app_handle.emit("ingestion-telemetry", &progress_event);
        }
    });

    dispatcher
}
