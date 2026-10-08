//! Document and request records, acquisition history, and extraction finalization.

use super::*;
use super::{
    connection::execute_transaction,
    integrity::{
        compute_extracted_text_sha256, derive_integrity_flags, has_heavy_redaction_density,
        normalize_sha256, replace_managed_integrity_flags, request_has_heavily_redacted_document,
        request_has_suspicious_density, synchronize_density_audit, update_automatic_audit_status,
        upsert_automatic_case_audit, validate_entropy_metadata, validate_extracted_text_sha256,
    },
    policy::synchronize_policy_evidence_gaps,
    search::synchronize_fts_row,
    timeline::{
        map_request_analytical_state, synchronize_cpra_finding, synchronize_timeline_audit,
        upsert_statutory_extension,
    },
};

/// The values `request_keyword_signals.keyword`'s CHECK admits in `0001_initial_schema.sql`.
const STORED_KEYWORD_SIGNALS: [&str; 3] = ["sweep", "encampment", "eviction"];
impl DuckDbRepository {
    pub async fn insert_or_update_document(
        &self,
        subdomain: &str,
        request_id: u64,
        document: &DocumentMetadata,
    ) -> Result<(), AppError> {
        let subdomain = subdomain.to_string();
        let document = document.clone();
        self.query_connection("Insert or update document", move |conn| {
            conn.execute(
                "INSERT INTO documents (
                id, request_id, portal_subdomain, file_name, file_size, description,
                content_type, download_url, visibility, created_at, updated_at, local_path,
                extracted_text_sha256, was_ocr_processed
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
             ON CONFLICT(id, portal_subdomain) DO UPDATE SET
                request_id = excluded.request_id,
                file_name = excluded.file_name,
                file_size = excluded.file_size,
                description = excluded.description,
                content_type = excluded.content_type,
                download_url = excluded.download_url,
                visibility = excluded.visibility,
                updated_at = excluded.updated_at,
                local_path = COALESCE(excluded.local_path, documents.local_path)",
                params![
                    document.id,
                    request_id,
                    subdomain,
                    document.file_name,
                    document.file_size,
                    document.description,
                    document.content_type,
                    document.download_url,
                    document.visibility,
                    document.created_at,
                    document.updated_at,
                    document.local_path,
                    document.extracted_text_sha256,
                    if document.was_ocr_processed {
                        1_i64
                    } else {
                        0_i64
                    },
                ],
            )
            .map_err(|error| {
                AppError::Internal(format!("Failed to persist document metadata: {error}"))
            })?;
            Ok(())
        })
        .await
    }

    pub async fn mark_document_downloaded(
        &self,
        finalization: DocumentDownloadFinalization<'_>,
    ) -> Result<(), AppError> {
        let DocumentDownloadFinalization {
            document_id,
            portal_subdomain: subdomain,
            raw_file_size,
            raw_file_sha256,
            local_path,
            shannon_entropy,
            entropy_sample_bytes,
            suspicious_data_density,
        } = finalization;
        let document_id = i64::try_from(document_id).map_err(|error| {
            AppError::DataParsing(format!(
                "Document identifier cannot be represented by the core database: {error}"
            ))
        })?;
        let file_size = i64::try_from(raw_file_size).map_err(|error| {
            AppError::DataParsing(format!(
                "Document size {raw_file_size} cannot be persisted: {error}"
            ))
        })?;
        let local_path = local_path
            .to_str()
            .ok_or_else(|| AppError::DataParsing("Document path must be valid UTF-8".to_string()))?
            .to_string();
        let raw_file_sha256 = normalize_sha256(raw_file_sha256)?;
        let (entropy_sample_bytes, suspicious_data_density) = validate_entropy_metadata(
            shannon_entropy,
            entropy_sample_bytes,
            raw_file_size,
            suspicious_data_density,
        )?;
        let subdomain = subdomain.to_string();
        self.query_connection("Mark document downloaded", move |conn| {
        execute_transaction(conn, "document download finalization", |tx| {
            let updated = tx
                .execute(
                    "UPDATE documents SET file_size = ?1, local_path = ?2, raw_file_sha256 = ?3,
                    shannon_entropy = ?4, entropy_sample_bytes = ?5,
                    suspicious_data_density = ?6, download_status = ?7,
                    extraction_status = 'DOWNLOADED', extraction_format = NULL,
                    extraction_deferred_reason = NULL,
                    retention_eligible_at = CASE WHEN (local_path IS NULL OR raw_file_sha256 IS DISTINCT FROM ?3)
                        AND EXISTS (SELECT 1 FROM data_maintenance_policy WHERE singleton
                            AND json_extract_string(policy_json, '$.retentionMode') = 'REFERENCE_ONLY'
                            AND json_extract_string(policy_json, '$.enabled') = 'true')
                        THEN strftime(now(), '%Y-%m-%dT%H:%M:%S.%fZ') ELSE NULL END
                 WHERE id = ?8 AND portal_subdomain = ?9",
                    params![
                        file_size,
                        local_path,
                        raw_file_sha256,
                        shannon_entropy,
                        entropy_sample_bytes,
                        suspicious_data_density,
                        DOWNLOAD_STATUS_COMPLETED,
                        document_id,
                        subdomain
                    ],
                )
                .map_err(|error| {
                    AppError::Internal(format!("Failed to mark document downloaded: {error}"))
                })?;
            if updated != 1 {
                return Err(AppError::Internal(format!(
                    "Document download update affected {updated} rows for {subdomain}/{document_id}"
                )));
            }
            synchronize_density_audit(tx, document_id, &subdomain)?;
            Ok(())
        })
        })
        .await
    }

