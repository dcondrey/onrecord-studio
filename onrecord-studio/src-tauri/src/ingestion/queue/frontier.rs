//! Enqueueing onto pool channels, durable frontier refill, and initial task seeding.

use super::*;

pub(super) async fn enqueue_task_resilient(
    context: &WorkerContext,
    task: IngestionTask,
) -> Result<(), AppError> {
    enqueue_tasks_resilient(context, vec![task]).await
}

pub(super) async fn enqueue_tasks_resilient(
    context: &WorkerContext,
    tasks: Vec<IngestionTask>,
) -> Result<(), AppError> {
    if tasks.is_empty() {
        return Ok(());
    }
    let mut encoded = Vec::with_capacity(tasks.len());
    for task in tasks {
        let target_id = task.target_id();
        let payload = serde_json::to_string(&task).map_err(|error| {
            AppError::Internal(format!(
                "Failed to encode a durable ingestion task: {error}"
            ))
        })?;
        encoded.push((target_id, payload));
    }
    // Counted before the rows become visible. Once the batch commits, any worker's refill can
    // claim a row and its dequeue decrements `queued_tasks`; a decrement that lands before this
    // increment saturates at zero and leaves the counter one too high for the rest of the run,
    // and `retire_portal_if_idle` then never sees the pool idle. Rows the UNIQUE constraint
    // ignored, or a failed batch, are given back below.
    let reserved = encoded.len();
    context
        .statistics
        .queued_tasks
        .fetch_add(reserved, Ordering::AcqRel);
    let inserted = match context
        .repository
        .persist_ingestion_frontier_tasks(
            &context.sync_run_id,
            &context.portal.portal_key,
            &encoded,
        )
        .await
    {
        Ok(inserted) => usize::try_from(inserted).unwrap_or(reserved).min(reserved),
        Err(error) => {
            context
                .statistics
                .queued_tasks
                .fetch_sub(reserved, Ordering::AcqRel);
            return Err(error);
        }
    };
    if inserted < reserved {
        context
            .statistics
            .queued_tasks
            .fetch_sub(reserved - inserted, Ordering::AcqRel);
    }
    if inserted > 0 {
        refill_ingestion_frontier(context).await?;
    }
    Ok(())
}

pub(super) async fn refill_ingestion_frontier(context: &WorkerContext) -> Result<(), AppError> {
    // Claim and channel reservation must remain serialized per portal. Concurrent refillers could
    // otherwise lease more rows than the bounded dispatcher can accept.
    let _refill_guard = context.crawl.frontier_refill_lock.lock().await;
    loop {
        let permit = match context.dispatcher.try_reserve() {
            Ok(permit) => permit,
            Err(mpsc::error::TrySendError::Full(_)) => return Ok(()),
            Err(mpsc::error::TrySendError::Closed(_)) => {
                return Err(AppError::Internal(
                    "Portal task queue closed while refilling its durable frontier".to_string(),
                ));
            }
        };
        let Some(claimed) = context
            .repository
            .claim_ingestion_frontier_task(&context.sync_run_id, &context.portal.portal_key)
            .await?
        else {
            return Ok(());
        };
        let task: IngestionTask = match serde_json::from_str(&claimed.payload_json) {
            Ok(task) => task,
            Err(error) => {
                context
                    .repository
                    .complete_ingestion_frontier_task(claimed.frontier_id)
                    .await?;
                decrement_atomic(&context.statistics.queued_tasks);
                return Err(AppError::Storage(format!(
                    "Stored ingestion frontier task {} is invalid and was quarantined: {error}",
                    claimed.frontier_id
                )));
            }
        };
        if task.portal_key() != context.portal.portal_key {
            let wrong_source = task.portal_key().to_string();
            context
                .repository
                .complete_ingestion_frontier_task(claimed.frontier_id)
                .await?;
            decrement_atomic(&context.statistics.queued_tasks);
            return Err(AppError::PortalSecurity(format!(
                "Stored ingestion frontier task {} belonged to '{wrong_source}' rather than '{}' and was quarantined",
                claimed.frontier_id, context.portal.portal_key
            )));
        }
        // Attempts count started executions, and retries scheduled in-process stop at the cap, so
        // a row still here at the cap means its last execution was interrupted, typically because
        // processing the task ended the process. Running it again would repeat that every launch.
        if claimed.attempts >= MAXIMUM_DURABLE_TASK_ATTEMPTS {
            context
                .repository
                .complete_ingestion_frontier_task(claimed.frontier_id)
                .await?;
            decrement_atomic(&context.statistics.queued_tasks);
            let error = AppError::Internal(format!(
                "Ingestion task was interrupted on {} consecutive attempts and was abandoned",
                claimed.attempts
            ));
            context
                .statistics
                .failed_tasks
                .fetch_add(1, Ordering::AcqRel);
            context
                .repository
                .record_source_task_failure(
                    &context.sync_run_id,
                    &context.portal.portal_key,
                    &task.target_id(),
                    &error,
                )
                .await?;
            continue;
        }
        permit.send(ScheduledTask {
            task,
            frontier_id: Some(claimed.frontier_id),
            frontier_attempt: claimed.attempts,
        });
    }
}

