use std::path::Path;

use duckdb::params;

use crate::error::AppError;

use crate::types::{
    CrawlPolicy, MetadataPageWindow, PortalRegistration, PublicRequest, SourceGuidance,
    StatutoryExtensionReason, StatutoryExtensionRecord,
};

use super::{
    integrity::{compute_extracted_text_sha256, validate_entropy_metadata},
    migrations::verify_logical_relations,
    sources::{normalized_submitted_url, publisher_pattern_for_url},
    timeline::statutory_clock_confidence_basis_points,
    DocumentDownloadFinalization, DocumentExtractionFinalization, DuckDbRepository,
    IngestionSchedulerCounts, PortalSyncRunFinalization, ProvenanceEvidenceFile,
    SourceLifecycleState, SQL_UTC_NOW,
};

mod analytical;
mod connection;
mod documents;
mod frontier;
mod legal;
mod migrations;
mod provenance;
mod purge;
mod release_events;
mod search;
mod source_checks;
mod sources;
mod timeline;

const MIGRATION: &str = include_str!("../../../../migrations/0001_initial_schema.sql");

const PORTAL_SUBDOMAIN: &str = "test-portal";

const REQUEST_ID: i64 = 41;

const DOCUMENT_ID: i64 = 73;

const RAW_FILE_SHA256: &str = "1111111111111111111111111111111111111111111111111111111111111111";

// Note the trailing slash. `canonical_url` is `Url::to_string()`, so this is the shape a
// registration actually holds, and it is not the shape an operator pastes.
const RECIPE_ROOT_URL: &str = "https://records.example.test/";

fn test_registration(portal_key: &str, canonical_url: &str) -> PortalRegistration {
    PortalRegistration {
        portal_key: portal_key.to_string(),
        display_name: "Records".to_string(),
        canonical_url: canonical_url.to_string(),
        origin_url: canonical_url.to_string(),
        api_layout: crate::types::PortalApiLayout {
            requests_endpoint: format!("{canonical_url}client/requests"),
            requests_pointer: "/requests".to_string(),
            metadata_pointer: None,
            page_parameter: "page".to_string(),
            page_size_parameter: "per_page".to_string(),
        },
        verified_at: "2026-08-29T00:00:00Z".to_string(),
        source_kind: crate::types::SourceKind::NextRequest,
    }
}

async fn repository_with_registered_portal() -> (DuckDbRepository, PortalRegistration) {
    let repository = DuckDbRepository::new(Path::new(":memory:")).expect("DuckDB core must open");
    repository
        .run_migrations(MIGRATION)
        .await
        .expect("test migration must succeed");
    let registration = test_registration(PORTAL_SUBDOMAIN, RECIPE_ROOT_URL);
    repository
        .insert_or_update_portal_registration(&registration)
        .await
        .expect("portal registration must persist");
    (repository, registration)
}

async fn repository_with_document() -> DuckDbRepository {
    file_repository_with_document(Path::new(":memory:")).await
}

async fn file_repository_with_document(path: &Path) -> DuckDbRepository {
    let repository = DuckDbRepository::new(path).expect("DuckDB core must open");
    repository
        .run_migrations(MIGRATION)
        .await
        .expect("test migration must succeed");

    {
        let connection = repository.conn.lock().await;
        connection
            .execute(
                // The fixture's requests are evaluated under the CPRA clock; a source names its
                // jurisdiction explicitly, since none is inferred for an unrecognized host.
                "INSERT INTO portals (subdomain, display_name, base_url, statutory_jurisdiction)
                 VALUES (?1, ?2, ?3, 'CA_CPRA')",
                params![PORTAL_SUBDOMAIN, "Test Portal", "https://example.test"],
            )
            .expect("portal fixture must insert");
        connection
            .execute(
                "INSERT INTO requests (
                    id,
                    portal_subdomain,
                    status,
                    created_at,
                    updated_at,
                    closed_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    REQUEST_ID,
                    PORTAL_SUBDOMAIN,
                    "closed",
                    "2026-08-24T12:00:00Z",
                    "2026-08-24T11:59:00Z",
                    "2026-08-24T12:04:00Z",
                ],
            )
            .expect("request fixture must insert");
        connection
            .execute(
                "INSERT INTO documents (
                    id,
                    request_id,
                    portal_subdomain,
                    file_name,
                    visibility,
                    created_at,
                    updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    DOCUMENT_ID,
                    REQUEST_ID,
                    PORTAL_SUBDOMAIN,
                    "release.pdf",
                    "public",
                    "2026-08-24T12:01:00Z",
                    "2026-08-24T12:01:00Z",
                ],
            )
            .expect("document fixture must insert");
    }

    repository
}

