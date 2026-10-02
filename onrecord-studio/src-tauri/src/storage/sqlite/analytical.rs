//! Analytical CSV imports and the restricted SQL surface over them.

use super::*;

impl DuckDbRepository {
    pub async fn import_analytical_csv(
        &self,
        request: AnalyticalImportRequest,
    ) -> Result<AnalyticalImportResult, AppError> {
        self.import_analytical_csv_recording_origin(request, None)
            .await
    }

    /// As [`Self::import_analytical_csv`], and when `automatic_origin` is given, also records the
    /// `data_maintenance_origins` receipt for it inside the same transaction as the snapshot
    /// write. The two have to commit or roll back together: a crash between two separate
    /// transactions would leave a snapshot on disk with no origin receipt, which is exactly what
    /// later maintenance and purge logic uses to tell an automatic import from an intentionally
    /// partial manual one.
    pub(crate) async fn import_analytical_csv_recording_origin(
        &self,
        request: AnalyticalImportRequest,
        automatic_origin: Option<(String, u64)>,
    ) -> Result<AnalyticalImportResult, AppError> {
        let dataset_kind = request.dataset_kind.clone();
        let local_path = request.local_path.clone();
        let worksheet = request.worksheet.clone();
        let format_name = request
            .source_file_name
            .clone()
            .unwrap_or_else(|| local_path.clone());
        let (source_sha256, prepared) = tokio::task::spawn_blocking(move || {
            let bytes = crate::parsers::source::read_regular_file(
                std::path::Path::new(&local_path),
                "ANALYTICAL",
                crate::storage::tabular::MAX_SOURCE_BYTES,
            )?;
            let prepared =
                crate::storage::tabular::prepare(&bytes, &format_name, worksheet.as_deref())?;
            Ok::<_, AppError>((sha256_hex(&bytes), prepared))
        })
        .await
        .map_err(|error| AppError::Internal(format!("Table preparation task failed: {error}")))??;
        if request
            .expected_source_sha256
            .as_ref()
            .is_some_and(|expected| !expected.eq_ignore_ascii_case(&source_sha256))
        {
            return Err(AppError::DataParsing(
                "The acquired file no longer matches its recorded SHA-256; import stopped.".into(),
            ));
        }
        // Equal bytes from different publishers or worksheets are distinct acquisitions.
        let identity_hash = sha256_hex(
            serde_json::json!([
                ANALYTICAL_PARSER_VERSION,
                source_sha256,
                request.source_uri,
                request.worksheet,
                request.observation_period_start,
                request.observation_period_end,
                request.partial_dataset
            ])
            .to_string()
            .as_bytes(),
        );
        let snapshot_id = format!(
            "{}-{}",
            dataset_kind.as_str().to_ascii_lowercase().replace('_', "-"),
            &identity_hash[..24]
        );
        let table_name = format!(
            "analysis_{}_{}",
            dataset_kind.as_str().to_ascii_lowercase(),
            &identity_hash[..24]
        );
        let latest_view_name = if dataset_kind == AnalyticalDatasetKind::RegionalData {
            let source_identity = request.source_uri.as_deref().unwrap_or(&request.local_path);
            let key = sha256_hex(
                serde_json::json!([source_identity, request.worksheet])
                    .to_string()
                    .as_bytes(),
            );
            format!("analysis_regional_{}_latest", &key[..16])
        } else {
            format!(
                "analysis_{}_latest",
                dataset_kind.as_str().to_ascii_lowercase()
            )
        };
        let headers = prepared.headers;
        let mut aliases = std::collections::BTreeSet::new();
        let columns = headers
            .iter()
            .enumerate()
            .map(|(index, header)| {
                let base = unique_column_alias(header, index, &headers);
                let mut alias = base.clone();
                let mut suffix = 2;
                while !aliases.insert(alias.clone()) {
                    alias = format!("{base}_{suffix}");
                    suffix += 1;
                }
                let logical_type = if dataset_kind == AnalyticalDatasetKind::RegionalData {
                    crate::storage::tabular::infer_type(header, index, &prepared.records)
                } else {
                    logical_type_for(&dataset_kind, header)
                };
                (header.clone(), alias, logical_type)
            })
            .collect::<Vec<_>>();
        let create_columns = columns
            .iter()
            .map(|(_, alias, logical)| format!("{} {}", quote_ident(alias), duckdb_type(logical)))
            .collect::<Vec<_>>()
            .join(", ");
        let mut rows = Vec::new();
        let mut malformed_rows = 0usize;
        let mut rejected_rows = 0usize;
        let mut repaired_trailing_rows = 0usize;
        let mut repair_row_numbers = Vec::new();
        let mut duplicate_key_rows = 0usize;
        let mut seen_keys = std::collections::BTreeSet::new();
        let mut null_counts = vec![0usize; columns.len()];
        for (row_index, record) in prepared.records.into_iter().enumerate() {
            let record = match record {
                Ok(record) => record,
                Err(error) => {
                    malformed_rows += 1;
                    rejected_rows += 1;
                    rows.push((
                        row_index + 1,
                        Vec::new(),
                        vec![format!("CSV row parse failed: {error}")],
                        serde_json::Value::Null,
                    ));
                    continue;
                }
            };
            // Position retains duplicate headers and surplus fields in malformed source rows.
            // Keep common headers once in memory; restore them in each persisted row ledger.
            let original = serde_json::json!(record.iter().collect::<Vec<_>>());
            let mut values = Vec::with_capacity(columns.len());
            let mut errors = Vec::new();
            for (index, (_, _, logical)) in columns.iter().enumerate() {
                let raw = record.get(index).unwrap_or("").trim();
                match normalize_csv_value(raw, logical) {
                    Ok(value) => {
                        if value.is_null() {
                            null_counts[index] += 1;
                        }
                        values.push(value);
                    }
                    Err(error) => {
                        errors.push(format!("{}: {error}", headers[index]));
                        values.push(serde_json::Value::Null);
                    }
                }
            }
            if record.len() > headers.len()
                && record
                    .iter()
                    .skip(headers.len())
                    .all(|cell| cell.trim().is_empty())
            {
                repaired_trailing_rows += 1;
                if repair_row_numbers.len() < 1000 {
                    repair_row_numbers.push(row_index + 1);
                }
            } else if record.len() != headers.len() {
                errors.push(format!(
                    "Expected {} fields, found {}",
                    headers.len(),
                    record.len()
                ));
            }
            if let Some(key) = analytical_counting_key(&dataset_kind, &headers, &record) {
                if !seen_keys.insert(key) {
                    duplicate_key_rows += 1;
                }
            }
            if !errors.is_empty() {
                malformed_rows += 1;
                rejected_rows += 1;
            }
            rows.push((row_index + 1, values, errors, original));
        }

        let display_name = request.display_name.clone();
        let source_uri = request.source_uri.clone();
        let local_path = request.local_path.clone();
        let observation_start = request.observation_period_start.clone();
        let observation_end = request.observation_period_end.clone();
        let partial = request.partial_dataset || prepared.partial;
        let mut schema_warnings = schema_warnings_for(&headers, &request.dataset_kind);
        schema_warnings.extend(prepared.warnings);
        if repaired_trailing_rows > 0 {
            schema_warnings.push(format!("Format repair: ignored only empty surplus trailing fields in {repaired_trailing_rows} rows; all original field values remain in the row ledger."));
        }
        if prepared.partial {
            schema_warnings.push(
                "The source declares additional pages or worksheets; this snapshot is partial."
                    .into(),
            );
        }
        let columns_for_tx = columns.clone();
        let validation_summary = serde_json::json!({
            "malformedRows": malformed_rows,
            "duplicateKeyRows": duplicate_key_rows,
            "rejectedRows": rejected_rows,
            "repairs": { "emptyTrailingFields": { "rowCount": repaired_trailing_rows, "rowNumbers": repair_row_numbers, "rowNumbersTruncated": repaired_trailing_rows > 1000 } },
            "parserVersion": ANALYTICAL_PARSER_VERSION,
            "sourceFormat": prepared.format,
            "sourceDeclaresPartial": prepared.partial,
            "worksheet": request.worksheet,
            "warnings": schema_warnings,
            "nullCounts": columns.iter().enumerate().map(|(i, (_, alias, _))| (alias.clone(), null_counts[i])).collect::<std::collections::BTreeMap<_, _>>(),
            "notes": validation_notes_for(&dataset_kind),
        });
        let validation_summary_string = validation_summary.to_string();
        let rows_len = rows.len();
        let source_sha_for_tx = source_sha256.clone();
        let snapshot_for_tx = snapshot_id.clone();
        let table_for_tx = table_name.clone();
        let latest_view_for_tx = latest_view_name.clone();
        let origin_for_tx = automatic_origin;
        self.query_connection("Analytical CSV import", move |conn| {
            let tx = conn.transaction().map_err(|error| AppError::Storage(format!("Failed to begin analytical import: {error}")))?;
            tx.execute(&format!("DROP TABLE IF EXISTS {}", quote_ident(&table_for_tx)), [])
                .map_err(|error| AppError::Storage(format!("Failed to replace old analytical table: {error}")))?;
            tx.execute(&format!("CREATE TABLE {} ({create_columns})", quote_ident(&table_for_tx)), [])
                .map_err(|error| AppError::Storage(format!("Failed to create analytical table: {error}")))?;
            let mut insert = tx.appender(&table_for_tx)
                .map_err(|error| AppError::Storage(format!("Failed to prepare analytical row appender: {error}")))?;
            for (_, values, errors, _) in &rows {
                if !errors.is_empty() {
                    continue;
                }
                let params = values.iter().map(json_to_sql_value).collect::<Vec<_>>();
                insert.append_row(duckdb::appender_params_from_iter(params))
                    .map_err(|error| AppError::Storage(format!("Failed to insert analytical row: {error}")))?;
            }
            insert.flush().map_err(|error| AppError::Storage(format!("Failed to flush analytical rows: {error}")))?;
            drop(insert);
            tx.execute(
                &format!(
                    "CREATE OR REPLACE VIEW {} AS SELECT * FROM {}",
                    quote_ident(&latest_view_for_tx),
                    quote_ident(&table_for_tx)
                ),
                [],
            )
            .map_err(|error| AppError::Storage(format!("Failed to refresh latest analytical view: {error}")))?;
            tx.execute("DELETE FROM analytical_rows WHERE snapshot_id = ?1", params![snapshot_for_tx])
                .map_err(|error| AppError::Storage(format!("Failed to clear analytical row ledger: {error}")))?;
            tx.execute("DELETE FROM analytical_columns WHERE snapshot_id = ?1", params![snapshot_for_tx])
                .map_err(|error| AppError::Storage(format!("Failed to clear analytical columns: {error}")))?;
            tx.execute("DELETE FROM analytical_snapshots WHERE snapshot_id = ?1", params![snapshot_for_tx])
                .map_err(|error| AppError::Storage(format!("Failed to clear analytical snapshot: {error}")))?;
            tx.execute(
                "INSERT INTO analytical_snapshots (
                    snapshot_id, dataset_kind, display_name, table_name, source_uri, local_path,
                    source_sha256, retrieved_at, observation_period_start, observation_period_end,
                    partial_dataset, imported_rows, rejected_rows, malformed_rows,
                    duplicate_key_rows, validation_summary_json
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7,
                    strftime(now(), '%Y-%m-%dT%H:%M:%S.%fZ'), ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                params![
                    snapshot_for_tx,
                    dataset_kind.as_str(),
                    display_name,
                    table_for_tx,
                    source_uri,
                    local_path,
                    source_sha_for_tx,
                    observation_start,
                    observation_end,
                    if partial { 1_i64 } else { 0_i64 },
                    rows_len.saturating_sub(malformed_rows) as i64,
                    rejected_rows as i64,
                    malformed_rows as i64,
                    duplicate_key_rows as i64,
                    validation_summary_string,
                ],
            ).map_err(|error| AppError::Storage(format!("Failed to record analytical snapshot: {error}")))?;
            for (ordinal, (original, alias, logical)) in columns_for_tx.iter().enumerate() {
                tx.execute(
                    "INSERT INTO analytical_columns (
                        snapshot_id, table_name, column_name, original_name, logical_type,
                        nullable, dictionary, ordinal
                     ) VALUES (?1, ?2, ?3, ?4, ?5, 1, NULL, ?6)",
                    params![snapshot_for_tx, table_for_tx, alias, original, logical, ordinal as i64],
                ).map_err(|error| AppError::Storage(format!("Failed to record analytical column: {error}")))?;
            }
            let mut ledger = tx.appender("analytical_rows")
                .map_err(|error| AppError::Storage(format!("Failed to open analytical row ledger: {error}")))?;
            for (row_number, values, errors, original) in rows {
                let normalized = serde_json::Value::Object(
                    columns_for_tx.iter().zip(values)
                        .map(|((_, alias, _), value)| (alias.clone(), value))
                        .collect()
                );
                let payload = if original.is_null() { original.to_string() } else {
                    serde_json::json!({"headers": headers, "values": original}).to_string()
                };
                let normalized_payload = normalized.to_string();
                let errors_json = serde_json::Value::Array(errors.iter().cloned().map(serde_json::Value::String).collect()).to_string();
                let row_hash = sha256_hex(format!("{payload}\n{normalized_payload}").as_bytes());
                ledger.append_row(duckdb::params![
                        &snapshot_for_tx,
                        row_number as i64,
                        row_hash,
                        payload,
                        normalized_payload,
                        if errors.is_empty() { 0_i64 } else { 1_i64 },
                        errors_json,
                    ]).map_err(|error| AppError::Storage(format!("Failed to record analytical row ledger: {error}")))?;
            }
            ledger.flush().map_err(|error| AppError::Storage(format!("Failed to flush analytical row ledger: {error}")))?;
            drop(ledger);
            if let Some((portal_key, document_id)) = origin_for_tx.as_ref() {
                tx.execute(
                    "INSERT INTO data_maintenance_origins VALUES (?1, ?2, ?3) ON CONFLICT DO NOTHING",
                    params![snapshot_for_tx, portal_key, document_id],
                ).map_err(|error| AppError::Storage(format!("Failed to record the automatic-import origin receipt: {error}")))?;
            }
            tx.commit().map_err(|error| AppError::Storage(format!("Failed to commit analytical import: {error}")))?;
            Ok(())
        }).await?;
        Ok(AnalyticalImportResult {
            snapshot_id,
            dataset_kind: request.dataset_kind.clone(),
            display_name: request.display_name,
            table_name,
            imported_rows: rows_len.saturating_sub(malformed_rows),
            rejected_rows,
            malformed_rows,
            duplicate_key_rows,
            schema_warnings,
            source_sha256,
        })
    }

    pub async fn analytical_import_candidates(
        &self,
    ) -> Result<Vec<crate::types::AnalyticalImportCandidate>, AppError> {
        self.query_connection("Acquired analytical tables", move |conn| {
            let mut statement = conn.prepare("SELECT id, portal_subdomain, COALESCE(file_name, 'Dataset'), local_path, download_url, raw_file_sha256
                FROM documents WHERE local_path IS NOT NULL AND download_status = 'completed'
                AND LOWER(TRIM(visibility)) = 'public'
                AND (regexp_matches(lower(COALESCE(file_name, '')), '\\.(csv|tsv|json|jsonl|ndjson|geojson|xlsx|xls|ods)$')
                OR content_type IN ('text/csv', 'text/tab-separated-values', 'application/json', 'application/geo+json'))
                ORDER BY updated_at DESC, portal_subdomain, id LIMIT 500")
                .map_err(|e| AppError::Storage(format!("Failed to list acquired tables: {e}")))?;
            let rows = statement.query_map([], |row| Ok(crate::types::AnalyticalImportCandidate {
                document_id: row.get::<_, i64>(0)? as u64, portal_key: row.get(1)?, file_name: row.get(2)?,
                local_path: row.get(3)?, source_uri: row.get(4)?, source_sha256: row.get(5)?,
            })).map_err(|e| AppError::Storage(format!("Failed to query acquired tables: {e}")))?;
            rows.collect::<Result<Vec<_>, _>>().map_err(|e| AppError::Storage(format!("Failed to decode acquired tables: {e}")))
        }).await
    }

    pub async fn analytical_schema(&self) -> Result<AnalyticalSchema, AppError> {
        self.query_connection("Analytical schema", move |conn| {
            let mut datasets_stmt = conn.prepare(
                "SELECT snapshot_id, dataset_kind, display_name, table_name, source_uri,
                        source_sha256, retrieved_at, observation_period_start,
                        observation_period_end, partial_dataset, imported_rows,
                        rejected_rows, validation_summary_json
                 FROM analytical_snapshots
                 ORDER BY retrieved_at DESC"
            ).map_err(|error| AppError::Storage(format!("Failed to prepare analytical datasets: {error}")))?;
            let datasets = datasets_stmt.query_map([], |row| {
                Ok(AnalyticalDatasetSummary {
                    snapshot_id: row.get(0)?,
                    dataset_kind: dataset_kind_from_str(&row.get::<_, String>(1)?),
                    display_name: row.get(2)?,
                    table_name: row.get(3)?,
                    source_uri: row.get(4)?,
                    source_sha256: row.get(5)?,
                    retrieved_at: row.get(6)?,
                    observation_period_start: row.get(7)?,
                    observation_period_end: row.get(8)?,
                    partial_dataset: row.get::<_, i64>(9)? != 0,
                    imported_rows: row.get::<_, i64>(10)?.max(0) as usize,
                    rejected_rows: row.get::<_, i64>(11)?.max(0) as usize,
                    validation_summary: row.get(12)?,
                })
            }).map_err(|error| AppError::Storage(format!("Failed to query analytical datasets: {error}")))?
              .collect::<Result<Vec<_>, _>>()
              .map_err(|error| AppError::Storage(format!("Failed to decode analytical dataset: {error}")))?;
            let mut columns_stmt = conn.prepare(
                "SELECT table_name, column_name, logical_type, original_name, nullable, dictionary
                 FROM analytical_columns
                 ORDER BY table_name, ordinal"
            ).map_err(|error| AppError::Storage(format!("Failed to prepare analytical columns: {error}")))?;
            let columns = columns_stmt.query_map([], |row| {
                Ok(AnalyticalColumnSummary {
                    table_name: row.get(0)?,
                    column_name: row.get(1)?,
                    logical_type: row.get(2)?,
                    original_name: row.get(3)?,
                    nullable: row.get::<_, i64>(4)? != 0,
                    dictionary: row.get(5)?,
                })
            }).map_err(|error| AppError::Storage(format!("Failed to query analytical columns: {error}")))?
              .collect::<Result<Vec<_>, _>>()
              .map_err(|error| AppError::Storage(format!("Failed to decode analytical column: {error}")))?;
            let mut templates = sql_templates();
            for dataset in &datasets {
                let dataset_columns = columns.iter().filter(|column| column.table_name == dataset.table_name).collect::<Vec<_>>();
                if dataset_columns.iter().any(|c| c.column_name == "report_fy") && !dataset_columns.iter().any(|c| c.column_name == "budget_cycle") {
                    templates.push(SqlTemplate {
                        template_id: format!("{}-fiscal-year-coverage", dataset.snapshot_id),
                        title: format!("Fiscal-year coverage: {}", dataset.display_name),
                        description: "Inspect the fiscal-year codes actually represented in this snapshot. A recent publication date does not establish recent observations.".into(),
                        sql: format!("SELECT report_fy, COUNT(*) AS imported_rows FROM {} GROUP BY 1 ORDER BY 1", quote_ident(&dataset.table_name)),
                        denominator: "Rows in the selected snapshot, grouped by its own fiscal-year code.".into(),
                    });
                }
                if ["report_fy", "budget_cycle", "amount"].iter().all(|name| dataset_columns.iter().any(|c| c.column_name == *name))
                    && dataset_columns.iter().any(|c| c.column_name == "amount" && matches!(c.logical_type.as_str(), "NUMBER" | "INTEGER" | "DECIMAL")) {
                    templates.push(SqlTemplate {
                        template_id: format!("{}-budget-coverage", dataset.snapshot_id),
                        title: format!("Budget year and cycle coverage: {}", dataset.display_name),
                        description: "Inspect fiscal years, reported budget cycles, and missing amounts before selecting appropriations. Blank cycles remain unspecified.".into(),
                        sql: format!("SELECT report_fy, COALESCE(NULLIF(LOWER(TRIM(budget_cycle)), ''), 'UNSPECIFIED') AS reported_cycle, COUNT(*) AS allocation_rows, COUNT(*) - COUNT(amount) AS missing_amounts FROM {} GROUP BY 1, 2 ORDER BY 1, 2", quote_ident(&dataset.table_name)),
                        denominator: "Imported budget rows by the publisher's fiscal-year and cycle fields. Missing status is not assumed to be adopted.".into(),
                    });
                }
                let profile_sql = columns.iter().filter(|column| column.table_name == dataset.table_name).take(64).map(|column| {
                    let name = quote_ident(&column.column_name);
                    let label = column.column_name.replace('\'', "''");
                    format!("SELECT '{label}' AS column_name, COUNT(*) AS rows, COUNT(*) - COUNT({name}) AS missing, COUNT(DISTINCT {name}) AS distinct_values, CAST(MIN({name}) AS VARCHAR) AS minimum, CAST(MAX({name}) AS VARCHAR) AS maximum FROM {}", quote_ident(&dataset.table_name))
                }).collect::<Vec<_>>().join(" UNION ALL ");
                templates.push(SqlTemplate {
                    template_id: format!("{}-profile", dataset.snapshot_id),
                    title: format!("Profile {}", dataset.display_name),
                    description: "Missing values, distinct values, and minimum/maximum for the first 64 columns; text bounds are lexical.".into(),
                    sql: profile_sql,
                    denominator: "Imported rows in this snapshot; rejected rows and partial coverage remain in the snapshot ledger.".into(),
                });
                templates.push(SqlTemplate {
                    template_id: format!("{}-count", dataset.snapshot_id),
                    title: format!("Count {}", dataset.display_name),
                    description: "Count imported rows without assuming that rows represent unique people, events, or properties.".into(),
                    sql: format!("SELECT COUNT(*) AS imported_rows FROM {}", quote_ident(&dataset.table_name)),
                    denominator: "One row of the selected snapshot.".into(),
                });
                if dataset.rejected_rows > 0 {
                    templates.push(SqlTemplate {
                        template_id: format!("{}-rejected", dataset.snapshot_id),
                        title: format!("Review rejected rows: {}", dataset.display_name),
                        description: "Inspect original field positions and validation errors for rows excluded from SQL counts.".into(),
                        sql: format!("SELECT row_number, source_payload_json, validation_errors_json FROM analytical_rows WHERE snapshot_id = '{}' AND malformed = 1 ORDER BY row_number", dataset.snapshot_id.replace('\'', "''")),
                        denominator: "Rejected rows in the selected snapshot.".into(),
                    });
                }
            }
            Ok(AnalyticalSchema { datasets, columns, templates })
        }).await
    }

    pub async fn execute_restricted_sql(
        &self,
        sql: &str,
        limit: usize,
    ) -> Result<SqlQueryResult, AppError> {
        if !(1..=10_000).contains(&limit) {
            return Err(AppError::DataParsing(
                "SQL row limit must be between 1 and 10000".to_string(),
            ));
        }
        let sql = sql.trim().trim_end_matches(';').to_string();
        let started = std::time::Instant::now();
        self.query_connection("Restricted analytical SQL", move |conn| {
            let _deadline = crate::analysis::sql::QueryDeadline::start(
                conn,
                std::time::Duration::from_secs(30),
            )?;
            let tables = {
                let mut statement = conn
                    .prepare("SELECT table_name FROM information_schema.tables")
                    .map_err(|e| AppError::Storage(e.to_string()))?;
                let names = statement
                    .query_map([], |row| row.get::<_, String>(0))
                    .map_err(|e| AppError::Storage(e.to_string()))?;
                names
                    .collect::<Result<std::collections::BTreeSet<_>, _>>()
                    .map_err(|e| AppError::Storage(e.to_string()))?
            };
            let referenced = crate::analysis::sql::validate(&sql, &tables)?;
            conn.execute_batch(
                "SET enable_external_access = false; SET enable_progress_bar = false;",
            )
            .map_err(|error| {
                AppError::Storage(format!("Failed to restrict SQL engine: {error}"))
            })?;
            let wrapped = format!(
                "SELECT * FROM ({sql}) AS onrecord_user_query LIMIT {}",
                limit + 1
            );
            let mut statement = conn.prepare(&wrapped).map_err(|error| {
                AppError::DataParsing(format!("SQL could not be prepared: {error}"))
            })?;
            // DuckDB exposes result columns after execution; asking a merely prepared statement
            // for them panics, unlike SQLite. Empty result sets still carry the executed schema.
            let mut rows = statement.query([]).map_err(|error| {
                AppError::DataParsing(format!(
                    "SQL execution failed (analytical work has a 30-second deadline): {error}"
                ))
            })?;
            let column_names = rows
                .as_ref()
                .expect("executed query retains its statement")
                .column_names();
            let column_count = column_names.len();
            let mut output = Vec::new();
            while let Some(row) = rows
                .next()
                .map_err(|error| AppError::Storage(format!("Failed to read SQL row: {error}")))?
            {
                let values = (0..column_count)
                    .map(|index| duckdb_value_to_json(row, index))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|error| {
                        AppError::Storage(format!("Failed to decode SQL row: {error}"))
                    })?;
                output.push(values);
            }
            let truncated = output.len() > limit;
            output.truncate(limit);
            Ok(SqlQueryResult {
                columns: column_names,
                row_count: output.len(),
                rows: output,
                truncated,
                elapsed_ms: started.elapsed().as_millis(),
                provenance: analytical_query_provenance(conn, &referenced)?,
            })
        })
        .await
    }

    pub async fn export_restricted_sql_csv(
        &self,
        sql: &str,
        output_path: &str,
        limit: usize,
    ) -> Result<SqlExportResult, AppError> {
        let result = self.execute_restricted_sql(sql, limit).await?;
        // A symlink at the target path would redirect this write to a location the caller never
        // actually chose — the real frontend only ever populates this path from a native Save
        // dialog, but the IPC boundary itself enforces nothing, so a compromised or scripted
        // caller could point it at a symlink planted somewhere reachable to overwrite an
        // arbitrary file (e.g. a shell profile or LaunchAgents entry) through an indirection the
        // user never saw. Overwriting an existing *regular* file the user re-selected is a
        // legitimate re-export and is intentionally still allowed. The IPC command already runs
        // this same check before the request reaches here; it is repeated at the point of the
        // actual write, through the one shared implementation, rather than trusted from upstream.
        crate::commands::validation::validate_filesystem_target_path(
            output_path,
            "Export path",
            crate::commands::validation::MAXIMUM_PATH_BYTES,
        )?;
        let mut writer = csv::Writer::from_path(output_path)
            .map_err(|error| AppError::Storage(format!("Failed to create CSV export: {error}")))?;
        writer
            .write_record(&result.columns)
            .map_err(|error| AppError::Storage(format!("Failed to write CSV header: {error}")))?;
        for row in &result.rows {
            writer
                .write_record(row.iter().map(json_cell_to_csv))
                .map_err(|error| AppError::Storage(format!("Failed to write CSV row: {error}")))?;
        }
        writer
            .flush()
            .map_err(|error| AppError::Storage(format!("Failed to flush CSV export: {error}")))?;
        let digest = crate::integrity::sha256_file(std::path::Path::new(output_path))
            .map_err(|error| AppError::Storage(format!("Failed to hash CSV export: {error}")))?;
        Ok(SqlExportResult {
            output_path: output_path.to_string(),
            row_count: result.row_count,
            sha256: digest,
        })
    }
}

