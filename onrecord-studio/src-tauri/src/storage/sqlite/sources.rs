//! Saved sources: portal registrations, sync runs, connector recipes, and lifecycle state.

use super::connection::execute_transaction;
use super::*;

impl DuckDbRepository {
    pub async fn insert_or_update_portal_registration(
        &self,
        portal: &PortalRegistration,
    ) -> Result<(), AppError> {
        validate_portal_registration(portal)?;
        let portal = portal.clone();
        self.query_connection("Provision portal partition", move |conn| {
            conn.execute(
                "INSERT INTO portals (
                subdomain, display_name, base_url, origin_url, requests_endpoint,
                requests_pointer, metadata_pointer, page_parameter, page_size_parameter,
                verified_at, source_kind, health_status, last_error, lifecycle_state
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'verified', NULL, 'ACTIVE')
             ON CONFLICT(subdomain) DO UPDATE SET
                display_name = excluded.display_name,
                base_url = excluded.base_url,
                origin_url = excluded.origin_url,
                requests_endpoint = excluded.requests_endpoint,
                requests_pointer = excluded.requests_pointer,
                metadata_pointer = excluded.metadata_pointer,
                page_parameter = excluded.page_parameter,
                page_size_parameter = excluded.page_size_parameter,
                verified_at = excluded.verified_at,
                source_kind = excluded.source_kind,
                health_status = 'verified',
                last_error = NULL,
                -- Provisioning a pool for a source is the act of synchronizing it, so it is also
                -- what un-pauses one and what brings a withdrawn source back. Without this a
                -- re-added source stays filtered out of every listing and re-adding it looks
                -- like it did nothing at all.
                lifecycle_state = 'ACTIVE'",
                params![
                    portal.portal_key,
                    portal.display_name,
                    portal.canonical_url,
                    portal.origin_url,
                    portal.api_layout.requests_endpoint,
                    portal.api_layout.requests_pointer,
                    portal.api_layout.metadata_pointer,
                    portal.api_layout.page_parameter,
                    portal.api_layout.page_size_parameter,
                    portal.verified_at,
                    portal.source_kind.as_str(),
                ],
            )
            .map_err(|error| {
                AppError::Storage(format!("Failed to provision portal partition: {error}"))
            })?;
            Ok(())
        })
        .await
    }

    pub async fn begin_portal_sync_run(
        &self,
        run_id: &str,
        portal_key: &str,
    ) -> Result<(), AppError> {
        if run_id.trim().is_empty() || portal_key.trim().is_empty() {
            return Err(AppError::DataParsing(
                "Portal sync run identifiers must not be empty".to_string(),
            ));
        }
        let run_id = run_id.to_string();
        let portal_key = portal_key.to_string();
        self.query_connection("Begin portal sync run", move |conn| {
            conn.execute(
                "INSERT INTO portal_sync_runs (
                run_id, portal_subdomain, started_at, status
             ) VALUES (?1, ?2, strftime(now(), '%Y-%m-%dT%H:%M:%S.%fZ'), 'RUNNING')",
                params![run_id, portal_key],
            )
            .map_err(|error| {
                AppError::Storage(format!("Failed to begin portal sync run: {error}"))
            })?;
            Ok(())
        })
        .await
    }

    /// Records the crawl policy and repair guidance an operator supplied for one source.
    ///
    /// Takes the registration rather than a key and a URL for the reason
    /// [`Self::save_source_metadata_page_window`] does: the operator's pasted string is not the
    /// URL anything reads back. `canonical_url` is the post-redirect `Url::to_string`, so a
    /// recipe written under a pasted bare host was invisible to every resume, and a recipe
    /// written under a pasted string with no scheme also lost its `publisher_pattern`, because
    /// that is parsed out of the same value. Both are unreachable from this signature.
    ///
    /// `submitted_url` is the string the operator actually gave, recorded because it is the only
    /// key `start_data_source` has: that reader runs before discovery, so on a source whose URL
    /// redirects it can never produce `canonical_url`. Stored normalized; see
    /// [`normalized_submitted_url`].
    pub async fn save_source_connector_recipe(
        &self,
        portal: &PortalRegistration,
        submitted_url: &str,
        crawl_policy: &CrawlPolicy,
        guidance: &SourceGuidance,
    ) -> Result<(), AppError> {
        let source_key = portal.portal_key.clone();
        let root_url = portal.canonical_url.clone();
        let submitted_url = normalized_submitted_url(submitted_url);
        let crawl_policy_json = serde_json::to_string(crawl_policy).map_err(|error| {
            AppError::DataParsing(format!("Failed to encode source crawl policy: {error}"))
        })?;
        let guidance_json = serde_json::to_string(guidance).map_err(|error| {
            AppError::DataParsing(format!("Failed to encode source repair guidance: {error}"))
        })?;
        let publisher_pattern = publisher_pattern_for_url(&root_url);
        let reuse_for_publisher_pattern = guidance.reuse_for_publisher_pattern;
        self.query_connection("Save source connector recipe", move |conn| {
            execute_transaction(conn, "source connector recipe", |transaction| {
                // `root_url` is UNIQUE and the upsert below targets `source_key`, so a row holding
                // this URL under a *different* key fails the insert and takes the source start down
                // with it. Two keys can reach one URL: the public-source key is a digest over the
                // source kind and the final URL, so a reclassified source at an unchanged URL
                // produces the pair, and `portals.base_url` is not UNIQUE, so a URL that later
                // discovers as NextRequest registers a second key beside the first.
                //
                // The row being written is the one the operator is starting now, so the other is the
                // one to lose. That does cost a still-registered source at the same URL its stored
                // guidance, which is the lesser of the two: the alternative is that the start the
                // operator asked for fails outright. It is lost only if the row replacing it is
                // actually written, which is why the two statements share a transaction rather than
                // running in order.
                //
                // `submitted_url` is deliberately not cleared the same way. It carries no UNIQUE, so
                // no insert can collide on it, and dropping another source's row for sharing a pasted
                // string would destroy that source's guidance to satisfy nothing.
                transaction
                    .execute(
                        "DELETE FROM source_connector_recipes
                     WHERE root_url = ?1 AND source_key <> ?2",
                        params![root_url, source_key],
                    )
                    .map_err(|error| {
                        AppError::Storage(format!(
                            "Failed to clear a superseded source connector recipe: {error}"
                        ))
                    })?;
                transaction
                    .execute(
                        "INSERT INTO source_connector_recipes (
                source_key, root_url, submitted_url, recipe_version, publisher_pattern,
                reuse_approved, crawl_policy_json, guidance_json, updated_at
             ) VALUES (?1, ?2, ?3, 2, ?4, ?5, ?6, ?7,
                       strftime(now(), '%Y-%m-%dT%H:%M:%S.%fZ'))
             ON CONFLICT(source_key) DO UPDATE SET
                root_url = excluded.root_url,
                submitted_url = excluded.submitted_url,
                recipe_version = excluded.recipe_version,
                publisher_pattern = excluded.publisher_pattern,
                reuse_approved = excluded.reuse_approved,
                crawl_policy_json = excluded.crawl_policy_json,
                guidance_json = excluded.guidance_json,
                updated_at = excluded.updated_at",
                        params![
                            source_key,
                            root_url,
                            submitted_url,
                            publisher_pattern,
                            reuse_for_publisher_pattern,
                            crawl_policy_json,
                            guidance_json
                        ],
                    )
                    .map_err(|error| {
                        AppError::Storage(format!(
                            "Failed to save source connector recipe: {error}"
                        ))
                    })?;
                Ok(())
            })
        })
        .await
    }

    /// Records the metadata page window an operator chose for a portal's API path.
    ///
    /// Deliberately not folded into [`Self::save_source_connector_recipe`]: that writer owns the
    /// crawl policy and the repair guidance, and a NextRequest source has neither, so reusing it
    /// would overwrite a stored recipe with defaults. On conflict this touches the window and the
    /// two URLs that address the row, and leaves the rest of it as it was.
    /// Takes the registration rather than a key and a URL so the row is keyed on
    /// `canonical_url` by construction. The operator's pasted string is not that URL:
    /// `Url::to_string` adds the path a bare host omits, so a window written under
    /// `https://city.nextrequest.com` could never be found by the resume paths, which look the
    /// recipe up by the registration's `https://city.nextrequest.com/`. Passing the two apart
    /// made that mistake expressible; this signature does not.
    ///
    /// `submitted_url` is recorded for the reason [`Self::save_source_connector_recipe`] gives.
    /// Both writers record it or neither does: a NextRequest source that only ever set a page
    /// window would otherwise be the one shape the URL reader still could not find after a
    /// redirect.
    pub async fn save_source_metadata_page_window(
        &self,
        portal: &PortalRegistration,
        submitted_url: &str,
        window: MetadataPageWindow,
    ) -> Result<(), AppError> {
        let source_key = portal.portal_key.clone();
        let root_url = portal.canonical_url.clone();
        let submitted_url = normalized_submitted_url(submitted_url);
        let window_json = serde_json::to_string(&window).map_err(|error| {
            AppError::DataParsing(format!("Failed to encode metadata page window: {error}"))
        })?;
        let crawl_policy_json =
            serde_json::to_string(&CrawlPolicy::default()).map_err(|error| {
                AppError::DataParsing(format!("Failed to encode source crawl policy: {error}"))
            })?;
        let guidance_json = serde_json::to_string(&SourceGuidance::default()).map_err(|error| {
            AppError::DataParsing(format!("Failed to encode source repair guidance: {error}"))
        })?;
        let publisher_pattern = publisher_pattern_for_url(&root_url);
        self.query_connection("Save source metadata page window", move |conn| {
            execute_transaction(conn, "source metadata page window", |transaction| {
                // Same `root_url` collision the recipe writer clears, for the same reason, under the
                // same transaction.
                transaction
                    .execute(
                        "DELETE FROM source_connector_recipes
                     WHERE root_url = ?1 AND source_key <> ?2",
                        params![root_url, source_key],
                    )
                    .map_err(|error| {
                        AppError::Storage(format!(
                            "Failed to clear a superseded source connector recipe: {error}"
                        ))
                    })?;
                transaction
                    .execute(
                        "INSERT INTO source_connector_recipes (
                source_key, root_url, submitted_url, recipe_version, publisher_pattern,
                reuse_approved, crawl_policy_json, guidance_json, metadata_page_window_json,
                updated_at
             ) VALUES (?1, ?2, ?3, 2, ?4, 0, ?5, ?6, ?7,
                       strftime(now(), '%Y-%m-%dT%H:%M:%S.%fZ'))
             ON CONFLICT(source_key) DO UPDATE SET
                root_url = excluded.root_url,
                submitted_url = excluded.submitted_url,
                metadata_page_window_json = excluded.metadata_page_window_json,
                updated_at = excluded.updated_at",
                        params![
                            source_key,
                            root_url,
                            submitted_url,
                            publisher_pattern,
                            crawl_policy_json,
                            guidance_json,
                            window_json
                        ],
                    )
                    .map_err(|error| {
                        AppError::Storage(format!("Failed to save metadata page window: {error}"))
                    })?;
                Ok(())
            })
        })
        .await
    }

    /// Resolves a source's recipe by its primary key.
    ///
    /// This is the lookup every resume path takes, and it is keyed rather than URL-keyed because
    /// `source_key` is the one identifier that has always been written and read the same way.
    /// The URL side has not been: recipes saved before this were keyed on the operator's pasted
    /// string while the resume paths queried `canonical_url`, so guidance applied on re-add and
    /// stopped applying on every resume and every launch. Matching on the key finds those rows
    /// too, which is why no rewrite of what is already stored is needed.
    ///
    /// The publisher-reuse arm still matches on the host, taken from `canonical_url` for the
    /// reason given on the writer.
    pub async fn load_source_connector_recipe_for_source(
        &self,
        portal: &PortalRegistration,
    ) -> Result<Option<SourceConnectorRecipe>, AppError> {
        self.load_connector_recipe(
            RecipeOwnRow::SourceKey,
            [portal.portal_key.clone(), portal.portal_key.clone()],
            publisher_pattern_for_url(&portal.canonical_url),
        )
        .await
    }

    /// Resolves a recipe from a URL alone, for the one caller that has no source key yet:
    /// `start_data_source` reads the recipe *before* discovery, because the guidance it holds is
    /// an input to discovery.
    ///
    /// Both the argument as typed and its normalized form are matched, and that is not belt and
    /// braces. Normalization is what lets a pasted bare host find a row stored under the canonical
    /// `Url::to_string`, but it is not idempotent against what is already stored: rows written
    /// before the writer was canonicalized hold the raw pasted string, and normalizing the query
    /// alone would have stopped finding them. That is the one path a guided-repair recipe still
    /// worked on, so matching only the normalized form would have closed two misses and opened a
    /// third.
    ///
    /// A source whose URL redirects is found through `submitted_url`, which both writers record
    /// as the operator gave it. The final URL is still not knowable before the fetch, so the
    /// recipe is not reachable through `root_url` at all on that source; the stored submission is
    /// what closes it. That column is absent on rows written before it existed, and those rows
    /// stay reachable through `root_url` exactly as they were.
    pub async fn load_source_connector_recipe(
        &self,
        root_url: &str,
    ) -> Result<Option<SourceConnectorRecipe>, AppError> {
        let submitted = root_url.trim().to_string();
        let normalized = normalized_submitted_url(&submitted);
        let publisher_pattern = publisher_pattern_for_url(&normalized);
        self.load_connector_recipe(
            RecipeOwnRow::RootUrl,
            [normalized, submitted],
            publisher_pattern,
        )
        .await
    }

    /// Reads the source's own row and any publisher recipe approved for reuse, and resolves the
    /// two.
    ///
    /// They are read separately because they are separately owned. A NextRequest source that only
    /// ever set a page window has a row carrying default policy and guidance, and letting that row
    /// win outright would silently withdraw an approved publisher recipe the source was running
    /// under until the window was set. Conversely the page window is never taken from a reused
    /// row: a window says where in *this* portal's request index to start, which is not a fact
    /// about a sibling source that happens to share a publisher.
    async fn load_connector_recipe(
        &self,
        own_row: RecipeOwnRow,
        own_values: [String; 2],
        publisher_pattern: Option<String>,
    ) -> Result<Option<SourceConnectorRecipe>, AppError> {
        // Not interpolated from anything a caller supplies: `RecipeOwnRow` names columns in this
        // file's own schema.
        let own_column = own_row.column();
        // Both query forms are matched against both columns, and that is not belt and braces.
        // Normalization is what lets a pasted bare host find a row stored under the canonical
        // `Url::to_string`, but it is not idempotent against what is already stored: rows written
        // before the writer was canonicalized hold the raw pasted string, and matching only the
        // normalized form would stop finding them.
        let own_match = format!("{own_column} IN (?1, ?3)");
        let alternate_match = own_row
            .alternate_column()
            .map(|column| format!("{column} IN (?1, ?3)"));
        // The rank is selected rather than re-derived from a returned column value, because a row
        // matched on `submitted_url` carries a `root_url` that equals neither query form. Reading
        // identity back off the column would find such a row's recipe and silently miss its page
        // window, which is the same two-arms-disagree failure this lookup was last repaired for.
        let (own_rank, own_where) = match &alternate_match {
            Some(alternate) => (
                format!("CASE WHEN {own_match} THEN 0 WHEN {alternate} THEN 1 ELSE 2 END"),
                format!("{own_match} OR {alternate}"),
            ),
            None => (
                format!("CASE WHEN {own_match} THEN 0 ELSE 2 END"),
                own_match.clone(),
            ),
        };
        // No LIMIT. `submitted_url` is not UNIQUE and the publisher arm matches a whole host, so
        // any bound small enough to be worth stating could truncate the reuse row that carries the
        // guidance before the resolution below ever sees it. The table holds one row per
        // registered source.
        let statement_sql = format!(
            "SELECT CAST({own_rank} AS BIGINT) AS own_rank,
                    crawl_policy_json, guidance_json, metadata_page_window_json
             FROM source_connector_recipes
             WHERE {own_where}
                OR (reuse_approved = 1 AND publisher_pattern = ?2)
             ORDER BY own_rank, updated_at DESC"
        );
        let [query_value, query_alternate] = own_values;
        let encoded = self
            .query_connection("Source connector recipe query", move |connection| {
                let mut statement = connection.prepare(&statement_sql).map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to prepare source connector recipe: {error}"
                    ))
                })?;
                let rows = statement
                    .query_map(
                        params![query_value, publisher_pattern, query_alternate],
                        |row| {
                            Ok((
                                row.get::<_, i64>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, String>(2)?,
                                row.get::<_, Option<String>>(3)?,
                            ))
                        },
                    )
                    .map_err(|error| {
                        AppError::Storage(format!(
                            "Failed to query source connector recipe: {error}"
                        ))
                    })?;
                rows.collect::<duckdb::Result<Vec<_>>>().map_err(|error| {
                    AppError::Storage(format!("Failed to read source connector recipe: {error}"))
                })
            })
            .await?;
        if encoded.is_empty() {
            return Ok(None);
        }
        let mut decoded = Vec::with_capacity(encoded.len());
        for (own_rank, crawl_policy_json, guidance_json, metadata_page_window_json) in encoded {
            let crawl_policy: CrawlPolicy =
                serde_json::from_str(&crawl_policy_json).map_err(|error| {
                    AppError::Storage(format!("Stored source crawl policy is invalid: {error}"))
                })?;
            let guidance: SourceGuidance =
                serde_json::from_str(&guidance_json).map_err(|error| {
                    AppError::Storage(format!("Stored source repair guidance is invalid: {error}"))
                })?;
            let metadata_page_window = metadata_page_window_json
                .map(|encoded| {
                    serde_json::from_str::<MetadataPageWindow>(&encoded).map_err(|error| {
                        AppError::Storage(format!(
                            "Stored metadata page window is invalid: {error}"
                        ))
                    })
                })
                .transpose()?;
            decoded.push((own_rank, crawl_policy, guidance, metadata_page_window));
        }
        // A window says where in *this* source's request index to start, so it is read only off a
        // row the query recognized as the caller's own -- by either URL column, in either form.
        // A reused publisher row ranks last and never supplies one.
        let metadata_page_window = decoded
            .iter()
            .find(|(own_rank, ..)| *own_rank < PUBLISHER_REUSE_RANK)
            .and_then(|(.., window)| *window);
        let (crawl_policy, guidance) = decoded
            .iter()
            .find(|(_, crawl_policy, guidance, _)| {
                *guidance != SourceGuidance::default() || *crawl_policy != CrawlPolicy::default()
            })
            .map(|(_, crawl_policy, guidance, _)| (crawl_policy.clone(), guidance.clone()))
            .unwrap_or_else(|| {
                let (_, crawl_policy, guidance, _) = &decoded[0];
                (crawl_policy.clone(), guidance.clone())
            });
        Ok(Some(SourceConnectorRecipe {
            crawl_policy,
            guidance,
            metadata_page_window,
        }))
    }

    pub async fn finish_portal_sync_run(
        &self,
        finalization: PortalSyncRunFinalization<'_>,
    ) -> Result<(), AppError> {
        let PortalSyncRunFinalization {
            run_id,
            portal_key,
            status,
            pages_processed,
            requests_processed,
            documents_processed,
            failures,
        } = finalization;
        if !matches!(status, "COMPLETED" | "COMPLETED_WITH_ERRORS" | "CANCELLED") {
            return Err(AppError::DataParsing(format!(
                "Unsupported portal sync terminal status '{status}'"
            )));
        }
        let pages_processed = i64::try_from(pages_processed).map_err(|_| {
            AppError::Storage("Portal page count exceeds SQLite capacity".to_string())
        })?;
        let requests_processed = i64::try_from(requests_processed).map_err(|_| {
            AppError::Storage("Portal request count exceeds SQLite capacity".to_string())
        })?;
        let documents_processed = i64::try_from(documents_processed).map_err(|_| {
            AppError::Storage("Portal document count exceeds SQLite capacity".to_string())
        })?;
        let failures = i64::try_from(failures).map_err(|_| {
            AppError::Storage("Portal failure count exceeds SQLite capacity".to_string())
        })?;
        let run_id = run_id.to_string();
        let portal_key = portal_key.to_string();
        let status = status.to_string();
        self.query_connection("Finish portal sync run", move |conn| {
            execute_transaction(conn, "portal sync finalization", |transaction| {
                let updated = transaction
                    .execute(
                        "UPDATE portal_sync_runs SET
                    completed_at = strftime(now(), '%Y-%m-%dT%H:%M:%S.%fZ'),
                    status = ?1,
                    pages_processed = ?2,
                    requests_processed = ?3,
                    documents_processed = ?4,
                    failures = ?5
                 WHERE run_id = ?6 AND portal_subdomain = ?7 AND status = 'RUNNING'",
                        params![
                            status,
                            pages_processed,
                            requests_processed,
                            documents_processed,
                            failures,
                            run_id,
                            portal_key,
                        ],
                    )
                    .map_err(|error| {
                        AppError::Storage(format!("Failed to finalize portal sync run: {error}"))
                    })?;
                if updated != 1 {
                    return Err(AppError::Storage(format!(
                        "Portal sync run '{run_id}' was missing or already finalized"
                    )));
                }
                // A cancelled run was stopped by the operator (pause, removal) or by a failed start,
                // so it says nothing about the source's health and leaves it as it was.
                transaction
                    .execute(
                        "UPDATE portals SET
                        last_synced_at = strftime(now(), '%Y-%m-%dT%H:%M:%S.%fZ'),
                        health_status = CASE ?2
                            WHEN 'COMPLETED' THEN 'verified'
                            WHEN 'CANCELLED' THEN health_status
                            ELSE 'degraded'
                        END,
                        last_error = CASE WHEN ?2 = 'COMPLETED' THEN NULL ELSE last_error END
                     WHERE subdomain = ?1",
                        params![portal_key, status],
                    )
                    .map_err(|error| {
                        AppError::Storage(format!("Failed to update portal sync time: {error}"))
                    })?;
                Ok(())
            })
        })
        .await
    }

    /// Records whether a saved source still synchronizes, and returns whether a source by that
    /// key existed to record it against.
    pub async fn set_source_lifecycle_state(
        &self,
        source_key: &str,
        state: SourceLifecycleState,
    ) -> Result<bool, AppError> {
        let source_key = source_key.to_string();
        self.query_connection("Set source lifecycle state", move |conn| {
            let updated = conn
                .execute(
                    "UPDATE portals SET lifecycle_state = ?2 WHERE subdomain = ?1",
                    params![source_key, state.as_str()],
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to record source lifecycle state: {error}"))
                })?;
            Ok(updated > 0)
        })
        .await
    }

    /// `None` distinguishes "no source by this key" from any lifecycle state a source can hold.
    pub async fn source_lifecycle_state(
        &self,
        source_key: &str,
    ) -> Result<Option<SourceLifecycleState>, AppError> {
        let source_key = source_key.to_string();
        self.query_connection("Source lifecycle lookup", move |conn| {
            let stored = conn
                .query_row(
                    "SELECT COALESCE(lifecycle_state, 'ACTIVE') FROM portals WHERE subdomain = ?1",
                    params![source_key],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|error| {
                    AppError::Storage(format!("Failed to read source lifecycle state: {error}"))
                })?;
            Ok(stored.as_deref().map(SourceLifecycleState::from_stored))
        })
        .await
    }

    /// Drops every queued and leased frontier row belonging to a source, and reports how many.
    ///
    /// Only a withdrawal calls this. A pause must not, because the frontier is the entire
    /// difference between resuming a crawl and restarting it.
    pub async fn discard_source_frontier(&self, source_key: &str) -> Result<u64, AppError> {
        let source_key = source_key.to_string();
        self.query_connection("Discard source frontier", move |conn| {
            let discarded = conn
                .execute(
                    "DELETE FROM ingestion_task_frontier WHERE source_key = ?1",
                    params![source_key],
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to discard source frontier: {error}"))
                })?;
            u64::try_from(discarded).map_err(|_| {
                AppError::Storage("Discarded frontier count exceeds supported capacity".to_string())
            })
        })
        .await
    }

    /// Forgets the learned crawl recipe for a source. Withdrawing a source withdraws the
    /// instructions for crawling it; re-adding it later must re-derive them rather than silently
    /// reuse a recipe the operator believed they had discarded.
    pub async fn delete_source_connector_recipe(&self, source_key: &str) -> Result<(), AppError> {
        let source_key = source_key.to_string();
        self.query_connection("Delete source connector recipe", move |conn| {
            conn.execute(
                "DELETE FROM source_connector_recipes WHERE source_key = ?1",
                params![source_key],
            )
            .map_err(|error| {
                AppError::Storage(format!(
                    "Failed to discard source connector recipe: {error}"
                ))
            })?;
            Ok(())
        })
        .await
    }

    /// How many ingested documents a source withdrawal leaves behind.
    pub async fn count_source_documents(&self, source_key: &str) -> Result<u64, AppError> {
        let source_key = source_key.to_string();
        self.query_connection("Source document count", move |conn| {
            let count = conn
                .query_row(
                    "SELECT COUNT(*) FROM documents WHERE portal_subdomain = ?1",
                    params![source_key],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to count source documents: {error}"))
                })?;
            u64::try_from(count).map_err(|_| {
                AppError::Storage("Source document count exceeds supported capacity".to_string())
            })
        })
        .await
    }

    /// Every source the operator still holds, paused ones included.
    ///
    /// A withdrawn source is excluded here rather than deleted: the `portals` row is the parent of
    /// the requests and documents ingested from it, so removing it would take the evidence with
    /// it. Withdrawal is therefore a state on the row, and this listing is the one place that
    /// state decides visibility.
    pub async fn list_portal_registrations(&self) -> Result<Vec<SavedSource>, AppError> {
        self.query_connection("Portal registration listing", |conn| {
            let mut statement = conn
                .prepare(
                    "SELECT subdomain, display_name, base_url, origin_url, requests_endpoint,
                    requests_pointer, metadata_pointer, page_parameter, page_size_parameter,
                    COALESCE(verified_at, created_at), source_kind,
                    COALESCE(lifecycle_state, 'ACTIVE'),
                    COALESCE(health_status, 'verified'), last_error
                 FROM portals
                 WHERE requests_endpoint <> ''
                   AND COALESCE(lifecycle_state, 'ACTIVE') <> 'REMOVED'
                 ORDER BY display_name COLLATE NOCASE",
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to prepare portal listing: {error}"))
                })?;
            let rows = statement
                .query_map([], |row| {
                    Ok(SavedSource {
                        registration: PortalRegistration {
                            portal_key: row.get(0)?,
                            display_name: row.get(1)?,
                            canonical_url: row.get(2)?,
                            origin_url: row.get(3)?,
                            api_layout: crate::types::PortalApiLayout {
                                requests_endpoint: row.get(4)?,
                                requests_pointer: row.get(5)?,
                                metadata_pointer: row.get(6)?,
                                page_parameter: row.get(7)?,
                                page_size_parameter: row.get(8)?,
                            },
                            verified_at: row.get(9)?,
                            source_kind: crate::types::SourceKind::from_stored(
                                &row.get::<_, String>(10)?,
                            ),
                        },
                        lifecycle_state: SourceLifecycleState::from_stored(
                            &row.get::<_, String>(11)?,
                        ),
                        health_status: row.get(12)?,
                        last_error: row.get(13)?,
                    })
                })
                .map_err(|error| AppError::Storage(format!("Failed to query portals: {error}")))?;
            rows.map(|row| {
                row.map_err(|error| AppError::Storage(format!("Failed to decode portal: {error}")))
            })
            .collect()
        })
        .await
    }
}

