//! Starting, resuming, pausing, and removing source pools, and their runtime snapshots.

use super::*;
use super::{
    crawl::{claim_crawl_path, mark_crawl_url},
    frontier::{
        enqueue_task_resilient, pool_seed_tasks, refill_ingestion_frontier, resume_initial_tasks,
    },
    guidance::{validate_crawl_policy, validate_page_parameters},
    worker::{cancel_and_join_workers, worker_loop},
};

impl QueueManager {
    pub fn new<P: AsRef<Path>>(
        client: ApiClient,
        repository: DuckDbRepository,
        corpus_index: CorpusIndex,
        limiter: RateLimiter,
        progress_sender: mpsc::Sender<IngestionProgress>,
        download_root: P,
    ) -> Result<Self, AppError> {
        let download_root = download_root.as_ref();
        if download_root.as_os_str().is_empty() {
            return Err(AppError::DataParsing(
                "Queue download root must not be empty".to_string(),
            ));
        }
        Ok(Self {
            client,
            repository,
            corpus_index,
            limiter,
            progress_sender,
            download_root: download_root.to_path_buf(),
            pools: Arc::new(RwLock::new(HashMap::new())),
            provisioning_lock: Arc::new(Mutex::new(())),
            max_active_portals: AtomicUsize::new(DEFAULT_MAXIMUM_ACTIVE_PORTALS),
            slot_freed: watch::channel(0u64).0,
            global_semaphore: Arc::new(Semaphore::new(GLOBAL_WORKER_LIMIT)),
            parser_semaphore: Arc::new(Semaphore::new(MAXIMUM_CPU_PARSERS)),
            ocr_semaphore: Arc::new(Semaphore::new(MAXIMUM_CONCURRENT_OCR)),
            parser_memory: Arc::new(Semaphore::new(PARSER_MEMORY_PERMITS as usize)),
        })
    }

    /// The current concurrent-portal cap. Defaults to `DEFAULT_MAXIMUM_ACTIVE_PORTALS`.
    pub fn max_active_portals(&self) -> usize {
        self.max_active_portals.load(Ordering::Acquire)
    }

    /// Raises or lowers the concurrent-portal cap at runtime.
    ///
    /// This is process-lifetime only: the codebase has no settings persistence mechanism
    /// (no settings table, no `Settings` struct, nothing under `src-tauri/migrations/` for
    /// user preferences) to survive it across restarts, and building one is out of scope for
    /// this change. A restart resets the cap to `DEFAULT_MAXIMUM_ACTIVE_PORTALS`.
    ///
    /// A start already parked on the old cap is woken immediately, not just on the next pool
    /// removal, so raising the limit takes effect without waiting for unrelated churn.
    ///
    /// This does not change `GLOBAL_WORKER_LIMIT`: total worker concurrency across every pool
    /// remains capped at that constant regardless of how many pools are allowed to run, so
    /// raising this setting increases how many sources may be active at once without raising
    /// how much work happens in parallel across all of them.
    pub fn set_max_active_portals(&self, value: usize) -> Result<(), AppError> {
        if value == 0 {
            return Err(AppError::DataParsing(
                "Concurrent portal limit must be at least 1".to_string(),
            ));
        }
        self.max_active_portals.store(value, Ordering::Release);
        self.slot_freed.send_modify(|generation| {
            *generation = generation.wrapping_add(1);
        });
        Ok(())
    }

    pub async fn discover_and_start(
        &self,
        portal_url: &str,
        window: MetadataPageWindow,
        rate_limit_policy: RateLimitPolicy,
    ) -> Result<PortalSyncStart, AppError> {
        validate_page_parameters(window.start_page, window.page_size)?;
        let portal = self.client.discover_portal(portal_url).await?;
        let _ = self
            .progress_sender
            .send(IngestionProgress::PortalVerified {
                portal: portal.clone(),
            })
            .await;
        // Persisted before the pool starts, and only when the operator moved off the built-in
        // window, so a resume seeds the slice they chose instead of silently reverting to 1 and
        // 50. The registration is written first because the recipe row has a foreign key to it;
        // this mirrors what the public-source path does with a non-default crawl recipe.
        if window != MetadataPageWindow::default() {
            self.repository
                .insert_or_update_portal_registration(&portal)
                .await?;
            self.repository
                .save_source_metadata_page_window(&portal, portal_url, window)
                .await?;
        }
        let sync_run_id = self
            .start_verified_portal(portal.clone(), window, rate_limit_policy)
            .await?;
        Ok(PortalSyncStart {
            portal,
            sync_run_id,
        })
    }

