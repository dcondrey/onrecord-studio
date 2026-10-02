//! Validation of crawl policy, source guidance, and page windows, and the URL and host rules guidance applies.

use super::*;

pub(super) fn validate_page_parameters(page: usize, per_page: usize) -> Result<(), AppError> {
    if page == 0 || page > MAXIMUM_METADATA_PAGE {
        return Err(AppError::DataParsing(format!(
            "Metadata page must be between 1 and {MAXIMUM_METADATA_PAGE}"
        )));
    }
    if !(1..=100).contains(&per_page) {
        return Err(AppError::DataParsing(
            "Metadata page size must be between 1 and 100".to_string(),
        ));
    }
    Ok(())
}

pub(super) fn validate_crawl_policy(policy: &CrawlPolicy) -> Result<(), AppError> {
    if !(1..=100_000).contains(&policy.maximum_pages) {
        return Err(AppError::DataParsing(
            "Crawl page limit must be between 1 and 100000".to_string(),
        ));
    }
    if policy.maximum_depth > 32 {
        return Err(AppError::DataParsing(
            "Crawl depth must not exceed 32".to_string(),
        ));
    }
    if !(1..=1_000_000).contains(&policy.maximum_resources) {
        return Err(AppError::DataParsing(
            "Crawl resource limit must be between 1 and 1000000".to_string(),
        ));
    }
    Ok(())
}

pub(super) fn validate_source_guidance(guidance: &SourceGuidance) -> Result<(), AppError> {
    if guidance.additional_seed_urls.len() > 100
        || guidance.allowed_hostnames.len() > 100
        || guidance.include_url_prefixes.len() > 100
        || guidance.exclude_url_prefixes.len() > 100
    {
        return Err(AppError::DataParsing(
            "Source repair guidance contains too many rules".to_string(),
        ));
    }
    for value in guidance
        .additional_seed_urls
        .iter()
        .chain(guidance.include_url_prefixes.iter())
        .chain(guidance.exclude_url_prefixes.iter())
    {
        if value.trim().is_empty() || value.len() > 4_096 {
            return Err(AppError::DataParsing(
                "Source repair URLs and prefixes must contain between 1 and 4096 characters"
                    .to_string(),
            ));
        }
    }
    for hostname in &guidance.allowed_hostnames {
        if hostname.trim().is_empty()
            || hostname.len() > 253
            || !hostname
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
        {
            return Err(AppError::DataParsing(format!(
                "Source repair hostname '{hostname}' is invalid"
            )));
        }
    }
    for selector in [
        guidance.link_selector.as_deref(),
        guidance.download_selector.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        if selector.trim().is_empty() || selector.len() > 1_024 {
            return Err(AppError::DataParsing(
                "Source repair selectors must contain between 1 and 1024 characters".to_string(),
            ));
        }
        scraper::Selector::parse(selector).map_err(|error| {
            AppError::DataParsing(format!("Source repair selector is invalid: {error}"))
        })?;
    }
    if guidance.include_filename_globs.len() > 100
        || guidance.exclude_filename_globs.len() > 100
        || guidance.include_filename_patterns.len() > 100
        || guidance.exclude_filename_patterns.len() > 100
        || guidance.include_content_keywords.len() > 100
        || guidance.exclude_content_keywords.len() > 100
        || guidance.include_content_patterns.len() > 100
        || guidance.exclude_content_patterns.len() > 100
        || guidance.include_organizations.len() > 100
        || guidance.exclude_organizations.len() > 100
    {
        return Err(AppError::DataParsing(
            "Source repair guidance contains too many rules".to_string(),
        ));
    }
    for value in guidance
        .include_filename_globs
        .iter()
        .chain(guidance.exclude_filename_globs.iter())
        .chain(guidance.include_filename_patterns.iter())
        .chain(guidance.exclude_filename_patterns.iter())
        .chain(guidance.include_content_keywords.iter())
        .chain(guidance.exclude_content_keywords.iter())
        .chain(guidance.include_content_patterns.iter())
        .chain(guidance.exclude_content_patterns.iter())
        .chain(guidance.include_organizations.iter())
        .chain(guidance.exclude_organizations.iter())
    {
        if value.trim().is_empty() || value.len() > 1_024 {
            return Err(AppError::DataParsing(
                "Source repair scope rules must contain between 1 and 1024 characters".to_string(),
            ));
        }
    }
    // Regex patterns are compiled here so a bad one is refused at save time with a message that
    // names the pattern, rather than silently contributing nothing once the crawl is already
    // running. `CompiledGuidancePatterns::compile` still tolerates an invalid pattern at runtime
    // (dropping it rather than panicking), which covers guidance saved before this check existed.
    for pattern in guidance
        .include_filename_patterns
        .iter()
        .chain(guidance.exclude_filename_patterns.iter())
        .chain(guidance.include_content_patterns.iter())
        .chain(guidance.exclude_content_patterns.iter())
    {
        Regex::new(pattern).map_err(|error| {
            AppError::DataParsing(format!(
                "Source repair pattern '{pattern}' is invalid: {error}"
            ))
        })?;
    }
    if let (Some(minimum), Some(maximum)) = (
        guidance.minimum_file_size_bytes,
        guidance.maximum_file_size_bytes,
    ) {
        if minimum > maximum {
            return Err(AppError::DataParsing(
                "Source repair minimum file size must not exceed the maximum".to_string(),
            ));
        }
    }
    let date_from = guidance
        .document_date_from
        .as_deref()
        .map(|value| {
            NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|error| {
                AppError::DataParsing(format!(
                    "Source repair document start date '{value}' is invalid: {error}"
                ))
            })
        })
        .transpose()?;
    let date_to = guidance
        .document_date_to
        .as_deref()
        .map(|value| {
            NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|error| {
                AppError::DataParsing(format!(
                    "Source repair document end date '{value}' is invalid: {error}"
                ))
            })
        })
        .transpose()?;
    if date_from.zip(date_to).is_some_and(|(from, to)| from > to) {
        return Err(AppError::DataParsing(
            "Source repair document start date must not be after the end date".to_string(),
        ));
    }
    Ok(())
}

