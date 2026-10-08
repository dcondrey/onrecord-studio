use super::*;
use crate::analysis::finding::{
    Finding, FindingCitation, FindingConfidence, FindingKind, FindingSeverity, FindingSubject,
};
use crate::analysis::gaps::{EvidenceGap, GapRole};
use crate::report::builder::{build_manifest, ManifestOptions};
use crate::report::{template_sha256, SourceDocument, SourceDocuments};

const GAP_ROUTE: &str = "Gap route sentinel about the vendor register";
const QUOTE: &str = "The request was closed on March 3.";

fn at(seconds: i64) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::from_timestamp(1_767_000_000 + seconds, 0).unwrap()
}

fn finding(id: &str, kind: FindingKind, confidence: FindingConfidence, quote: &str) -> Finding {
    Finding {
        id: id.to_string(),
        kind,
        severity: FindingSeverity::Notable,
        confidence,
        subject: FindingSubject {
            portal_key: Some("sd-city".into()),
            ..FindingSubject::default()
        },
        headline: format!("Headline {id}"),
        explanation: format!("Compared the closing date with the deadline for {id}."),
        citations: vec![FindingCitation {
            portal_key: "sd-city".into(),
            document_id: 7,
            character_start: None,
            character_end: None,
            quote: Some(quote.to_string()),
            label: None,
        }],
        discovered_at: at(0),
        last_seen_at: at(0),
        dismissed_at: None,
        source_detector: "policy_violation".into(),
    }
}

fn pattern(id: &str) -> Finding {
    finding(
        id,
        FindingKind::HeldBackResponse,
        FindingConfidence::Unassessed,
        QUOTE,
    )
}

fn legal(id: &str) -> Finding {
    finding(
        id,
        FindingKind::PolicyViolationDetected,
        FindingConfidence::measured(8_250),
        QUOTE,
    )
}

fn documents(visibility: &str, url: Option<&str>) -> SourceDocuments {
    let mut map = SourceDocuments::new();
    map.insert(
        ("sd-city".into(), 7),
        SourceDocument {
            file_name: Some("response.pdf".into()),
            visibility: visibility.into(),
            raw_file_sha256: Some("a".repeat(64)),
            extracted_text_projection_sha256: None,
            text: None,
            download_url: url.map(str::to_string),
        },
    );
    map
}

fn gap() -> EvidenceGap {
    EvidenceGap {
        detector: "policy_violation",
        key: "policy_violation:sd-city:7".into(),
        route: GAP_ROUTE.into(),
        blocked: "Whether payments exceeded the ceiling.".into(),
        role: GapRole::Payments,
        request: "Provide the existing payment ledger.".into(),
    }
}

fn packet_with(
    findings: Vec<Finding>,
    docs: &SourceDocuments,
    gaps: Vec<EvidenceGap>,
    generated_at: &str,
) -> EvidencePacket {
    let manifest = build_manifest(
        findings,
        docs,
        ManifestOptions {
            template_sha256: template_sha256(),
            generated_at: generated_at.to_string(),
            author_name: Some("A. Author".into()),
            scope_portal_key: None,
            extra_limitations: Vec::new(),
            disclosure: crate::profile::DisclosureContext::unset(),
        },
    )
    .unwrap();
    build_packet(
        manifest,
        docs,
        gaps,
        &[],
        None,
        generated_at.to_string(),
        "1.2.3",
    )
    .unwrap()
}

fn packet(findings: Vec<Finding>) -> EvidencePacket {
    packet_with(findings, &documents("public", None), Vec::new(), "t0")
}

#[test]
fn the_hash_ignores_order_generation_time_timestamps_and_build() {
    let first = packet(vec![pattern("b-finding"), legal("a-finding")]);
    let mut refreshed_b = pattern("b-finding");
    refreshed_b.last_seen_at = at(86_400);
    let mut second = packet_with(
        vec![legal("a-finding"), refreshed_b],
        &documents("public", None),
        Vec::new(),
        "a later time",
    );
    second.build_version = "9.9.9".into();
    assert_eq!(first.content_sha256, second.content_sha256);
    assert!(hash_matches(&first));
}

#[test]
fn the_hash_changes_when_a_quote_or_a_confidence_changes() {
    let base = packet(vec![pattern("x")]);
    let other_quote = packet(vec![finding(
        "x",
        FindingKind::HeldBackResponse,
        FindingConfidence::Unassessed,
        "A different sentence.",
    )]);
    let zero_confidence = packet(vec![finding(
        "x",
        FindingKind::HeldBackResponse,
        FindingConfidence::measured(0),
        QUOTE,
    )]);
    assert_ne!(base.content_sha256, other_quote.content_sha256);
    // Not assessed and measured zero are different statements and must hash differently.
    assert_ne!(base.content_sha256, zero_confidence.content_sha256);
}

