use super::*;
use crate::analysis::finding::{
    Finding, FindingCitation, FindingConfidence, FindingKind, FindingSeverity, FindingSubject,
};
use crate::commands::bundle::write_bundle;
use crate::report::SourceDocument;
use crate::storage::sqlite::DuckDbRepository;

const RAW_SSN: &str = "123-45-6789";
const RAW_PHONE: &str = "619-555-0142";
const PRIVATE_MARKER: &str = "PRIVATE-RECORD-MARKER";

fn finding(id: &str, kind: FindingKind, citations: Vec<FindingCitation>) -> Finding {
    let now = chrono::DateTime::parse_from_rfc3339("2026-01-02T03:04:05Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    Finding {
        id: id.into(),
        kind,
        severity: FindingSeverity::Notable,
        confidence: FindingConfidence::measured(7500),
        subject: FindingSubject {
            portal_key: Some("sd-city".into()),
            ..FindingSubject::default()
        },
        headline: format!("Clerk line {RAW_PHONE} was the only contact"),
        explanation: format!("SSN {RAW_SSN} appears. </script><b>markup</b>"),
        citations,
        discovered_at: now,
        last_seen_at: now,
        dismissed_at: None,
        source_detector: "test".into(),
    }
}

fn citation(portal: &str, document: i64) -> FindingCitation {
    FindingCitation {
        portal_key: portal.into(),
        document_id: document,
        character_start: None,
        character_end: None,
        quote: Some(format!("Call {RAW_PHONE} about file {RAW_SSN}.")),
        label: Some(format!("Letter from {RAW_PHONE}")),
    }
}

fn document(visibility: &str, name: &str) -> SourceDocument {
    SourceDocument {
        file_name: Some(name.into()),
        visibility: visibility.into(),
        raw_file_sha256: Some("a".repeat(64)),
        extracted_text_projection_sha256: None,
        text: None,
        download_url: None,
    }
}

fn public_documents() -> SourceDocuments {
    let mut map = SourceDocuments::new();
    map.insert(("sd-city".into(), 7), document("public", "response.pdf"));
    map.insert(
        ("sd-city".into(), 8),
        document("public", &format!("scan {RAW_PHONE}.pdf")),
    );
    map
}

fn assemble_public(findings: Vec<Finding>) -> Assembly {
    assemble(
        findings,
        &public_documents(),
        &BTreeSet::new(),
        &DisclosureContext::unset(),
    )
    .unwrap()
}

fn rendered(assembly: Assembly) -> (BundleDocument, String) {
    let document = BundleDocument::new(assembly.content, "2026-02-03T04:05:06Z".into()).unwrap();
    let html = render_html(&document).unwrap();
    (document, html)
}

fn one_public_finding() -> Assembly {
    assemble_public(vec![finding(
        "aaaa",
        FindingKind::HeldBackResponse,
        vec![citation("sd-city", 7), citation("sd-city", 8)],
    )])
}

#[test]
fn unmasked_personal_information_appears_nowhere_in_the_file() {
    let (document, html) = rendered(one_public_finding());
    for raw in [RAW_SSN, RAW_PHONE] {
        assert!(!html.contains(raw), "{raw} reached the file");
        assert!(!serde_json::to_string(&document).unwrap().contains(raw));
    }
    assert!(html.contains("[MASKED_SSN]") && html.contains("[MASKED_PHONE]"));
}

#[test]
fn a_finding_touching_a_non_public_or_removed_record_is_left_out_whole() {
    let mut documents = public_documents();
    documents.insert(
        ("sd-city".into(), 9),
        document("staff_only", PRIVATE_MARKER),
    );
    documents.insert(("gone".into(), 1), document("public", "removed.pdf"));
    let mut private = finding(
        "bbbb",
        FindingKind::HeldBackResponse,
        vec![citation("sd-city", 7), citation("sd-city", 9)],
    );
    private.headline = format!("{PRIVATE_MARKER} headline");
    let removed = finding(
        "cccc",
        FindingKind::HeldBackResponse,
        vec![citation("gone", 1)],
    );
    let unknown_document = finding(
        "dddd",
        FindingKind::HeldBackResponse,
        vec![citation("sd-city", 404)],
    );
    let ok = finding(
        "aaaa",
        FindingKind::HeldBackResponse,
        vec![citation("sd-city", 7)],
    );
    let assembly = assemble(
        vec![private, removed, unknown_document, ok],
        &documents,
        &BTreeSet::from(["gone".to_string()]),
        &DisclosureContext::unset(),
    )
    .unwrap();
    let reasons: Vec<(String, ExclusionReason)> = assembly
        .exclusions
        .iter()
        .map(|item| (item.finding_id.clone(), item.reason))
        .collect();
    assert_eq!(
        reasons,
        vec![
            ("bbbb".into(), ExclusionReason::CitesNonPublicRecord),
            ("cccc".into(), ExclusionReason::CitesRemovedSource),
            ("dddd".into(), ExclusionReason::CitesNonPublicRecord),
        ]
    );
    let ids: Vec<&str> = assembly
        .content
        .findings
        .iter()
        .map(|item| item.claim.finding_id.as_str())
        .collect();
    assert_eq!(ids, ["aaaa"]);
    let (_, html) = rendered(assembly);
    assert!(!html.contains(PRIVATE_MARKER) && !html.contains("removed.pdf"));
}

