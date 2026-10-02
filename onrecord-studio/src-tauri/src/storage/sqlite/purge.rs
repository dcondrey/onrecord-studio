//! Removing documents and sources without leaving orphaned rows or objects.

use super::*;
use super::{
    analytical::quote_ident, connection::execute_transaction,
    integrity::resynchronize_request_density_flags,
};

impl DuckDbRepository {
    /// Discards a document row that guidance refused after extraction, before any extracted text
    /// was ever committed for it. Scoped to that narrow case: a document with no `documents_fts`
    /// row, which is exactly the state a stub row is in between `insert_or_update_document` and a
    /// successful `finalize_document_extraction`. The `NOT EXISTS` guard makes that precondition
    /// real rather than documentation-only: a resumed sync re-extracts every document regardless
    /// of prior completion, and a guidance edit between syncs must not retroactively purge a
    /// document a prior sync already finalized as evidence -- `documents_fts` carries no foreign
    /// key back to `documents`, so an unconditional delete here would orphan it (and a stale
    /// corpus index entry) silently. Returns whether a row was actually removed; `false` means
    /// the document was already finalized and was left in place, and callers must not count that
    /// as a refusal. `provenance_manifests` needs no matching guard: its foreign key to
    /// `documents` already makes DuckDB refuse the delete outright if a manifest was recorded.
    pub async fn delete_unextracted_document(
        &self,
        subdomain: &str,
        document_id: u64,
    ) -> Result<bool, AppError> {
        let document_id = i64::try_from(document_id).map_err(|error| {
            AppError::DataParsing(format!(
                "Document identifier cannot be represented by SQLite: {error}"
            ))
        })?;
        let subdomain = subdomain.to_string();
        self.query_connection("Delete unextracted document", move |conn| {
            execute_transaction(conn, "refused document discard", |transaction| {
                let removed = transaction
                    .execute(
                        "DELETE FROM documents
                     WHERE id = ?1 AND portal_subdomain = ?2
                       AND NOT EXISTS (
                         SELECT 1 FROM documents_fts
                         WHERE documents_fts.document_id = documents.id
                           AND documents_fts.portal_subdomain = documents.portal_subdomain
                       )",
                        params![document_id, subdomain],
                    )
                    .map_err(|error| {
                        AppError::Internal(format!("Failed to discard refused document: {error}"))
                    })?;
                if removed > 0 {
                    transaction
                        .execute(
                            "DELETE FROM document_acquisitions
                             WHERE document_id = ?1 AND portal_subdomain = ?2",
                            params![document_id, subdomain],
                        )
                        .map_err(|error| {
                            AppError::Internal(format!(
                                "Failed to discard a refused document's fetch history: {error}"
                            ))
                        })?;
                }
                Ok(removed > 0)
            })
        })
        .await
    }

    /// Hard-deletes one document and every row derived from it.
    ///
    /// This is the deliberate counterpart to [`Self::delete_unextracted_document`], which refuses
    /// any document a sync already finalized so that a guidance refusal can never destroy
    /// evidence. Purging is the operator saying the opposite, and it must be reachable only from
    /// an explicitly destructive command.
    ///
    /// The caller unlinks the returned `local_path` afterwards and purges the corpus projection.
    pub async fn purge_document(
        &self,
        subdomain: &str,
        document_id: u64,
    ) -> Result<CommittedPurge<DocumentPurge>, AppError> {
        let stored_id = i64::try_from(document_id).map_err(|error| {
            AppError::DataParsing(format!(
                "Document identifier cannot be represented by the core database: {error}"
            ))
        })?;
        let subdomain = subdomain.to_string();
        self.query_connection("Purge document", move |conn| {
            let transaction = conn.transaction().map_err(|error| {
                AppError::Storage(format!(
                    "Failed to open a document purge transaction: {error}"
                ))
            })?;
            let (purge, retired_snapshots) =
                purge_document_rows(&transaction, &subdomain, stored_id, document_id)?;
            transaction.commit().map_err(|error| {
                AppError::Storage(format!("Failed to commit a document purge: {error}"))
            })?;
            let retirement_error = retire_fk_parent_rows(
                conn,
                "analytical_snapshots",
                "snapshot_id",
                &retired_snapshots,
                "Purged an analytical snapshot's rows but could not remove its snapshot record, \
             so it is still listed with nothing under it. Running the purge again is safe \
             and finishes it",
            )
            .err();
            Ok(CommittedPurge {
                purge,
                retirement_error,
            })
        })
        .await
    }