    pub async fn start_verified_portal(
        &self,
        portal: PortalRegistration,
        window: MetadataPageWindow,
        rate_limit_policy: RateLimitPolicy,
    ) -> Result<String, AppError> {
        validate_page_parameters(window.start_page, window.page_size)?;
        let portal_key = portal.portal_key.clone();
        self.start_task_pool(
            portal,
            rate_limit_policy,
            vec![IngestionTask::FetchMetadataPage {
                subdomain: portal_key,
                page: window.start_page,
                per_page: window.page_size,
            }],
            CrawlPolicy::default(),
            SourceGuidance::default(),
            PoolSeedPolicy::OperatorRequested,
        )
        .await
    }

    /// `window` is the page window recorded for this source, or `None` when none was recorded and
    /// the built-in window applies. A resume that ignored it would re-seed page 1 at size 50 and
    /// walk the range the operator deliberately started past.
    pub async fn resume_registered_source(
        &self,
        source: PortalRegistration,
        crawl_policy: CrawlPolicy,
        guidance: SourceGuidance,
        window: Option<MetadataPageWindow>,
    ) -> Result<String, AppError> {
        let window = window.unwrap_or_default();
        validate_page_parameters(window.start_page, window.page_size)?;
        let initial_tasks = resume_initial_tasks(&source, window);
        self.start_task_pool(
            source,
            RateLimitPolicy::default(),
            initial_tasks,
            crawl_policy,
            guidance,
            PoolSeedPolicy::ContinueInterruptedWalk,
        )
        .await
    }

    /// Hold the same lock as source starts so an idle check cannot race a new extraction.
    pub async fn maintain_idle_storage(
        &self,
        effective_at: Option<&str>,
    ) -> Result<bool, AppError> {
        let _provisioning_guard = self.provisioning_lock.lock().await;
        if self
            .runtime_snapshots()
            .await?
            .iter()
            .any(|runtime| runtime.is_running)
        {
            return Ok(false);
        }
        match effective_at {
            Some(at) => {
                self.repository
                    .compact_one_document(&self.download_root, at)
                    .await
            }
            None => {
                self.repository
                    .recover_maintenance_storage(&self.download_root)
                    .await
            }
        }
    }

    /// Background rechecks must never reactivate a source paused or removed by its owner.
    pub async fn refresh_registered_source(
        &self,
        source: PortalRegistration,
        crawl_policy: CrawlPolicy,
        guidance: SourceGuidance,
        window: Option<MetadataPageWindow>,
    ) -> Result<String, AppError> {
        let window = window.unwrap_or_default();
        validate_page_parameters(window.start_page, window.page_size)?;
        let initial_tasks = resume_initial_tasks(&source, window);
        self.start_task_pool(
            source,
            RateLimitPolicy::default(),
            initial_tasks,
            crawl_policy,
            guidance,
            PoolSeedPolicy::AutomaticRefresh,
        )
        .await
    }

