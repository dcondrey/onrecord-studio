use super::builder::{build_manifest, ManifestOptions};
use super::{
    sign_and_render, template_sha256, SourceDocument, SourceDocuments, PAYLOAD_PLACEHOLDER,
};
use crate::analysis::finding::{
    Finding, FindingCitation, FindingConfidence, FindingKind, FindingSeverity, FindingSubject,
};
use crate::types::report::{QuoteStatus, ReportConfidence, WithheldReason};
use ed25519_dalek::SigningKey;
use std::path::PathBuf;
use std::process::Command;

const DOCUMENT_TEXT: &str = "The request was closed. Call the clerk at 619-555-0142 for the file. \
     Reference SSN 123-45-6789 appears in the attachment.";

fn finding(
    kind: FindingKind,
    confidence: FindingConfidence,
    citations: Vec<FindingCitation>,
) -> Finding {
    let now = chrono::DateTime::parse_from_rfc3339("2026-01-02T03:04:05Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    Finding {
        id: format!("{:0>20}", kind.as_str().len()),
        kind,
        severity: FindingSeverity::Notable,
        confidence,
        subject: FindingSubject {
            portal_key: Some("sd-city".into()),
            ..FindingSubject::default()
        },
        headline: format!("Headline for {}", kind.as_str()),
        explanation: "Explanation with </script><b>markup</b>".into(),
        citations,
        discovered_at: now,
        last_seen_at: now,
        dismissed_at: None,
        source_detector: "test".into(),
    }
}

fn citation(start: Option<i64>, end: Option<i64>, quote: Option<&str>) -> FindingCitation {
    FindingCitation {
        portal_key: "sd-city".into(),
        document_id: 7,
        character_start: start,
        character_end: end,
        quote: quote.map(str::to_string),
        label: None,
    }
}

fn documents(visibility: &str, text: Option<&str>) -> SourceDocuments {
    let mut map = SourceDocuments::new();
    map.insert(
        ("sd-city".into(), 7),
        SourceDocument {
            file_name: Some("response.pdf".into()),
            visibility: visibility.into(),
            raw_file_sha256: Some("a".repeat(64)),
            extracted_text_projection_sha256: None,
            text: text.map(str::to_string),
            download_url: None,
        },
    );
    map
}

fn options() -> ManifestOptions {
    ManifestOptions {
        template_sha256: template_sha256(),
        generated_at: "2026-01-02T03:04:05Z".into(),
        author_name: None,
        scope_portal_key: None,
        extra_limitations: Vec::new(),
        disclosure: crate::profile::DisclosureContext::unset(),
    }
}

fn char_range_of(needle: &str) -> (i64, i64) {
    let byte = DOCUMENT_TEXT.find(needle).expect("needle in fixture");
    let start = DOCUMENT_TEXT[..byte].chars().count() as i64;
    (start, start + needle.chars().count() as i64)
}

#[test]
fn inferred_redaction_identity_is_never_a_claim_and_its_text_is_not_reproduced() {
    let mut inferred = finding(
        FindingKind::RedactionCandidateInferred,
        FindingConfidence::measured(9_000),
        vec![],
    );
    inferred.headline = "Candidate identity Jordan Q. Example".into();
    inferred.explanation = "Jordan Q. Example likely fills the redaction".into();
    let manifest = build_manifest(
        vec![
            inferred,
            finding(
                FindingKind::BoilerplateDenial,
                FindingConfidence::measured(5_000),
                vec![],
            ),
        ],
        &SourceDocuments::new(),
        options(),
    )
    .unwrap();
    assert_eq!(manifest.claims.len(), 1);
    assert_eq!(manifest.withheld.len(), 1);
    assert_eq!(
        manifest.withheld[0].reason,
        WithheldReason::InferredRedactionIdentity
    );
    let serialized = serde_json::to_string(&manifest).unwrap();
    assert!(!serialized.contains("Jordan"));
    assert!(!serialized.contains("REDACTION_CANDIDATE_INFERRED"));
}

