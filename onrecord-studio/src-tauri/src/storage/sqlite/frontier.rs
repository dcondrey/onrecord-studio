//! The durable ingestion frontier: crawl budgets, leased tasks, failures, and recovery.

use super::connection::execute_transaction;
use super::*;

impl DuckDbRepository {
    pub async fn ingestion_scheduler_counts(
        &self,
        run_id: &str,
        portal_key: &str,
    ) -> Result<IngestionSchedulerCounts, AppError> {
        let run_id = run_id.to_string();
        let portal_key = portal_key.to_string();
        let (crawl_pages_started, resources_started): (i64, i64) = self
            .query_connection("Read ingestion scheduler counters", move |conn| {
                conn.query_row(
                    "SELECT crawl_pages_started, resources_started
                 FROM portal_sync_runs
                 WHERE run_id = ?1 AND portal_subdomain = ?2",
                    params![run_id, portal_key],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to load ingestion scheduler counters: {error}"
                    ))
                })
            })
            .await?;
        Ok(IngestionSchedulerCounts {
            crawl_pages_started: u64::try_from(crawl_pages_started).map_err(|_| {
                AppError::Storage("Stored crawl page counter was negative".to_string())
            })?,
            resources_started: u64::try_from(resources_started).map_err(|_| {
                AppError::Storage("Stored resource counter was negative".to_string())
            })?,
        })
    }

    pub async fn reserve_ingestion_crawl_page(
        &self,
        run_id: &str,
        portal_key: &str,
        maximum: usize,
    ) -> Result<Option<u64>, AppError> {
        self.reserve_ingestion_budget(run_id, portal_key, "crawl_pages_started", maximum)
            .await
    }

    pub async fn reserve_ingestion_resource(
        &self,
        run_id: &str,
        portal_key: &str,
        maximum: usize,
    ) -> Result<Option<u64>, AppError> {
        self.reserve_ingestion_budget(run_id, portal_key, "resources_started", maximum)
            .await
    }

    async fn reserve_ingestion_budget(
        &self,
        run_id: &str,
        portal_key: &str,
        counter_column: &'static str,
        maximum: usize,
    ) -> Result<Option<u64>, AppError> {
        debug_assert!(matches!(
            counter_column,
            "crawl_pages_started" | "resources_started"
        ));
        let maximum = i64::try_from(maximum).map_err(|_| {
            AppError::PortalCapacity(
                "Ingestion scheduler budget exceeds DuckDB capacity".to_string(),
            )
        })?;
        let run_id = run_id.to_string();
        let portal_key = portal_key.to_string();
        let current = self
            .query_connection("Reserve ingestion scheduler capacity", move |conn| {
                let statement = format!(
                    "UPDATE portal_sync_runs
             SET {counter_column} = {counter_column} + 1
             WHERE run_id = ?1 AND portal_subdomain = ?2 AND status = 'RUNNING'
               AND {counter_column} < ?3"
                );
                let updated = conn
                    .execute(&statement, params![run_id, portal_key, maximum])
                    .map_err(|error| {
                        AppError::Storage(format!(
                            "Failed to reserve ingestion scheduler capacity: {error}"
                        ))
                    })?;
                if updated == 0 {
                    return Ok(None);
                }
                let current: i64 = conn
                    .query_row(
                        &format!(
                            "SELECT {counter_column} FROM portal_sync_runs
                     WHERE run_id = ?1 AND portal_subdomain = ?2"
                        ),
                        params![run_id, portal_key],
                        |row| row.get(0),
                    )
                    .map_err(|error| {
                        AppError::Storage(format!(
                            "Failed to read reserved ingestion scheduler capacity: {error}"
                        ))
                    })?;
                Ok(Some(current))
            })
            .await?;
        Ok(match current {
            Some(current) => Some(u64::try_from(current).map_err(|_| {
                AppError::Storage("Stored ingestion scheduler counter was negative".to_string())
            })?),
            None => None,
        })
    }

    pub async fn record_source_task_failure(
        &self,
        run_id: &str,
        source_key: &str,
        target_id: &str,
        error: &AppError,
    ) -> Result<(), AppError> {
        let error_type = error.category();
        let error_message = error.to_string();
        let retryable = error.is_retryable();
        let repairable = error.is_source_repairable();
        let run_id = run_id.to_string();
        let source_key = source_key.to_string();
        let target_id = target_id.to_string();
        self.query_connection("Record source task failure", move |conn| {
            execute_transaction(conn, "source task failure recording", |transaction| {
                transaction
                    .execute(
                        "INSERT INTO source_ingestion_failures (
                        run_id, source_key, target_id, error_type, error_message,
                        retryable, repairable
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                     ON CONFLICT(run_id, target_id, error_type) DO UPDATE SET
                        error_message = excluded.error_message,
                        retryable = excluded.retryable,
                        repairable = excluded.repairable,
                        occurrences = source_ingestion_failures.occurrences + 1,
                        last_occurred_at = strftime(now(), '%Y-%m-%dT%H:%M:%S.%fZ')",
                        params![
                            run_id,
                            source_key,
                            target_id,
                            error_type,
                            error_message,
                            retryable,
                            repairable
                        ],
                    )
                    .map_err(|database_error| {
                        AppError::Storage(format!(
                            "Failed to record source ingestion failure: {database_error}"
                        ))
                    })?;
                // One unreadable document or dead link is that item's failure, recorded above; it
                // must not flag the whole source as needing attention.
                if repairable {
                    return Ok(());
                }
                transaction
                    .execute(
                        "UPDATE portals
                     SET health_status = 'degraded', last_error = ?1
                     WHERE subdomain = ?2",
                        params![error_message, source_key],
                    )
                    .map_err(|database_error| {
                        AppError::Storage(format!(
                            "Failed to update source health after a task failure: {database_error}"
                        ))
                    })?;
                Ok(())
            })
        })
        .await
    }

    /// Persists a failure that happens before startup recovery can create a replacement run.
    ///
    /// These failures cannot use `source_ingestion_failures`, whose `run_id` is a required foreign
    /// key. The portal health row is the durable source of truth exposed by `SavedSource`, while
    /// live telemetry provides the same detail to an already-open webview.
    pub async fn record_source_recovery_failure(
        &self,
        source_key: &str,
        stage: &str,
        error: &AppError,
    ) -> Result<(), AppError> {
        let message = format!("Could not start collecting during {stage}: {error}");
        let source_key = source_key.to_string();
        self.query_connection("Record source recovery failure", move |conn| {
            let updated = conn
                .execute(
                    "UPDATE portals
                 SET health_status = 'degraded', last_error = ?1
                 WHERE subdomain = ?2",
                    params![message, source_key],
                )
                .map_err(|database_error| {
                    AppError::Storage(format!(
                        "Failed to persist startup recovery failure for {source_key}: {database_error}"
                    ))
                })?;
            if updated != 1 {
                return Err(AppError::Storage(format!(
                    "Cannot persist startup recovery failure for unknown source '{source_key}'"
                )));
            }
            Ok(())
        })
        .await
    }

    /// One task through [`Self::persist_ingestion_frontier_tasks`]; returns whether it was new.
    #[cfg(test)]
    pub async fn persist_ingestion_frontier_task(
        &self,
        run_id: &str,
        source_key: &str,
        target_id: &str,
        payload_json: &str,
    ) -> Result<bool, AppError> {
        self.persist_ingestion_frontier_tasks(
            run_id,
            source_key,
            &[(target_id.to_string(), payload_json.to_string())],
        )
        .await
        .map(|inserted| inserted == 1)
    }

    pub async fn persist_ingestion_frontier_tasks(
        &self,
        run_id: &str,
        source_key: &str,
        tasks: &[(String, String)],
    ) -> Result<u64, AppError> {
        if tasks.is_empty() {
            return Ok(0);
        }
        if tasks.len() > 100_000 {
            return Err(AppError::PortalCapacity(
                "A single ingestion frontier batch may not exceed 100000 tasks".to_string(),
            ));
        }
        let run_id = run_id.to_string();
        let source_key = source_key.to_string();
        let tasks = tasks.to_vec();
        self.query_connection("Persist ingestion frontier batch", move |conn| {
            execute_transaction(conn, "ingestion frontier batch", |transaction| {
                let mut statement = transaction
                    .prepare_cached(
                        "INSERT OR IGNORE INTO ingestion_task_frontier (
                        run_id, source_key, target_id, payload_json
                     ) VALUES (?1, ?2, ?3, ?4)",
                    )
                    .map_err(|error| {
                        AppError::Storage(format!(
                            "Failed to prepare ingestion frontier batch: {error}"
                        ))
                    })?;
                let mut inserted = 0_u64;
                for (target_id, payload_json) in &tasks {
                    let changed = statement
                        .execute(params![run_id, source_key, target_id, payload_json])
                        .map_err(|error| {
                            AppError::Storage(format!(
                            "Failed to persist ingestion frontier batch task '{target_id}': {error}"
                        ))
                        })?;
                    inserted = inserted.saturating_add(changed as u64);
                }
                Ok(inserted)
            })
        })
        .await
    }

    pub async fn claim_ingestion_frontier_task(
        &self,
        run_id: &str,
        source_key: &str,
    ) -> Result<Option<ClaimedIngestionTask>, AppError> {
        let run_id = run_id.to_string();
        let source_key = source_key.to_string();
        self.query_connection("Claim ingestion frontier task", move |conn| {
            execute_transaction(conn, "ingestion frontier claim", |transaction| {
                transaction
                    .execute(
                        "UPDATE ingestion_task_frontier
                     SET status = 'PENDING', lease_expires_at = NULL,
                         updated_at = strftime(now(), '%Y-%m-%dT%H:%M:%S.%fZ')
                     WHERE run_id = ?1 AND source_key = ?2 AND status = 'LEASED'
                       AND lease_expires_at <= strftime(now(), '%Y-%m-%dT%H:%M:%S.%fZ')",
                        params![run_id, source_key],
                    )
                    .map_err(|error| {
                        AppError::Storage(format!(
                            "Failed to recover expired ingestion frontier leases: {error}"
                        ))
                    })?;
                let candidate = transaction
                    .query_row(
                        "SELECT frontier_id, payload_json, executions
                     FROM ingestion_task_frontier
                     WHERE run_id = ?1 AND source_key = ?2 AND status = 'PENDING'
                     ORDER BY frontier_id ASC
                     LIMIT 1",
                        params![run_id, source_key],
                        |row| {
                            Ok((
                                row.get::<_, i64>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, u64>(2)?,
                            ))
                        },
                    )
                    .optional()
                    .map_err(|error| {
                        AppError::Storage(format!(
                            "Failed to select an ingestion frontier task: {error}"
                        ))
                    })?;
                let Some((frontier_id, payload_json, attempts)) = candidate else {
                    return Ok(None);
                };
                // A claimed row waits in a dispatcher holding up to PORTAL_QUEUE_CAPACITY tasks for
                // two workers, often far longer than any fixed lease. It is held by this live run
                // until acknowledged or deferred, so it carries no expiry: a timed lease re-claimed
                // queued rows as duplicates and charged each re-claim as an attempt, abandoning
                // hundreds of tasks that never ran. A crashed run's rows are recovered by adoption,
                // and attempts are charged when a worker starts the task.
                let claimed = transaction
                    .execute(
                        "UPDATE ingestion_task_frontier
                     SET status = 'LEASED', lease_expires_at = NULL,
                         updated_at = strftime(now(), '%Y-%m-%dT%H:%M:%S.%fZ')
                     WHERE frontier_id = ?1 AND status = 'PENDING'",
                        params![frontier_id],
                    )
                    .map_err(|error| {
                        AppError::Storage(format!(
                            "Failed to lease an ingestion frontier task: {error}"
                        ))
                    })?;
                if claimed != 1 {
                    return Err(AppError::Storage(format!(
                        "Ingestion frontier task {frontier_id} changed while being claimed"
                    )));
                }
                Ok(Some(ClaimedIngestionTask {
                    frontier_id,
                    payload_json,
                    attempts,
                }))
            })
        })
        .await
    }

    /// Charges one attempt as a worker begins executing a claimed task, returning the attempt
    /// number this execution is.
    pub async fn start_ingestion_frontier_task(&self, frontier_id: i64) -> Result<u64, AppError> {
        self.query_connection("Start ingestion frontier task", move |conn| {
            conn.query_row(
                "UPDATE ingestion_task_frontier
                 SET executions = executions + 1,
                     updated_at = strftime(now(), '%Y-%m-%dT%H:%M:%S.%fZ')
                 WHERE frontier_id = ?1 AND status = 'LEASED'
                 RETURNING executions",
                params![frontier_id],
                |row| row.get::<_, u64>(0),
            )
            .optional()
            .map_err(|error| {
                AppError::Storage(format!(
                    "Failed to start ingestion frontier task {frontier_id}: {error}"
                ))
            })?
            .ok_or_else(|| {
                AppError::Storage(format!(
                    "Ingestion frontier task {frontier_id} was missing when its worker started"
                ))
            })
        })
        .await
    }

    pub async fn defer_ingestion_frontier_task(
        &self,
        frontier_id: i64,
        delay_seconds: u64,
    ) -> Result<(), AppError> {
        let delay_modifier = format!("+{} seconds", delay_seconds.clamp(1, 300));
        self.query_connection("Defer ingestion frontier task", move |conn| {
            let changed = conn
                .execute(
                    "UPDATE ingestion_task_frontier
                 SET status = 'LEASED',
                     lease_expires_at = strftime(now()::TIMESTAMP + CAST(?2 AS INTERVAL), '%Y-%m-%dT%H:%M:%S.%fZ'),
                     updated_at = strftime(now(), '%Y-%m-%dT%H:%M:%S.%fZ')
                 WHERE frontier_id = ?1 AND status = 'LEASED'",
                    params![frontier_id, delay_modifier],
                )
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to defer ingestion frontier task {frontier_id}: {error}"
                    ))
                })?;
            if changed != 1 {
                return Err(AppError::Storage(format!(
                    "Ingestion frontier task {frontier_id} was missing during retry scheduling"
                )));
            }
            Ok(())
        })
        .await
    }

    pub async fn complete_ingestion_frontier_task(&self, frontier_id: i64) -> Result<(), AppError> {
        self.query_connection("Complete ingestion frontier task", move |conn| {
            let deleted = conn
                .execute(
                    "DELETE FROM ingestion_task_frontier WHERE frontier_id = ?1",
                    params![frontier_id],
                )
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to acknowledge ingestion frontier task {frontier_id}: {error}"
                    ))
                })?;
            if deleted != 1 {
                return Err(AppError::Storage(format!(
                    "Ingestion frontier task {frontier_id} was missing during acknowledgement"
                )));
            }
            Ok(())
        })
        .await
    }

    /// Active sources whose queued work outlived the run that queued it. Read only; the operator
    /// chooses which of them to resume.
    pub async fn sources_with_unfinished_work(&self) -> Result<Vec<String>, AppError> {
        self.query_connection("List sources with unfinished work", |conn| {
            let mut statement = conn
                .prepare(
                    "SELECT DISTINCT frontier.source_key
                     FROM ingestion_task_frontier frontier
                     JOIN portals ON portals.subdomain = frontier.source_key
                     WHERE COALESCE(portals.lifecycle_state, 'ACTIVE') = 'ACTIVE'
                     ORDER BY 1",
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to prepare unfinished-work scan: {error}"))
                })?;
            let keys = statement
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(|error| {
                    AppError::Storage(format!("Failed to scan unfinished work: {error}"))
                })?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| {
                    AppError::Storage(format!("Failed to read unfinished work: {error}"))
                })?;
            Ok(keys)
        })
        .await
    }

    pub async fn recover_interrupted_source_keys(&self) -> Result<Vec<String>, AppError> {
        self.query_connection("Recover interrupted source keys", |conn| {
            execute_transaction(conn, "interrupted ingestion recovery", |transaction| {
                let mut statement = transaction
                    .prepare(
                        "SELECT candidate_key FROM (
                         SELECT source_key AS candidate_key FROM ingestion_task_frontier
                         UNION
                         SELECT portal_subdomain AS candidate_key FROM portal_sync_runs
                         WHERE status = 'RUNNING'
                     ) candidates
                     JOIN portals ON portals.subdomain = candidates.candidate_key
                     WHERE COALESCE(portals.lifecycle_state, 'ACTIVE') = 'ACTIVE'
                     ORDER BY 1",
                    )
                    .map_err(|error| {
                        AppError::Storage(format!(
                            "Failed to prepare interrupted source recovery: {error}"
                        ))
                    })?;
                let keys = statement
                    .query_map([], |row| row.get::<_, String>(0))
                    .map_err(|error| {
                        AppError::Storage(format!(
                            "Failed to query interrupted source recovery: {error}"
                        ))
                    })?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|error| {
                        AppError::Storage(format!(
                            "Failed to read interrupted source recovery: {error}"
                        ))
                    })?;
                drop(statement);
                transaction
                    .execute(
                        "UPDATE portal_sync_runs
                     SET status = 'INTERRUPTED',
                         completed_at = strftime(now(), '%Y-%m-%dT%H:%M:%S.%fZ')
                     WHERE status = 'RUNNING'",
                        [],
                    )
                    .map_err(|error| {
                        AppError::Storage(format!(
                            "Failed to mark interrupted ingestion runs: {error}"
                        ))
                    })?;
                Ok(keys)
            })
        })
        .await
    }

    /// Hands every frontier row no live run owns to `new_run_id`.
    ///
    /// The predicate is "no run that is still `RUNNING` holds this work", not any particular
    /// terminal status. A crash leaves `INTERRUPTED`, an operator stop leaves `CANCELLED`, and the
    /// queued work each abandons is equally resumable: why a run ended says nothing about whether
    /// its frontier survives. Matching `INTERRUPTED` alone stranded every stopped source's rows,
    /// which were then never adopted and never deleted, so the source stayed in the startup
    /// recovery list forever while each relaunch re-crawled from the first page.
    pub async fn adopt_orphaned_frontier_tasks(
        &self,
        source_key: &str,
        new_run_id: &str,
    ) -> Result<u64, AppError> {
        let source_key = source_key.to_string();
        let new_run_id = new_run_id.to_string();
        self.query_connection("Adopt orphaned frontier tasks", move |conn| {
            execute_transaction(conn, "orphaned frontier adoption", |transaction| {
                let inherited_counts: (i64, i64) = transaction
                    .query_row(
                        "SELECT
                        COALESCE(MAX(previous.crawl_pages_started), 0),
                        COALESCE(MAX(previous.resources_started), 0)
                     FROM portal_sync_runs previous
                     WHERE previous.portal_subdomain = ?1
                       AND previous.status <> 'RUNNING'
                       AND EXISTS (
                           SELECT 1 FROM ingestion_task_frontier frontier
                           WHERE frontier.run_id = previous.run_id
                             AND frontier.source_key = ?1
                       )",
                        params![source_key],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .map_err(|error| {
                        AppError::Storage(format!(
                            "Failed to recover ingestion scheduler counters: {error}"
                        ))
                    })?;
                let counter_update = transaction
                    .execute(
                        "UPDATE portal_sync_runs
                     SET crawl_pages_started = ?1, resources_started = ?2
                     WHERE run_id = ?3 AND portal_subdomain = ?4 AND status = 'RUNNING'",
                        params![
                            inherited_counts.0,
                            inherited_counts.1,
                            new_run_id,
                            source_key
                        ],
                    )
                    .map_err(|error| {
                        AppError::Storage(format!(
                            "Failed to restore ingestion scheduler counters: {error}"
                        ))
                    })?;
                if counter_update != 1 {
                    return Err(AppError::Storage(format!(
                        "New ingestion run '{new_run_id}' was unavailable during frontier adoption"
                    )));
                }
                let adopted = transaction
                    .execute(
                        "UPDATE ingestion_task_frontier
                     SET run_id = ?1, status = 'PENDING', lease_expires_at = NULL,
                         -- An operator stop interrupted a lease on purpose; only interruptions
                         -- nobody chose count toward abandoning the task.
                         executions = CASE
                             WHEN status = 'LEASED' AND run_id IN (
                                 SELECT run_id FROM portal_sync_runs
                                 WHERE portal_subdomain = ?2 AND status = 'CANCELLED'
                             ) THEN greatest(executions - 1, 0)
                             ELSE executions
                         END,
                         updated_at = strftime(now(), '%Y-%m-%dT%H:%M:%S.%fZ')
                     WHERE source_key = ?2
                       AND run_id IN (
                           SELECT run_id FROM portal_sync_runs
                           WHERE portal_subdomain = ?2 AND status <> 'RUNNING'
                       )
                       AND NOT EXISTS (
                           SELECT 1 FROM ingestion_task_frontier current_frontier
                           WHERE current_frontier.run_id = ?1
                             AND current_frontier.source_key = ingestion_task_frontier.source_key
                             AND current_frontier.target_id = ingestion_task_frontier.target_id
                       )",
                        params![new_run_id, source_key],
                    )
                    .map_err(|error| {
                        AppError::Storage(format!(
                            "Failed to adopt orphaned ingestion tasks: {error}"
                        ))
                    })?;
                transaction
                    .execute(
                        "DELETE FROM ingestion_task_frontier
                     WHERE source_key = ?1
                       AND run_id IN (
                           SELECT run_id FROM portal_sync_runs
                           WHERE portal_subdomain = ?1 AND status <> 'RUNNING'
                       )",
                        params![source_key],
                    )
                    .map_err(|error| {
                        AppError::Storage(format!(
                            "Failed to remove duplicate orphaned ingestion tasks: {error}"
                        ))
                    })?;
                u64::try_from(adopted).map_err(|_| {
                    AppError::Storage(
                        "Adopted ingestion frontier count exceeds supported capacity".to_string(),
                    )
                })
            })
        })
        .await
    }

    /// Whether `run_id` still holds a frontier row for this source whose target identifier begins
    /// with `target_prefix`.
    ///
    /// Written as a prefix test rather than a task-kind column because the frontier stores a task
    /// as its encoded payload and its target identifier, and nothing else; the identifier is the
    /// only thing about a row that is queryable. `starts_with` and not `LIKE`: a source key is
    /// operator-derived and `%` or `_` inside one would be a wildcard under `LIKE` and match rows
    /// belonging to a different prefix.
    ///
    /// Every status counts. A page task that is leased, deferred for retry, or pending is equally
    /// work this run still owes, and the caller is asking whether the walk was interrupted, not
    /// what state its last task happened to be parked in.
    pub async fn frontier_holds_target_prefix(
        &self,
        run_id: &str,
        source_key: &str,
        target_prefix: &str,
    ) -> Result<bool, AppError> {
        let run_id = run_id.to_string();
        let source_key = source_key.to_string();
        let target_prefix = target_prefix.to_string();
        self.query_connection("Read ingestion frontier prefix", move |conn| {
            conn.query_row(
                "SELECT EXISTS (
                 SELECT 1 FROM ingestion_task_frontier
                 WHERE run_id = ?1 AND source_key = ?2
                   AND starts_with(target_id, ?3)
             )",
                params![run_id, source_key, target_prefix],
                |row| row.get::<_, bool>(0),
            )
            .map_err(|error| {
                AppError::Storage(format!(
                    "Failed to read the ingestion frontier for source '{source_key}': {error}"
                ))
            })
        })
        .await
    }
}