pub(super) fn quote_ident(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn unique_column_alias(header: &str, index: usize, headers: &[String]) -> String {
    let mut alias = header
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string();
    if alias.is_empty() || alias.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
        alias = format!("field_{}", index + 1);
    }
    let duplicates_before = headers[..index]
        .iter()
        .filter(|prior| {
            prior
                .chars()
                .map(|ch| {
                    if ch.is_ascii_alphanumeric() {
                        ch.to_ascii_lowercase()
                    } else {
                        '_'
                    }
                })
                .collect::<String>()
                .trim_matches('_')
                == alias
        })
        .count();
    if duplicates_before > 0 {
        format!("{alias}_{}", duplicates_before + 1)
    } else {
        alias
    }
}

pub(crate) const ANALYTICAL_PARSER_VERSION: &str = "regional-tables-v3";

fn logical_type_for(kind: &AnalyticalDatasetKind, header: &str) -> &'static str {
    let header = header.to_ascii_lowercase();
    if matches!(kind, AnalyticalDatasetKind::OperatingBudget) {
        return match header.as_str() {
            "amount" => "NUMBER",
            "report_fy" | "budget_cycle" => "CATEGORY",
            name if name.ends_with("_number") => "IDENTIFIER",
            _ => "CATEGORY",
        };
    }
    if header.contains("date") || matches!(header.as_str(), "occured_on" | "approved_on") {
        return "DATE";
    }
    if header.contains("time") {
        return "TIMESTAMP";
    }
    if header.contains("amount") || header.contains("budget") {
        return "DECIMAL";
    }
    if matches!(
        header.as_str(),
        "uid" | "id" | "pid" | "incident_num" | "case_number" | "nibrs_uniq"
    ) {
        return "IDENTIFIER";
    }
    if matches!(kind, AnalyticalDatasetKind::OperatingBudget) && header.ends_with("_number") {
        return "IDENTIFIER";
    }
    "CATEGORY"
}