/// A minimal glob matcher: `*` matches any run of characters (including none), every other byte
/// matches literally. Sufficient for filename rules like `*.pdf` or `budget-*-final.xlsx` without
/// pulling in a glob crate for a single wildcard character.
///
/// Iterative with a single backtrack point, so it is O(pattern x text). The filename is taken from
/// a URL a remote site chose, and this runs on a worker thread with no await point: a recursive
/// matcher takes exponential time on `*a*a*a*a*a*b` against a long run of `a`.
pub(super) fn glob_matches(pattern: &str, text: &str) -> bool {
    let (pattern, text) = (pattern.as_bytes(), text.as_bytes());
    let (mut pattern_index, mut text_index) = (0, 0);
    // Where to resume after the most recent `*`: the pattern just past it, and the next text
    // position it should swallow.
    let mut resume: Option<(usize, usize)> = None;
    while text_index < text.len() {
        if pattern.get(pattern_index) == Some(&b'*') {
            pattern_index += 1;
            resume = Some((pattern_index, text_index));
        } else if pattern.get(pattern_index) == Some(&text[text_index]) {
            pattern_index += 1;
            text_index += 1;
        } else if let Some((resume_pattern, resume_text)) = resume {
            pattern_index = resume_pattern;
            text_index = resume_text + 1;
            resume = Some((resume_pattern, text_index));
        } else {
            return false;
        }
    }
    pattern[pattern_index..].iter().all(|byte| *byte == b'*')
}

/// The immediate last path segment of a URL, treated as the filename a `SourceGuidance` filename
/// rule evaluates against. Mirrors `client::file_name_from_url`'s convention: a trailing slash
/// (a directory, not a file) is an empty segment and yields `None`, never the directory name one
/// level up -- an include-glob like `*.pdf` must not silently prune a crawl into `/reports/2024/`
/// because "2024" was mistaken for a filename. A non-URL value falls back to its immediate last
/// slash-delimited segment for compatibility with the pre-parser path. Absence means the filename
/// predicates contribute nothing, not that they exclude, per the absence-versus-measurement rule.
fn filename_from_url(value: &str) -> Option<String> {
    let last_segment = reqwest::Url::parse(value)
        .ok()
        .and_then(|url| {
            url.path_segments()
                .and_then(|mut segments| segments.next_back())
                .map(str::to_string)
        })
        .unwrap_or_else(|| value.rsplit('/').next().unwrap_or(value).to_string());
    (!last_segment.is_empty()).then_some(last_segment)
}

/// Whether a discovered URL survives the source's saved scoping guidance, naming the specific
/// rule that refused it. `content_length` is `Some` only at call sites where a size is already
/// known pre-download (a catalog listing that reports it); elsewhere it is `None` and the size
/// predicate contributes nothing rather than excluding on an unmeasured value.
pub(super) fn guidance_allows_url(
    guidance: &SourceGuidance,
    compiled: &CompiledGuidancePatterns,
    value: &str,
    content_length: Option<u64>,
) -> Result<(), FrontierExclusion> {
    if guidance
        .exclude_url_prefixes
        .iter()
        .any(|prefix| value.starts_with(prefix))
    {
        return Err(FrontierExclusion::Guidance);
    }
    if !guidance.include_url_prefixes.is_empty()
        && !guidance
            .include_url_prefixes
            .iter()
            .any(|prefix| value.starts_with(prefix))
    {
        return Err(FrontierExclusion::Guidance);
    }
    if !guidance.allowed_hostnames.is_empty() {
        let host_allowed = reqwest::Url::parse(value)
            .ok()
            .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
            .is_some_and(|host| {
                guidance.allowed_hostnames.iter().any(|allowed| {
                    let allowed = allowed.to_ascii_lowercase();
                    host == allowed || host.ends_with(&format!(".{allowed}"))
                })
            });
        if !host_allowed {
            return Err(FrontierExclusion::Guidance);
        }
    }
    if let Some(filename) = filename_from_url(value) {
        let filename_lower = filename.to_ascii_lowercase();
        let excluded = guidance
            .exclude_filename_globs
            .iter()
            .any(|pattern| glob_matches(&pattern.to_ascii_lowercase(), &filename_lower))
            || compiled
                .exclude_filename_patterns
                .iter()
                .any(|pattern| pattern.is_match(&filename));
        if excluded {
            return Err(FrontierExclusion::FilenamePattern);
        }
        let has_includes = !guidance.include_filename_globs.is_empty()
            || !compiled.include_filename_patterns.is_empty();
        if has_includes {
            let included = guidance
                .include_filename_globs
                .iter()
                .any(|pattern| glob_matches(&pattern.to_ascii_lowercase(), &filename_lower))
                || compiled
                    .include_filename_patterns
                    .iter()
                    .any(|pattern| pattern.is_match(&filename));
            if !included {
                return Err(FrontierExclusion::FilenamePattern);
            }
        }
    }
    if let Some(length) = content_length {
        let out_of_range = guidance
            .minimum_file_size_bytes
            .is_some_and(|minimum| length < minimum)
            || guidance
                .maximum_file_size_bytes
                .is_some_and(|maximum| length > maximum);
        if out_of_range {
            return Err(FrontierExclusion::FileSize);
        }
    }
    Ok(())
}