    pub(super) async fn start_task_pool(
        &self,
        portal: PortalRegistration,
        rate_limit_policy: RateLimitPolicy,
        initial_tasks: Vec<IngestionTask>,
        crawl_policy: CrawlPolicy,
        guidance: SourceGuidance,
        seed_policy: PoolSeedPolicy,
    ) -> Result<String, AppError> {
        crate::licensing::recheck()?;
        validate_crawl_policy(&crawl_policy)?;
        // At capacity, this parks rather than refusing: it releases `provisioning_lock` and
        // waits for `slot_freed` (bumped whenever a running pool is removed, or the cap is
        // raised) before re-checking. It must not hold the lock while parked, or the pool
        // removals it is waiting on could never take the lock to run.
        let mut was_parked = false;
        let _provisioning_guard = loop {
            let guard = self.provisioning_lock.lock().await;
            if let Some(existing) = self.pools.read().await.get(&portal.portal_key) {
                if existing.portal != portal {
                    return Err(AppError::PortalSecurity(format!(
                        "Portal key '{}' is already bound to a different verified endpoint",
                        portal.portal_key
                    )));
                }
                return Err(AppError::PortalCapacity(format!(
                    "Portal '{}' is already synchronizing",
                    portal.portal_key
                )));
            }
            // Subscribed before the cap is even read: `set_max_active_portals` sends outside
            // `provisioning_lock`, so a raise racing this check must be observable by the
            // subscription rather than possibly landing between the check and the subscribe,
            // where it would be a silently lost wakeup with no timeout to recover from it. A
            // raise that lands before this subscribe is at worst a spurious wake below, which
            // just re-checks and re-parks if the cap is still exceeded.
            let mut slot_freed = self.slot_freed.subscribe();
            let cap = self.max_active_portals.load(Ordering::Acquire);
            if self.pools.read().await.len() < cap {
                break guard;
            }
            was_parked = true;
            drop(guard);
            // `subscribe()` above baselines against the value at this instant, so a
            // `slot_freed` send that happens after it (even before this `await` starts
            // polling) is guaranteed to be observed here rather than lost.
            let _ = slot_freed.changed().await;
        };
        if matches!(&seed_policy, PoolSeedPolicy::AutomaticDiscovery { .. }) {
            // Discovery may have followed an alias or outlived a pause/removal. The
            // submitted URL is not the source identity. Check the resolved key AND
            // publication URL while holding the same lock as lifecycle mutations,
            // before registration or a recipe can overwrite the owner's choices.
            let key = portal.portal_key.clone();
            let canonical = portal.canonical_url.trim_end_matches('/').to_string();
            let already_registered = self.repository.query_connection(
                "Check automatic discovery identity", move |conn| {
                    conn.query_row(
                        "SELECT EXISTS (SELECT 1 FROM portals WHERE subdomain = ?1 OR rtrim(base_url, '/') = ?2)",
                        duckdb::params![key, canonical], |row| row.get::<_, bool>(0),
                    ).map_err(|error| AppError::Storage(error.to_string()))
                },
            ).await?;
            if already_registered {
                return Err(AppError::DataParsing(
                    "Automatic discovery resolved to an existing source. Its lifecycle and saved collection scope were preserved; use its existing refresh schedule or an explicit operator start.".into(),
                ));
            }
        }
        if was_parked || seed_policy == PoolSeedPolicy::AutomaticRefresh {
            // The park may have outlived the source: it could have been withdrawn or paused
            // while queued for a slot. Re-check before provisioning so a promoted start never
            // resurrects a source the operator removed while it waited. A brand-new source has
            // no lifecycle row yet (`None`), which is not a reason to refuse it.
            match self
                .repository
                .source_lifecycle_state(&portal.portal_key)
                .await?
            {
                Some(SourceLifecycleState::Removed) => {
                    return Err(AppError::DataParsing(format!(
                        "Source '{}' was withdrawn while its start was queued",
                        portal.portal_key
                    )));
                }
                Some(SourceLifecycleState::Paused) => {
                    return Err(AppError::DataParsing(format!(
                        "Source '{}' was paused while its start was queued",
                        portal.portal_key
                    )));
                }
                None if seed_policy == PoolSeedPolicy::AutomaticRefresh => {
                    return Err(AppError::DataParsing(
                        "Automatic refresh requires an active registered source".into(),
                    ));
                }
                Some(SourceLifecycleState::Active) | None => {}
            }
        }
        self.repository
            .insert_or_update_portal_registration(&portal)
            .await?;
        if let PoolSeedPolicy::AutomaticDiscovery { submitted_url } = &seed_policy {
            if guidance != SourceGuidance::default() || crawl_policy != CrawlPolicy::default() {
                self.repository
                    .save_source_connector_recipe(&portal, submitted_url, &crawl_policy, &guidance)
                    .await?;
            }
        }
        self.limiter
            .register_portal(&portal.portal_key, rate_limit_policy)
            .await?;
        let sync_run_id = Uuid::new_v4().to_string();
        if let Err(error) = self
            .repository
            .begin_portal_sync_run(&sync_run_id, &portal.portal_key)
            .await
        {
            self.limiter.unregister_portal(&portal.portal_key).await;
            return Err(error);
        }
        let adopted_frontier_tasks = match self
            .repository
            .adopt_orphaned_frontier_tasks(&portal.portal_key, &sync_run_id)
            .await
        {
            Ok(count) => count,
            Err(error) => {
                self.cancel_task_pool_start(&sync_run_id, &portal.portal_key)
                    .await;
                return Err(error);
            }
        };
        let scheduler_counts = match self
            .repository
            .ingestion_scheduler_counts(&sync_run_id, &portal.portal_key)
            .await
        {
            Ok(counts) => counts,
            Err(error) => {
                self.cancel_task_pool_start(&sync_run_id, &portal.portal_key)
                    .await;
                return Err(error);
            }
        };
        let recovered_pages_started = usize::try_from(scheduler_counts.crawl_pages_started)
            .map_err(|_| {
                AppError::PortalCapacity(
                    "Recovered crawl page counter exceeds platform capacity".to_string(),
                )
            })?;
        let recovered_resources_started = usize::try_from(scheduler_counts.resources_started)
            .map_err(|_| {
                AppError::PortalCapacity(
                    "Recovered resource counter exceeds platform capacity".to_string(),
                )
            })?;
        // Whether the adopted frontier still owes this source a request-index page, which is what
        // an interrupted walk looks like on disk: pagination enqueues page N+1 in the same durable
        // batch as page N's documents, and page N's row is deleted only after that batch is
        // written, so a surviving page row means the walk stopped part-way. Asked on every start
        // rather than only on a resume, so the condition is not written twice; `pool_seed_tasks`
        // owns what a start does with the answer.
        let walk_interrupted = if adopted_frontier_tasks > 0 {
            let prefix = IngestionTask::metadata_page_target_prefix(&portal.portal_key);
            match self
                .repository
                .frontier_holds_target_prefix(&sync_run_id, &portal.portal_key, &prefix)
                .await
            {
                Ok(interrupted) => interrupted,
                Err(error) => {
                    self.cancel_task_pool_start(&sync_run_id, &portal.portal_key)
                        .await;
                    return Err(error);
                }
            }
        } else {
            false
        };
        let initial_tasks = pool_seed_tasks(initial_tasks, seed_policy, walk_interrupted);
        if initial_tasks.is_empty() && adopted_frontier_tasks == 0 {
            self.cancel_task_pool_start(&sync_run_id, &portal.portal_key)
                .await;
            return Err(AppError::DataParsing(
                "An ingestion pool requires an initial or recoverable task".to_string(),
            ));
        }
        let (sender, receiver) = mpsc::channel(PORTAL_QUEUE_CAPACITY);
        let (cancellation, cancellation_receiver) = watch::channel(false);
        let statistics = Arc::new(PortalStatistics::default());
        statistics.queued_tasks.store(
            usize::try_from(adopted_frontier_tasks).map_err(|_| {
                AppError::PortalCapacity(
                    "Recovered ingestion frontier exceeds platform capacity".to_string(),
                )
            })?,
            Ordering::Release,
        );
        statistics.running.store(true, Ordering::Release);
        statistics
            .live_workers
            .store(WORKERS_PER_PORTAL, Ordering::Release);
        let receiver = Arc::new(Mutex::new(receiver));
        let compiled_guidance = CompiledGuidancePatterns::compile(&guidance);
        let crawl = Arc::new(CrawlState {
            policy: crawl_policy,
            guidance,
            compiled_guidance,
            visited_urls: Mutex::new(HashSet::new()),
            resource_urls: Mutex::new(HashSet::new()),
            crawl_path_claims: Mutex::new(HashSet::new()),
            announced_host_stops: Mutex::new(HashSet::new()),
            frontier_exclusions: Mutex::new(BTreeMap::new()),
            pages_started: AtomicUsize::new(recovered_pages_started),
            resources_started: AtomicUsize::new(recovered_resources_started),
            frontier_refill_lock: Mutex::new(()),
            arcgis_page_hashes: Mutex::new(HashSet::new()),
        });
        let context = WorkerContext {
            portal: portal.clone(),
            client: self.client.clone(),
            repository: self.repository.clone(),
            corpus_index: self.corpus_index.clone(),
            limiter: self.limiter.clone(),
            dispatcher: sender.clone(),
            progress_sender: self.progress_sender.clone(),
            global_semaphore: Arc::clone(&self.global_semaphore),
            parser_semaphore: Arc::clone(&self.parser_semaphore),
            ocr_semaphore: Arc::clone(&self.ocr_semaphore),
            parser_memory: Arc::clone(&self.parser_memory),
            download_root: self.download_root.clone(),
            statistics: Arc::clone(&statistics),
            pools: Arc::clone(&self.pools),
            provisioning_lock: Arc::clone(&self.provisioning_lock),
            slot_freed: self.slot_freed.clone(),
            cancellation: cancellation.clone(),
            sync_run_id: sync_run_id.clone(),
            crawl: Arc::clone(&crawl),
        };
        let worker_handles = (0..WORKERS_PER_PORTAL)
            .map(|_| {
                tokio::spawn(worker_loop(
                    context.clone(),
                    Arc::clone(&receiver),
                    cancellation_receiver.clone(),
                ))
            })
            .collect();
        let pool = PortalPool {
            portal: portal.clone(),
            cancellation: cancellation.clone(),
            statistics: Arc::clone(&statistics),
            sync_run_id: sync_run_id.clone(),
            worker_handles,
            crawl,
        };
        self.pools
            .write()
            .await
            .insert(portal.portal_key.clone(), pool);
        let enqueue_result = async {
            for task in initial_tasks {
                if let IngestionTask::CrawlPublicResource { url, .. } = &task {
                    if !mark_crawl_url(&context, url).await {
                        continue;
                    }
                    claim_crawl_path(&context, url).await;
                }
                enqueue_task_resilient(&context, task).await?;
            }
            refill_ingestion_frontier(&context).await?;
            Ok::<(), AppError>(())
        }
        .await;
        if let Err(error) = enqueue_result {
            let pool = self.pools.write().await.remove(&portal.portal_key);
            statistics.running.store(false, Ordering::Release);
            // Set before the workers stop, as `stop_source_pool_locked` does: the last worker out
            // announces `SyncStopped` unless a stop was requested, and this run never announced
            // `SyncStarted`.
            statistics.stop_requested.store(true, Ordering::Release);
            if let Some(pool) = pool {
                let _ = cancel_and_join_workers(&pool.cancellation, pool.worker_handles).await;
            }
            self.slot_freed.send_modify(|generation| {
                *generation = generation.wrapping_add(1);
            });
            self.cancel_task_pool_start(&context.sync_run_id, &portal.portal_key)
                .await;
            return Err(error);
        }
        let _ = self
            .progress_sender
            .send(IngestionProgress::SyncStarted {
                portal_key: portal.portal_key,
                sync_run_id: sync_run_id.clone(),
            })
            .await;
        Ok(sync_run_id)
    }