fn duckdb_type(logical: &str) -> &'static str {
    match logical {
        "INTEGER" => "BIGINT",
        "NUMBER" => "DECIMAL(38,10)",
        "DATE" => "DATE",
        "TIMESTAMP" => "TIMESTAMP",
        "DECIMAL" => "DECIMAL(18,2)",
        _ => "TEXT",
    }
}

fn normalize_csv_value(raw: &str, logical: &str) -> Result<serde_json::Value, String> {
    if raw.is_empty() {
        return Ok(serde_json::Value::Null);
    }
    match logical {
        "INTEGER" => raw
            .parse::<i64>()
            .map(|value| serde_json::json!(value))
            .map_err(|_| format!("could not parse integer '{raw}'")),
        // Bind decimal strings directly, avoiding a binary floating-point round trip.
        "NUMBER" if crate::storage::tabular::exact_decimal(raw) => {
            Ok(serde_json::Value::String(raw.to_string()))
        }
        "NUMBER" => Err(format!("could not parse exact decimal '{raw}'")),
        "DATE" => normalize_date(raw)
            .map(serde_json::Value::String)
            .ok_or_else(|| format!("could not parse date '{raw}'")),
        "TIMESTAMP" => normalize_timestamp(raw)
            .map(serde_json::Value::String)
            .ok_or_else(|| format!("could not parse timestamp '{raw}'")),
        "DECIMAL" => raw
            .replace(',', "")
            .parse::<f64>()
            .map(serde_json::Number::from_f64)
            .ok()
            .flatten()
            .map(serde_json::Value::Number)
            .ok_or_else(|| format!("could not parse amount '{raw}'")),
        _ => Ok(serde_json::Value::String(raw.to_string())),
    }
}

