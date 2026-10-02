//! Legal authority import and versions, and the legal brief evidence read model.

use super::*;
use super::{connection::execute_transaction, timeline::parse_extension_reason};

impl DuckDbRepository {
    pub async fn import_legal_authority(
        &self,
        input: &LegalAuthorityImport,
        parsed: &ParsedLegalAuthority,
    ) -> Result<LegalAuthorityImportResult, AppError> {
        let imported_at = chrono::Utc::now().to_rfc3339();
        let statute_id = input.statute_id.trim().to_string();
        let source_uri = input
            .canonical_source_uri
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let effective_from = input
            .effective_from
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let effective_to = input
            .effective_to
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let mandated_deadline_days = input.mandated_deadline_days.map(i64::from);
        let input = input.clone();
        let parsed = parsed.clone();
        let statute_id_for_summary = statute_id.clone();
        let inserted_new_version = self
            .query_connection("Import legal authority", move |conn| {
                let statute_id = statute_id.as_str();
                execute_transaction(conn, "legal authority import", |tx| {
                    let inserted_new_version = tx
                        .query_row(
                            "SELECT NOT EXISTS(
                    SELECT 1 FROM legal_statute_versions
                    WHERE statute_id = ?1 AND content_sha256 = ?2
                 )",
                            params![statute_id, parsed.content_sha256],
                            |row| row.get::<_, bool>(0),
                        )
                        .map_err(|error| {
                            AppError::Storage(format!(
                                "Failed to inspect legal authority version: {error}"
                            ))
                        })?;

                    tx.execute(
                        "INSERT INTO legal_statutes (
                statute_id, jurisdiction, statute_citation, title, authority_kind, full_text,
                mandated_deadline_days, canonical_source_uri, current_version_id,
                content_sha256, effective_from, effective_to, imported_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?13)
             ON CONFLICT(statute_id) DO UPDATE SET
                jurisdiction = excluded.jurisdiction,
                statute_citation = excluded.statute_citation,
                title = excluded.title,
                authority_kind = excluded.authority_kind,
                full_text = excluded.full_text,
                mandated_deadline_days = excluded.mandated_deadline_days,
                canonical_source_uri = excluded.canonical_source_uri,
                current_version_id = excluded.current_version_id,
                content_sha256 = excluded.content_sha256,
                effective_from = excluded.effective_from,
                effective_to = excluded.effective_to,
                updated_at = excluded.updated_at",
                        params![
                            statute_id,
                            input.jurisdiction.trim(),
                            input.statute_citation.trim(),
                            input.title.trim(),
                            input.authority_kind.as_str(),
                            parsed.normalized_text,
                            mandated_deadline_days,
                            source_uri,
                            parsed.version_id,
                            parsed.content_sha256,
                            effective_from,
                            effective_to,
                            imported_at,
                        ],
                    )
                    .map_err(|error| {
                        AppError::Storage(format!("Failed to persist legal authority: {error}"))
                    })?;

                    tx.execute(
                        "INSERT INTO legal_statute_versions (
                version_id, statute_id, full_text, content_sha256, effective_from,
                effective_to, source_uri, imported_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(version_id) DO UPDATE SET
                effective_from = excluded.effective_from,
                effective_to = excluded.effective_to,
                source_uri = excluded.source_uri",
                        params![
                            parsed.version_id,
                            statute_id,
                            parsed.normalized_text,
                            parsed.content_sha256,
                            effective_from,
                            effective_to,
                            source_uri,
                            imported_at,
                        ],
                    )
                    .map_err(|error| {
                        AppError::Storage(format!(
                            "Failed to persist legal authority version: {error}"
                        ))
                    })?;

                    tx.execute(
                        "DELETE FROM legal_provisions_fts WHERE version_id = ?1",
                        params![parsed.version_id],
                    )
                    .map_err(|error| {
                        AppError::Storage(format!("Failed to clear legal provision index: {error}"))
                    })?;
                    tx.execute(
                        "DELETE FROM legal_provisions WHERE version_id = ?1",
                        params![parsed.version_id],
                    )
                    .map_err(|error| {
                        AppError::Storage(format!("Failed to clear legal provisions: {error}"))
                    })?;
                    for provision in &parsed.provisions {
                        let ordinal = i64::try_from(provision.ordinal).map_err(|_| {
                            AppError::Storage(
                                "Legal provision ordinal exceeds SQLite capacity".to_string(),
                            )
                        })?;
                        let heading_level =
                            i64::try_from(provision.heading_level).map_err(|_| {
                                AppError::Storage(
                                    "Legal heading level exceeds SQLite capacity".to_string(),
                                )
                            })?;
                        let token_count = i64::try_from(provision.token_count).map_err(|_| {
                            AppError::Storage(
                                "Legal provision token count exceeds SQLite capacity".to_string(),
                            )
                        })?;
                        tx.execute(
                            "INSERT INTO legal_provisions (
                    provision_id, version_id, statute_id, ordinal, heading_level,
                    heading_path, title, provision_text, token_count, content_sha256
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                            params![
                                provision.provision_id,
                                parsed.version_id,
                                statute_id,
                                ordinal,
                                heading_level,
                                provision.heading_path,
                                provision.title,
                                provision.provision_text,
                                token_count,
                                provision.content_sha256,
                            ],
                        )
                        .map_err(|error| {
                            AppError::Storage(format!("Failed to persist legal provision: {error}"))
                        })?;
                        tx.execute(
                            "INSERT INTO legal_provisions_fts (
                    provision_id, version_id, statute_id, title, heading_path, provision_text
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                            params![
                                provision.provision_id,
                                parsed.version_id,
                                statute_id,
                                provision.title,
                                provision.heading_path,
                                provision.provision_text,
                            ],
                        )
                        .map_err(|error| {
                            AppError::Storage(format!("Failed to index legal provision: {error}"))
                        })?;
                    }

                    tx.execute(
                        "UPDATE legal_mandates SET enabled = 0 WHERE statute_id = ?1",
                        params![statute_id],
                    )
                    .map_err(|error| {
                        AppError::Storage(format!(
                            "Failed to retire superseded legal mandates: {error}"
                        ))
                    })?;
                    let version_suffix = parsed
                        .version_id
                        .rsplit('-')
                        .next()
                        .unwrap_or(parsed.version_id.as_str());
                    for mandate in &input.mandates {
                        let mandate_id = format!(
                            "{}:{}:{}",
                            statute_id,
                            mandate.mandate_id.trim(),
                            &version_suffix[..version_suffix.len().min(12)]
                        );
                        let applicability_json = serde_json::to_string(
                            &mandate.applicability_phrases,
                        )
                        .map_err(|error| {
                            AppError::DataParsing(format!(
                                "Failed to encode mandate applicability phrases: {error}"
                            ))
                        })?;
                        let evidence_json = serde_json::to_string(
                            &mandate.required_evidence_phrases,
                        )
                        .map_err(|error| {
                            AppError::DataParsing(format!(
                                "Failed to encode mandate evidence phrases: {error}"
                            ))
                        })?;
                        tx.execute(
                            "INSERT INTO legal_mandates (
                    mandate_id, statute_id, version_id, title, description,
                    applicability_phrases_json, required_evidence_phrases_json,
                    severity, enabled, created_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1, ?9)
                 ON CONFLICT(mandate_id) DO UPDATE SET
                    title = excluded.title,
                    description = excluded.description,
                    applicability_phrases_json = excluded.applicability_phrases_json,
                    required_evidence_phrases_json = excluded.required_evidence_phrases_json,
                    severity = excluded.severity,
                    enabled = 1",
                            params![
                                mandate_id,
                                statute_id,
                                parsed.version_id,
                                mandate.title.trim(),
                                mandate.description.trim(),
                                applicability_json,
                                evidence_json,
                                mandate.severity.as_str(),
                                imported_at,
                            ],
                        )
                        .map_err(|error| {
                            AppError::Storage(format!("Failed to persist legal mandate: {error}"))
                        })?;
                    }
                    Ok(inserted_new_version)
                })
            })
            .await?;
        let authority = self
            .load_legal_authority_summary(&statute_id_for_summary)
            .await?;
        Ok(LegalAuthorityImportResult {
            authority,
            inserted_new_version,
        })
    }

    pub async fn list_legal_authorities(&self) -> Result<Vec<LegalAuthoritySummary>, AppError> {
        self.query_connection("List legal authorities", |conn| {
            let query = format!(
                "{LEGAL_AUTHORITY_SUMMARY_SELECT}
             ORDER BY legal_statutes.jurisdiction COLLATE NOCASE,
                      legal_statutes.statute_citation COLLATE NOCASE"
            );
            let mut statement = conn.prepare(&query).map_err(|error| {
                AppError::Storage(format!(
                    "Failed to prepare legal authority listing: {error}"
                ))
            })?;
            let rows = statement
                .query_map([], map_legal_authority_summary_row)
                .map_err(|error| {
                    AppError::Storage(format!("Failed to query legal authorities: {error}"))
                })?;
            rows.map(|row| {
                row.map_err(|error| {
                    AppError::Storage(format!("Failed to decode legal authority: {error}"))
                })
                .and_then(convert_legal_authority_summary)
            })
            .collect()
        })
        .await
    }

    pub async fn list_legal_authority_versions(
        &self,
        statute_id: &str,
    ) -> Result<Vec<LegalAuthorityVersionSummary>, AppError> {
        let statute_id = statute_id.to_string();
        self.query_connection("List legal authority versions", move |conn| {
            let mut statement = conn
                .prepare(
                    "SELECT version_id, statute_id, content_sha256, effective_from, effective_to,
                        source_uri, imported_at
                 FROM legal_statute_versions
                 WHERE statute_id = ?1
                 ORDER BY imported_at DESC, version_id DESC
                 LIMIT 500",
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to prepare legal version listing: {error}"))
                })?;
            let rows = statement
                .query_map(params![statute_id], map_legal_authority_version_summary)
                .map_err(|error| {
                    AppError::Storage(format!("Failed to query legal versions: {error}"))
                })?;
            rows.map(|row| {
                row.map_err(|error| {
                    AppError::Storage(format!("Failed to decode legal version: {error}"))
                })
            })
            .collect()
        })
        .await
    }

    pub async fn get_legal_authority_version(
        &self,
        statute_id: &str,
        version_id: &str,
    ) -> Result<LegalAuthorityVersionDetail, AppError> {
        let statute_id = statute_id.to_string();
        let version_id = version_id.to_string();
        self.query_connection("Get legal authority version", move |conn| {
            conn.query_row(
                "SELECT version_id, statute_id, content_sha256, effective_from, effective_to,
                    source_uri, imported_at, full_text
             FROM legal_statute_versions
             WHERE statute_id = ?1 AND version_id = ?2",
                params![statute_id, version_id],
                |row| {
                    Ok((
                        map_legal_authority_version_summary(row)?,
                        row.get::<_, String>(7)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| {
                AppError::Storage(format!("Failed to read legal authority version: {error}"))
            })?
            .map(|(version, full_text)| LegalAuthorityVersionDetail { version, full_text })
            .ok_or_else(|| {
                AppError::DataParsing(format!(
                    "Legal authority version '{version_id}' was not found for '{statute_id}'"
                ))
            })
        })
        .await
    }

    async fn load_legal_authority_summary(
        &self,
        statute_id: &str,
    ) -> Result<LegalAuthoritySummary, AppError> {
        let statute_id = statute_id.to_string();
        self.query_connection("Load legal authority summary", move |conn| {
            let query = format!(
                "{LEGAL_AUTHORITY_SUMMARY_SELECT}
             WHERE legal_statutes.statute_id = ?1"
            );
            let row = conn
                .query_row(&query, params![statute_id], map_legal_authority_summary_row)
                .map_err(|error| match error {
                    duckdb::Error::QueryReturnedNoRows => {
                        AppError::Storage(format!("Legal authority '{statute_id}' was not found"))
                    }
                    other => AppError::Storage(format!("Failed to load legal authority: {other}")),
                })?;
            convert_legal_authority_summary(row)
        })
        .await
    }

    pub async fn load_legal_brief_evidence(
        &self,
        request_id: u64,
        portal_key: &str,
    ) -> Result<LegalBriefEvidence, AppError> {
        let portal_key = portal_key.to_string();
        self.query_connection("Load legal brief evidence", move |conn| {
            let row = conn
                .query_row(
                    "SELECT
                    requests.id, requests.portal_subdomain, portals.display_name,
                    portals.base_url, COALESCE(agencies.name, 'Unmapped Department'),
                    requests.rpid, COALESCE(requests.title, ''),
                    COALESCE(requests.description, ''), requests.status,
                    requests.created_at, requests.updated_at, requests.closed_at,
                    requests.determination_at, requests.determination_source,
                    statutory_extensions.notice_at,
                    statutory_extensions.expected_determination_at,
                    statutory_extensions.reason, statutory_extensions.notice_source
                 FROM requests
                 INNER JOIN portals ON portals.subdomain = requests.portal_subdomain
                 LEFT JOIN agencies
                    ON agencies.id = requests.agency_id
                   AND agencies.portal_subdomain = requests.portal_subdomain
                LEFT JOIN statutory_extensions
                    ON statutory_extensions.request_id = requests.id
                   AND statutory_extensions.portal_subdomain = requests.portal_subdomain
                 WHERE requests.id = ?1 AND requests.portal_subdomain = ?2",
                    params![request_id, portal_key],
                    map_legal_brief_row,
                )
                .map_err(|error| match error {
                    duckdb::Error::QueryReturnedNoRows => AppError::Storage(format!(
                        "Request {portal_key}/{request_id} was not found"
                    )),
                    other => AppError::Storage(format!("Failed to load legal evidence: {other}")),
                })?;
            let extension = match (
                row.extension_notice_at,
                row.extension_expected_determination_at,
                row.extension_reason,
                row.extension_notice_source,
            ) {
                (
                    Some(notice_at),
                    Some(expected_determination_at),
                    Some(reason),
                    Some(notice_source),
                ) => Some(StatutoryExtensionRecord {
                    notice_at,
                    expected_determination_at,
                    reason: parse_extension_reason(&reason)?,
                    notice_source,
                }),
                (None, None, None, None) => None,
                _ => {
                    return Err(AppError::Storage(format!(
                    "Request {portal_key}/{request_id} has incomplete statutory extension evidence"
                )))
                }
            };
            let mut flags_statement = conn
                .prepare(
                    "SELECT DISTINCT document_integrity_flags.flag_type
                 FROM case_audits
                 INNER JOIN document_integrity_flags
                    ON document_integrity_flags.audit_id = case_audits.audit_id
                 WHERE case_audits.target_request_id = ?1
                   AND case_audits.target_subdomain = ?2
                 ORDER BY document_integrity_flags.flag_type",
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to prepare evidence flags: {error}"))
                })?;
            let integrity_flags = flags_statement
                .query_map(params![request_id, portal_key], |row| {
                    row.get::<_, String>(0)
                })
                .map_err(|error| {
                    AppError::Storage(format!("Failed to query evidence flags: {error}"))
                })?
                .map(|value| value.map_err(AppError::from))
                .collect::<Result<Vec<_>, _>>()?;
            let mut documents_statement = conn
                .prepare(
                    "SELECT id, COALESCE(file_name, ''), file_size, visibility,
                    raw_file_sha256, extracted_text_sha256, shannon_entropy,
                    entropy_sample_bytes, suspicious_data_density
                 FROM documents
                 WHERE request_id = ?1 AND portal_subdomain = ?2
                 ORDER BY id",
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to prepare evidence documents: {error}"))
                })?;
            let documents = documents_statement
                .query_map(params![request_id, portal_key], map_evidence_document_row)
                .map_err(|error| {
                    AppError::Storage(format!("Failed to query evidence documents: {error}"))
                })?
                .map(|value| value.map_err(AppError::from))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(LegalBriefEvidence {
                request_id: row.request_id,
                portal_key: row.portal_key,
                portal_name: row.portal_name,
                portal_url: row.portal_url,
                agency_name: row.agency_name,
                rpid: row.rpid,
                title: row.title,
                description: row.description,
                status: row.status,
                created_at: row.created_at,
                updated_at: row.updated_at,
                closed_at: row.closed_at,
                determination_at: row.determination_at,
                determination_source: row.determination_source,
                extension,
                integrity_flags,
                documents,
            })
        })
        .await
    }

    pub async fn record_report_export(
        &self,
        result: &LegalBriefExportResult,
    ) -> Result<(), AppError> {
        let markers =
            serde_json::to_string(&result.included_violation_markers).map_err(|error| {
                AppError::Export(format!("Failed to encode report markers: {error}"))
            })?;
        let result = result.clone();
        self.query_connection("Record report export", move |conn| {
            conn.execute(
                "INSERT INTO report_exports (
                export_id, request_id, portal_subdomain, output_path, report_sha256,
                package_path, package_sha256, signature_algorithm, signer_key_id,
                generated_at, violation_markers_json
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    uuid::Uuid::new_v4().to_string(),
                    result.request_id,
                    result.portal_key,
                    result.output_path,
                    result.report_sha256,
                    result.package_path,
                    result.package_sha256,
                    result.signature_algorithm,
                    result.signer_key_id,
                    result.generated_at,
                    markers,
                ],
            )
            .map_err(|error| {
                AppError::Export(format!("Failed to record report provenance: {error}"))
            })?;
            Ok(())
        })
        .await
    }
}