    async fn cancel_task_pool_start(&self, sync_run_id: &str, portal_key: &str) {
        self.limiter.unregister_portal(portal_key).await;
        let _ = self
            .repository
            .finish_portal_sync_run(PortalSyncRunFinalization {
                run_id: sync_run_id,
                portal_key,
                status: "CANCELLED",
                pages_processed: 0,
                requests_processed: 0,
                documents_processed: 0,
                failures: 1,
            })
            .await;
    }

    /// Halts a source and records the halt durably, so the next launch leaves it alone.
    ///
    /// The frontier is deliberately kept. It is the whole difference between a resume that
    /// continues and one that restarts from the first page.
    pub async fn pause_source(&self, source_key: &str) -> Result<bool, AppError> {
        let _provisioning_guard = self.provisioning_lock.lock().await;
        // Checked before anything is written, because pausing is a write to the same column
        // withdrawal uses, and an unguarded pause would put a withdrawn source back on the list.
        match self.repository.source_lifecycle_state(source_key).await? {
            None => {
                return Err(AppError::DataParsing(format!(
                    "No saved source is registered as '{source_key}'"
                )))
            }
            Some(SourceLifecycleState::Removed) => {
                return Err(AppError::DataParsing(format!(
                    "Source '{source_key}' has been removed and cannot be paused"
                )))
            }
            Some(SourceLifecycleState::Active | SourceLifecycleState::Paused) => {}
        }
        let stopped = self.stop_source_pool_locked(source_key).await?;
        self.repository
            .set_source_lifecycle_state(source_key, SourceLifecycleState::Paused)
            .await?;
        Ok(stopped)
    }

