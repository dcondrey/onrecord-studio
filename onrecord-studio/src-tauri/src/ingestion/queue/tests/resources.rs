use super::super::{
    guidance::{download_host_allowed, is_supported_external_data_platform},
    resources::prepare_discovered_download,
};
use super::*;

fn off_host_download() -> PublicResourceInspection {
    PublicResourceInspection {
        final_url: "https://records-files.s3.amazonaws.com/report.pdf".to_string(),
        content_type: Some("application/pdf".to_string()),
        title: "Report".to_string(),
        kind: PublicResourceKind::Download,
        links: Vec::new(),
        body: None,
        embedded_connectors: Vec::new(),
        content_length: None,
        acquisition: None,
    }
}

// REGRESSION(queue): a crawled page could steer a download to any host, and there was no way
// for an operator to say otherwise. Proven by deleting the `download_host_allowed` branch in
// `prepare_discovered_download`: the restricted cases below then produce a task and the
// exclusion ledger stays empty.
#[tokio::test]
async fn a_cross_host_download_is_admitted_by_default_and_refused_once_restricted() {
    let directory = tempdir().expect("temp dir must be created");
    let permissive = crawl_worker_context(&directory, SourceGuidance::default()).await;
    let task = prepare_discovered_download(
        &permissive,
        &off_host_download(),
        Some(7),
        "https://records.example.test/portal/reports",
        PayloadDepth::discovered(1, 0),
    )
    .await
    .expect("the permissive default must not error");
    assert!(
        task.is_some(),
        "today's behaviour admits a payload hosted off the source root"
    );
    assert!(permissive.crawl.frontier_exclusions.lock().await.is_empty());

    let restricted = crawl_worker_context(
        &directory,
        SourceGuidance {
            restrict_downloads_to_source_host: true,
            ..SourceGuidance::default()
        },
    )
    .await;
    let task = prepare_discovered_download(
        &restricted,
        &off_host_download(),
        Some(7),
        "https://records.example.test/portal/reports",
        PayloadDepth::discovered(1, 0),
    )
    .await
    .expect("a refused host is a policy outcome, not an error");
    assert!(task.is_none(), "the restricted source must refuse the host");
    assert_eq!(
        restricted
            .crawl
            .frontier_exclusions
            .lock()
            .await
            .get("off-source-host-download"),
        Some(&1),
        "a refused download has to be countable, not silent"
    );
}

#[tokio::test]
async fn restricting_downloads_still_admits_the_source_root_and_the_explicit_allowlist() {
    let directory = tempdir().expect("temp dir must be created");
    let context = crawl_worker_context(
        &directory,
        SourceGuidance {
            restrict_downloads_to_source_host: true,
            allowed_hostnames: vec!["s3.amazonaws.com".to_string()],
            ..SourceGuidance::default()
        },
    )
    .await;
    for url in [
        "https://records.example.test/files/report.pdf",
        "https://records-files.s3.amazonaws.com/report.pdf",
    ] {
        let inspection = PublicResourceInspection {
            final_url: url.to_string(),
            ..off_host_download()
        };
        let task = prepare_discovered_download(
            &context,
            &inspection,
            Some(7),
            "https://records.example.test/portal/reports",
            PayloadDepth::discovered(1, 0),
        )
        .await
        .expect("an admitted host must not error");
        assert!(task.is_some(), "{url} must survive the download gate");
    }
}

// The rejected fix was to apply the follow-path gate to downloads. This pins that the new flag
// did not do that by another route: `is_supported_external_data_platform` omits S3, Azure Blob,
// Granicus and CivicPlus, so a source that restricts its downloads must still follow links onto
// the platforms that gate does admit.
#[test]
fn restricting_downloads_does_not_narrow_which_links_are_followed() {
    let guidance = SourceGuidance {
        restrict_downloads_to_source_host: true,
        ..SourceGuidance::default()
    };
    assert!(is_supported_external_data_platform(
        "https://services.arcgis.com/abc/FeatureServer/0"
    ));
    assert!(guidance.allowed_hostnames.is_empty());
    assert!(!download_host_allowed(
        &guidance,
        "https://records.example.test/portal",
        "https://services.arcgis.com/abc/FeatureServer/0"
    ));
}