/// True while a payload `structured_depth` hops into a JSON chain may still expand the links it
/// contains. Extracted so the bound is testable without a live `WorkerContext`. Deliberately
/// takes no `CrawlPolicy`: site depth does not fund structured expansion.
pub(super) fn may_expand_structured_links(structured_depth: usize) -> bool {
    structured_depth < MAXIMUM_STRUCTURED_CHAIN
}

/// Returns the depth assigned to an Outlook attachment when the container budget permits another
/// descent. `.msg` files deliberately emit only their immediate attachments; if one attachment is
/// itself a message, the ordinary format detector classifies it on the next task. This keeps all
/// nesting on the queue's single container-depth axis instead of hiding recursion inside a parser.
pub(super) fn next_msg_attachment_container_depth(container_depth: usize) -> Option<usize> {
    (container_depth < MAXIMUM_CONTAINER_DEPTH).then(|| container_depth + 1)
}

/// Provenance for a discovered payload. The portal is not the origin: a crawled page can point
/// at any host, and in an evidence tool the recorded origin has to be the page the link was on
/// plus the host the bytes actually came from.
pub(super) fn describe_discovery_origin(discovered_from: &str, download_url: &str) -> String {
    match (url_host(discovered_from), url_host(download_url)) {
        (Some(origin), Some(payload)) if origin != payload => {
            format!("Discovered from {discovered_from}; payload hosted on {payload}")
        }
        _ => format!("Discovered from {discovered_from}"),
    }
}

fn url_host(value: &str) -> Option<String> {
    reqwest::Url::parse(value)
        .ok()
        .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
}

pub(super) fn source_host_matches(source_url: &str, candidate_url: &str) -> bool {
    let source_host = url_host(source_url);
    let candidate_host = url_host(candidate_url);
    source_host.is_some() && source_host == candidate_host
}

pub(super) fn guidance_explicitly_allows_host(guidance: &SourceGuidance, value: &str) -> bool {
    if guidance.allowed_hostnames.is_empty() {
        return false;
    }
    reqwest::Url::parse(value)
        .ok()
        .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
        .is_some_and(|host| {
            guidance.allowed_hostnames.iter().any(|allowed| {
                let allowed = allowed.to_ascii_lowercase();
                host == allowed || host.ends_with(&format!(".{allowed}"))
            })
        })
}

/// Whether a payload hosted at `candidate_url` may be downloaded for a source rooted at
/// `source_url`.
///
/// Unset (the default) this admits every host, which is what the crawl has always done: the link
/// that reached the payload was already admitted by the follow gate, and real portals serve their
/// documents from storage hosts that gate does not enumerate. Set, the source root's own host and
/// any host the operator wrote into `allowed_hostnames` are the whole permitted set. The explicit
/// allowlist has to stay in the permitted set, or setting the flag would refuse hosts the operator
/// named on purpose.
pub(super) fn download_host_allowed(
    guidance: &SourceGuidance,
    source_url: &str,
    candidate_url: &str,
) -> bool {
    !guidance.restrict_downloads_to_source_host
        || source_host_matches(source_url, candidate_url)
        || guidance_explicitly_allows_host(guidance, candidate_url)
}

pub(super) fn is_supported_external_data_platform(value: &str) -> bool {
    // Host policy fetches, caches, and enforces robots rules before any supported external host is
    // crawled. This list decides only which off-source platforms are eligible for that policy.
    let Some(host) = reqwest::Url::parse(value)
        .ok()
        .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
    else {
        return false;
    };
    [
        "arcgis.com",
        "arcgisonline.com",
        "tableau.com",
        "google.com",
        "googleusercontent.com",
        "socrata.com",
        "granicus.com",
        "hylandcloud.com",
        "netfile.com",
        "github.com",
        "githubusercontent.com",
        "box.com",
        "dropbox.com",
        "sharepoint.com",
        "onedrive.live.com",
    ]
    .iter()
    .any(|platform| host == *platform || host.ends_with(&format!(".{platform}")))
}
