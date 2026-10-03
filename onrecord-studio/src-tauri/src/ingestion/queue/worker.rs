//! The per-portal worker loop, task dispatch, durable retry policy, and pool retirement.

use super::*;
use super::{
    crawl::{announce_host_stop, fetch_hosted_request, fetch_metadata_page},
    frontier::{decrement_atomic, refill_ingestion_frontier},
    objects::import_local_payload,
    payload::process_binary_payload,
    resources::{
        process_arcgis_layer, process_data_catalog_page, process_public_resource,
        process_wordpress_google_drive,
    },
};

pub(super) async fn worker_loop(
    context: WorkerContext,
    receiver: Arc<Mutex<mpsc::Receiver<ScheduledTask>>>,
    mut cancellation: watch::Receiver<bool>,
) {
    loop {
        let task = tokio::select! {
            changed = cancellation.changed() => {
                if changed.is_err() || *cancellation.borrow() {
                    break;
                }
                continue;
            }
            task = receive_task(&receiver) => task,
        };
        let Some(scheduled) = task else {
            break;
        };
        decrement_atomic(&context.statistics.queued_tasks);
        context
            .statistics
            .active_tasks
            .fetch_add(1, Ordering::AcqRel);
        let target_id = scheduled.task.target_id();
        let target_url = scheduled.task.target_url().map(str::to_string);
        let frontier_id = scheduled.frontier_id;
        let mut frontier_attempt = scheduled.frontier_attempt;
        let started = match frontier_id {
            Some(frontier_id) => context
                .repository
                .start_ingestion_frontier_task(frontier_id)
                .await
                .map(|attempt| frontier_attempt = attempt),
            None => Ok(()),
        };
        let mut result = match started {
            Ok(()) => execute_task(&context, scheduled.task).await,
            Err(error) => Err(error),
        };
        let retry_delay =
            frontier_id
                .zip(result.as_ref().err())
                .and_then(|(frontier_id, error)| {
                    durable_retry_delay_for_error(
                        error,
                        frontier_attempt,
                        frontier_id.unsigned_abs(),
                    )
                });
        if let (Some(frontier_id), Some(delay_seconds)) = (frontier_id, retry_delay) {
            // Counted back in before the deferral commits, for the reason `enqueue_tasks_resilient`
            // gives: a refill may claim the row the moment it is available again.
            context
                .statistics
                .queued_tasks
                .fetch_add(1, Ordering::AcqRel);
            match context
                .repository
                .defer_ingestion_frontier_task(frontier_id, delay_seconds)
                .await
            {
                Ok(()) => {
                    let error_msg = result
                        .as_ref()
                        .err()
                        .map(ToString::to_string)
                        .unwrap_or_default();
                    let _ = context
                        .progress_sender
                        .send(IngestionProgress::TaskRetryScheduled {
                            portal_key: context.portal.portal_key.clone(),
                            target_id,
                            attempt: frontier_attempt,
                            delay_seconds,
                            error_msg,
                        })
                        .await;
                    decrement_atomic(&context.statistics.active_tasks);
                    if *context.cancellation.borrow() {
                        break;
                    }
                    // This wake is the only thing that re-dispatches a deferred row: workers
                    // idle in `receive_task`, and nothing ticks. A refill that fails once must
                    // therefore be retried and reported, or the pool holds `queued_tasks` above
                    // zero forever, never retires, and nobody is told why.
                    let retry_context = context.clone();
                    tokio::spawn(async move {
                        for _ in 0..MAXIMUM_DURABLE_TASK_ATTEMPTS {
                            tokio::time::sleep(std::time::Duration::from_secs(
                                delay_seconds.saturating_add(1),
                            ))
                            .await;
                            if *retry_context.cancellation.borrow() {
                                return;
                            }
                            match refill_ingestion_frontier(&retry_context).await {
                                Ok(()) => return,
                                Err(error) => {
                                    // A stop that landed mid-refill closes the channel; that is
                                    // the stop's outcome, not a failure of this pool.
                                    if *retry_context.cancellation.borrow() {
                                        return;
                                    }
                                    send_progress(
                                        &retry_context,
                                        IngestionProgress::TaskFailed {
                                            portal_key: retry_context.portal.portal_key.clone(),
                                            target_id: format!(
                                                "{}-frontier-refill",
                                                retry_context.sync_run_id
                                            ),
                                            error_msg: error.to_string(),
                                        },
                                    )
                                    .await;
                                }
                            }
                        }
                    });
                    retire_portal_if_idle(&context).await;
                    continue;
                }
                Err(error) => {
                    decrement_atomic(&context.statistics.queued_tasks);
                    result = Err(error);
                }
            }
        }
        // A terminal result leaves the retry frontier even when it failed; the failure is retained
        // in sync telemetry/statistics instead of making every future launch replay it forever.
        if let Some(frontier_id) = frontier_id {
            if let Err(error) = context
                .repository
                .complete_ingestion_frontier_task(frontier_id)
                .await
            {
                if result.is_ok() {
                    result = Err(error);
                }
            }
        }
        decrement_atomic(&context.statistics.active_tasks);
        match result {
            Ok(()) => {
                context
                    .statistics
                    .completed_tasks
                    .fetch_add(1, Ordering::AcqRel);
            }
            Err(error) => {
                context
                    .statistics
                    .failed_tasks
                    .fetch_add(1, Ordering::AcqRel);
                if let Ok(mut last_failure) = context.statistics.last_failure.lock() {
                    *last_failure = Some(error.to_string());
                }
                let _ = context
                    .repository
                    .record_source_task_failure(
                        &context.sync_run_id,
                        &context.portal.portal_key,
                        &target_id,
                        &error,
                    )
                    .await;
                if let Some(target_url) = target_url.as_deref() {
                    announce_host_stop(&context, target_url).await;
                }
                let _ = context
                    .progress_sender
                    .send(IngestionProgress::TaskFailed {
                        portal_key: context.portal.portal_key.clone(),
                        target_id,
                        error_msg: error.to_string(),
                    })
                    .await;
            }
        }
        if *context.cancellation.borrow() {
            break;
        }
        if let Err(error) = refill_ingestion_frontier(&context).await {
            context
                .statistics
                .failed_tasks
                .fetch_add(1, Ordering::AcqRel);
            let _ = context
                .progress_sender
                .send(IngestionProgress::TaskFailed {
                    portal_key: context.portal.portal_key.clone(),
                    target_id: format!("{}-frontier-refill", context.sync_run_id),
                    error_msg: error.to_string(),
                })
                .await;
        }
        retire_portal_if_idle(&context).await;
    }
    if context
        .statistics
        .live_workers
        .fetch_sub(1, Ordering::AcqRel)
        == 1
        && !context.statistics.stop_requested.load(Ordering::Acquire)
    {
        send_progress(
            &context,
            IngestionProgress::SyncStopped {
                portal_key: context.portal.portal_key.clone(),
                sync_run_id: context.sync_run_id.clone(),
            },
        )
        .await;
    }
}