/// Which column identifies a source's *own* recipe row in a lookup.
///
/// The distinction is not cosmetic: the row a caller owns is the only one its page window may be
/// taken from, and the two callers reach the same row by different identifiers. Resumes hold the
/// registration and match the primary key; `start_data_source` runs before discovery and has
/// nothing but the URL.
#[derive(Clone, Copy)]
enum RecipeOwnRow {
    SourceKey,
    RootUrl,
}

/// The rank a row gets when it matched only the publisher-reuse arm, and so is not the caller's
/// own row. Kept as a constant because the SQL that assigns it and the resolution that reads it
/// are written apart.
const PUBLISHER_REUSE_RANK: i64 = 2;

impl RecipeOwnRow {
    fn column(self) -> &'static str {
        match self {
            Self::SourceKey => "source_key",
            Self::RootUrl => "root_url",
        }
    }

    /// The second column, if any, on which this variant may recognize its own row.
    ///
    /// Only the URL variant has one. `submitted_url` records what the operator typed before
    /// discovery resolved it, which is the only thing `start_data_source` can match on a source
    /// whose URL redirects. A key lookup has the key and must not consult it: a submitted string
    /// is not a source key and matching one would attach another source's recipe.
    fn alternate_column(self) -> Option<&'static str> {
        match self {
            Self::SourceKey => None,
            Self::RootUrl => Some("submitted_url"),
        }
    }
}

