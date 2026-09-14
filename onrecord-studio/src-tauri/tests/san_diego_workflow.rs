use onrecord_studio_lib::storage::sqlite::DuckDbRepository;
use onrecord_studio_lib::types::{PortalRegistration, PublicRequest};
use std::path::Path;

#[tokio::test]
async fn request_metadata_is_searchable_before_attachments_arrive() {
    let repository = DuckDbRepository::new(Path::new(":memory:")).unwrap();
    repository
        .run_migrations(include_str!("../migrations/0001_initial_schema.sql"))
        .await
        .unwrap();
    let portal: PortalRegistration = serde_json::from_value(serde_json::json!({
        "portalKey":"fixture-gov", "displayName":"Synthetic fixture", "canonicalUrl":"https://example.test/",
        "originUrl":"https://example.test/", "verifiedAt":"2026-09-01T00:00:00Z", "sourceKind":"NEXT_REQUEST",
        "apiLayout":{"requestsEndpoint":"https://example.test/api/requests", "requestsPointer":"/requests", "metadataPointer":null, "pageParameter":"page", "pageSizeParameter":"per_page"}
    })).unwrap();
    repository
        .insert_or_update_portal_registration(&portal)
        .await
        .unwrap();
    let request: PublicRequest = serde_json::from_value(serde_json::json!({
        "id":42, "title":"San Diego surveillance policy", "description":"Cámara retention contracts", "status":"open",
        "created_at":"2026-09-01T00:00:00Z", "updated_at":"2026-09-02T00:00:00Z", "documents":[]
    })).unwrap();
    repository
        .insert_or_update_request("fixture-gov", &request)
        .await
        .unwrap();
    let unfiltered = repository
        .search_requests("", Some("fixture-gov"), None, None, None, 500)
        .await
        .unwrap();
    let by_title = repository
        .search_requests("surveillance", Some("fixture-gov"), None, None, None, 500)
        .await
        .unwrap();
    let by_description = repository
        .search_requests("cámara", Some("fixture-gov"), None, None, None, 500)
        .await
        .unwrap();

    assert_eq!(unfiltered.len(), 1);
    assert_eq!(
        by_title.len(),
        1,
        "A stored request should be searchable by a word in its title"
    );
    assert_eq!(
        by_description.len(),
        1,
        "A stored request should be searchable by a word in its description"
    );
    assert_eq!(
        repository
            .search_requests(
                "surveillance contracts",
                Some("fixture-gov"),
                None,
                None,
                None,
                500
            )
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        repository
            .search_requests(
                "surveillance absent",
                Some("fixture-gov"),
                None,
                None,
                None,
                500
            )
            .await
            .unwrap()
            .len(),
        0
    );
    assert_eq!(
        repository
            .search_requests("42", Some("fixture-gov"), None, None, None, 500)
            .await
            .unwrap()
            .len(),
        1
    );
    let mut updated = request.clone();
    updated.title = Some("San Diego camera policy".to_string());
    repository
        .insert_or_update_request("fixture-gov", &updated)
        .await
        .unwrap();
    assert!(repository
        .search_requests("surveillance", Some("fixture-gov"), None, None, None, 500)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn city_document_pages_skip_unrelated_api_probes() {
    use onrecord_studio_lib::ingestion::client::{ApiClient, HttpClientConfig};
    let client = ApiClient::new(HttpClientConfig::default()).unwrap();
    let url = "https://www.sandiego.gov/cpp/meetingsagendas";
    let result = client.discover_portal(url).await.unwrap_err();
    assert!(result.to_string().contains("San Diego City document pages"));
    assert!(client.detect_data_catalog(url).await.is_none());
    assert!(
        client.host_governor().snapshots().is_empty(),
        "routing a known City page must not send API probes"
    );
}