pub(super) fn durable_retry_delay_seconds(attempt: u64) -> u64 {
    backoff::exponential_delay(
        attempt.saturating_sub(1),
        Duration::from_secs(1),
        Duration::from_secs(30),
    )
    .as_secs()
}

pub(super) fn durable_retry_delay_for_error(
    error: &AppError,
    attempt: u64,
    jitter_seed: u64,
) -> Option<u64> {
    (error.is_retryable() && attempt < MAXIMUM_DURABLE_TASK_ATTEMPTS).then(|| {
        let base = durable_retry_delay_seconds(attempt);
        // Bounded positive jitter keeps the exponential floor while distributing frontier rows
        // that fail together. The deterministic seed makes a persisted task's schedule explainable
        // and stable across restart; different frontier identifiers still spread over the window.
        let mixed = jitter_seed
            .wrapping_add(attempt.wrapping_mul(0x9e37_79b9_7f4a_7c15))
            .wrapping_mul(0xbf58_476d_1ce4_e5b9);
        let jitter_window = (base / 2).saturating_add(1);
        base.saturating_add(mixed % jitter_window)
    })
}

/// Signals cancellation, then waits for the workers. Workers leave their loop only on that signal
/// (the dispatcher sender they share keeps the task channel open), so a join that precedes the
/// signal waits forever, and every caller holds `provisioning_lock` while it waits.
pub(super) async fn cancel_and_join_workers(
    cancellation: &watch::Sender<bool>,
    worker_handles: Vec<tokio::task::JoinHandle<()>>,
) -> Result<(), AppError> {
    let _ = cancellation.send(true);
    join_workers(worker_handles).await
}

