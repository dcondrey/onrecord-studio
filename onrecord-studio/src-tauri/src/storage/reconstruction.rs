use crate::{
    analysis::reconstruction::{reconstruct, search_terms, ReconstructionDocument},
    error::AppError,
    storage::sqlite::DuckDbRepository,
    types::*,
};
use duckdb::{params, OptionalExt};
use std::collections::BTreeSet;

fn error(error: impl std::fmt::Display) -> AppError {
    AppError::Storage(error.to_string())
}
fn document(row: &duckdb::Row<'_>) -> duckdb::Result<ReconstructionDocument> {
    Ok(ReconstructionDocument {
        document_id: row.get(0)?,
        portal_key: row.get(1)?,
        title: row.get(2)?,
        source_url: row.get(3)?,
        source_sha256: row.get(4)?,
        extraction_sha256: row.get(5)?,
        text: row.get(6)?,
        character_offset: row.get(7)?,
        total_characters: row.get(8)?,
    })
}

type SourceIdentity = (String, u64, String, String);

fn validate_source_identities(
    conn: &duckdb::Connection,
    identities: &BTreeSet<SourceIdentity>,
) -> Result<(), AppError> {
    for (portal, id, digest, extraction) in identities {
        let valid: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM documents d JOIN portals p ON p.subdomain = d.portal_subdomain WHERE d.id = ?1 AND d.portal_subdomain = ?2 AND lower(trim(d.visibility)) = 'public' AND p.lifecycle_state <> 'REMOVED' AND COALESCE(d.raw_file_sha256, '') = ?3 AND COALESCE(d.extracted_text_projection_sha256, '') = ?4 AND d.extraction_status IN ('SEARCHABLE','SALVAGED') AND EXISTS(SELECT 1 FROM documents_fts f WHERE f.document_id=d.id AND f.portal_subdomain=d.portal_subdomain))",params![id,portal,digest,extraction],|row|row.get(0)).map_err(error)?;
        if !valid {
            return Err(AppError::DataParsing("A reconstruction source changed or was removed during analysis. Run the comparison again.".into()));
        }
    }
    Ok(())
}