#[test]
fn quotes_are_masked_with_whole_document_context_and_verified_against_the_range() {
    let (start, end) = char_range_of("SSN 123-45-6789 appears");
    let manifest = build_manifest(
        vec![finding(
            FindingKind::BoilerplateDenial,
            FindingConfidence::measured(5_000),
            vec![citation(
                Some(start),
                Some(end),
                Some("SSN 123-45-6789 appears"),
            )],
        )],
        &documents("public", Some(DOCUMENT_TEXT)),
        options(),
    )
    .unwrap();
    let cited = &manifest.claims[0].citations[0];
    assert_eq!(cited.quote_status, QuoteStatus::VerifiedAgainstStoredText);
    let quote = cited.quote.as_deref().unwrap();
    assert!(quote.contains("[MASKED_SSN]"), "{quote}");
    assert!(!quote.contains("123-45-6789"));
    assert!(cited.pii_masked_count >= 1);
    assert!(!serde_json::to_string(&manifest)
        .unwrap()
        .contains("123-45-6789"));
}

#[test]
fn a_range_that_cuts_through_a_phone_number_still_masks_it() {
    let (start, _) = char_range_of("619-555-0142");
    // Range ends mid-number; detection over the pre-sliced quote would miss it.
    let manifest = build_manifest(
        vec![finding(
            FindingKind::BoilerplateDenial,
            FindingConfidence::measured(5_000),
            vec![citation(Some(start - 8), Some(start + 7), None)],
        )],
        &documents("public", Some(DOCUMENT_TEXT)),
        options(),
    )
    .unwrap();
    let quote = manifest.claims[0].citations[0].quote.clone().unwrap();
    assert!(!quote.contains("619-555"), "{quote}");
}

#[test]
fn a_quote_absent_from_the_cited_range_is_withheld_not_published() {
    let (start, end) = char_range_of("The request was closed.");
    let manifest = build_manifest(
        vec![finding(
            FindingKind::BoilerplateDenial,
            FindingConfidence::measured(5_000),
            vec![citation(
                Some(start),
                Some(end),
                Some("the request was DENIED"),
            )],
        )],
        &documents("public", Some(DOCUMENT_TEXT)),
        options(),
    )
    .unwrap();
    let cited = &manifest.claims[0].citations[0];
    assert_eq!(cited.quote_status, QuoteStatus::WithheldMismatch);
    assert!(cited.quote.is_none());
    assert!(manifest
        .withheld
        .iter()
        .any(|item| item.reason == WithheldReason::QuoteMismatch));
}

#[test]
fn a_non_public_document_contributes_no_excerpt_name_or_digest() {
    let manifest = build_manifest(
        vec![finding(
            FindingKind::BoilerplateDenial,
            FindingConfidence::measured(5_000),
            vec![citation(Some(0), Some(10), Some("secret text"))],
        )],
        &documents("staff_only", Some(DOCUMENT_TEXT)),
        options(),
    )
    .unwrap();
    let cited = &manifest.claims[0].citations[0];
    assert_eq!(cited.quote_status, QuoteStatus::WithheldNonPublicDocument);
    assert!(cited.quote.is_none() && cited.file_name.is_none() && cited.raw_file_sha256.is_none());
}

#[test]
fn unassessed_confidence_never_serializes_as_a_measurement() {
    let manifest = build_manifest(
        vec![
            finding(
                FindingKind::BoilerplateDenial,
                FindingConfidence::Unassessed,
                vec![],
            ),
            finding(
                FindingKind::ExtensionAbuse,
                FindingConfidence::measured(0),
                vec![],
            ),
        ],
        &SourceDocuments::new(),
        options(),
    )
    .unwrap();
    assert_eq!(manifest.claims[0].confidence, ReportConfidence::Unassessed);
    assert_eq!(
        manifest.claims[1].confidence,
        ReportConfidence::Measured { basis_points: 0 }
    );
    let unassessed = serde_json::to_value(manifest.claims[0].confidence).unwrap();
    assert_eq!(unassessed, serde_json::json!({ "state": "UNASSESSED" }));
}

#[test]
fn a_legal_claim_without_measured_confidence_is_withheld() {
    let manifest = build_manifest(
        vec![finding(
            FindingKind::PolicyViolationDetected,
            FindingConfidence::Unassessed,
            vec![],
        )],
        &SourceDocuments::new(),
        options(),
    )
    .unwrap();
    assert!(manifest.claims.is_empty());
    assert_eq!(
        manifest.withheld[0].reason,
        WithheldReason::LegalClaimWithoutMeasuredConfidence
    );
}

fn node_verify(path: &std::path::Path) -> std::process::Output {
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../scripts/verify-report.mjs");
    Command::new("node")
        .arg(script)
        .arg(path)
        .output()
        .expect("node must be installed to run the report round-trip test")
}