    /// Withdraws a source: stops it, discards the crawl frontier and the learned recipe, and
    /// marks the registration withdrawn.
    ///
    /// Documents already ingested are retained, and the count of them is returned so the caller
    /// can say so. The `portals` row is the parent of every request and document ingested from
    /// the source, so deleting it would be a corpus purge dressed up as a list operation.
    pub async fn remove_source(&self, source_key: &str) -> Result<SourceRemoval, AppError> {
        let _provisioning_guard = self.provisioning_lock.lock().await;
        if self
            .repository
            .source_lifecycle_state(source_key)
            .await?
            .is_none()
        {
            // Reporting zero discarded and zero retained for a key that was never registered
            // would read as a successful removal of an empty source.
            return Err(AppError::DataParsing(format!(
                "No saved source is registered as '{source_key}'"
            )));
        }
        // Workers are joined inside the stop before anything is deleted. A worker still running
        // could enqueue a link it just discovered and re-populate the frontier behind the delete.
        self.stop_source_pool_locked(source_key).await?;
        let discarded_frontier_tasks = self.repository.discard_source_frontier(source_key).await?;
        self.repository
            .delete_source_connector_recipe(source_key)
            .await?;
        let retained_documents = self.repository.count_source_documents(source_key).await?;
        self.repository
            .set_source_lifecycle_state(source_key, SourceLifecycleState::Removed)
            .await?;
        Ok(SourceRemoval {
            source_key: source_key.to_string(),
            discarded_frontier_tasks,
            retained_documents,
        })
    }