    /// Appends one fetch event. The digest the document held before this fetch is captured in the
    /// same statement, so the history reads as a chain even when content changes between fetches.
    pub async fn record_document_acquisition(
        &self,
        portal_subdomain: &str,
        document_id: u64,
        sync_run_id: Option<&str>,
        raw_file_sha256: &str,
        file_size: u64,
        acquisition: &crate::ingestion::client::AcquisitionRecord,
    ) -> Result<(), AppError> {
        let document_id = i64::try_from(document_id).map_err(|error| {
            AppError::DataParsing(format!("Document identifier cannot be stored: {error}"))
        })?;
        let file_size = i64::try_from(file_size).map_err(|error| {
            AppError::DataParsing(format!("Document size cannot be stored: {error}"))
        })?;
        let raw_file_sha256 = normalize_sha256(raw_file_sha256)?;
        let portal_subdomain = portal_subdomain.to_string();
        let sync_run_id = sync_run_id.map(str::to_string);
        let acquisition = acquisition.clone();
        self.query_connection("Record document acquisition", move |conn| {
            insert_document_acquisition(
                conn,
                &portal_subdomain,
                document_id,
                sync_run_id.as_deref(),
                &raw_file_sha256,
                file_size,
                &acquisition,
            )
        })
        .await
    }