    /// Counts what [`Self::purge_source`] would destroy, without destroying it.
    ///
    /// Every predicate here is copied from the corresponding delete in `purge_source`, and
    /// `purge_source_preview_matches_purge` proves the two agree by previewing and then purging
    /// the same database. A confirmation dialog that names a count the purge does not honour is a
    /// false statement about evidence, so the counts are not allowed to drift silently.
    ///
    /// The evidence-file count is the number of documents holding a `local_path`, which is what
    /// the purge collects and unlinks. It is reported separately from the document count because
    /// a document whose bytes were never retained still exists as a record.
    pub async fn preview_source_purge(
        &self,
        source_key: &str,
    ) -> Result<SourcePurgePreview, AppError> {
        let key = source_key.to_string();
        let source_key = source_key.to_string();
        self.query_connection("Source purge preview", move |conn| {
            let count = |sql: &str| -> Result<u64, AppError> {
                let value = conn
                    .query_row(sql, params![key], |row| row.get::<_, i64>(0))
                    .map_err(|error| {
                        AppError::Storage(format!("Failed to count a source purge: {error}"))
                    })?;
                u64::try_from(value).map_err(|_| {
                    AppError::Storage("A source purge count exceeds supported capacity".to_string())
                })
            };
            Ok(SourcePurgePreview {
                source_key: source_key.clone(),
                document_count: count(
                    "SELECT COUNT(*) FROM documents WHERE portal_subdomain = ?1",
                )?,
                request_count: count("SELECT COUNT(*) FROM requests WHERE portal_subdomain = ?1")?,
                evidence_file_count: count(
                    "SELECT COUNT(DISTINCT d.local_path) FROM documents d
                     WHERE d.portal_subdomain = ?1 AND d.local_path IS NOT NULL
                       AND NOT EXISTS (
                           SELECT 1 FROM documents other
                           WHERE other.local_path = d.local_path
                             AND other.portal_subdomain <> ?1
                       )",
                )?,
            })
        })
        .await
    }