    /// Counts what [`Self::purge_source`] would destroy.
    ///
    /// Read-only, so it takes no provisioning lock and stops nothing. A crawl finishing between
    /// the preview and the purge makes the purge destroy more than the preview named; the purge
    /// reports what it actually destroyed, and that report is the authoritative one.
    pub async fn preview_source_purge(
        &self,
        source_key: &str,
    ) -> Result<SourcePurgePreview, AppError> {
        if self
            .repository
            .source_lifecycle_state(source_key)
            .await?
            .is_none()
        {
            return Err(AppError::DataParsing(format!(
                "No saved source is registered as '{source_key}'"
            )));
        }
        self.repository.preview_source_purge(source_key).await
    }

    /// Destroys a source and everything ingested under it, including the `portals` row.
    ///
    /// [`Self::remove_source`] withdraws a source and keeps its documents. This is the other
    /// operation, and the two are deliberately not one function with a flag: an operator who meant
    /// "stop crawling this" and an operator who meant "erase this" have opposite intentions, and a
    /// boolean parameter is a poor place for that distinction to live.
    ///
    /// Workers are joined before anything is deleted, for the same reason the withdrawal path does
    /// it: a worker still in flight can finalize a document behind the delete, which would leave
    /// rows referencing a `portals` row that no longer exists and make the next launch fail
    /// `verify_logical_relations`.
    ///
    /// The returned evidence paths are unlinked by the caller. The repository owns rows, not files.
    pub async fn purge_source(
        &self,
        source_key: &str,
    ) -> Result<CommittedPurge<SourcePurge>, AppError> {
        let _provisioning_guard = self.provisioning_lock.lock().await;
        if self
            .repository
            .source_lifecycle_state(source_key)
            .await?
            .is_none()
        {
            return Err(AppError::DataParsing(format!(
                "No saved source is registered as '{source_key}'"
            )));
        }
        self.stop_source_pool_locked(source_key).await?;
        self.repository.purge_source(source_key).await
    }

    /// Destroys one ingested document and everything derived from it.
    ///
    /// Takes the provisioning lock and stops nothing else: a purge is rare and a running crawl for
    /// the same source could otherwise re-finalize the document between the delete and the commit.
    pub async fn purge_document(
        &self,
        source_key: &str,
        document_id: u64,
    ) -> Result<CommittedPurge<DocumentPurge>, AppError> {
        let _provisioning_guard = self.provisioning_lock.lock().await;
        self.repository
            .purge_document(source_key, document_id)
            .await
    }