const LEGAL_AUTHORITY_SUMMARY_SELECT: &str = "SELECT
    legal_statutes.statute_id, legal_statutes.jurisdiction,
    legal_statutes.statute_citation, legal_statutes.title,
    legal_statutes.authority_kind, legal_statutes.current_version_id,
    legal_statutes.content_sha256, legal_statutes.mandated_deadline_days,
    legal_statutes.canonical_source_uri, legal_statutes.effective_from,
    legal_statutes.effective_to,
    (SELECT COUNT(*) FROM legal_provisions
     WHERE legal_provisions.version_id = legal_statutes.current_version_id),
    (SELECT COUNT(*) FROM legal_mandates
     WHERE legal_mandates.version_id = legal_statutes.current_version_id
       AND legal_mandates.enabled = 1),
    legal_statutes.imported_at
 FROM legal_statutes";

fn map_legal_brief_row(row: &Row<'_>) -> duckdb::Result<LegalBriefRow> {
    Ok(LegalBriefRow {
        request_id: row.get(0)?,
        portal_key: row.get(1)?,
        portal_name: row.get(2)?,
        portal_url: row.get(3)?,
        agency_name: row.get(4)?,
        rpid: row.get(5)?,
        title: row.get(6)?,
        description: row.get(7)?,
        status: row.get(8)?,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
        closed_at: row.get(11)?,
        determination_at: row.get(12)?,
        determination_source: row.get(13)?,
        extension_notice_at: row.get(14)?,
        extension_expected_determination_at: row.get(15)?,
        extension_reason: row.get(16)?,
        extension_notice_source: row.get(17)?,
    })
}