    /// Hard-deletes a source and everything ingested under it, including the `portals` row.
    ///
    /// [`crate::ingestion::queue::QueueManager::remove_source`] withdraws a source while keeping
    /// its documents, and says so. This destroys them. Both are legitimate and they are not the
    /// same operation, so they stay separate commands with separate confirmations rather than one
    /// with a flag.
    ///
    /// Deleting the `portals` row is the point of the difference: a withdrawn source is invisible
    /// but immortal, since `list_portal_registrations` filters `REMOVED` out and nothing ever
    /// re-activates it, so the same URL can never be added again cleanly. A purged source is gone,
    /// and adding it back is an ordinary first ingestion.
    pub async fn purge_source(
        &self,
        source_key: &str,
    ) -> Result<CommittedPurge<SourcePurge>, AppError> {
        let source_key = source_key.to_string();
        self.query_connection("Purge source", move |conn| {
        let source_key = source_key.as_str();
        let transaction = conn.transaction().map_err(|error| {
            AppError::Storage(format!(
                "Failed to open a source purge transaction: {error}"
            ))
        })?;
        let local_paths = {
            let mut statement = transaction
                .prepare(
                    "WITH candidates(path) AS (
                         SELECT local_path FROM documents
                         WHERE portal_subdomain = ?1 AND local_path IS NOT NULL
                         UNION
                         SELECT pending_path FROM data_retention_receipts
                         WHERE portal_key = ?1 AND state IN ('PREPARED', 'REFERENCED')
                     )
                     SELECT path FROM candidates
                     WHERE NOT EXISTS (
                         SELECT 1 FROM documents other
                         WHERE other.local_path = candidates.path AND other.portal_subdomain <> ?1
                     )
                     AND NOT EXISTS (
                         SELECT 1 FROM data_retention_receipts other
                         WHERE other.pending_path = candidates.path
                           AND other.portal_key <> ?1
                           AND other.state IN ('PREPARED', 'REFERENCED')
                     )",
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to prepare evidence path lookup: {error}"))
                })?;
            let rows = statement
                .query_map(params![source_key], |row| row.get::<_, String>(0))
                .map_err(|error| {
                    AppError::Storage(format!("Failed to query evidence paths: {error}"))
                })?;
            let mut collected = Vec::new();
            for row in rows {
                collected.push(row.map_err(|error| {
                    AppError::Storage(format!("Failed to decode an evidence path: {error}"))
                })?);
            }
            collected
        };

        let retired_snapshots =
            crate::storage::maintenance::purge_automatic_origins(&transaction, source_key, None)?;
        let mut documents_removed = 0u64;
        let mut requests_removed = 0u64;
        // Innermost first. DuckDB implements no cascading foreign-key actions, so every one of
        // these is a relation the schema declares and this function has to honour by hand. The
        // ordering is not cosmetic: `verify_logical_relations` audits ten of these relations at
        // startup and refuses to open a core database with orphans, so a table missed here does
        // not corrupt quietly, it makes the next launch fail.
        for (table, statement) in [
            ("data_retention_receipts", "DELETE FROM data_retention_receipts WHERE portal_key = ?1"),
            (
                "document_acquisitions",
                "DELETE FROM document_acquisitions WHERE portal_subdomain = ?1",
            ),
            ("data_maintenance_jobs", "DELETE FROM data_maintenance_jobs WHERE job_key = 'refresh:' || ?1 OR starts_with(job_key, 'import:' || ?1 || ':') OR starts_with(job_key, 'compact:' || ?1 || ':')"),
            (
                "provenance_assertions",
                "DELETE FROM provenance_assertions WHERE portal_subdomain = ?1",
            ),
            (
                "provenance_verification_log",
                "DELETE FROM provenance_verification_log WHERE portal_subdomain = ?1",
            ),
            (
                "provenance_manifests",
                "DELETE FROM provenance_manifests WHERE portal_subdomain = ?1",
            ),
            (
                "document_search_index_state",
                "DELETE FROM document_search_index_state WHERE portal_subdomain = ?1",
            ),
            (
                "document_search_tokens",
                "DELETE FROM document_search_tokens WHERE portal_subdomain = ?1",
            ),
            (
                "documents_fts",
                "DELETE FROM documents_fts WHERE portal_subdomain = ?1",
            ),
            (
                "documents",
                "DELETE FROM documents WHERE portal_subdomain = ?1",
            ),
            (
                "statutory_violations",
                "DELETE FROM statutory_violations WHERE portal_subdomain = ?1",
            ),
            ("findings", "DELETE FROM findings WHERE subject_portal_key = ?1"),
            (
                "report_exports",
                "DELETE FROM report_exports WHERE portal_subdomain = ?1",
            ),
            (
                "cross_correlation_alerts",
                "DELETE FROM cross_correlation_alerts WHERE portal_subdomain = ?1",
            ),
            (
                "state_transition_logs",
                "DELETE FROM state_transition_logs WHERE portal_subdomain = ?1",
            ),
            (
                "statutory_extensions",
                "DELETE FROM statutory_extensions WHERE portal_subdomain = ?1",
            ),
            (
                "request_keyword_signals",
                "DELETE FROM request_keyword_signals WHERE portal_subdomain = ?1",
            ),
            (
                "document_integrity_flags",
                "DELETE FROM document_integrity_flags WHERE audit_id IN (
                   SELECT audit_id FROM case_audits WHERE target_subdomain = ?1
                 )",
            ),
            (
                "case_audits",
                "DELETE FROM case_audits WHERE target_subdomain = ?1",
            ),
            (
                "requests",
                "DELETE FROM requests WHERE portal_subdomain = ?1",
            ),
            (
                "auxiliary_events",
                "DELETE FROM auxiliary_events WHERE source_id IN (
                   SELECT source_id FROM auxiliary_data_sources WHERE portal_subdomain = ?1
                 )",
            ),
            (
                "auxiliary_data_sources",
                "DELETE FROM auxiliary_data_sources WHERE portal_subdomain = ?1",
            ),
            (
                "agencies",
                "DELETE FROM agencies WHERE portal_subdomain = ?1",
            ),
            (
                "categories",
                "DELETE FROM categories WHERE portal_subdomain = ?1",
            ),
            (
                "source_ingestion_failures",
                "DELETE FROM source_ingestion_failures WHERE source_key = ?1",
            ),
            (
                "ingestion_task_frontier",
                "DELETE FROM ingestion_task_frontier WHERE source_key = ?1",
            ),
            (
                "source_connector_recipes",
                "DELETE FROM source_connector_recipes WHERE source_key = ?1",
            ),
            (
                "portal_sync_runs",
                "DELETE FROM portal_sync_runs WHERE portal_subdomain = ?1",
            ),
        ] {
            let affected = transaction
                .execute(statement, params![source_key])
                .map_err(|error| AppError::Storage(format!("Failed to purge {table}: {error}")))?;
            match table {
                "documents" => documents_removed = affected as u64,
                "requests" => requests_removed = affected as u64,
                _ => {}
            }
        }

        transaction.commit().map_err(|error| {
            AppError::Storage(format!("Failed to commit a source purge: {error}"))
        })?;
        let snapshots_retired = retire_fk_parent_rows(
            conn,
            "analytical_snapshots",
            "snapshot_id",
            &retired_snapshots,
            "Purged an analytical snapshot's rows but could not remove its snapshot record, \
             so it is still listed with nothing under it. Running the purge again is safe \
             and finishes it",
        );
        let portal_retired = retire_fk_parent_rows(
            conn,
            "portals",
            "subdomain",
            &[source_key.to_string()],
            &format!(
                "Purged every record of '{source_key}' but could not remove its registration, \
                 so the source is still listed with nothing under it. Running the purge again \
                 is safe and finishes it"
            ),
        );

        Ok(CommittedPurge {
            purge: SourcePurge {
                source_key: source_key.to_string(),
                documents_removed,
                requests_removed,
                local_paths,
            },
            retirement_error: snapshots_retired.and(portal_retired).err(),
        })
        })
        .await
    }
}

