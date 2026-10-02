use super::super::{
    objects::parser_memory_permits,
    payload::{document_date_for_guidance, evaluate_extracted_document_guidance},
};
use super::*;

#[tokio::test]
async fn release_stress_harness_applies_parser_memory_backpressure() {
    assert_eq!(
        parser_memory_permits(u64::MAX, u32::MAX),
        PARSER_MEMORY_PERMITS
    );
    let semaphore = Arc::new(Semaphore::new(PARSER_MEMORY_PERMITS as usize));
    let all_memory = Arc::clone(&semaphore)
        .acquire_many_owned(PARSER_MEMORY_PERMITS)
        .await
        .unwrap();
    assert!(semaphore.try_acquire().is_err());
    drop(all_memory);
    assert_eq!(
        semaphore.available_permits(),
        PARSER_MEMORY_PERMITS as usize
    );
}

#[test]
fn only_nextrequest_metadata_supplies_a_document_date_for_guidance() {
    let timestamp = "2024-06-01T00:00:00Z";
    assert_eq!(
        document_date_for_guidance(SourceKind::NextRequest, timestamp),
        Some("2024-06-01")
    );
    assert_eq!(
        document_date_for_guidance(SourceKind::NextRequest, "2024-02-30T00:00:00Z"),
        None
    );
    assert_eq!(
        document_date_for_guidance(SourceKind::NextRequest, "not-a-date"),
        None
    );
    for kind in [
        SourceKind::WebCrawl,
        SourceKind::DirectFile,
        SourceKind::OpenDataCatalog,
        SourceKind::ArcGis,
        SourceKind::Tableau,
        SourceKind::GoogleDrive,
        SourceKind::LocalFile,
        SourceKind::PastedContent,
    ] {
        assert_eq!(document_date_for_guidance(kind, timestamp), None);
    }
}

/// Each post-extraction predicate must report the rule that actually fired rather than a
/// shared or wrong variant -- a content-pattern refusal reported as `ContentKeyword` (or vice
/// versa) would misname the rule an operator sees in the refusal count. Proven per predicate;
/// deleting any one branch's distinct return would collapse two of these to the same variant
/// and fail the corresponding assertion.
#[test]
fn evaluate_extracted_document_guidance_names_the_predicate_that_fired() {
    let base = SourceGuidance::default();
    let compiled_base = CompiledGuidancePatterns::compile(&base);

    let keyword_guidance = SourceGuidance {
        exclude_content_keywords: vec!["classified".to_string()],
        ..SourceGuidance::default()
    };
    assert_eq!(
        evaluate_extracted_document_guidance(
            &keyword_guidance,
            &CompiledGuidancePatterns::compile(&keyword_guidance),
            "this record is classified pending review",
            None,
            Some("2024-06-01T00:00:00Z"),
        ),
        Err(FrontierExclusion::ContentKeyword)
    );

    let pattern_guidance = SourceGuidance {
        exclude_content_patterns: vec![r"\bSSN\b".to_string()],
        ..SourceGuidance::default()
    };
    assert_eq!(
        evaluate_extracted_document_guidance(
            &pattern_guidance,
            &CompiledGuidancePatterns::compile(&pattern_guidance),
            "applicant SSN on file",
            None,
            Some("2024-06-01T00:00:00Z"),
        ),
        Err(FrontierExclusion::ContentPattern)
    );

    let organization_guidance = SourceGuidance {
        include_organizations: vec!["Parks".to_string()],
        ..SourceGuidance::default()
    };
    assert_eq!(
        evaluate_extracted_document_guidance(
            &organization_guidance,
            &CompiledGuidancePatterns::compile(&organization_guidance),
            "unrelated content",
            Some("Department of Public Works"),
            Some("2024-06-01T00:00:00Z"),
        ),
        Err(FrontierExclusion::Organization)
    );

    let date_guidance = SourceGuidance {
        document_date_from: Some("2025-01-01".to_string()),
        ..SourceGuidance::default()
    };
    assert_eq!(
        evaluate_extracted_document_guidance(
            &date_guidance,
            &CompiledGuidancePatterns::compile(&date_guidance),
            "unrelated content",
            None,
            Some("2024-06-01T00:00:00Z"),
        ),
        Err(FrontierExclusion::DocumentDate)
    );

    assert_eq!(
        evaluate_extracted_document_guidance(
            &base,
            &compiled_base,
            "unrelated content",
            None,
            Some("2024-06-01T00:00:00Z"),
        ),
        Ok(())
    );

    // A missing document-carried date is an absent measurement, not evidence that the
    // document falls outside the range. Non-NextRequest ingestion paths rely on this behavior
    // because their metadata date is only the ingestion instant.
    assert_eq!(
        evaluate_extracted_document_guidance(
            &date_guidance,
            &CompiledGuidancePatterns::compile(&date_guidance),
            "unrelated content",
            None,
            None,
        ),
        Ok(())
    );
}
