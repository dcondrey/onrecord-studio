//! Opt-in, read-only checks: inspect public metadata; do not start a production crawl.
use onrecord_studio_lib::ingestion::client::{ApiClient, HttpClientConfig};

fn client() -> ApiClient {
    ApiClient::new(HttpClientConfig {
        max_retries: 0,
        request_timeout_seconds: 25,
        host_request_budget: 15,
        ..Default::default()
    })
    .unwrap()
}

#[tokio::test]
#[ignore = "Reads one live SANDAG catalog page"]
async fn sandag_catalog_downloads_belong_to_sandag() {
    let client = client();
    let endpoint = client
        .detect_data_catalog("https://opendata.sandag.org")
        .await
        .expect("SANDAG Socrata catalog");
    let page = client
        .fetch_data_catalog_page(&endpoint, None)
        .await
        .unwrap();
    assert!(!page.downloads.is_empty());
    assert!(page
        .downloads
        .iter()
        .all(|d| d.url.starts_with("https://opendata.sandag.org/")));
    println!(
        "SANDAG: {} catalog assets, {} exports on first page",
        endpoint.asset_count,
        page.downloads.len()
    );
}

#[tokio::test]
#[ignore = "Reads the live County budget configuration"]
async fn county_budget_exposes_published_csv_and_schema() {
    let client = client();
    let (_, page) = client
        .discover_public_source("https://sandiegocounty.budget.socrata.com")
        .await
        .unwrap();
    let exports = page
        .links
        .iter()
        .filter(|l| {
            l.url
                .starts_with("https://data.sandiegocounty.gov/api/views/")
                && l.url.contains("rows.csv")
        })
        .collect::<Vec<_>>();
    assert!(exports.len() >= 2);
    println!("County budget: {} published exports", exports.len());
}

#[tokio::test]
#[ignore = "Reads the live SanGIS/SANDAG ArcGIS Hub configuration"]
async fn sandag_hub_exposes_a_content_group() {
    let client = client();
    let (_, page) = client
        .discover_public_source("https://sdgis-sandag.opendata.arcgis.com")
        .await
        .unwrap();
    assert!(page
        .links
        .iter()
        .any(|l| l.url.contains("/sharing/rest/search?") && l.url.contains("group%3A")));
    println!("SANDAG Hub: public group discovery succeeded");
}

#[tokio::test]
#[ignore = "Reads the live public Council agenda index"]
async fn hyland_exposes_distinct_meetings_and_downloads() {
    let client = client();
    let (_, page) = client
        .discover_public_source("https://sandiego.hylandcloud.com/211agendaonlinecouncil")
        .await
        .unwrap();
    assert!(
        page.links
            .iter()
            .filter(|l| l.url.contains("/Meetings/ViewMeeting?"))
            .count()
            > 1
    );
    let meeting = page
        .links
        .iter()
        .find(|l| l.url.contains("/Meetings/ViewMeeting?"))
        .unwrap();
    let detail = client.inspect_public_resource(&meeting.url).await.unwrap();
    let agenda = detail
        .links
        .iter()
        .find(|l| l.url.contains("/Documents/ViewAgenda?"))
        .expect("published agenda endpoint");
    let content = client.inspect_public_resource(&agenda.url).await.unwrap();
    assert!(content.body.as_ref().is_some_and(|body| body.len() > 1000));
    assert!(content.links.iter().any(|l| l.download_candidate));
    println!(
        "Council archive: {} public index links; {} attachment links in first agenda",
        page.links.len(),
        content
            .links
            .iter()
            .filter(|l| l.download_candidate)
            .count()
    );
}

#[tokio::test]
#[ignore = "Reads one live Transportation Socrata catalog page"]
async fn transportation_catalog_preserves_its_domain() {
    let client = client();
    let endpoint = client
        .detect_data_catalog("https://data.transportation.gov")
        .await
        .expect("Transportation Socrata catalog");
    let page = client
        .fetch_data_catalog_page(&endpoint, None)
        .await
        .unwrap();
    assert!(!page.downloads.is_empty());
    assert!(page
        .downloads
        .iter()
        .all(|d| d.url.starts_with("https://data.transportation.gov/")));
    println!(
        "Transportation: {} advertised assets, {} exports",
        endpoint.asset_count,
        page.downloads.len()
    );
}

#[tokio::test]
#[ignore = "Reads the live UCSD Homelessness Hub configuration"]
async fn ucsd_homelessness_hub_exposes_published_content() {
    let (_, page) = client()
        .discover_public_source("https://hhubsandiego-ucsdonline.hub.arcgis.com")
        .await
        .unwrap();
    assert!(page
        .links
        .iter()
        .any(|l| l.url.contains("group%3Afc3e345822664a3cb7ffb771453f1ba8")));
}

#[tokio::test]
#[ignore = "Reads the selected City budget page and one Data.gov dataset page"]
async fn selected_catalog_pages_expose_cdn_files_and_original_metadata() {
    let client = client();
    let (_, page) = client
        .discover_public_source("https://data.sandiego.gov/datasets/operating-budget/")
        .await
        .unwrap();
    assert!(page.links.iter().any(|l| l.download_candidate
        && l.url == "https://seshat.datasd.org/operating_budget/budget_operating_datasd.csv"));
    assert!(client
        .detect_data_catalog("https://catalog.data.gov")
        .await
        .is_none());
    let (_, page) = client.discover_public_source("https://catalog.data.gov/dataset/supply-chain-greenhouse-gas-emission-factors-v1-3-by-naics-6").await.unwrap();
    assert!(page.links.iter().any(|l| l.download_candidate
        && l.url.contains("/harvest_record/")
        && l.url.ends_with("/raw")));
    assert!(page
        .links
        .iter()
        .any(|l| l.download_candidate && l.url.starts_with("https://pasteur.epa.gov/")));
}

#[tokio::test]
#[ignore = "Reads City OpenGov bootstrap and one publicly listed report"]
async fn city_opengov_resolves_report_metadata_and_supporting_files() {
    let client = client();
    let (_, page) = client
        .discover_public_source("https://budget.sandiego.gov")
        .await
        .unwrap();
    let reports = page
        .links
        .iter()
        .filter(|l| {
            l.url
                .starts_with("https://budget.sandiego.gov/api/v1/reports/")
        })
        .collect::<Vec<_>>();
    assert!(reports.len() >= 6);
    assert!(page
        .links
        .iter()
        .any(|l| l.download_candidate && l.url.contains("/original/") && l.url.contains(".pdf")));
    let report = client
        .inspect_public_resource(&reports[0].url)
        .await
        .unwrap();
    assert!(report
        .content_type
        .as_deref()
        .is_some_and(|s| s.contains("application/json")));
    println!(
        "City OpenGov: {} published report metadata URLs and supporting documents",
        reports.len()
    );
}