    /// Tears down a running pool and finalizes its run. Returns `false` when the source was not
    /// running, which is not an error: a saved source can be paused or withdrawn while idle.
    ///
    /// The caller must already hold `provisioning_lock`. It is a plain `tokio::sync::Mutex` and
    /// re-entering it deadlocks rather than erroring.
    pub(super) async fn stop_source_pool_locked(&self, portal_key: &str) -> Result<bool, AppError> {
        let pool = self.pools.write().await.remove(portal_key);
        let Some(pool) = pool else {
            return Ok(false);
        };
        self.slot_freed.send_modify(|generation| {
            *generation = generation.wrapping_add(1);
        });
        pool.statistics
            .stop_requested
            .store(true, Ordering::Release);
        pool.statistics.running.store(false, Ordering::Release);
        let worker_result = cancel_and_join_workers(&pool.cancellation, pool.worker_handles).await;
        let finalization = self
            .repository
            .finish_portal_sync_run(PortalSyncRunFinalization {
                run_id: &pool.sync_run_id,
                portal_key,
                status: "CANCELLED",
                pages_processed: pool.statistics.pages_processed.load(Ordering::Acquire),
                requests_processed: pool.statistics.requests_processed.load(Ordering::Acquire),
                documents_processed: pool.statistics.documents_processed.load(Ordering::Acquire),
                failures: pool.statistics.failed_tasks.load(Ordering::Acquire),
            })
            .await;
        self.limiter.unregister_portal(portal_key).await;
        finalization?;
        worker_result?;
        let _ = self
            .progress_sender
            .send(IngestionProgress::SyncStopped {
                portal_key: portal_key.to_string(),
                sync_run_id: pool.sync_run_id,
            })
            .await;
        Ok(true)
    }

    pub async fn runtime_snapshots(&self) -> Result<Vec<PortalRuntimeSnapshot>, AppError> {
        let pools = {
            let pools = self.pools.read().await;
            pools
                .values()
                .map(|pool| {
                    (
                        pool.portal.clone(),
                        pool.sync_run_id.clone(),
                        Arc::clone(&pool.statistics),
                        Arc::clone(&pool.crawl),
                    )
                })
                .collect::<Vec<_>>()
        };
        let mut snapshots = Vec::with_capacity(pools.len());
        for (portal, sync_run_id, statistics, crawl) in pools {
            let frontier_exclusions = crawl
                .frontier_exclusions
                .lock()
                .await
                .iter()
                .map(|(rule, count)| FrontierExclusionCount {
                    rule: rule.clone(),
                    excluded_links: *count,
                })
                .collect();
            snapshots.push(PortalRuntimeSnapshot {
                rate_limit_tokens: self.limiter.available_tokens(&portal.portal_key).await?,
                portal,
                sync_run_id,
                queued_tasks: statistics.queued_tasks.load(Ordering::Acquire),
                active_tasks: statistics.active_tasks.load(Ordering::Acquire),
                completed_tasks: statistics.completed_tasks.load(Ordering::Acquire),
                failed_tasks: statistics.failed_tasks.load(Ordering::Acquire),
                pages_processed: statistics.pages_processed.load(Ordering::Acquire),
                requests_processed: statistics.requests_processed.load(Ordering::Acquire),
                documents_processed: statistics.documents_processed.load(Ordering::Acquire),
                is_running: statistics.running.load(Ordering::Acquire),
                frontier_exclusions,
                stopped_hosts: self
                    .client
                    .host_governor()
                    .snapshots()
                    .into_iter()
                    .filter_map(|host| {
                        host.stopped.map(|reason| StoppedHost {
                            host: host.host,
                            requests_spent: host.requests_spent,
                            reason: reason.describe(),
                        })
                    })
                    .collect(),
            });
        }
        snapshots.sort_by(|left, right| left.portal.portal_key.cmp(&right.portal.portal_key));
        Ok(snapshots)
    }

    pub async fn is_source_running(&self, source_key: &str) -> bool {
        self.pools.read().await.contains_key(source_key)
    }
}