/// Waits for workers to observe the cancellation signal at a task boundary. Aborting their async
/// handles would detach any `spawn_blocking` parser already in flight, allowing it to keep writing
/// after the source run had been finalized or withdrawn.
pub(super) async fn join_workers(
    worker_handles: Vec<tokio::task::JoinHandle<()>>,
) -> Result<(), AppError> {
    for handle in worker_handles {
        if let Err(error) = handle.await {
            return Err(AppError::Internal(format!(
                "Ingestion worker failed while stopping: {error}"
            )));
        }
    }
    Ok(())
}

pub(super) async fn retire_portal_if_idle(context: &WorkerContext) {
    // A pause/removal holds this lock while joining the workers. A worker that
    // finishes its current task must observe cancellation instead of waiting on
    // the lock held by its own joiner. Do not abort the task/parser: all task
    // writes have already completed before this retirement boundary.
    let mut cancellation = context.cancellation.subscribe();
    if *cancellation.borrow() {
        return;
    }
    let _provisioning_guard = tokio::select! {
        biased;
        _ = cancellation.changed() => return,
        guard = context.provisioning_lock.lock() => guard,
    };
    if *cancellation.borrow() {
        return;
    }
    if context.statistics.queued_tasks.load(Ordering::Acquire) != 0
        || context.statistics.active_tasks.load(Ordering::Acquire) != 0
    {
        return;
    }
    let removed = {
        let mut pools = context.pools.write().await;
        let is_current_pool = pools
            .get(&context.portal.portal_key)
            .is_some_and(|pool| Arc::ptr_eq(&pool.statistics, &context.statistics));
        is_current_pool
            .then(|| pools.remove(&context.portal.portal_key))
            .flatten()
    };
    if removed.is_none() {
        return;
    }
    context.slot_freed.send_modify(|generation| {
        *generation = generation.wrapping_add(1);
    });
    context.statistics.running.store(false, Ordering::Release);
    let _ = context.cancellation.send(true);
    let failures = context.statistics.failed_tasks.load(Ordering::Acquire);
    let documents_processed = context
        .statistics
        .documents_processed
        .load(Ordering::Acquire);
    let data_documents_processed = context
        .statistics
        .data_documents_processed
        .load(Ordering::Acquire);
    let terminal_status = if failures == 0 {
        "COMPLETED"
    } else {
        "COMPLETED_WITH_ERRORS"
    };
    if let Err(error) = context
        .repository
        .finish_portal_sync_run(PortalSyncRunFinalization {
            run_id: &context.sync_run_id,
            portal_key: &context.portal.portal_key,
            status: terminal_status,
            pages_processed: context.statistics.pages_processed.load(Ordering::Acquire),
            requests_processed: context
                .statistics
                .requests_processed
                .load(Ordering::Acquire),
            documents_processed,
            failures,
        })
        .await
    {
        send_progress(
            context,
            IngestionProgress::TaskFailed {
                portal_key: context.portal.portal_key.clone(),
                target_id: context.sync_run_id.clone(),
                error_msg: error.to_string(),
            },
        )
        .await;
    }
    let specialized_source_without_data = matches!(
        context.portal.source_kind,
        SourceKind::OpenDataCatalog
            | SourceKind::ArcGis
            | SourceKind::Tableau
            | SourceKind::GoogleDrive
    ) && data_documents_processed == 0;
    if context.portal.source_kind != SourceKind::NextRequest
        && (documents_processed == 0 || specialized_source_without_data)
    {
        let reason = context
            .statistics
            .last_failure
            .lock()
            .ok()
            .and_then(|value| value.clone())
            .unwrap_or_else(|| {
                "The source was reachable, but the completed crawl produced no usable documents."
                    .to_string()
            });
        send_progress(
            context,
            IngestionProgress::SourceNeedsGuidance {
                portal_key: context.portal.portal_key.clone(),
                source_url: context.portal.canonical_url.clone(),
                reason,
            },
        )
        .await;
    }
    context
        .limiter
        .unregister_portal(&context.portal.portal_key)
        .await;
}