#[test]
fn the_json_a_recipient_holds_recomputes_to_the_stored_hash() {
    let original = packet(vec![legal("a"), pattern("b")]);
    let decoded: EvidencePacket =
        serde_json::from_slice(&serde_json::to_vec_pretty(&original).unwrap()).unwrap();
    assert!(hash_matches(&decoded));
    let mut altered = decoded;
    altered.content.claims[0].headline.push('!');
    assert!(!hash_matches(&altered));
}

fn claim_blocks(markdown: &str) -> Vec<&str> {
    let claims = markdown
        .split("\n## Evidence gaps")
        .next()
        .expect("claims section");
    claims
        .split(&format!("\n{CLAIM_HEADING_PREFIX}"))
        .skip(1)
        .collect()
}

#[test]
fn every_rendered_claim_states_its_basis_and_confidence_state() {
    let markdown = render_markdown(&packet(vec![legal("a"), pattern("b")]));
    let blocks = claim_blocks(&markdown);
    assert_eq!(blocks.len(), 2);
    let counted = ReportEvidenceBasis::CountedViolation.label();
    let signal = ReportEvidenceBasis::PatternSignal.label();
    let legal_block = blocks.iter().find(|b| b.contains("Headline a")).unwrap();
    let pattern_block = blocks.iter().find(|b| b.contains("Headline b")).unwrap();
    assert!(legal_block.contains(&format!("- Basis: {counted}")));
    assert!(!legal_block.contains(signal));
    assert!(legal_block.contains("Measured 82.50%"));
    assert!(pattern_block.contains(&format!("- Basis: {signal}")));
    assert!(!pattern_block.contains(counted));
    assert!(pattern_block.contains("Not assessed"));
    assert!(!pattern_block.contains("0.00%"));
    for block in blocks {
        assert!(block.contains("What this does not show"));
        assert!(block.contains("Detector: `policy_violation`"));
    }
}

#[test]
fn a_gap_is_listed_apart_from_claims_and_never_counted_as_one() {
    let findings = || vec![legal("a"), pattern("b")];
    let docs = documents("public", None);
    let without = packet_with(findings(), &docs, Vec::new(), "t0");
    let with = packet_with(findings(), &docs, vec![gap()], "t0");
    let markdown = render_markdown(&with);
    let (claims_part, rest) = markdown.split_once("\n## Evidence gaps").unwrap();
    assert!(!claims_part.contains(GAP_ROUTE));
    assert!(rest.contains(GAP_ROUTE));
    assert_eq!(with.content.claims.len(), without.content.claims.len());
    assert!(markdown.contains("## Claims (2)"));
    assert_eq!(claim_blocks(&markdown).len(), 2);
    // The gap region carries no claim heading, basis label, or confidence statement.
    let gap_region = rest.split("\n## Withheld").next().unwrap();
    assert!(!gap_region.contains(CLAIM_HEADING_PREFIX));
    assert!(!gap_region.contains("Basis:"));
    assert!(!gap_region.contains("Confidence:"));
    // A gap cannot be mistaken for a finding in the machine-readable form either.
    assert!(with
        .content
        .claims
        .iter()
        .all(|claim| claim.finding_id != with.content.gaps[0].key));
    assert_ne!(without.content_sha256, with.content_sha256);
}

#[test]
fn a_gap_of_a_detector_with_no_claim_is_not_attached() {
    let mut unrelated = gap();
    unrelated.detector = "spending_cases";
    let packet = packet_with(
        vec![pattern("a")],
        &documents("public", None),
        vec![unrelated],
        "t0",
    );
    assert!(packet.content.gaps.is_empty());
    assert!(render_markdown(&packet).contains("That is not a statement that none exist"));
}

#[test]
fn a_failed_gap_sweep_is_disclosed_not_read_as_no_gaps() {
    let docs = documents("public", None);
    let manifest = build_manifest(
        vec![pattern("a")],
        &docs,
        ManifestOptions {
            template_sha256: template_sha256(),
            generated_at: "t0".into(),
            author_name: None,
            scope_portal_key: None,
            extra_limitations: Vec::new(),
            disclosure: crate::profile::DisclosureContext::unset(),
        },
    )
    .unwrap();
    let failures = vec!["policy_violation gap sweep failed: database is locked".to_string()];
    let packet = build_packet(
        manifest,
        &docs,
        Vec::new(),
        &failures,
        None,
        "t0".into(),
        "1",
    )
    .unwrap();
    let limits = packet.content.limitations.join("\n");
    assert!(limits.contains("policy_violation"));
    assert!(limits.contains("may be incomplete"));
    assert!(!limits.contains("database is locked"));
}

