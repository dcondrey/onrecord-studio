//! Opt-in public-network check; no model credentials or document downloads.
use onrecord_studio_lib::ingestion::client::{ApiClient, HttpClientConfig, PublicResourceKind};

#[tokio::test]
#[ignore = "Reads a published San Diego Council meeting and up to eight attachment indexes"]
async fn council_packet_resolves_riverbed_grant_supporting_records() {
    let client = ApiClient::new(HttpClientConfig {
        max_retries: 0,
        host_concurrency: 1,
        host_request_budget: 20,
        ..Default::default()
    })
    .unwrap();
    let meeting = "https://sandiego.hylandcloud.com/211agendaonlinecouncil/Meetings/ViewMeeting?doctype=1&id=7192&site=council";
    let inspected = tokio::time::timeout(
        std::time::Duration::from_secs(120),
        client.inspect_public_resource(meeting),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(inspected.kind, PublicResourceKind::HtmlPage);
    assert!(inspected
        .final_url
        .contains("/Documents/ViewAgenda?meetingId=7192"));
    let body = String::from_utf8_lossy(inspected.body.as_ref().unwrap());
    assert!(body.contains("Riverbed"));
    let riverbed = inspected
        .links
        .iter()
        .filter(|link| link.download_candidate && link.url.contains("itemId=265583"))
        .collect::<Vec<_>>();
    assert!(
        riverbed.len() >= 3,
        "Expected staff report, agreement and amendment: {riverbed:?}"
    );
    assert!(riverbed
        .iter()
        .any(|link| link.label.to_lowercase().contains("amendment")));
    assert!(riverbed.iter().all(|link| link.url.starts_with(
        "https://sandiego.hylandcloud.com/211agendaonlinecouncil/Documents/DownloadFile/"
    )));
    let results = serde_json::json!({
        "meeting":meeting,"agenda":inspected.final_url,
        "downloadLinks":inspected.links.iter().filter(|link|link.download_candidate).count(),
        "riverbedSupportingRecords":riverbed.iter().map(|link|serde_json::json!({"title":link.label,"url":link.url})).collect::<Vec<_>>(),
    });
    let output = std::env::temp_dir().join("onrecord-council-packet-validation.json");
    std::fs::write(&output, serde_json::to_vec_pretty(&results).unwrap()).unwrap();
    println!("{}", results);
}
