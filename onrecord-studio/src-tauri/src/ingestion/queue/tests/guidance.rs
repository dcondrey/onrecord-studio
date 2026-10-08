use super::super::guidance::{
    glob_matches, guidance_allows_url, validate_page_parameters, validate_source_guidance,
};
use super::*;

#[test]
fn glob_matching_is_linear_and_keeps_its_wildcard_semantics() {
    assert!(glob_matches("*.pdf", "report.pdf"));
    assert!(glob_matches(
        "budget-*-final.xlsx",
        "budget-2024-final.xlsx"
    ));
    assert!(glob_matches("*", ""));
    assert!(glob_matches("a**b", "ab"));
    assert!(glob_matches("*a*b*", "xxaxxbxx"));
    assert!(!glob_matches("*.pdf", "report.pdf.exe"));
    assert!(!glob_matches("a*b", "ba"));
    assert!(!glob_matches("abc", "ab"));
    assert!(!glob_matches("", "a"));
    assert!(glob_matches("", ""));
    // A recursive matcher needs on the order of C(60, 12) steps to reject this.
    let pattern = format!("{}b", "*a".repeat(12));
    let text = "a".repeat(60);
    let started = std::time::Instant::now();
    assert!(!glob_matches(&pattern, &text));
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
}

#[test]
fn validates_page_bounds_at_the_queue_boundary() {
    assert!(validate_page_parameters(1, 1).is_ok());
    assert!(validate_page_parameters(MAXIMUM_METADATA_PAGE, 100).is_ok());
    assert!(validate_page_parameters(0, 50).is_err());
    assert!(validate_page_parameters(MAXIMUM_METADATA_PAGE + 1, 50).is_err());
    assert!(validate_page_parameters(1, 0).is_err());
    assert!(validate_page_parameters(1, 101).is_err());
}

/// A filename rule must name itself in the refusal count, not fall back to the generic
/// `Guidance` variant that predates it. Proven against the pre-wiring behavior: before the
/// filename glob/regex checks were added, every guidance refusal -- including this one --
/// returned `FrontierExclusion::Guidance`, which would make this assertion fail.
#[test]
fn guidance_allows_url_reports_the_specific_filename_rule_not_the_generic_one() {
    // The rule sees the decoded name, so a glob written as the name displays matches its escape.
    for (glob, url) in [
        ("*.exe", "https://records.example.test/files/installer.exe"),
        (
            "*résumé*",
            "https://records.example.test/files/R%C3%A9sum%C3%A9.pdf",
        ),
    ] {
        let guidance = SourceGuidance {
            exclude_filename_globs: vec![glob.to_string()],
            ..Default::default()
        };
        let compiled = CompiledGuidancePatterns::compile(&guidance);
        let result = guidance_allows_url(&guidance, &compiled, url, None);
        assert_eq!(result, Err(FrontierExclusion::FilenamePattern), "{glob}");
    }
}

/// The size predicate must not fire on an unknown size: excluding on an absent measurement is
/// exactly the absence-versus-measurement violation this project forbids elsewhere. A caller
/// that has no `content_length` (most discovered links) must see the URL admitted even when a
/// size range is configured.
#[test]
fn guidance_allows_url_does_not_exclude_on_an_unknown_size() {
    let guidance = SourceGuidance {
        minimum_file_size_bytes: Some(1_000),
        maximum_file_size_bytes: Some(2_000),
        ..Default::default()
    };
    let compiled = CompiledGuidancePatterns::compile(&guidance);
    let result = guidance_allows_url(
        &guidance,
        &compiled,
        "https://records.example.test/files/report.pdf",
        None,
    );
    assert_eq!(result, Ok(()));
}

#[test]
fn source_guidance_requires_valid_ordered_calendar_dates() {
    let invalid_date = SourceGuidance {
        document_date_from: Some("2026-02-30".to_string()),
        ..Default::default()
    };
    assert!(validate_source_guidance(&invalid_date).is_err());

    let reversed_range = SourceGuidance {
        document_date_from: Some("2026-08-28".to_string()),
        document_date_to: Some("2026-08-27".to_string()),
        ..Default::default()
    };
    assert!(validate_source_guidance(&reversed_range).is_err());

    let valid_range = SourceGuidance {
        document_date_from: Some("2024-02-29".to_string()),
        document_date_to: Some("2026-08-27".to_string()),
        ..Default::default()
    };
    assert!(validate_source_guidance(&valid_range).is_ok());
}