#[cfg(test)]
pub(super) fn enqueue_task(
    sender: &mpsc::Sender<ScheduledTask>,
    statistics: &PortalStatistics,
    task: IngestionTask,
) -> Result<(), AppError> {
    statistics.queued_tasks.fetch_add(1, Ordering::AcqRel);
    sender
        .try_send(ScheduledTask {
            task,
            frontier_id: None,
            frontier_attempt: 0,
        })
        .map_err(|error| {
            decrement_atomic(&statistics.queued_tasks);
            match error {
                mpsc::error::TrySendError::Full(_) => AppError::PortalCapacity(
                    "Portal task queue reached its bounded backpressure limit".to_string(),
                ),
                mpsc::error::TrySendError::Closed(_) => {
                    AppError::Internal("Portal task queue is closed".to_string())
                }
            }
        })
}

pub(super) fn decrement_atomic(value: &AtomicUsize) {
    let _ = value.try_update(Ordering::AcqRel, Ordering::Acquire, |current| {
        Some(current.saturating_sub(1))
    });
}

/// The seed a pool start actually enqueues, once the adopted frontier has been consulted.
///
/// Dropping the request-index seed is confined to a resume, which is the continuation of an
/// earlier start rather than a new instruction. A source paused at page 900 already carries page
/// 901 in its frontier, so seeding its configured start page re-fetches all 900 pages in between;
/// deduplication makes that correct but not free, and the cost lands on a portal that rate-limits.
///
/// The seed stands in the two cases that are not that one. An operator-requested start is an
/// instruction: they chose the page window on this start, and an inherited frontier does not
/// outrank it. And a walk that ran to the end leaves no page row at all, so `walk_interrupted` is
/// false there and the source re-enters at its start page, which is how records filed since the
/// last run are found. Recording a high-water page instead would have ended that: page numbers
/// here are positions in whatever order the portal returns, not cursors, and seeding past the last
/// page walked means never looking at the first page again.
///
/// Only the request-index seed is dropped. A crawl seed is left alone deliberately; a crawl's
/// visited set is rebuilt per pool and its re-walk is a separate, unclosed question.
pub(super) fn pool_seed_tasks(
    initial_tasks: Vec<IngestionTask>,
    seed_policy: PoolSeedPolicy,
    walk_interrupted: bool,
) -> Vec<IngestionTask> {
    if seed_policy == PoolSeedPolicy::OperatorRequested || !walk_interrupted {
        return initial_tasks;
    }
    initial_tasks
        .into_iter()
        .filter(|task| !matches!(task, IngestionTask::FetchMetadataPage { .. }))
        .collect()
}

/// The seed a resume adds alongside whatever the durable frontier already holds.
///
/// Split out so the window a resume applies is provable without provisioning a pool: the seed is
/// enqueued unconditionally by `start_task_pool`, so a resume that ignored the operator's saved
/// window would re-fetch page 1 at size 50 and walk the range they started past.
pub(super) fn resume_initial_tasks(
    source: &PortalRegistration,
    window: MetadataPageWindow,
) -> Vec<IngestionTask> {
    match source.source_kind {
        SourceKind::NextRequest => vec![IngestionTask::FetchMetadataPage {
            subdomain: source.portal_key.clone(),
            page: window.start_page,
            per_page: window.page_size,
            search: None,
        }],
        SourceKind::LocalFile | SourceKind::PastedContent => Vec::new(),
        _ => vec![IngestionTask::CrawlPublicResource {
            subdomain: source.portal_key.clone(),
            url: source.canonical_url.clone(),
            depth: 0,
        }],
    }
}