/// The operator's submitted URL in the one form the writers and the URL reader agree on.
///
/// Normalization fills in a missing scheme and drops a fragment. It does not resolve redirects,
/// which is exactly why this value is stored beside `root_url` rather than in place of it: the
/// final URL is not knowable until the fetch, and `start_data_source` reads the recipe before it.
pub(super) fn normalized_submitted_url(value: &str) -> String {
    let trimmed = value.trim();
    normalize_submitted_url(trimmed)
        .map(|url| url.to_string())
        .unwrap_or_else(|_| trimmed.to_string())
}

pub(super) fn publisher_pattern_for_url(value: &str) -> Option<String> {
    reqwest::Url::parse(value)
        .ok()?
        .host_str()
        .map(|host| host.trim_end_matches('.').to_ascii_lowercase())
        .filter(|host| !host.is_empty())
}

fn validate_portal_registration(portal: &PortalRegistration) -> Result<(), AppError> {
    if portal.portal_key.is_empty()
        || portal.portal_key.len() > 128
        || !portal
            .portal_key
            .bytes()
            .all(|value| value.is_ascii_alphanumeric() || matches!(value, b'-' | b'_'))
    {
        return Err(AppError::DataParsing(format!(
            "Portal key '{}' is invalid",
            portal.portal_key
        )));
    }
    for (label, value) in [
        ("canonical URL", portal.canonical_url.as_str()),
        ("origin URL", portal.origin_url.as_str()),
        (
            "requests endpoint",
            portal.api_layout.requests_endpoint.as_str(),
        ),
    ] {
        let url = reqwest::Url::parse(value).map_err(|error| {
            AppError::DataParsing(format!("Portal {label} '{value}' is invalid: {error}"))
        })?;
        let local_source_uri = matches!(
            portal.source_kind,
            crate::types::SourceKind::LocalFile | crate::types::SourceKind::PastedContent
        ) && url.scheme() == "onrecord"
            && url.host_str() == Some("local");
        if url.scheme() != "https" && !local_source_uri {
            return Err(AppError::PortalSecurity(format!(
                "Portal {label} must use HTTPS or an internal onrecord://local URI"
            )));
        }
    }
    if portal.display_name.trim().is_empty()
        || portal.api_layout.page_parameter.trim().is_empty()
        || portal.api_layout.page_size_parameter.trim().is_empty()
    {
        return Err(AppError::DataParsing(
            "Portal display name and pagination parameters must not be empty".to_string(),
        ));
    }
    chrono::DateTime::parse_from_rfc3339(&portal.verified_at).map_err(|error| {
        AppError::DataParsing(format!("Portal verification timestamp is invalid: {error}"))
    })?;
    Ok(())
}