fn normalize_date(raw: &str) -> Option<String> {
    chrono::NaiveDate::parse_from_str(raw, "%Y-%m-%d")
        .or_else(|_| chrono::NaiveDate::parse_from_str(raw, "%m/%d/%Y"))
        .or_else(|_| chrono::NaiveDate::parse_from_str(raw, "%-m/%-d/%Y"))
        .map(|date| date.format("%Y-%m-%d").to_string())
        .ok()
}

fn normalize_timestamp(raw: &str) -> Option<String> {
    chrono::DateTime::parse_from_rfc3339(raw)
        .map(|date| date.to_rfc3339())
        .ok()
        .or_else(|| {
            chrono::NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S")
                .map(|date| format!("{}Z", date.format("%Y-%m-%dT%H:%M:%S")))
                .ok()
        })
        .or_else(|| normalize_date(raw).map(|date| format!("{date}T00:00:00Z")))
}

fn json_to_sql_value(value: &serde_json::Value) -> duckdb::types::Value {
    match value {
        serde_json::Value::Null => duckdb::types::Value::Null,
        serde_json::Value::Number(number) => number
            .as_i64()
            .map(duckdb::types::Value::BigInt)
            .unwrap_or_else(|| duckdb::types::Value::Double(number.as_f64().unwrap_or(0.0))),
        serde_json::Value::Bool(value) => duckdb::types::Value::Boolean(*value),
        other => duckdb::types::Value::Text(match other {
            serde_json::Value::String(value) => value.clone(),
            _ => other.to_string(),
        }),
    }
}