    pub async fn mark_document_extraction_state(
        &self,
        document_id: u64,
        portal_subdomain: &str,
        status: &str,
        format: Option<&str>,
        deferred_reason: Option<&str>,
    ) -> Result<(), AppError> {
        if !matches!(
            status,
            "SEARCHABLE" | "SALVAGED" | "EXPANDED" | "DEFERRED" | "DOWNLOADED" | "FAILED"
        ) {
            return Err(AppError::DataParsing(format!(
                "Unsupported document extraction status '{status}'"
            )));
        }
        if portal_subdomain.trim().is_empty()
            || format.is_some_and(|value| value.trim().is_empty() || value.len() > 128)
            || deferred_reason.is_some_and(|value| value.trim().is_empty() || value.len() > 2_048)
            || (status == "DEFERRED" && deferred_reason.is_none())
            || (status != "DEFERRED" && deferred_reason.is_some())
        {
            return Err(AppError::DataParsing(
                "Document extraction state is invalid".to_string(),
            ));
        }
        let document_id = i64::try_from(document_id).map_err(|_| {
            AppError::DataParsing("Document identifier exceeds DuckDB capacity".to_string())
        })?;
        let status = status.to_string();
        let portal_subdomain = portal_subdomain.to_string();
        let format = format.map(str::to_string);
        let deferred_reason = deferred_reason.map(str::to_string);
        self.query_connection("Mark document extraction state", move |conn| {
            let updated = conn
                .execute(
                    "UPDATE documents
                 SET extraction_status = ?1, extraction_format = ?2,
                     extraction_deferred_reason = ?3
                 WHERE id = ?4 AND portal_subdomain = ?5",
                    params![
                        status,
                        format,
                        deferred_reason,
                        document_id,
                        portal_subdomain
                    ],
                )
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to persist document extraction state: {error}"
                    ))
                })?;
            if updated != 1 {
                return Err(AppError::Storage(format!(
                "Document extraction state update affected {updated} rows for {portal_subdomain}/{document_id}"
            )));
            }
            Ok(())
        })
        .await
    }

    /// Commits a PublicRequest object and its sub-relations inside an isolated ACID transaction block.
    pub async fn insert_or_update_request(
        &self,
        subdomain: &str,
        request: &PublicRequest,
    ) -> Result<(), AppError> {
        let subdomain = subdomain.to_string();
        let request = request.clone();
        self.query_connection("Insert or update request", move |conn| {
            execute_transaction(conn, "request persistence", |tx| {
                if let Some(ref agency) = request.agency {
                    tx.execute(
                        "INSERT INTO agencies (id, portal_subdomain, name, department_name)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(id, portal_subdomain) DO UPDATE SET
                    name = excluded.name,
                    department_name = excluded.department_name",
                        params![agency.id, subdomain, agency.name, agency.department_name],
                    )
                    .map_err(|e| {
                        AppError::Internal(format!("Agency mapping write block failed: {}", e))
                    })?;
                }

                if let Some(ref category) = request.category {
                    tx.execute(
                        "INSERT INTO categories (id, portal_subdomain, name)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(id, portal_subdomain) DO UPDATE SET name = excluded.name",
                        params![category.id, subdomain, category.name],
                    )
                    .map_err(|e| {
                        AppError::Internal(format!("Category mapping write block failed: {}", e))
                    })?;
                }

                let request_updated = tx
                    .execute(
                        "UPDATE requests SET
                rpid = ?3, title = ?4, description = ?5, status = ?6,
                current_status_id = ?7, updated_at = ?8, closed_at = ?9,
                due_date = ?10, neighborhood_block = COALESCE(?11, neighborhood_block),
                agency_id = ?12, category_id = ?13,
                determination_at = ?14, determination_source = ?15
             WHERE id = ?1 AND portal_subdomain = ?2",
                        params![
                            request.id,
                            subdomain,
                            request.rpid,
                            request.title,
                            request.description,
                            request.status,
                            request.current_status_id,
                            request.updated_at,
                            request.closed_at,
                            request.due_date,
                            request.neighborhood_block,
                            request.agency.as_ref().map(|a| a.id),
                            request.category.as_ref().map(|c| c.id),
                            request.determination_at,
                            request.determination_source,
                        ],
                    )
                    .map_err(|e| {
                        AppError::Internal(format!("Main case envelope update failure: {}", e))
                    })?;
                if request_updated == 0 {
                    tx.execute(
                        "INSERT INTO requests (
                id, portal_subdomain, rpid, title, description, status, current_status_id,
                created_at, updated_at, closed_at, due_date, neighborhood_block,
                agency_id, category_id, determination_at, determination_source
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
             ",
                        params![
                            request.id,
                            subdomain,
                            request.rpid,
                            request.title,
                            request.description,
                            request.status,
                            request.current_status_id,
                            request.created_at,
                            request.updated_at,
                            request.closed_at,
                            request.due_date,
                            request.neighborhood_block,
                            request.agency.as_ref().map(|a| a.id),
                            request.category.as_ref().map(|c| c.id),
                            request.determination_at,
                            request.determination_source,
                        ],
                    )
                    .map_err(|e| {
                        AppError::Internal(format!(
                            "Main case envelope transaction write failure: {}",
                            e
                        ))
                    })?;
                }

                let mut request_text = String::with_capacity(
                    request.title.as_deref().map(str::len).unwrap_or(0)
                        + request.description.as_deref().map(str::len).unwrap_or(0)
                        + 1,
                );
                if let Some(title) = &request.title {
                    request_text.push_str(title);
                }
                request_text.push(' ');
                if let Some(description) = &request.description {
                    request_text.push_str(description);
                }
                let keyword_counts = extract_enforcement_keyword_counts(&request_text);
                tx.execute(
                    "DELETE FROM request_keyword_signals
             WHERE request_id = ?1 AND portal_subdomain = ?2",
                    params![request.id, subdomain],
                )
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to refresh request keyword signals: {error}"
                    ))
                })?;
                // `request_keyword_signals.keyword` carries a CHECK that DuckDB cannot widen on an
                // existing database, and a keyword outside it aborted the whole request's insert.
                // Only the signals the column admits are stored; the request itself always is.
                for (keyword, match_count) in keyword_counts
                    .into_iter()
                    .filter(|(keyword, _)| STORED_KEYWORD_SIGNALS.contains(&keyword.as_str()))
                {
                    let match_count = i64::try_from(match_count).map_err(|_| {
                        AppError::Storage(
                            "Request keyword count exceeds SQLite capacity".to_string(),
                        )
                    })?;
                    tx.execute(
                        "INSERT INTO request_keyword_signals (
                    request_id, portal_subdomain, keyword, match_count
                 ) VALUES (?1, ?2, ?3, ?4)",
                        params![request.id, subdomain, keyword, match_count],
                    )
                    .map_err(|error| {
                        AppError::Storage(format!(
                            "Failed to persist request keyword signal: {error}"
                        ))
                    })?;
                }

                if let Some(extension) = &request.statutory_extension {
                    upsert_statutory_extension(tx, request.id, &subdomain, extension)?;
                }

                for doc in &request.documents {
                    tx.execute(
                        "INSERT INTO documents (
                    id, request_id, portal_subdomain, file_name, file_size, description,
                    content_type, download_url, visibility, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT(id, portal_subdomain) DO UPDATE SET
                    file_name = excluded.file_name,
                    file_size = CASE
                        WHEN documents.download_status = ?12 THEN documents.file_size
                        ELSE excluded.file_size
                    END,
                    updated_at = excluded.updated_at",
                        params![
                            doc.id,
                            request.id,
                            subdomain,
                            doc.file_name,
                            doc.file_size,
                            doc.description,
                            doc.content_type,
                            doc.download_url,
                            doc.visibility,
                            doc.created_at,
                            doc.updated_at,
                            DOWNLOAD_STATUS_COMPLETED,
                        ],
                    )
                    .map_err(|e| {
                        AppError::Internal(format!("Document attachment matrix sync error: {}", e))
                    })?;
                }

                let audit_id = format!("auto-{}-{}", subdomain, request.id);
                let mut timeline_flags = Vec::with_capacity(DERIVED_TIMELINE_FLAG_TYPES.len());
                for anomaly_type in &request.timeline_anomalies {
                    if let Some(known_flag) = DERIVED_TIMELINE_FLAG_TYPES
                        .iter()
                        .copied()
                        .find(|known_flag| anomaly_type == known_flag)
                    {
                        if !timeline_flags.contains(&known_flag) {
                            timeline_flags.push(known_flag);
                        }
                    }
                }
                synchronize_timeline_audit(tx, &audit_id, request.id, &subdomain, &timeline_flags)?;
                let statutory_analysis =
                    crate::analysis::evaluate_statutory_compliance_with_determination_at(
                        request.id,
                        &request.created_at,
                        Some(&request.updated_at),
                        request.closed_at.as_deref(),
                        request.determination_at.as_deref(),
                        request.determination_source.as_deref(),
                        request.statutory_extension.as_ref(),
                        &chrono::Utc::now().to_rfc3339(),
                    )?;
                synchronize_cpra_finding(tx, request.id, &subdomain, &statutory_analysis)?;
                Ok(())
            })
        })
        .await
    }

    /// One keyset page of the documents the embedding index is built from, ordered by
    /// `(portal_key, document_id)` and starting after `after`. Paged because each row carries the
    /// document's full extracted text, so reading the corpus at once holds all of it in memory.
    pub async fn list_vector_source_documents_page(
        &self,
        after: Option<(String, u64)>,
        limit: usize,
    ) -> Result<Vec<VectorSourceDocument>, AppError> {
        let (after_portal, after_document_id) = match after {
            Some((portal_key, document_id)) => (Some(portal_key), document_id),
            None => (None, 0),
        };
        let after_document_id = i64::try_from(after_document_id).map_err(|_| {
            AppError::DataParsing("Vector source cursor exceeds supported capacity".to_string())
        })?;
        let limit = i64::try_from(limit.max(1)).unwrap_or(i64::MAX);
        self.query_connection("Vector source document listing", move |conn| {
            let mut statement = conn.prepare(
                "SELECT
                    documents.portal_subdomain, documents.request_id, documents.id,
                    COALESCE(documents.file_name, 'Untitled document'),
                    COALESCE(requests.title, ''),
                    COALESCE(agencies.name, 'Unmapped Department'),
                    COALESCE(portals.base_url, ''),
                    documents.content_type,
                    CASE WHEN portals.source_kind = 'NEXT_REQUEST' THEN documents.created_at ELSE NULL END,
                    NULL,
                    NULL,
                    COALESCE(documents.raw_file_sha256, ''),
                    COALESCE(documents.extracted_text_sha256, ''),
                    documents_fts.extracted_text
                 FROM documents
                 INNER JOIN documents_fts
                   ON documents_fts.document_id = documents.id
                  AND documents_fts.portal_subdomain = documents.portal_subdomain
                 INNER JOIN requests
                   ON requests.id = documents.request_id
                  AND requests.portal_subdomain = documents.portal_subdomain
                 LEFT JOIN agencies
                   ON agencies.id = requests.agency_id
                  AND agencies.portal_subdomain = requests.portal_subdomain
                 LEFT JOIN portals
                   ON portals.subdomain = requests.portal_subdomain
                 WHERE LOWER(TRIM(documents.visibility)) = 'public'
                   AND COALESCE(documents.extracted_text_sha256, '') <> ''
                   AND LENGTH(documents_fts.extracted_text) > 0
                   AND (?1 IS NULL OR documents.portal_subdomain > ?1
                        OR (documents.portal_subdomain = ?1 AND documents.id > ?2))
                 ORDER BY documents.portal_subdomain, documents.id
                 LIMIT ?3"
            ).map_err(|error| AppError::Storage(format!("Failed to prepare vector source list: {error}")))?;
            let rows = statement.query_map(params![after_portal, after_document_id, limit], |row| {
                Ok(VectorSourceDocument {
                    portal_key: row.get(0)?,
                    request_id: row.get(1)?,
                    document_id: row.get(2)?,
                    file_name: row.get(3)?,
                    title: row.get(4)?,
                    agency_name: row.get(5)?,
                    jurisdiction: crate::ingestion::regional::source_jurisdiction(&row.get::<_, String>(6)?).to_string(),
                    document_type: row.get(7)?,
                    publication_date: row.get(8)?,
                    observation_date: row.get(9)?,
                    effective_date: row.get(10)?,
                    raw_file_sha256: row.get(11)?,
                    extracted_text_sha256: row.get(12)?,
                    text: row.get(13)?,
                })
            }).map_err(|error| AppError::Storage(format!("Failed to query vector sources: {error}")))?;
            rows.map(|row| row.map_err(|error| AppError::Storage(format!("Failed to read vector source: {error}")))).collect()
        }).await
    }

    pub async fn count_vector_source_documents(&self) -> Result<usize, AppError> {
        self.query_connection("Vector source count", move |conn| {
            conn.query_row(
                "SELECT COUNT(*)
                 FROM documents
                 INNER JOIN documents_fts
                   ON documents_fts.document_id = documents.id
                  AND documents_fts.portal_subdomain = documents.portal_subdomain
                 WHERE LOWER(TRIM(documents.visibility)) = 'public'
                   AND COALESCE(documents.extracted_text_sha256, '') <> ''
                   AND LENGTH(documents_fts.extracted_text) > 0",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map(|value| value.max(0) as usize)
            .map_err(|error| {
                AppError::Storage(format!("Failed to count vector source documents: {error}"))
            })
        })
        .await
    }

    pub async fn list_request_documents_for_view(
        &self,
        request_id: u64,
        portal_key: &str,
    ) -> Result<Vec<RequestDocumentSummary>, AppError> {
        let portal_key = portal_key.to_string();
        let rows = self
            .query_connection("Request document listing", move |conn| {
                let mut statement = conn
                    .prepare(
                        "SELECT
                    id, portal_subdomain, COALESCE(file_name, 'Untitled document'),
                    file_size, content_type, visibility, COALESCE(download_status, 'pending'),
                    raw_file_sha256, extracted_text_sha256,
                    COALESCE(extraction_status, 'PENDING'), extraction_format,
                    extraction_deferred_reason, download_url
                 FROM documents
                 WHERE request_id = ?1
                   AND portal_subdomain = ?2
                   AND LOWER(TRIM(visibility)) = 'public'
                 ORDER BY created_at, id",
                    )
                    .map_err(|error| {
                        AppError::Storage(format!(
                            "Failed to prepare request document list: {error}"
                        ))
                    })?;
                let rows = statement
                    .query_map(params![request_id, portal_key], map_request_document_row)
                    .map_err(|error| {
                        AppError::Storage(format!("Failed to query request documents: {error}"))
                    })?;
                rows.map(|row| {
                    row.map_err(|error| {
                        AppError::Storage(format!("Failed to decode request document: {error}"))
                    })
                })
                .collect::<Result<Vec<_>, _>>()
            })
            .await?;
        let mut documents = Vec::with_capacity(rows.len());
        for row in rows {
            let file_size = row
                .file_size
                .map(u64::try_from)
                .transpose()
                .map_err(|_| AppError::Storage("Document size was negative".to_string()))?;
            documents.push(RequestDocumentSummary {
                document_id: row.document_id,
                portal_key: row.portal_key,
                file_name: crate::storage::pii::mask_pii(&row.file_name)?.masked_text,
                file_size,
                content_type: row.content_type,
                visibility: row.visibility,
                download_status: row.download_status,
                raw_file_sha256: row.raw_file_sha256,
                extracted_text_sha256: row.extracted_text_sha256,
                extraction_status: row.extraction_status,
                extraction_format: row.extraction_format,
                extraction_deferred_reason: row.extraction_deferred_reason,
                original_source_uri: row.original_source_uri.filter(|raw| {
                    reqwest::Url::parse(raw).is_ok_and(|url| {
                        matches!(url.scheme(), "https" | "http")
                            && url.username().is_empty()
                            && url.password().is_none()
                            && url.query().is_none()
                    })
                }),
            });
        }
        Ok(documents)
    }

    pub async fn resolve_public_document_path(
        &self,
        document_id: u64,
        portal_key: &str,
    ) -> Result<PathBuf, AppError> {
        let portal_key = portal_key.to_string();
        let error_portal_key = portal_key.clone();
        let local_path = self.query_connection("Public document path lookup", move |conn| {
            conn.query_row(
                "SELECT local_path
                 FROM documents
                 WHERE id = ?1
                   AND portal_subdomain = ?2
                   AND LOWER(TRIM(visibility)) = 'public'",
                params![document_id, portal_key],
                |row| row.get::<_, Option<String>>(0),
            )
            .map_err(|error| match error {
                duckdb::Error::QueryReturnedNoRows => AppError::Storage(format!(
                    "Public document {error_portal_key}/{document_id} was not found"
                )),
                other => AppError::Storage(format!(
                    "Failed to resolve local evidence path for {error_portal_key}/{document_id}: {other}"
                )),
            })?
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                AppError::Storage(format!(
                    "Public document {error_portal_key}/{document_id} is not available in the local evidence store"
                ))
            })
        }).await?;
        Ok(PathBuf::from(local_path))
    }

    pub async fn load_document_text_chunk(
        &self,
        document_id: u64,
        portal_key: &str,
        character_offset: usize,
        character_limit: usize,
    ) -> Result<DocumentTextChunk, AppError> {
        if !(1..=MAXIMUM_DOCUMENT_CHUNK_CHARACTERS).contains(&character_limit) {
            return Err(AppError::DataParsing(format!(
                "Document chunk size must be between 1 and {MAXIMUM_DOCUMENT_CHUNK_CHARACTERS} characters"
            )));
        }
        let requested_character_offset = character_offset;
        let requested_character_limit = character_limit;
        // Inspect context outside the requested window so a PII token spanning either boundary is
        // detected before the response is trimmed back to the caller's range.
        let query_character_offset =
            requested_character_offset.saturating_sub(DOCUMENT_CHUNK_PII_BOUNDARY_CHARACTERS);
        let query_character_limit = character_limit
            .checked_add(DOCUMENT_CHUNK_PII_BOUNDARY_CHARACTERS * 2)
            .ok_or_else(|| {
                AppError::DataParsing("Document chunk boundary overflowed".to_string())
            })?;
        let character_offset = i64::try_from(query_character_offset).map_err(|_| {
            AppError::DataParsing("Document text offset exceeds SQLite capacity".to_string())
        })?;
        let character_limit = i64::try_from(query_character_limit).map_err(|_| {
            AppError::DataParsing("Document text limit exceeds SQLite capacity".to_string())
        })?;
        let portal_key = portal_key.to_string();
        let query_portal_key = portal_key.clone();
        let (total_character_count, text) = self
            .query_connection("Document text chunk query", move |conn| {
                conn.query_row(
                    "SELECT
                    COALESCE((
                        SELECT LENGTH(documents_fts.extracted_text)
                        FROM documents_fts
                        WHERE documents_fts.document_id = documents.id
                          AND documents_fts.portal_subdomain = documents.portal_subdomain
                        ORDER BY documents_fts.document_id
                        LIMIT 1
                    ), 0),
                    COALESCE((
                        SELECT SUBSTR(documents_fts.extracted_text, ?3 + 1, ?4)
                        FROM documents_fts
                        WHERE documents_fts.document_id = documents.id
                          AND documents_fts.portal_subdomain = documents.portal_subdomain
                        ORDER BY documents_fts.document_id
                        LIMIT 1
                    ), '')
                 FROM documents
                 WHERE documents.id = ?1
                   AND documents.portal_subdomain = ?2
                   AND LOWER(TRIM(documents.visibility)) = 'public'",
                    params![
                        document_id,
                        query_portal_key,
                        character_offset,
                        character_limit
                    ],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
                )
                .map_err(|error| match error {
                    duckdb::Error::QueryReturnedNoRows => AppError::Storage(format!(
                        "Public document {query_portal_key}/{document_id} was not found"
                    )),
                    other => AppError::Storage(format!("Failed to load document text: {other}")),
                })
            })
            .await?;
        let total_character_count = usize::try_from(total_character_count).map_err(|_| {
            AppError::Storage("Document text length was negative or too large".to_string())
        })?;
        if requested_character_offset > total_character_count {
            return Err(AppError::DataParsing(format!(
                "Document text offset {requested_character_offset} exceeds length {total_character_count}"
            )));
        }
        let returned_character_count = requested_character_limit
            .min(total_character_count.saturating_sub(requested_character_offset));
        let next_offset = requested_character_offset
            .checked_add(returned_character_count)
            .ok_or_else(|| AppError::Storage("Document text position overflowed".to_string()))?;
        let visible_character_start = requested_character_offset - query_character_offset;
        let masked = crate::storage::pii::mask_pii_character_range(
            &text,
            visible_character_start,
            returned_character_count,
        )?;
        Ok(DocumentTextChunk {
            document_id,
            portal_key,
            character_offset: requested_character_offset,
            returned_character_count,
            next_character_offset: (next_offset < total_character_count).then_some(next_offset),
            total_character_count,
            masked_finding_count: masked.findings.len(),
            masked_text: masked.masked_text,
        })
    }

    /// Atomically finalizes document metadata, derived audit flags, and the search-text projection.
    pub async fn finalize_document_extraction(
        &self,
        finalization: DocumentExtractionFinalization<'_>,
    ) -> Result<(), AppError> {
        let DocumentExtractionFinalization {
            document_id: doc_id,
            portal_subdomain: subdomain,
            extracted_text: text_content,
            extracted_text_sha256: sha256_hash,
            raw_file_sha256,
            was_ocr_processed: was_ocr,
            raw_file_size,
            local_path,
            shannon_entropy,
            entropy_sample_bytes,
            suspicious_data_density,
        } = finalization;
        let document_id = i64::try_from(doc_id).map_err(|_| {
            AppError::DataParsing(format!(
                "Document identifier {} exceeds SQLite INTEGER capacity",
                doc_id
            ))
        })?;
        let stored_file_size = i64::try_from(raw_file_size).map_err(|_| {
            AppError::DataParsing(format!(
                "Raw file size {} exceeds SQLite INTEGER capacity",
                raw_file_size
            ))
        })?;
        let stored_path = local_path
            .to_str()
            .filter(|path| !path.trim().is_empty())
            .ok_or_else(|| {
                AppError::DataParsing(
                    "Document extraction disk path must be non-empty, valid UTF-8".to_string(),
                )
            })?;
        let normalized_sha256 = validate_extracted_text_sha256(text_content, sha256_hash)?;
        let normalized_raw_file_sha256 = normalize_sha256(raw_file_sha256)?;
        let cleaned_text = clean_plain_text_for_index(text_content);
        // `extracted_text_sha256` covers `text_content`, which is discarded after this call: only
        // the normalized projection is kept. That digest is therefore checkable at write time and
        // never again, while the text search returns and legal briefs quote had no digest at all.
        // Recording one over the projection makes the derived evidence verifiable later.
        let projection_sha256 = compute_extracted_text_sha256(&cleaned_text);
        let (entropy_sample_bytes, suspicious_data_density) = validate_entropy_metadata(
            shannon_entropy,
            entropy_sample_bytes,
            raw_file_size,
            suspicious_data_density,
        )?;

        if subdomain.trim().is_empty() {
            return Err(AppError::DataParsing(
                "Document portal subdomain must not be empty".to_string(),
            ));
        }

        // The heavy-redaction density check is a pure function of the extracted text; computing it
        // before the connection lock keeps `text_content` from having to cross into the closure
        // (which would require an owned copy of the entire extracted body). If the check errors,
        // no state has been written yet, matching the original transaction's roll-back behavior.
        let text_content_has_heavy_redaction = has_heavy_redaction_density(text_content)?;

        let subdomain = subdomain.to_string();
        let stored_path = stored_path.to_string();
        self.query_connection("Document extraction finalization", move |conn| {
            execute_transaction(conn, "document extraction finalization", |tx| {
                let analytical_state = tx
                    .query_row(
                        "SELECT
                    requests.id,
                    requests.created_at,
                    requests.updated_at,
                    requests.closed_at,
                    requests.status,
                    documents.file_name
                 FROM documents
                 INNER JOIN requests
                    ON requests.id = documents.request_id
                   AND requests.portal_subdomain = documents.portal_subdomain
                 WHERE documents.id = ?1 AND documents.portal_subdomain = ?2",
                        params![document_id, subdomain],
                        map_request_analytical_state,
                    )
                    .map_err(|error| match error {
                        duckdb::Error::QueryReturnedNoRows => AppError::Internal(format!(
                            "Document {} for portal '{}' does not exist or has no associated request",
                            doc_id, subdomain
                        )),
                        other => AppError::Internal(format!(
                            "Failed to load document analytical context: {}",
                            other
                        )),
                    })?;

                let request_has_heavy_redaction = text_content_has_heavy_redaction
                    || request_has_heavily_redacted_document(
                        tx,
                        analytical_state.request_id,
                        &subdomain,
                        Some(document_id),
                    )?;
                let audit_id = format!("auto-{}-{}", subdomain, analytical_state.request_id);

                let updated_documents = tx
                    .execute(
                        "UPDATE documents SET
                    file_size = ?1,
                    local_path = ?2,
                    extracted_text_sha256 = ?3,
                    raw_file_sha256 = ?4,
                    was_ocr_processed = ?5,
                    shannon_entropy = ?6,
                    entropy_sample_bytes = ?7,
                    suspicious_data_density = ?8,
                    download_status = ?9,
                    extracted_text_projection_sha256 = ?12,
                    extraction_status = 'SEARCHABLE', extraction_format = NULL,
                    extraction_deferred_reason = NULL,
                    retention_eligible_at = CASE WHEN (raw_file_sha256 IS DISTINCT FROM ?4 OR extraction_deferred_reason LIKE 'REFERENCE_ONLY:%')
                        AND EXISTS (SELECT 1 FROM data_maintenance_policy WHERE singleton
                            AND json_extract_string(policy_json, '$.retentionMode') = 'REFERENCE_ONLY'
                            AND json_extract_string(policy_json, '$.enabled') = 'true')
                        THEN strftime(now(), '%Y-%m-%dT%H:%M:%S.%fZ') ELSE retention_eligible_at END
                 WHERE id = ?10 AND portal_subdomain = ?11",
                        params![
                            stored_file_size,
                            stored_path,
                            normalized_sha256,
                            normalized_raw_file_sha256,
                            if was_ocr { 1_i64 } else { 0_i64 },
                            shannon_entropy,
                            entropy_sample_bytes,
                            suspicious_data_density,
                            DOWNLOAD_STATUS_COMPLETED,
                            document_id,
                            subdomain,
                            projection_sha256,
                        ],
                    )
                    .map_err(|e| {
                        AppError::Internal(format!("Failed to update document metadata record: {}", e))
                    })?;

                if updated_documents != 1 {
                    return Err(AppError::Internal(format!(
                        "Document extraction update affected {} rows for document {} in portal '{}'",
                        updated_documents, doc_id, subdomain
                    )));
                }

                let request_has_suspicious_data_density =
                    request_has_suspicious_density(tx, analytical_state.request_id, &subdomain)?;
                let derived_flags = derive_integrity_flags(
                    &analytical_state,
                    request_has_heavy_redaction,
                    request_has_suspicious_data_density,
                );
                let verification_status = if derived_flags.is_empty() {
                    VERIFICATION_STATUS_COMPLIANT
                } else {
                    VERIFICATION_STATUS_ANOMALY
                };

                upsert_automatic_case_audit(
                    tx,
                    &audit_id,
                    analytical_state.request_id,
                    &subdomain,
                    "Automatically derived during document extraction finalization",
                    verification_status,
                    true,
                )
                .map_err(|error| {
                    AppError::Internal(format!(
                        "Failed to synchronize automatic case audit state: {error}"
                    ))
                })?;

                replace_managed_integrity_flags(
                    tx,
                    &audit_id,
                    &DERIVED_DOCUMENT_FLAG_TYPES,
                    &derived_flags,
                )?;

                synchronize_fts_row(
                    tx,
                    document_id,
                    &subdomain,
                    analytical_state.file_name.as_deref(),
                    &cleaned_text,
                )?;
                let request_id = u64::try_from(analytical_state.request_id).map_err(|_| {
                    AppError::Storage("Document request identifier was negative".to_string())
                })?;
                synchronize_policy_evidence_gaps(tx, request_id, &subdomain)?;
                update_automatic_audit_status(tx, &audit_id)?;
                Ok(())
            })
        })
        .await
    }

    /// The owning agency's name and department, joined into one searchable label, for the
    /// request a document belongs to. `None` covers both "the request has no agency on file" and
    /// "the request row itself is not found" -- either way there is no organization signal to
    /// evaluate against, and callers must treat that as the predicate contributing nothing rather
    /// than as a reason to exclude.
    pub async fn agency_label_for_request(
        &self,
        subdomain: &str,
        request_id: u64,
    ) -> Result<Option<String>, AppError> {
        let request_id = i64::try_from(request_id).map_err(|error| {
            AppError::DataParsing(format!(
                "Request identifier cannot be represented by the core database: {error}"
            ))
        })?;
        let subdomain = subdomain.to_string();
        self.query_connection("Request agency label lookup", move |conn| {
            let row = conn
                .query_row(
                    "SELECT agencies.name, agencies.department_name
                     FROM requests
                     LEFT JOIN agencies
                        ON agencies.id = requests.agency_id
                       AND agencies.portal_subdomain = requests.portal_subdomain
                     WHERE requests.id = ?1 AND requests.portal_subdomain = ?2",
                    params![request_id, subdomain],
                    |row| {
                        let name: Option<String> = row.get(0)?;
                        let department: Option<String> = row.get(1)?;
                        Ok((name, department))
                    },
                )
                .optional()
                .map_err(|error| {
                    AppError::Internal(format!("Failed to look up request agency label: {error}"))
                })?;
            Ok(row.and_then(|(name, department)| match (name, department) {
                (Some(name), Some(department)) if !department.trim().is_empty() => {
                    Some(format!("{name} {department}"))
                }
                (Some(name), _) => Some(name),
                (None, Some(department)) => Some(department),
                (None, None) => None,
            }))
        })
        .await
    }
}