impl DuckDbRepository {
    pub async fn reconstruct_document(
        &self,
        document_id: u64,
        portal_key: &str,
        options: ReconstructionOptions,
    ) -> Result<ReconstructionReport, AppError> {
        let portal_key = portal_key.to_string();
        let target = self.query_connection("Read reconstruction target", move |conn| {
            conn.query_row("SELECT d.id, d.portal_subdomain, COALESCE(d.file_name, 'Document'), d.download_url,
                COALESCE(d.raw_file_sha256, ''), COALESCE(d.extracted_text_projection_sha256, ''), substr(f.extracted_text, 1, 1000000), 0, length(f.extracted_text)
                FROM documents d JOIN documents_fts f ON d.id = f.document_id AND d.portal_subdomain = f.portal_subdomain
                JOIN portals p ON p.subdomain = d.portal_subdomain
                WHERE d.id = ?1 AND d.portal_subdomain = ?2 AND lower(trim(d.visibility)) = 'public'
                AND p.lifecycle_state <> 'REMOVED' AND d.extraction_status IN ('SEARCHABLE', 'SALVAGED')", params![document_id, portal_key], document).optional().map_err(error)?
                .ok_or_else(|| AppError::DataParsing("This public document has no retained searchable extraction. Run ingestion/OCR before cross-record analysis.".into()))
        }).await?;
        let terms = search_terms(&target, &options)?;
        let mut related = Vec::new();
        let mut seen = BTreeSet::new();
        let mut windows_limited = false;
        for term in terms {
            if related.len() >= 160 {
                windows_limited = true;
                break;
            }
            let query = term.to_lowercase();
            let matches = self.query_connection("Retrieve reconstruction comparisons", move |conn| {
                let _deadline = crate::analysis::sql::QueryDeadline::start(conn, std::time::Duration::from_secs(15))?;
                let mut statement = conn.prepare("WITH hits AS (
                    SELECT d.id, d.portal_subdomain, COALESCE(d.file_name, 'Document') AS title, d.download_url,
                    COALESCE(d.raw_file_sha256, '') AS digest, COALESCE(d.extracted_text_projection_sha256, '') AS extraction,
                    f.extracted_text, greatest(0, strpos(lower(f.extracted_text), ?1) - 801) AS start_at
                    FROM documents d JOIN documents_fts f ON d.id = f.document_id AND d.portal_subdomain = f.portal_subdomain
                    JOIN portals p ON p.subdomain = d.portal_subdomain
                    WHERE lower(trim(d.visibility)) = 'public' AND p.lifecycle_state <> 'REMOVED'
                    AND d.extraction_status IN ('SEARCHABLE', 'SALVAGED') AND contains(lower(f.extracted_text), ?1)
                ) SELECT id, portal_subdomain, title, download_url, digest, extraction,
                    substr(extracted_text, start_at + 1, 120000), start_at, length(extracted_text)
                    FROM hits ORDER BY portal_subdomain, id LIMIT 16").map_err(error)?;
                let records = statement.query_map(params![query], document).map_err(error)?.collect::<Result<Vec<_>,_>>().map_err(error)?;
                Ok(records)
            }).await?;
            let pattern = regex::RegexBuilder::new(&regex::escape(&term))
                .case_insensitive(true)
                .build()
                .map_err(error)?;
            for source in matches {
                if source.document_id == target.document_id
                    && source.portal_key == target.portal_key
                {
                    continue;
                }
                for hit in pattern.find_iter(&source.text).take(6) {
                    let start = source.text[..hit.start()]
                        .chars()
                        .count()
                        .saturating_sub(800);
                    let text: String = source.text.chars().skip(start).take(2400).collect();
                    let offset = source.character_offset + start;
                    if !seen.insert((source.portal_key.clone(), source.document_id, offset)) {
                        continue;
                    }
                    related.push(ReconstructionDocument {
                        character_offset: offset,
                        text,
                        ..source.clone()
                    });
                    if related.len() >= 160 {
                        windows_limited = true;
                        break;
                    }
                }
                if related.len() >= 160 {
                    break;
                }
            }
        }
        let identities: BTreeSet<_> = std::iter::once(&target)
            .chain(&related)
            .map(|d| {
                (
                    d.portal_key.clone(),
                    d.document_id,
                    d.source_sha256.clone(),
                    d.extraction_sha256.clone(),
                )
            })
            .collect();
        let mut report = crate::blocking::spawn("Cross-record constraints", move || {
            reconstruct(&target, &related, &options)
        })
        .await??;
        // Data may be removed/replaced while retrieval and CPU analysis are in flight.
        self.query_connection("Validate reconstruction source identities", move |conn| {
            validate_source_identities(conn, &identities)
        })
        .await?;
        if windows_limited {
            report.limitations.push("The 160-passage retrieval limit was reached. Additional related records were not compared.".into());
        }
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integrity::sha256_hex;

    const TARGET: &str = "Project: Riverbed restoration\nContract: ****6789\n";
    const PUBLIC_SOURCE: &str = "Project: Riverbed restoration\nContract: 12346789\n";

    async fn setup() -> DuckDbRepository {
        let repository = DuckDbRepository::new(":memory:").unwrap();
        repository
            .run_migrations(include_str!("../../migrations/0001_initial_schema.sql"))
            .await
            .unwrap();
        let conn = repository.analytics_connection().await.unwrap();
        for (portal, lifecycle) in [("city", "ACTIVE"), ("removed-city", "REMOVED")] {
            conn.execute("INSERT INTO portals(subdomain,display_name,base_url,lifecycle_state) VALUES (?1,?1,'https://www.sandiego.gov',?2)",params![portal,lifecycle]).unwrap();
            conn.execute("INSERT INTO requests(id,portal_subdomain,title,status,created_at,updated_at) VALUES (42,?1,'Reconstruction fixture','closed','2026-09-26','2026-09-26')",params![portal]).unwrap();
        }
        repository
    }

    fn insert_document(
        conn: &duckdb::Connection,
        id: u64,
        portal: &str,
        visibility: &str,
        extraction_status: &str,
        text: &str,
    ) {
        let projection = sha256_hex(text.as_bytes());
        conn.execute("INSERT INTO documents(id,request_id,portal_subdomain,file_name,download_url,visibility,created_at,updated_at,local_path,raw_file_sha256,extracted_text_sha256,extracted_text_projection_sha256,extraction_status) VALUES (?1,42,?2,?3,?4,?5,'2026-09-26','2026-09-26',NULL,?6,?7,?8,?9)",params![id,portal,format!("release-{id}.pdf"),format!("https://www.sandiego.gov/fixture-{id}.pdf"),visibility,"a".repeat(64),"b".repeat(64),projection,extraction_status]).unwrap();
        conn.execute("INSERT INTO documents_fts(document_id,portal_subdomain,file_name,extracted_text) VALUES (?1,?2,?3,?4)", params![id,portal,format!("release-{id}.pdf"),text]).unwrap();
    }

    #[tokio::test]
    async fn retained_text_reconstruction_finds_late_unicode_passages_after_originals_are_purged() {
        let repository = setup().await;
        let conn = repository.analytics_connection().await.unwrap();
        insert_document(&conn, 1, "city", "public", "SEARCHABLE", TARGET);
        let text = format!(
            "{}\n{PUBLIC_SOURCE}",
            "Résumé de l’annexe.\n".repeat(12_000)
        );
        assert!(text.chars().count() > 120_000);
        insert_document(&conn, 2, "city", "public", "SALVAGED", &text);
        let stored_originals: u64 = conn
            .query_row(
                "SELECT COUNT(*) FROM documents WHERE local_path IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored_originals, 0);

        let report = repository
            .reconstruct_document(1, "city", Default::default())
            .await
            .unwrap();
        assert_eq!(report.documents_compared, 2);
        let finding = report
            .findings
            .iter()
            .find(|finding| finding.method == "complementary_fragments")
            .unwrap();
        assert_eq!(
            finding.observation.extraction_sha256,
            sha256_hex(TARGET.as_bytes())
        );
        let candidate = finding.candidates.iter().find(|candidate| candidate.value == "12346789").expect("targeted search must reach an unredacted record beyond the initial 120k characters");
        assert!(!candidate.contradicted);
        let citation = candidate
            .evidence
            .iter()
            .find(|citation| citation.document_id == 2)
            .unwrap();
        assert!(citation.character_offset > 120_000);
        assert_eq!(
            citation.source_url.as_deref(),
            Some("https://www.sandiego.gov/fixture-2.pdf")
        );
        assert_eq!(citation.source_sha256, "a".repeat(64));
        assert_eq!(citation.extraction_sha256, sha256_hex(text.as_bytes()), "citations must use the retained projection digest, not the discarded raw extraction digest");
        assert_ne!(citation.extraction_sha256, "b".repeat(64));
        let cited: String = text
            .chars()
            .skip(citation.character_offset)
            .take(citation.quote.chars().count())
            .collect();
        assert_eq!(
            cited, citation.quote,
            "Unicode locators address characters in retained text, not bytes or guessed PDF pages"
        );
    }

    #[tokio::test]
    async fn reconstruction_excludes_private_removed_and_unsearchable_sources_and_targets() {
        let repository = setup().await;
        let conn = repository.analytics_connection().await.unwrap();
        insert_document(&conn, 1, "city", " Public ", "SEARCHABLE", TARGET);
        insert_document(&conn, 2, "city", "public", "SEARCHABLE", PUBLIC_SOURCE);
        for (id, portal, visibility, status, value) in [
            (3, "city", "private", "SEARCHABLE", "33336789"),
            (4, "removed-city", "public", "SEARCHABLE", "44446789"),
            (5, "city", "public", "DEFERRED", "55556789"),
        ] {
            insert_document(
                &conn,
                id,
                portal,
                visibility,
                status,
                &format!("Project: Riverbed restoration\nContract: {value}\n"),
            );
        }
        let report = repository
            .reconstruct_document(1, "city", Default::default())
            .await
            .unwrap();
        assert_eq!(report.documents_compared, 2);
        let finding = report
            .findings
            .iter()
            .find(|finding| finding.method == "complementary_fragments")
            .unwrap();
        assert_eq!(
            finding
                .candidates
                .iter()
                .map(|candidate| candidate.value.as_str())
                .collect::<Vec<_>>(),
            vec!["12346789"]
        );
        for (id, portal) in [(3, "city"), (4, "removed-city"), (5, "city")] {
            assert!(
                repository
                    .reconstruct_document(id, portal, Default::default())
                    .await
                    .is_err(),
                "excluded records must not become reconstruction targets"
            );
        }
    }

    #[tokio::test]
    async fn source_identity_validation_rejects_replacement_removal_or_loss_of_public_evidence() {
        let repository = setup().await;
        let conn = repository.analytics_connection().await.unwrap();
        insert_document(&conn, 1, "city", "public", "SEARCHABLE", TARGET);
        insert_document(&conn, 2, "city", "public", "SEARCHABLE", PUBLIC_SOURCE);
        let identities: BTreeSet<_> = [(1, TARGET), (2, PUBLIC_SOURCE)]
            .into_iter()
            .map(|(id, text)| {
                (
                    "city".into(),
                    id,
                    "a".repeat(64),
                    sha256_hex(text.as_bytes()),
                )
            })
            .collect();
        validate_source_identities(&conn, &identities).unwrap();
        for mutation in [
            "UPDATE documents SET raw_file_sha256=repeat('c',64) WHERE id=2",
            "UPDATE documents SET extracted_text_projection_sha256=repeat('d',64) WHERE id=2",
            "UPDATE documents SET visibility='private' WHERE id=2",
            "UPDATE documents SET extraction_status='FAILED' WHERE id=2",
            "UPDATE portals SET lifecycle_state='REMOVED' WHERE subdomain='city'",
        ] {
            conn.execute(mutation, []).unwrap();
            assert!(
                validate_source_identities(&conn, &identities).is_err(),
                "captured source identities must reject mutation: {mutation}"
            );
            conn.execute("UPDATE documents SET raw_file_sha256=repeat('a',64),extracted_text_projection_sha256=?1,visibility='public',extraction_status='SEARCHABLE' WHERE id=2",params![sha256_hex(PUBLIC_SOURCE.as_bytes())]).unwrap();
            conn.execute(
                "UPDATE portals SET lifecycle_state='ACTIVE' WHERE subdomain='city'",
                [],
            )
            .unwrap();
            validate_source_identities(&conn, &identities).unwrap();
        }
        conn.execute("DELETE FROM documents_fts WHERE document_id=2", [])
            .unwrap();
        assert!(
            validate_source_identities(&conn, &identities).is_err(),
            "missing retained text must invalidate an otherwise searchable source"
        );
        conn.execute("INSERT INTO documents_fts(document_id,portal_subdomain,file_name,extracted_text) VALUES (2,'city','release-2.pdf',?1)",params![PUBLIC_SOURCE]).unwrap();
        validate_source_identities(&conn, &identities).unwrap();
        conn.execute("DELETE FROM documents WHERE id=2", [])
            .unwrap();
        assert!(
            validate_source_identities(&conn, &identities).is_err(),
            "a deleted comparison source must invalidate the report"
        );
        conn.execute(
            "UPDATE documents SET extracted_text_projection_sha256=repeat('e',64) WHERE id=1",
            [],
        )
        .unwrap();
        let target_only: BTreeSet<_> = identities
            .into_iter()
            .filter(|(_, id, _, _)| *id == 1)
            .collect();
        assert!(
            validate_source_identities(&conn, &target_only).is_err(),
            "a replaced target extraction must also invalidate the report"
        );
    }
}