#[test]
fn hypotheses_dismissed_and_uncited_findings_are_not_shared() {
    let inferred = finding(
        "eeee",
        FindingKind::RedactionCandidateInferred,
        vec![citation("sd-city", 7)],
    );
    let mut dismissed = finding(
        "ffff",
        FindingKind::HeldBackResponse,
        vec![citation("sd-city", 7)],
    );
    dismissed.dismissed_at = Some(chrono::Utc::now());
    let uncited = finding("gggg", FindingKind::HeldBackResponse, vec![]);
    let assembly = assemble_public(vec![inferred, dismissed, uncited]);
    assert!(assembly.content.findings.is_empty());
    let reasons: Vec<ExclusionReason> = assembly.exclusions.iter().map(|e| e.reason).collect();
    assert_eq!(
        reasons,
        [
            ExclusionReason::InferredRedactionIdentity,
            ExclusionReason::DismissedByReviewer,
            ExclusionReason::NoCitation
        ]
    );
    assert_eq!(assembly.exclusions[0].headline, None);
}

#[test]
fn the_page_loads_nothing_and_runs_nothing() {
    let (_, html) = rendered(one_public_finding());
    for forbidden in [
        "http://", "https://", "//cdn", "href=", "src=", "action=", "<link", "<iframe", "<img",
        "@import", "url(", "<form", "<object", "<embed",
    ] {
        assert!(!html.contains(forbidden), "page contains {forbidden}");
    }
    assert_eq!(html.matches("<script").count(), 1);
    assert!(html.contains("type=\"application/json\""));
    assert!(html.contains("default-src 'none'"));
    assert!(
        !html.contains("<input") && !html.contains("<button") && !html.contains("contenteditable")
    );
    // The hostile `</script>` in a detector's prose must not end the data block early.
    assert_eq!(html.matches("</script>").count(), 1);
    assert!(html.contains("&lt;/script&gt;&lt;b&gt;"));
}

#[test]
fn the_embedded_data_round_trips_and_its_hash_detects_change() {
    let (document, html) = rendered(one_public_finding());
    let parsed = parse_embedded(&html).unwrap();
    assert_eq!(parsed, document);
    assert!(parsed.hash_is_intact().unwrap());
    assert_eq!(parsed.generated_at, "2026-02-03T04:05:06Z");
    assert_eq!(parsed.schema_version, BUNDLE_SCHEMA_VERSION);
    let mut tampered = parsed.clone();
    tampered.content.findings[0].claim.headline.push('!');
    assert!(!tampered.hash_is_intact().unwrap());
    // Time of writing is outside the hash: the same findings hash the same at any time.
    let later = BundleDocument::new(parsed.content.clone(), "2030-01-01T00:00:00Z".into()).unwrap();
    assert_eq!(later.content_sha256, parsed.content_sha256);
}

#[test]
fn an_unknown_schema_version_is_refused() {
    let (_, html) = rendered(one_public_finding());
    let future = html.replace("\"schemaVersion\": 1", "\"schemaVersion\": 2");
    assert_ne!(future, html);
    assert!(parse_embedded(&future).is_err());
}

#[test]
fn canonical_json_sorts_keys_at_every_depth() {
    let value = serde_json::json!({"b": [1, {"z": 1, "a": "x\"y"}], "a": null});
    assert_eq!(
        canonical_json(&value),
        r#"{"a":null,"b":[1,{"a":"x\"y","z":1}]}"#
    );
}

#[test]
fn embedded_packets_ride_inside_the_hash_without_a_schema_change() {
    let mut assembly = one_public_finding();
    let without = content_sha256(&assembly.content).unwrap();
    assembly.content.packets.push(EmbeddedPacket {
        packet_format: "example-packet".into(),
        packet_version: 1,
        packet_sha256: "b".repeat(64),
        payload: serde_json::json!({"claim": "x"}),
    });
    assert_ne!(content_sha256(&assembly.content).unwrap(), without);
    let (document, html) = rendered(assembly);
    assert_eq!(parse_embedded(&html).unwrap(), document);
}

#[tokio::test]
async fn a_selection_with_nothing_left_is_not_written() {
    let assembly = assemble_public(vec![finding("gggg", FindingKind::HeldBackResponse, vec![])]);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("b.html");
    let error = write_bundle(assembly, "x", path.clone(), "t".into()).await;
    assert!(error.is_err());
    assert!(!path.exists());
}