/// Deletes rows from `table` (matched on `column`) in a transaction of its own, after the
/// caller has already committed the transaction that emptied their children.
///
/// It has to be a second transaction: DuckDB resolves foreign keys through an index whose entries
/// are not released until the referencing delete commits, so deleting a parent in the same
/// transaction that emptied its children fails with "still referenced by a foreign key in a
/// different table" even though no child row will survive. This is the same family of constraint
/// limitation CLAUDE.md records for `ADD CONSTRAINT` and `ALTER COLUMN SET NOT NULL`.
///
/// A failure here leaves a registered parent with nothing under it: visible, harmless, and
/// finished by running the purge again. That is a better failure than holding every child delete
/// open across a second statement. `retry_message` is folded into the returned error so the
/// operator sees that the fix is to run the purge again.
fn retire_fk_parent_rows(
    conn: &mut Connection,
    table: &str,
    column: &str,
    ids: &[String],
    retry_message: &str,
) -> Result<(), AppError> {
    if ids.is_empty() {
        return Ok(());
    }
    let transaction = conn.transaction().map_err(|error| {
        AppError::Storage(format!("Failed to open the {table} removal: {error}"))
    })?;
    let quoted_table = quote_ident(table);
    let quoted_column = quote_ident(column);
    for id in ids {
        transaction
            .execute(
                &format!("DELETE FROM {quoted_table} WHERE {quoted_column} = ?1"),
                params![id],
            )
            .map_err(|error| AppError::Storage(format!("{retry_message}: {error}")))?;
    }
    transaction.commit().map_err(|error| {
        AppError::Storage(format!("Failed to commit the {table} removal: {error}"))
    })
}