fn dataset_kind_from_str(value: &str) -> AnalyticalDatasetKind {
    match value {
        "REGIONAL_DATA" => AnalyticalDatasetKind::RegionalData,
        "RIPA_STOPS" => AnalyticalDatasetKind::RipaStops,
        "RIPA_SEARCH_BASIS" => AnalyticalDatasetKind::RipaSearchBasis,
        "CALLS_FOR_SERVICE" => AnalyticalDatasetKind::CallsForService,
        "CALLS_CODEBOOK" => AnalyticalDatasetKind::CallsCodebook,
        "NIBRS_OFFENSES" => AnalyticalDatasetKind::NibrsOffenses,
        "OPERATING_BUDGET" => AnalyticalDatasetKind::OperatingBudget,
        _ => AnalyticalDatasetKind::BudgetDimension,
    }
}

fn analytical_counting_key(
    kind: &AnalyticalDatasetKind,
    headers: &[String],
    record: &csv::StringRecord,
) -> Option<String> {
    let key_fields: &[&str] = match kind {
        AnalyticalDatasetKind::RipaStops => &["UID", "ID", "PID"],
        AnalyticalDatasetKind::CallsForService => &["INCIDENT_NUM"],
        AnalyticalDatasetKind::NibrsOffenses => &["nibrs_uniq"],
        AnalyticalDatasetKind::OperatingBudget => &[
            "report_fy",
            "budget_cycle",
            "funds_center_number",
            "account_number",
        ],
        _ => return None,
    };
    let mut parts = Vec::new();
    for key in key_fields {
        let index = headers
            .iter()
            .position(|header| header.eq_ignore_ascii_case(key))?;
        parts.push(record.get(index).unwrap_or("").to_string());
    }
    Some(parts.join("\u{1f}"))
}