fn map_legal_authority_summary_row(row: &Row<'_>) -> duckdb::Result<RawLegalAuthoritySummary> {
    Ok(RawLegalAuthoritySummary {
        statute_id: row.get(0)?,
        jurisdiction: row.get(1)?,
        statute_citation: row.get(2)?,
        title: row.get(3)?,
        authority_kind: row.get(4)?,
        current_version_id: row.get(5)?,
        content_sha256: row.get(6)?,
        mandated_deadline_days: row.get(7)?,
        canonical_source_uri: row.get(8)?,
        effective_from: row.get(9)?,
        effective_to: row.get(10)?,
        provision_count: row.get(11)?,
        mandate_count: row.get(12)?,
        imported_at: row.get(13)?,
    })
}

fn map_legal_authority_version_summary(
    row: &Row<'_>,
) -> duckdb::Result<LegalAuthorityVersionSummary> {
    Ok(LegalAuthorityVersionSummary {
        version_id: row.get(0)?,
        statute_id: row.get(1)?,
        content_sha256: row.get(2)?,
        effective_from: row.get(3)?,
        effective_to: row.get(4)?,
        source_uri: row.get(5)?,
        imported_at: row.get(6)?,
    })
}

fn convert_legal_authority_summary(
    row: RawLegalAuthoritySummary,
) -> Result<LegalAuthoritySummary, AppError> {
    let authority_kind = match row.authority_kind.as_str() {
        "STATE_STATUTE" => LegalAuthorityKind::StateStatute,
        "MUNICIPAL_ORDINANCE" => LegalAuthorityKind::MunicipalOrdinance,
        "AGENCY_POLICY" => LegalAuthorityKind::AgencyPolicy,
        "PROCEDURE_MANUAL" => LegalAuthorityKind::ProcedureManual,
        "REGULATION" => LegalAuthorityKind::Regulation,
        "OTHER" => LegalAuthorityKind::Other,
        value => {
            return Err(AppError::Storage(format!(
                "Legal authority '{}' has unsupported kind '{value}'",
                row.statute_id
            )))
        }
    };
    let mandated_deadline_days = row
        .mandated_deadline_days
        .map(|value| {
            u32::try_from(value).map_err(|_| {
                AppError::Storage(format!(
                    "Legal authority '{}' has an invalid deadline",
                    row.statute_id
                ))
            })
        })
        .transpose()?;
    let provision_count = usize::try_from(row.provision_count).map_err(|_| {
        AppError::Storage(format!(
            "Legal authority '{}' has an invalid provision count",
            row.statute_id
        ))
    })?;
    let mandate_count = usize::try_from(row.mandate_count).map_err(|_| {
        AppError::Storage(format!(
            "Legal authority '{}' has an invalid mandate count",
            row.statute_id
        ))
    })?;
    Ok(LegalAuthoritySummary {
        statute_id: row.statute_id,
        jurisdiction: row.jurisdiction,
        statute_citation: row.statute_citation,
        title: row.title,
        authority_kind,
        current_version_id: row.current_version_id,
        content_sha256: row.content_sha256,
        mandated_deadline_days,
        canonical_source_uri: row.canonical_source_uri,
        effective_from: row.effective_from,
        effective_to: row.effective_to,
        provision_count,
        mandate_count,
        imported_at: row.imported_at,
    })
}

fn map_evidence_document_row(row: &Row<'_>) -> duckdb::Result<EvidenceDocument> {
    Ok(EvidenceDocument {
        document_id: row.get(0)?,
        file_name: row.get(1)?,
        file_size: row.get(2)?,
        visibility: row.get(3)?,
        raw_file_sha256: row.get(4)?,
        extracted_text_sha256: row.get(5)?,
        shannon_entropy: row.get(6)?,
        entropy_sample_bytes: row.get(7)?,
        suspicious_data_density: row.get(8)?,
    })
}