/// Deletes one document and every row that references it, innermost first.
///
/// DuckDB implements no cascading foreign-key actions, so this ordering is the schema's foreign-key
/// ordering reversed and it has to be maintained by hand. It is also what keeps the next launch
/// working: `verify_logical_relations` audits ten of these relations and refuses to open a core
/// database containing orphans, so a table missed here does not produce quiet inconsistency, it
/// makes the application unlaunchable.
///
/// `provenance_verification_log` is documented append-only and this is the only operation permitted
/// to delete from it. What that contract protects is retraction: a verification result may never be
/// edited or quietly withdrawn while the document it describes remains, because a reader would then
/// see a document whose provenance history had been rewritten. Removing the log together with the
/// document leaves no verification claim standing about anything, which is the absence of a
/// statement rather than a false one.
fn purge_document_rows(
    transaction: &Transaction<'_>,
    subdomain: &str,
    stored_id: i64,
    document_id: u64,
) -> Result<(DocumentPurge, Vec<String>), AppError> {
    // Objects are content-addressed, so identical bytes under another document share this file;
    // a path any other row still references is not this purge's to release.
    let local_path = transaction
        .query_row(
            "SELECT local_path FROM documents d
             WHERE d.id = ?1 AND d.portal_subdomain = ?2
               AND NOT EXISTS (
                   SELECT 1 FROM documents other
                   WHERE other.local_path = d.local_path
                     AND NOT (other.id = ?1 AND other.portal_subdomain = ?2)
               )
               AND NOT EXISTS (
                   SELECT 1 FROM data_retention_receipts other
                   WHERE other.pending_path = d.local_path
                     AND NOT (other.document_id = ?1 AND other.portal_key = ?2)
                     AND other.state IN ('PREPARED', 'REFERENCED')
               )",
            params![stored_id, subdomain],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()
        .map_err(|error| {
            AppError::Storage(format!(
                "Failed to read the document evidence path: {error}"
            ))
        })?
        .flatten();

    let additional_local_paths = {
        let mut query = transaction.prepare("SELECT pending_path FROM data_retention_receipts WHERE portal_key = ?1 AND document_id = ?2 AND state IN ('PREPARED', 'REFERENCED')").map_err(|e| AppError::Storage(e.to_string()))?;
        query
            .query_map(params![subdomain, stored_id], |row| row.get::<_, String>(0))
            .map_err(|e| AppError::Storage(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| AppError::Storage(e.to_string()))?
    };
    let request_id = transaction
        .query_row(
            "SELECT request_id FROM documents WHERE id = ?1 AND portal_subdomain = ?2",
            params![stored_id, subdomain],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|error| {
            AppError::Storage(format!("Failed to read the document's request: {error}"))
        })?;
    let retired_snapshots = crate::storage::maintenance::purge_automatic_origins(
        transaction,
        subdomain,
        Some(stored_id),
    )?;
    let mut search_rows_removed = 0u64;
    let mut provenance_rows_removed = 0u64;
    for (table, statement) in [
        ("data_retention_receipts", "DELETE FROM data_retention_receipts WHERE document_id = ?1 AND portal_key = ?2"),
        (
            "document_acquisitions",
            "DELETE FROM document_acquisitions WHERE document_id = ?1 AND portal_subdomain = ?2",
        ),
        ("data_maintenance_jobs", "DELETE FROM data_maintenance_jobs WHERE starts_with(job_key, 'import:' || ?2 || ':' || CAST(?1 AS VARCHAR) || ':') OR starts_with(job_key, 'compact:' || ?2 || ':' || CAST(?1 AS VARCHAR) || ':')"),
        (
            "provenance_assertions",
            "DELETE FROM provenance_assertions WHERE document_id = ?1 AND portal_subdomain = ?2",
        ),
        (
            "provenance_verification_log",
            "DELETE FROM provenance_verification_log
             WHERE document_id = ?1 AND portal_subdomain = ?2",
        ),
        (
            "provenance_manifests",
            "DELETE FROM provenance_manifests WHERE document_id = ?1 AND portal_subdomain = ?2",
        ),
        (
            "document_search_index_state",
            "DELETE FROM document_search_index_state
             WHERE document_id = ?1 AND portal_subdomain = ?2",
        ),
        (
            "document_search_tokens",
            "DELETE FROM document_search_tokens WHERE document_id = ?1 AND portal_subdomain = ?2",
        ),
        (
            "documents_fts",
            "DELETE FROM documents_fts WHERE document_id = ?1 AND portal_subdomain = ?2",
        ),
        (
            "documents",
            "DELETE FROM documents WHERE id = ?1 AND portal_subdomain = ?2",
        ),
        (
            "findings",
            "DELETE FROM findings WHERE subject_document_id = ?1 AND subject_portal_key = ?2",
        ),
    ] {
        let affected = transaction
            .execute(statement, params![stored_id, subdomain])
            .map_err(|error| AppError::Storage(format!("Failed to purge {table}: {error}")))?
            as u64;
        match table {
            "provenance_assertions" | "provenance_verification_log" | "provenance_manifests" => {
                provenance_rows_removed += affected;
            }
            "document_search_index_state" | "document_search_tokens" | "documents_fts" => {
                search_rows_removed += affected;
            }
            _ => {}
        }
    }

    if let Some(request_id) = request_id {
        resynchronize_request_density_flags(transaction, request_id, subdomain)?;
    }

    Ok((
        DocumentPurge {
            document_id,
            source_key: subdomain.to_string(),
            local_path,
            additional_local_paths,
            search_rows_removed,
            provenance_rows_removed,
        },
        retired_snapshots,
    ))
}