fn validation_notes_for(kind: &AnalyticalDatasetKind) -> Vec<&'static str> {
    match kind {
        AnalyticalDatasetKind::RipaStops => vec![
            "RIPA UID/ID values can repeat across current and historical collections.",
            "PID identifies a person within one stop, not a persistent person across encounters.",
        ],
        AnalyticalDatasetKind::RipaSearchBasis => vec![
            "Companion tables can multiply rows; aggregate to the intended counting unit before joining.",
        ],
        AnalyticalDatasetKind::CallsForService => vec![
            "Public calls-for-service data excludes publisher-documented sensitive categories.",
            "Calls are not crimes, cases, offenses, or stopped-person records.",
        ],
        AnalyticalDatasetKind::NibrsOffenses => vec![
            "Rows are offenses; distinct cases require an explicit case-number grouping rule.",
        ],
        AnalyticalDatasetKind::OperatingBudget => vec![
            "Budget fiscal year and budget cycle must both be retained.",
            "Budget allocations are not actual expenditures.",
        ],
        _ => vec!["Preserve source dictionary and table family before joining."],
    }
}

fn schema_warnings_for(headers: &[String], kind: &AnalyticalDatasetKind) -> Vec<String> {
    let required: &[&str] = match kind {
        AnalyticalDatasetKind::RipaStops => &["UID", "ID", "PID", "STOPDATE"],
        AnalyticalDatasetKind::CallsForService => &["INCIDENT_NUM", "DATE_TIME", "CALL_TYPE"],
        AnalyticalDatasetKind::NibrsOffenses => &["nibrs_uniq", "case_number", "ibr_offense"],
        AnalyticalDatasetKind::OperatingBudget => &["amount", "report_fy", "budget_cycle"],
        _ => &[],
    };
    required
        .iter()
        .filter(|required| {
            !headers
                .iter()
                .any(|header| header.eq_ignore_ascii_case(required))
        })
        .map(|required| format!("Expected column '{required}' was not present"))
        .collect()
}

