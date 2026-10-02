use super::super::{
    crawl::crawl_host_in_scope,
    guidance::{
        describe_discovery_origin, is_supported_external_data_platform,
        may_expand_structured_links, next_msg_attachment_container_depth,
    },
    structured::{
        arcgis_item_companion_urls, arcgis_layer_url, arcgis_layers_from_metadata,
        arcgis_metadata_url, arcgis_search_next_page, arcgis_structured_item_companion_urls,
        collect_structured_link_candidates, discover_structured_document_links,
        tableau_export_companion_urls,
    },
};
use super::*;

/// A crawl node named by a structured payload obeys the same host rule as one found in HTML.
/// Proven against the pre-fix file by inspection: `expand_structured_document_links` handed
/// every non-download candidate to `prepare_crawl_url` with no host check, while
/// `admit_crawl_link` refused the identical URL as `OffScopeHost`.
#[test]
fn structured_crawl_candidates_obey_the_html_host_scope_rule() {
    let source = "https://records.example.gov/portal";
    let guidance = SourceGuidance::default();
    let mut policy = CrawlPolicy {
        follow_supported_external_platforms: false,
        ..Default::default()
    };

    assert!(crawl_host_in_scope(
        source,
        &guidance,
        &policy,
        "https://records.example.gov/api/3/action/package_show?id=x"
    ));
    assert!(!crawl_host_in_scope(
        source,
        &guidance,
        &policy,
        "https://anywhere.example/landing"
    ));
    assert!(
        !crawl_host_in_scope(
            source,
            &guidance,
            &policy,
            "https://services.arcgis.com/abc/FeatureServer"
        ),
        "a supported platform is off scope until the policy opts in"
    );

    policy.follow_supported_external_platforms = true;
    assert!(crawl_host_in_scope(
        source,
        &guidance,
        &policy,
        "https://services.arcgis.com/abc/FeatureServer"
    ));

    let allowing = SourceGuidance {
        allowed_hostnames: vec!["anywhere.example".to_string()],
        ..Default::default()
    };
    assert!(crawl_host_in_scope(
        source,
        &allowing,
        &policy,
        "https://data.anywhere.example/landing"
    ));
}

#[test]
fn structured_expansion_is_bounded_by_the_chain_budget_not_by_crawl_depth() {
    assert!(may_expand_structured_links(0));
    assert!(may_expand_structured_links(MAXIMUM_STRUCTURED_CHAIN - 1));
    assert!(!may_expand_structured_links(MAXIMUM_STRUCTURED_CHAIN));
    assert!(!may_expand_structured_links(MAXIMUM_STRUCTURED_CHAIN + 7));
}

#[test]
fn every_msg_attachment_descent_spends_the_shared_container_budget() {
    assert_eq!(next_msg_attachment_container_depth(0), Some(1));
    assert_eq!(
        next_msg_attachment_container_depth(MAXIMUM_CONTAINER_DEPTH - 1),
        Some(MAXIMUM_CONTAINER_DEPTH)
    );
    assert_eq!(
        next_msg_attachment_container_depth(MAXIMUM_CONTAINER_DEPTH),
        None
    );
    assert_eq!(
        next_msg_attachment_container_depth(MAXIMUM_CONTAINER_DEPTH + 1),
        None
    );
}

/// A catalog eight pages into a portal is exactly the case the previous gate killed: its
/// crawl depth sits at the policy ceiling while its structured chain is untouched.
#[test]
fn a_catalog_at_the_crawl_ceiling_still_expands() {
    let policy = CrawlPolicy::default();
    let depth = PayloadDepth::discovered(policy.maximum_depth, 0);

    assert!(may_expand_structured_links(depth.structured));
}