async fn receive_task(
    receiver: &Arc<Mutex<mpsc::Receiver<ScheduledTask>>>,
) -> Option<ScheduledTask> {
    receiver.lock().await.recv().await
}

async fn execute_task(context: &WorkerContext, task: IngestionTask) -> Result<(), AppError> {
    if task.portal_key() != context.portal.portal_key {
        return Err(AppError::PortalSecurity(format!(
            "Task for portal '{}' reached isolated pool '{}'",
            task.portal_key(),
            context.portal.portal_key
        )));
    }
    let _permit =
        context.global_semaphore.acquire().await.map_err(|error| {
            AppError::Internal(format!("Global queue semaphore closed: {error}"))
        })?;
    match task {
        IngestionTask::FetchMetadataPage {
            page,
            per_page,
            search,
            ..
        } => fetch_metadata_page(context, page, per_page, search).await,
        IngestionTask::FetchHostedRequest { request, .. } => {
            fetch_hosted_request(context, request).await
        }
        IngestionTask::DownloadBinaryPayload {
            request_id,
            document,
            output_directory,
            container_depth,
            container_root_directory,
            crawl_depth,
            structured_depth,
            ..
        } => {
            process_binary_payload(
                context,
                request_id,
                *document,
                PathBuf::from(output_directory),
                PayloadDepth {
                    container: container_depth,
                    crawl: crawl_depth,
                    structured: structured_depth,
                },
                container_root_directory.map(PathBuf::from),
            )
            .await
        }
        IngestionTask::FetchDataCatalogPage {
            endpoint, cursor, ..
        } => process_data_catalog_page(context, endpoint, cursor).await,
        IngestionTask::CrawlPublicResource { url, depth, .. } => {
            process_public_resource(context, url, depth).await
        }
        IngestionTask::CrawlWordPressGoogleDrive {
            connector, depth, ..
        } => process_wordpress_google_drive(context, connector, depth).await,
        IngestionTask::CrawlArcGisLayer {
            layer_url,
            offset,
            crawl_depth,
            structured_depth,
            ..
        } => process_arcgis_layer(context, layer_url, offset, crawl_depth, structured_depth).await,
        IngestionTask::ImportLocalPayload {
            request_id,
            mut document,
            source_path,
            output_directory,
            ..
        } => {
            let (managed_path, byte_count) =
                import_local_payload(&context.download_root, Path::new(&source_path), document.id)
                    .await?;
            document.local_path = Some(managed_path.to_string_lossy().into_owned());
            document.file_size = Some(byte_count);
            process_binary_payload(
                context,
                request_id,
                *document,
                PathBuf::from(output_directory),
                PayloadDepth::discovered(0, 0),
                None,
            )
            .await
        }
    }
}

pub(super) async fn send_progress(context: &WorkerContext, progress: IngestionProgress) {
    let _ = context.progress_sender.send(progress).await;
}