#[test]
fn download_urls_drop_query_strings_and_are_public_only() {
    let signed = "https://files.example.gov/a/b.pdf?X-Amz-Signature=secret#frag";
    let public = packet_with(
        vec![pattern("a")],
        &documents("public", Some(signed)),
        Vec::new(),
        "t0",
    );
    let source = &public.content.claims[0].sources[0];
    assert_eq!(
        source.download_url.as_deref(),
        Some("https://files.example.gov/a/b.pdf")
    );
    let private = packet_with(
        vec![pattern("a")],
        &documents("staff_only", Some(signed)),
        Vec::new(),
        "t0",
    );
    let source = &private.content.claims[0].sources[0];
    assert_eq!(source.download_url, None);
    assert_eq!(source.file_name, None);
    let json = serde_json::to_string(&private).unwrap();
    assert!(!json.contains("Signature"));
    assert_eq!(public_url("https://user:pw@host.example/x"), None);
    assert_eq!(public_url("file:///etc/passwd"), None);
}

#[test]
fn what_was_not_observed_is_stated_per_claim() {
    let packet = packet(vec![pattern("a")]);
    let lines = packet.content.claims[0].does_not_show.join("\n");
    assert!(lines.contains("not checked against stored text"));
    assert!(lines.contains("extracted-text digest"));
    assert!(lines.contains("No download URL is recorded"));
    assert!(lines.contains("No confidence was assessed"));
    assert!(!packet
        .content
        .limitations
        .iter()
        .any(|line| line.contains("signature shows")));
}

fn heading_lines(markdown: &str) -> usize {
    markdown
        .replace('\r', "\n")
        .lines()
        .filter(|line| line.starts_with(CLAIM_HEADING_PREFIX))
        .count()
}

#[test]
fn quoted_text_cannot_forge_a_claim_heading_or_open_html() {
    let mut forged = pattern("x");
    forged.explanation =
        "ok\n### Claim 99: forged\r### Claim 9: forged\r<img src=x onerror=alert(1)>".into();
    forged.headline = "<script>alert(1)</script> headline".into();
    forged.citations[0].quote = Some("line\r### Claim 8: forged quote\n<b>bold</b>".into());
    let markdown = render_markdown(&packet(vec![forged]));
    assert_eq!(heading_lines(&markdown), 1);
    assert!(!markdown.contains('<'));
}

#[test]
fn a_download_url_carrying_a_personal_value_is_not_reproduced() {
    let docs = documents("public", Some("https://x.example/files/123-45-6789.pdf"));
    let packet = packet_with(vec![pattern("x")], &docs, Vec::new(), "t0");
    assert_eq!(packet.content.claims[0].sources[0].download_url, None);
    let clean = documents("public", Some("https://x.example/files/7.pdf?token=s"));
    let packet = packet_with(vec![pattern("x")], &clean, Vec::new(), "t0");
    assert_eq!(
        packet.content.claims[0].sources[0].download_url.as_deref(),
        Some("https://x.example/files/7.pdf")
    );
}

mod reviewer_annotations {
    use super::*;
    use crate::report::packet::annotations::{build_section, SECTION_KEY, SECTION_TITLE};
    use crate::types::annotation::{
        AnchorState, AnnotationDocument, AnnotationKind, AnnotationStatus, DocumentAnnotation,
    };
    use std::collections::BTreeSet;

    const TEXT: &str = "The request was closed on March 3. Fees were waived in full.";

    fn mark(
        id: &str,
        document_id: u64,
        quote: &str,
        offset: u64,
        note: &str,
    ) -> DocumentAnnotation {
        DocumentAnnotation {
            id: id.into(),
            portal_key: "sd-city".into(),
            document_id,
            kind: AnnotationKind::Issue,
            character_offset: Some(offset),
            quote: quote.into(),
            prefix: String::new(),
            suffix: String::new(),
            text_sha256: None,
            context_unanchored: false,
            note: note.into(),
            tag: Some("CLOSED".into()),
            author: Some("Reviewer A".into()),
            status: AnnotationStatus::Disputed,
            finding_id: None,
            imported_from_key: None,
            created_at: "2026-01-01T00:00:00Z".into(),
            page_region: None,
        }
    }