#[test]
fn discovery_origin_names_the_page_and_flags_a_cross_host_payload() {
    assert_eq!(
        describe_discovery_origin(
            "https://records.example.gov/page/4",
            "https://records.example.gov/files/report.pdf"
        ),
        "Discovered from https://records.example.gov/page/4"
    );
    assert_eq!(
        describe_discovery_origin(
            "https://records.example.gov/page/4",
            "https://cdn.attacker.example/report.pdf"
        ),
        "Discovered from https://records.example.gov/page/4; payload hosted on cdn.attacker.example"
    );
    assert_eq!(
        describe_discovery_origin("local document 73", "https://cdn.example/report.pdf"),
        "Discovered from local document 73"
    );
}

#[test]
fn expands_dcat_distributions_without_following_licenses_or_context_urls() {
    let catalog = serde_json::json!({
        "@context": "https://project-open-data.cio.gov/schema/catalog.jsonld",
        "dataset": [{
            "title": "Public records",
            "license": "https://example.gov/license",
            "distribution": [{
                "title": "CSV export",
                "downloadURL": "https://data.example.gov/records.csv",
                "mediaType": "text/csv"
            }, {
                "title": "Query endpoint",
                "accessURL": "https://data.example.gov/api/records",
                "mediaType": "application/json"
            }]
        }]
    });
    let mut candidates = HashMap::new();
    collect_structured_link_candidates(&catalog, None, 0, None, &mut candidates);

    assert_eq!(candidates.len(), 2);
    assert!(candidates["https://data.example.gov/records.csv"].download_candidate);
    assert!(!candidates["https://data.example.gov/api/records"].download_candidate);
    assert!(!candidates.contains_key("https://example.gov/license"));
}

#[test]
fn structured_discovery_returns_only_scheduling_summaries_and_ignores_malformed_json() {
    let source_url = "https://gis.example.gov/arcgis/rest/services/Records/FeatureServer?f=json";
    let payload = serde_json::json!({
        "type": "Feature Service",
        "id": "e75d5c7a59ff4d6a90e862a481c3ba96",
        "title": "Records service",
        "downloadURL": "https://files.example.gov/records.csv",
        "mediaType": "text/csv",
        "layers": [{"id": 0}],
        "tables": [{"id": 2}]
    });
    let bytes = serde_json::to_vec(&payload).unwrap();

    let discovery = discover_structured_document_links(&bytes, Some(source_url))
        .expect("valid structured payload must produce a scheduling summary");
    assert_eq!(discovery.candidates.len(), 1);
    assert_eq!(
        discovery.candidates[0].url,
        "https://files.example.gov/records.csv"
    );
    assert!(discovery.candidates[0].download_candidate);
    assert_eq!(discovery.companion_urls.len(), 2);
    assert_eq!(
        discovery.layer_urls,
        vec![
            "https://gis.example.gov/arcgis/rest/services/Records/FeatureServer/0".to_string(),
            "https://gis.example.gov/arcgis/rest/services/Records/FeatureServer/2".to_string(),
        ]
    );
    assert!(discovery.next_page.is_none());

    let replay = include_bytes!("../../../../fixtures/open-data/arcgis-item.json");
    let replay_discovery = discover_structured_document_links(
        replay,
        Some("https://hub.example.gov/datasets/facilities"),
    )
    .expect("recorded ArcGIS item fixture must replay");
    assert_eq!(replay_discovery.candidates.len(), 1);
    assert!(replay_discovery.candidates[0]
        .url
        .ends_with("/PublicFacilities/FeatureServer/0"));
    assert!(discover_structured_document_links(b"{broken", Some(source_url)).is_none());
}