fn sql_templates() -> Vec<SqlTemplate> {
    vec![
        SqlTemplate {
            template_id: "ripa-stop-search-rate".to_string(),
            title: "RIPA stops and searches".to_string(),
            description: "Counts stopped-person records and records with a search flag when that column exists.".to_string(),
            denominator: "Distinct stopped-person records within the selected RIPA snapshot.".to_string(),
            sql: "SELECT stopdate, COUNT(*) AS stopped_person_rows, COUNT(*) FILTER (WHERE lower(coalesce(searchbasis, '')) <> '') AS rows_with_search_basis FROM analysis_ripa_stops_latest GROUP BY stopdate ORDER BY stopdate".to_string(),
        },
        SqlTemplate {
            template_id: "calls-by-category-period".to_string(),
            title: "Calls by category and period".to_string(),
            description: "Counts public calls for service by call type and month.".to_string(),
            denominator: "Published public CFS rows; excluded categories remain outside the denominator.".to_string(),
            sql: "SELECT date_trunc('month', date_time) AS month, call_type, COUNT(*) AS calls FROM analysis_calls_for_service_latest GROUP BY 1, 2 ORDER BY 1, 2".to_string(),
        },
        SqlTemplate {
            template_id: "nibrs-offenses-vs-cases".to_string(),
            title: "NIBRS offenses versus cases".to_string(),
            description: "Compares offense rows to distinct case numbers.".to_string(),
            denominator: "Offense rows and distinct case_number values are different units.".to_string(),
            sql: "SELECT year, COUNT(*) AS offense_rows, COUNT(DISTINCT case_number) AS distinct_cases FROM analysis_nibrs_offenses_latest GROUP BY year ORDER BY year".to_string(),
        },
        SqlTemplate {
            template_id: "budget-cycle-comparison".to_string(),
            title: "Budget comparison by cycle".to_string(),
            description: "Compare adopted allocations by fiscal year, department, fund, and revenue/expense account class.".to_string(),
            denominator: "Budget allocation rows for the selected fiscal year and budget cycle, not actual expenditures.".to_string(),
            sql: "SELECT report_fy, LOWER(TRIM(budget_cycle)) AS budget_cycle, dept_name, fund_number, CASE WHEN account_number LIKE '4%' THEN 'Revenue' WHEN account_number LIKE '5%' THEN 'Expense' ELSE 'Other' END AS account_class, SUM(amount) AS allocated_amount FROM analysis_operating_budget_latest WHERE LOWER(TRIM(budget_cycle)) = 'adopted' GROUP BY 1, 2, 3, 4, 5 ORDER BY report_fy, allocated_amount DESC".to_string(),
        },
    ]
}