    fn docs_with_text() -> SourceDocuments {
        let mut docs = documents("public", None);
        docs.get_mut(&("sd-city".to_string(), 7)).unwrap().text = Some(TEXT.into());
        docs
    }

    fn store() -> AnnotationDocument {
        AnnotationDocument {
            annotations: vec![
                mark(
                    "a-found",
                    7,
                    "closed on March 3",
                    16,
                    "Date matches the log.",
                ),
                mark(
                    "b-gone",
                    7,
                    "text that was edited away",
                    40,
                    "Private view of this.",
                ),
                mark("c-other-doc", 8, "unrelated", 0, "Not cited."),
            ],
            ..AnnotationDocument::default()
        }
    }

    fn cited() -> BTreeSet<(String, i64)> {
        BTreeSet::from([("sd-city".to_string(), 7)])
    }

    fn with_section(docs: &SourceDocuments, store: &AnnotationDocument) -> EvidencePacket {
        let manifest = build_manifest(
            vec![legal("a-finding")],
            docs,
            ManifestOptions {
                template_sha256: template_sha256(),
                generated_at: "t0".into(),
                author_name: None,
                scope_portal_key: None,
                extra_limitations: Vec::new(),
                disclosure: crate::profile::DisclosureContext::unset(),
            },
        )
        .unwrap();
        let section = build_section(store, &cited(), docs).unwrap();
        build_packet(
            manifest,
            docs,
            Vec::new(),
            &[],
            Some(section),
            "t0".into(),
            "1",
        )
        .unwrap()
    }

    #[test]
    fn a_packet_without_the_section_is_byte_identical_and_names_none_of_it() {
        let docs = docs_with_text();
        let first = packet_with(vec![legal("a-finding")], &docs, Vec::new(), "t0");
        let second = packet_with(vec![legal("a-finding")], &docs, Vec::new(), "t0");
        let json = serde_json::to_vec_pretty(&first).unwrap();
        assert_eq!(json, serde_json::to_vec_pretty(&second).unwrap());
        assert!(!String::from_utf8(json).unwrap().contains(SECTION_KEY));
        assert!(!render_markdown(&first).contains(SECTION_TITLE));
    }

    #[test]
    fn an_orphan_is_kept_and_labelled_and_the_hash_covers_the_section() {
        let docs = docs_with_text();
        let included = with_section(&docs, &store());
        let section = included.content.reviewer_annotations.as_ref().unwrap();
        let states: Vec<_> = section
            .annotations
            .iter()
            .map(|mark| (mark.id.as_str(), mark.anchor))
            .collect();
        assert_eq!(
            states,
            [
                ("a-found", Some(AnchorState::Anchored)),
                ("b-gone", Some(AnchorState::Orphaned))
            ]
        );
        let markdown = render_markdown(&included);
        assert!(markdown.contains(SECTION_TITLE));
        assert!(markdown.contains("`Orphaned`") && markdown.contains("Private view of this."));

        let without = packet_with(vec![legal("a-finding")], &docs, Vec::new(), "t0");
        assert_ne!(included.content_sha256, without.content_sha256);
        assert!(hash_matches(&included));

        let mut edited = store();
        edited.annotations[0].note = "A different note.".into();
        assert_ne!(
            with_section(&docs, &edited).content_sha256,
            included.content_sha256
        );
    }

    #[test]
    fn a_page_region_mark_is_located_by_page_never_text_anchored() {
        let docs = docs_with_text();
        let mut store = store();
        // The text layer's words happen to match the document text; that must not anchor it.
        let mut region = mark("c-region", 7, "alpha beta", 0, "On the page.");
        region.page_region = Some(crate::types::annotation::PageRegion {
            rectangle: crate::types::annotation::PageRectangle {
                page_index: 2,
                x: 0.1,
                y: 0.2,
                width: 0.3,
                height: 0.1,
            },
            original_sha256: "ab".repeat(32),
        });
        store.annotations.push(region);
        let section = with_section(&docs, &store)
            .content
            .reviewer_annotations
            .unwrap();
        let region = section
            .annotations
            .iter()
            .find(|mark| mark.id == "c-region")
            .unwrap();
        assert_eq!(region.anchor, None);
        assert!(region
            .unchecked_reason
            .as_deref()
            .unwrap()
            .contains("page and rectangle"));
        assert_eq!(region.page_region.as_ref().unwrap().rectangle.page_index, 2);
    }

    #[test]
    fn marks_on_a_non_public_document_are_counted_not_written() {
        let section = build_section(&store(), &cited(), &documents("private", None)).unwrap();
        assert!(section.annotations.is_empty());
        assert_eq!(section.not_public_omitted, 2);
    }
}