fn map_request_document_row(row: &Row<'_>) -> duckdb::Result<RequestDocumentRow> {
    Ok(RequestDocumentRow {
        document_id: row.get(0)?,
        portal_key: row.get(1)?,
        file_name: row.get(2)?,
        file_size: row.get(3)?,
        content_type: row.get(4)?,
        visibility: row.get(5)?,
        download_status: row.get(6)?,
        raw_file_sha256: row.get(7)?,
        extracted_text_sha256: row.get(8)?,
        extraction_status: row.get(9)?,
        extraction_format: row.get(10)?,
        extraction_deferred_reason: row.get(11)?,
        original_source_uri: row.get(12)?,
    })
}

/// The insert behind [`DuckDbRepository::record_document_acquisition`], for callers that write a
/// fetch event inside a larger transaction. `raw_file_sha256` must already be normalized.
pub(super) fn insert_document_acquisition(
    conn: &Connection,
    portal_subdomain: &str,
    document_id: i64,
    sync_run_id: Option<&str>,
    raw_file_sha256: &str,
    file_size: i64,
    acquisition: &crate::ingestion::client::AcquisitionRecord,
) -> Result<(), AppError> {
    let redirect_chain_json = serde_json::to_string(&acquisition.redirect_chain)
        .map_err(|error| AppError::Internal(format!("Redirect chain encoding: {error}")))?;
    conn.execute(
        "INSERT INTO document_acquisitions (
            portal_subdomain, document_id, sync_run_id, requested_url, resolved_url,
            redirect_chain_json, http_status, content_type, etag, last_modified,
            raw_file_sha256, file_size, previous_raw_file_sha256, retrieved_at, response_date
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
            (SELECT raw_file_sha256 FROM documents WHERE id = ?2 AND portal_subdomain = ?1),
            ?13, ?14
         )",
        params![
            portal_subdomain,
            document_id,
            sync_run_id,
            acquisition.requested_url,
            acquisition.resolved_url,
            redirect_chain_json,
            i64::from(acquisition.http_status),
            acquisition.content_type,
            acquisition.etag,
            acquisition.last_modified,
            raw_file_sha256,
            file_size,
            acquisition.retrieved_at,
            acquisition.response_date,
        ],
    )
    .map_err(|error| {
        AppError::Storage(format!("Failed to record document acquisition: {error}"))
    })?;
    Ok(())
}