async fn repository() -> DuckDbRepository {
    let repo = DuckDbRepository::new(":memory:").unwrap();
    repo.run_migrations(include_str!("../../migrations/0001_initial_schema.sql"))
        .await
        .unwrap();
    repo.query_connection("seed bundle fixture", |conn| {
        conn.execute_batch(&format!(
            "INSERT INTO portals(subdomain,display_name,base_url,lifecycle_state) VALUES
               ('sd-city','City','https://www.sandiego.gov','ACTIVE'),
               ('gone','Gone','https://gone.example','REMOVED');
             INSERT INTO requests(id,portal_subdomain,title,status,created_at,updated_at) VALUES
               (1,'sd-city','R','closed','2026-01-01','2026-01-01'),
               (2,'gone','R','closed','2026-01-01','2026-01-01');
             INSERT INTO documents(id,request_id,portal_subdomain,file_name,download_url,visibility,created_at,updated_at,raw_file_sha256,extraction_status) VALUES
               (7,1,'sd-city','response.pdf','https://x.example/7','public','2026-01-01','2026-01-01',repeat('a',64),'SEARCHABLE'),
               (9,1,'sd-city','{PRIVATE_MARKER}.pdf','https://x.example/9','staff_only','2026-01-01','2026-01-01',repeat('b',64),'SEARCHABLE'),
               (1,2,'gone','removed.pdf','https://x.example/1','public','2026-01-01','2026-01-01',repeat('c',64),'SEARCHABLE');"
        ))
        .map_err(|error| crate::error::AppError::Storage(error.to_string()))
    })
    .await
    .unwrap();
    repo
}

#[tokio::test]
async fn a_written_bundle_reads_back_as_the_confirmed_content_and_holds_only_public_masked_text() {
    let repo = repository().await;
    let mut private = finding(
        "bbbb",
        FindingKind::HeldBackResponse,
        vec![citation("sd-city", 9)],
    );
    private.headline = format!("{PRIVATE_MARKER} headline");
    let removed = finding(
        "cccc",
        FindingKind::HeldBackResponse,
        vec![citation("gone", 1)],
    );
    let ok = finding(
        "aaaa",
        FindingKind::HeldBackResponse,
        vec![citation("sd-city", 7)],
    );
    repo.record_findings(vec![private, removed, ok])
        .await
        .unwrap();
    let ids = vec!["aaaa".into(), "bbbb".into(), "cccc".into(), "zzzz".into()];
    let disclosure = DisclosureContext::unset();

    let assembly = prepare(&repo, &disclosure, ids.clone()).await.unwrap();
    let preview = preview_of(&assembly).unwrap();
    assert_eq!(preview.included.len(), 1);
    assert_eq!(preview.included[0].finding_id, "aaaa");
    assert_eq!(preview.excluded.len(), 3);
    assert!(preview
        .excluded
        .iter()
        .any(|item| item.reason == ExclusionReason::NotFound));
    assert_eq!(preview.source_names, ["response.pdf"]);

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bundle.html");
    let result = write_bundle(
        assembly,
        &preview.content_sha256,
        path.clone(),
        "2026-03-04T05:06:07Z".into(),
    )
    .await
    .unwrap();
    assert_eq!(result.content_sha256, preview.content_sha256);
    assert_eq!((result.finding_count, result.excluded_count), (1, 3));

    let written = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        result.file_sha256,
        crate::integrity::sha256_hex(written.as_bytes())
    );
    let document = parse_embedded(&written).unwrap();
    assert!(document.hash_is_intact().unwrap());
    assert_eq!(document.content_sha256, preview.content_sha256);
    assert_eq!(document.content.findings.len(), 1);
    for forbidden in [
        RAW_SSN,
        RAW_PHONE,
        PRIVATE_MARKER,
        "removed.pdf",
        "https://",
    ] {
        assert!(!written.contains(forbidden), "{forbidden} reached the file");
    }
    assert!(written.contains("[MASKED_PHONE]"));

    // A changed selection no longer matches what was confirmed.
    let other = prepare(&repo, &disclosure, vec!["aaaa".into(), "bbbb".into()])
        .await
        .unwrap();
    let stale = write_bundle(
        other,
        "0".repeat(64).as_str(),
        dir.path().join("second.html"),
        "t".into(),
    )
    .await;
    assert!(stale.is_err());
    assert!(!dir.path().join("second.html").exists());
}

#[tokio::test]
async fn an_existing_file_is_never_replaced() {
    let assembly = one_public_finding();
    let hash = content_sha256(&assembly.content).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("b.html");
    std::fs::write(&path, "precious").unwrap();
    let result = write_bundle(assembly, &hash, path.clone(), "t".into()).await;
    assert!(result.is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "precious");
}

#[test]
fn an_unrecorded_file_digest_is_stated_not_omitted() {
    let mut assembly = one_public_finding();
    assembly.content.findings[0].claim.citations[0].raw_file_sha256 = None;
    let (_, html) = rendered(assembly);
    assert!(html.contains("File SHA-256 not recorded"));
}

#[test]
fn a_packet_carrying_reviewer_annotations_cannot_be_embedded() {
    let mut assembly = one_public_finding();
    assembly.content.packets.push(EmbeddedPacket {
        packet_format: "onrecord-evidence-packet-v1".into(),
        packet_version: 1,
        packet_sha256: "c".repeat(64),
        payload: serde_json::json!({"content": {"reviewerAnnotations": {"annotations": []}}}),
    });
    let document = BundleDocument::new(assembly.content, "t0".into()).unwrap();
    assert!(render_html(&document).is_err());
}