fn duckdb_value_to_json(row: &duckdb::Row<'_>, index: usize) -> duckdb::Result<serde_json::Value> {
    use duckdb::types::ValueRef;
    Ok(match row.get_ref(index)? {
        ValueRef::Null => serde_json::Value::Null,
        ValueRef::Boolean(value) => serde_json::Value::Bool(value),
        ValueRef::TinyInt(value) => serde_json::json!(value),
        ValueRef::SmallInt(value) => serde_json::json!(value),
        ValueRef::Int(value) => serde_json::json!(value),
        ValueRef::BigInt(value)
            if !(-9_007_199_254_740_991..=9_007_199_254_740_991).contains(&value) =>
        {
            serde_json::json!(value.to_string())
        }
        ValueRef::BigInt(value) => serde_json::json!(value),
        ValueRef::HugeInt(value) => serde_json::json!(value.to_string()),
        ValueRef::UHugeInt(value) => serde_json::json!(value.to_string()),
        ValueRef::UTinyInt(value) => serde_json::json!(value),
        ValueRef::USmallInt(value) => serde_json::json!(value),
        ValueRef::UInt(value) => serde_json::json!(value),
        ValueRef::UBigInt(value) if value > 9_007_199_254_740_991 => {
            serde_json::json!(value.to_string())
        }
        ValueRef::UBigInt(value) => serde_json::json!(value),
        ValueRef::Date32(days) => serde_json::json!(chrono::NaiveDate::from_ymd_opt(1970, 1, 1)
            .and_then(|epoch| epoch.checked_add_signed(chrono::Duration::days(i64::from(days))))
            .map(|date| date.to_string())),
        ValueRef::Float(value) => serde_json::json!(value),
        ValueRef::Double(value) => serde_json::json!(value),
        ValueRef::Decimal(value) => serde_json::json!(value.to_string()),
        ValueRef::Text(value) => {
            serde_json::Value::String(String::from_utf8_lossy(value).to_string())
        }
        ValueRef::Blob(value) | ValueRef::Geometry(value) => {
            serde_json::Value::String(format!("[binary:{} bytes]", value.len()))
        }
        other => serde_json::Value::String(format!("{other:?}")),
    })
}

fn approved_analysis_tables(conn: &Connection) -> Result<Vec<String>, AppError> {
    let mut statement = conn.prepare("SELECT table_name || '@' || snapshot_id FROM analytical_snapshots ORDER BY retrieved_at DESC")
        .map_err(|error| AppError::Storage(format!("Failed to prepare provenance table list: {error}")))?;
    statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| {
            AppError::Storage(format!("Failed to query provenance table list: {error}"))
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            AppError::Storage(format!("Failed to read provenance table list: {error}"))
        })
}

pub(crate) fn analytical_query_provenance(
    conn: &Connection,
    referenced: &std::collections::BTreeSet<String>,
) -> Result<Vec<String>, AppError> {
    let mut dependencies = referenced.clone();
    let mut statement = conn
        .prepare("SELECT view_name, sql FROM duckdb_views() WHERE schema_name = 'main'")
        .map_err(|e| AppError::Storage(e.to_string()))?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .map_err(|e| AppError::Storage(e.to_string()))?;
    for row in rows {
        let (view, sql) = row.map_err(|e| AppError::Storage(e.to_string()))?;
        if referenced.contains(&view) {
            // Application-created latest views contain a direct SELECT of one immutable table.
            // Resolve that actual target rather than attributing every historical snapshot.
            if let Some(sql) = sql {
                dependencies.extend(
                    sql.split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                        .filter(|word| word.starts_with("analysis_"))
                        .map(str::to_string),
                );
            }
        }
    }
    Ok(approved_analysis_tables(conn)?
        .into_iter()
        .filter(|entry| {
            entry
                .split_once('@')
                .is_some_and(|(table, _)| dependencies.contains(table))
        })
        .collect())
}

fn json_cell_to_csv(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => String::new(),
        serde_json::Value::String(value) => sanitize_csv_string_cell(value),
        other => other.to_string(),
    }
}

/// Neutralizes spreadsheet formula/DDE injection: a cell value beginning with `=`, `+`, `-`, or
/// `@` is interpreted as a formula by Excel/Sheets/LibreOffice, not as literal text, when the CSV
/// is opened by a human reviewer. This application treats exported cell content as externally
/// supplied evidence (portal-sourced request titles/descriptions, imported analytical data), not
/// as internally generated text, so a crafted record could otherwise execute a formula or DDE
/// command in the reviewer's spreadsheet application. A leading apostrophe forces text
/// interpretation in every mainstream spreadsheet application and is invisible in the rendered
/// cell. Restricted to strings only — a number never carries untrusted text, and quoting a
/// negative number's leading `-` would corrupt it into a non-numeric text cell for this
/// application's own financial exports.
fn sanitize_csv_string_cell(value: &str) -> String {
    if value.starts_with(['=', '+', '-', '@']) {
        format!("'{value}")
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod semantic_analysis_tests {
    use super::*;

    #[test]
    fn ripa_key_includes_person_within_stop() {
        let headers = vec!["UID".to_string(), "ID".to_string(), "PID".to_string()];
        let one = csv::StringRecord::from(vec!["1", "STOP-A", "1"]);
        let two = csv::StringRecord::from(vec!["1", "STOP-A", "2"]);
        assert_ne!(
            analytical_counting_key(&AnalyticalDatasetKind::RipaStops, &headers, &one),
            analytical_counting_key(&AnalyticalDatasetKind::RipaStops, &headers, &two)
        );
    }

    #[test]
    fn budget_cycle_template_keeps_cycle_filter_explicit() {
        let template = sql_templates()
            .into_iter()
            .find(|template| template.template_id == "budget-cycle-comparison")
            .unwrap();
        assert!(template.sql.contains("budget_cycle"));
        assert!(template.denominator.contains("budget cycle"));
    }
}