#[test]
fn round_trip_verifies_and_every_tampered_byte_class_fails() {
    let (start, end) = char_range_of("SSN 123-45-6789 appears");
    let manifest = build_manifest(
        vec![
            finding(
                FindingKind::BoilerplateDenial,
                FindingConfidence::measured(5_000),
                vec![citation(
                    Some(start),
                    Some(end),
                    Some("SSN 123-45-6789 appears"),
                )],
            ),
            finding(
                FindingKind::ExtensionAbuse,
                FindingConfidence::Unassessed,
                vec![],
            ),
        ],
        &documents("public", Some(DOCUMENT_TEXT)),
        options(),
    )
    .unwrap();
    let rendered = sign_and_render(manifest, &SigningKey::from_bytes(&[7_u8; 32])).unwrap();
    // The `</script>` inside detector prose must not add a script terminator to the page.
    assert_eq!(
        rendered.html.matches("</script>").count(),
        super::TEMPLATE.matches("</script>").count()
    );
    assert!(!rendered.html.contains(PAYLOAD_PLACEHOLDER));

    let directory = tempfile::tempdir().unwrap();
    let write = |name: &str, html: &str| {
        let path = directory.path().join(name);
        std::fs::write(&path, html).unwrap();
        path
    };
    let good = node_verify(&write("good.html", &rendered.html));
    assert!(
        good.status.success(),
        "{}",
        String::from_utf8_lossy(&good.stdout)
    );

    let flip_after = |marker: &str| {
        let mut bytes = rendered.html.clone().into_bytes();
        let at = rendered.html.find(marker).expect(marker) + marker.len();
        bytes[at] = if bytes[at] == b'A' { b'B' } else { b'A' };
        String::from_utf8(bytes).unwrap()
    };
    let tampered = [
        // Inside the signed bytes.
        (
            "canonical.html",
            flip_after("\"manifestCanonicalBase64\":\""),
        ),
        // Visible claim text in the unsigned mirror of the manifest.
        ("mirror.html", flip_after("\"headline\":\"")),
        // The page's own markup, outside the payload entirely.
        ("template.html", flip_after("<title>")),
    ];
    for (name, html) in tampered {
        let output = node_verify(&write(name, &html));
        assert_eq!(
            output.status.code(),
            Some(1),
            "{name}: {}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
}

#[test]
fn the_disclosure_is_inside_the_signed_payload_and_editing_it_breaks_verification() {
    use crate::profile::matching::ConfirmedSubjects;
    use crate::profile::{build_disclosure, DisclosureContext};
    use crate::types::profile::{
        Affiliation, AffiliationBasis, UserProfile, PROFILE_SCHEMA_VERSION,
    };
    let profile = UserProfile {
        schema_version: PROFILE_SCHEMA_VERSION,
        display_name: None,
        affiliations: vec![Affiliation {
            id: "a".into(),
            name: "City Clerk".into(),
            aliases: vec![],
            basis: AffiliationBasis::SelfDeclared,
            created_at: "2026-01-01T00:00:00Z".into(),
            verified_domain: None,
        }],
        match_decisions: vec![],
        verified_domains: vec![],
        updated_at: "2026-01-01T00:00:00Z".into(),
    };
    let mut opts = options();
    opts.disclosure = DisclosureContext::from_disclosure(&build_disclosure(
        Some(&profile),
        ConfirmedSubjects {
            portal_keys: ["sd-city".to_string()].into(),
            ..Default::default()
        },
    ));
    let claim = finding(
        FindingKind::AbsenceOfRecords,
        FindingConfidence::measured(5_000),
        vec![],
    );
    let claim_id = claim.id.clone();
    let baseline = build_manifest(vec![claim.clone()], &SourceDocuments::new(), options()).unwrap();
    let manifest = build_manifest(vec![claim], &SourceDocuments::new(), opts).unwrap();
    let sentence = "Prepared by a user who has declared an affiliation with City Clerk (self-declared, not verified).";
    assert_eq!(manifest.limitations[0], sentence);
    assert!(manifest.limitations[1].contains("subject of this document"));
    assert!(manifest.limitations[1].contains(&claim_id));
    assert_eq!(
        manifest.claims, baseline.claims,
        "claim bodies do not depend on the profile"
    );

    let key = SigningKey::from_bytes(&[7_u8; 32]);
    let package = crate::integrity::sign_package(manifest, &key).unwrap();
    assert!(crate::integrity::verify_package(&package).is_ok());
    let mut tampered = package.clone();
    tampered.manifest.limitations[0] = "Prepared by an independent party.".to_string();
    assert!(crate::integrity::verify_package(&tampered).is_err());
}