#[test]
fn derives_arcgis_item_and_paginated_layer_endpoints() {
    let package = discover_structured_document_links(br#"{"success":true,"result":{"resources":[{"name":"Measurements","format":"CSV","url":"https://data.example.test/export?id=42"}]}}"#, Some("https://data.ca.gov/api/3/action/package_show?id=sample")).unwrap();
    assert_eq!(package.candidates.len(), 1);
    assert!(package.candidates[0].download_candidate);
    assert_eq!(
        package.candidates[0].media_type.as_deref(),
        Some("text/csv")
    );
    assert!(arcgis_layer_url(
        "https://services.arcgis.com/a/FeatureServer/0/query?where=county%3D73&f=geojson"
    )
    .is_none());
    assert!(
        arcgis_layer_url("https://services.arcgis.com/a/FeatureServer/0?where=county%3D73")
            .is_none()
    );
    let companions = arcgis_item_companion_urls(
        "https://gis-portal.example.gov/arcgis/home/webmap/viewer.html?webmap=e75d5c7a59ff4d6a90e862a481c3ba96",
    );
    assert_eq!(companions.len(), 2);
    assert!(companions[0].contains("/items/e75d5c7a59ff4d6a90e862a481c3ba96?f=json"));

    let service = "https://services.arcgis.com/example/arcgis/rest/services/Records/FeatureServer";
    let metadata = serde_json::json!({"layers": [{"id": 0}, {"id": 4}], "tables": [{"id": 7}]});
    let layers = arcgis_layers_from_metadata(Some(service), &metadata);
    assert_eq!(layers.len(), 3);
    assert_eq!(arcgis_layer_url(&layers[1]), Some(layers[1].clone()));
    assert_eq!(
        arcgis_metadata_url(service),
        Some(format!("{service}?f=json"))
    );
}

#[test]
fn discovers_arcgis_hub_catalogs_and_structured_items() {
    let hub = arcgis_hub_companion_urls(
        "https://records.hub.arcgis.com/pages/data",
        b"%7B%22contentGroupId%22%3A%22fc3e345822664a3cb7ffb771453f1ba8%22%2C%22siteId%22%3A%2254054311055a4758b9d641896521648e%22%7D",
    );
    assert!(hub.iter().any(|url| {
        url.contains("/sharing/rest/search?")
            && url.contains("group%3Afc3e345822664a3cb7ffb771453f1ba8")
    }));
    assert!(hub
        .iter()
        .any(|url| { url.contains("/items/54054311055a4758b9d641896521648e/data?f=json") }));

    let search = serde_json::json!({
        "results": [{
            "id": "e75d5c7a59ff4d6a90e862a481c3ba96",
            "type": "Feature Service",
            "title": "Public records"
        }]
    });
    let companions = arcgis_structured_item_companion_urls(&search);
    assert_eq!(companions.len(), 2);
    assert!(companions
        .iter()
        .any(|url| url.ends_with("e75d5c7a59ff4d6a90e862a481c3ba96?f=json")));
    let next = arcgis_search_next_page(
        Some("https://www.arcgis.com/sharing/rest/search?f=json&q=group%3Aabc&num=100&start=1"),
        &serde_json::json!({"nextStart": 101}),
    )
    .unwrap();
    assert!(next.contains("q=group%3Aabc"));
    assert!(next.ends_with("start=101"));
}

#[test]
fn follows_only_explicitly_supported_external_data_platforms() {
    assert!(is_supported_external_data_platform(
        "https://records.hub.arcgis.com/pages/data"
    ));
    assert!(is_supported_external_data_platform(
        "https://public.tableau.com/views/workbook/view"
    ));
    assert!(!is_supported_external_data_platform(
        "https://unrelated.example/blog"
    ));
}

#[test]
fn derives_tableau_public_csv_and_pdf_exports() {
    let exports = tableau_export_companion_urls(
        "https://public.tableau.com/app/profile/chsu/viz/CommunityProfiles/HHSA?publish=yes",
    );
    assert_eq!(exports.len(), 2);
    assert_eq!(
        exports[0],
        "https://public.tableau.com/views/CommunityProfiles/HHSA.csv?:showVizHome=no"
    );
    assert_eq!(
        exports[1],
        "https://public.tableau.com/views/CommunityProfiles/HHSA.pdf?:showVizHome=no"
    );
}