/// Both defects above were spelling mistakes that no type checker sees, in SQL that only a
/// reached call site executes. This reads the SQL as text so an unreached one cannot
/// reintroduce either: `current_timestamp` inside `DO UPDATE SET` refuses to bind, and the
/// `%f`-without-`%S` format writes microseconds into the seconds field.
#[test]
fn every_sql_timestamp_is_written_as_parseable_utc() {
    const SOURCES: [(&str, &str); 16] = [
        ("storage/sqlite/mod.rs", include_str!("../mod.rs")),
        (
            "storage/sqlite/analytical.rs",
            include_str!("../analytical.rs"),
        ),
        (
            "storage/sqlite/connection.rs",
            include_str!("../connection.rs"),
        ),
        (
            "storage/sqlite/documents.rs",
            include_str!("../documents.rs"),
        ),
        ("storage/sqlite/frontier.rs", include_str!("../frontier.rs")),
        (
            "storage/sqlite/integrity.rs",
            include_str!("../integrity.rs"),
        ),
        ("storage/sqlite/legal.rs", include_str!("../legal.rs")),
        (
            "storage/sqlite/migrations.rs",
            include_str!("../migrations.rs"),
        ),
        ("storage/sqlite/policy.rs", include_str!("../policy.rs")),
        (
            "storage/sqlite/provenance.rs",
            include_str!("../provenance.rs"),
        ),
        ("storage/sqlite/purge.rs", include_str!("../purge.rs")),
        ("storage/sqlite/search.rs", include_str!("../search.rs")),
        ("storage/sqlite/sources.rs", include_str!("../sources.rs")),
        ("storage/sqlite/timeline.rs", include_str!("../timeline.rs")),
        ("storage/corpus.rs", include_str!("../../corpus.rs")),
        (
            "migrations/0001_initial_schema.sql",
            include_str!("../../../../migrations/0001_initial_schema.sql"),
        ),
    ];
    // Assembled rather than written out, so that this test's own source does not match the
    // needles it searches for in itself.
    let seconds_dropping_format = format!("'%Y-%m-%dT%H:%M:{}fZ'", '%');
    let unbindable_now = format!("strftime({}_timestamp", "current");
    for (name, source) in SOURCES {
        assert!(
            !source.contains(&seconds_dropping_format),
            "{name} formats a timestamp with %f and no %S, which writes microseconds into the \
             seconds field and yields a string RFC 3339 parsing rejects"
        );
        assert!(
            !source.contains(&unbindable_now),
            "{name} renders a timestamp from current_timestamp; use now(), which also binds \
             inside ON CONFLICT ... DO UPDATE SET"
        );
    }
    assert!(
        SOURCES
            .iter()
            .map(|(_, source)| source.matches(SQL_UTC_NOW).count())
            .sum::<usize>()
            > 1,
        "the pinned spelling must be the one actually in use"
    );
}

fn test_acquisition() -> crate::ingestion::client::AcquisitionRecord {
    crate::ingestion::client::AcquisitionRecord {
        requested_url: "https://records.example.gov/doc".to_string(),
        resolved_url: "https://cdn.example.gov/doc.pdf".to_string(),
        redirect_chain: vec!["https://records.example.gov/doc".to_string()],
        http_status: 200,
        content_type: Some("application/pdf".to_string()),
        etag: None,
        last_modified: None,
        retrieved_at: "2026-08-24T12:03:00.000000Z".to_string(),
    }
}

async fn set_document_file(
    repository: &DuckDbRepository,
    local_path: &Path,
    recorded_digest: Option<&str>,
) {
    let connection = repository.conn.lock().await;
    connection
        .execute(
            "UPDATE documents
             SET local_path = ?1, raw_file_sha256 = ?2, download_status = 'completed'
             WHERE id = ?3 AND portal_subdomain = ?4",
            params![
                local_path.to_string_lossy().to_string(),
                recorded_digest,
                DOCUMENT_ID,
                PORTAL_SUBDOMAIN
            ],
        )
        .expect("document file fixture must update");
}
