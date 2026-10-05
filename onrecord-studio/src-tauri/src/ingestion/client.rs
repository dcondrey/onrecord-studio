//! Hardened HTTP discovery and bounded ingestion for public-record portals.

use crate::error::AppError;
use crate::ingestion::backoff;
use crate::ingestion::host_policy::{
    HostGovernor, HostPermit, HostStopReason, RequestRefusal, RobotsRules,
    DEFAULT_HOST_CONCURRENCY, DEFAULT_HOST_INTERVAL, DEFAULT_HOST_REQUEST_BUDGET,
    MAXIMUM_ROBOTS_BYTES,
};
use crate::ingestion::hosted_nextrequest;
use crate::integrity::{digest_hex, sha256_hex};
use crate::storage::sqlite::ReleaseEvent;
use crate::types::{
    NextRequestResponse, PaginationMeta, PortalApiLayout, PortalRegistration, PublicRequest,
    SourceKind,
};
use base64::Engine;
use chrono::Utc;
use futures_util::StreamExt;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, CONTENT_LENGTH, LOCATION, RETRY_AFTER};
use reqwest::{Client, StatusCode, Url};
use scraper::{Html, Selector};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::future::Future;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;

const MAX_DISCOVERY_CANDIDATES: usize = 24;
const MAX_REDIRECTS: usize = 5;
const MAX_RETRIES: usize = 10;
const MAX_QUERY_PARAMETER_LENGTH: usize = 64;
const DEFAULT_JSON_LIMIT_BYTES: usize = 16 * 1024 * 1024;
const DEFAULT_DOCUMENT_LIMIT_BYTES: usize = 512 * 1024 * 1024;
const MAX_PUBLIC_LINKS_PER_PAGE: usize = 50_000;
const MAX_HYLAND_ITEMS_PER_MEETING: usize = 8;
const JAVASCRIPT_MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
/// A pinned DNS answer outlives the validation that created it just long enough to cover the
/// request, then expires so a legitimate address change is seen at the next validation.
const DNS_PIN_TTL: Duration = Duration::from_secs(120);
/// Bounds the pin map against a crawl that touches an unbounded number of hostnames.
const MAX_DNS_PINS: usize = 512;
/// A hostile or black-holed resolver must not stall a worker indefinitely.
const DNS_LOOKUP_TIMEOUT: Duration = Duration::from_secs(5);
/// The product token this client presents, and the token `RobotsRules` matches on. Keep the two
/// in step: a User-Agent that does not contain the token a host writes rules against means the
/// host cannot address this client specifically.
const ROBOTS_PRODUCT_TOKEN_DISPLAY: &str = "OnRecordStudio";
const APPLICATION_VERSION: &str = env!("CARGO_PKG_VERSION");

// Authenticated, CAPTCHA-gated, and session-bound systems are intentionally manual-import-only:
// this public acquisition client never collects browser credentials or bypasses an access gate.
// Document bodies use `stream_response_to_path`; only bounded discovery/metadata responses remain
// in `ResponsePayload.bytes` because their decoders require one complete JSON or HTML value.

#[derive(Debug, Clone)]
pub struct HttpClientConfig {
    pub connect_timeout_seconds: u64,
    pub request_timeout_seconds: u64,
    pub max_retries: usize,
    pub retry_initial_delay_ms: u64,
    pub retry_max_delay_ms: u64,
    pub max_json_response_bytes: usize,
    pub max_document_bytes: usize,
    /// How an agency reaches the operator whose client is making these requests. There is no
    /// default: fabricating a contact address is worse than declaring none, and the operator's
    /// own address must never be sent without them choosing to send it.
    pub contact_address: Option<String>,
    pub host_concurrency: usize,
    pub host_request_budget: usize,
}

impl Default for HttpClientConfig {
    fn default() -> Self {
        Self {
            connect_timeout_seconds: 10,
            request_timeout_seconds: 45,
            max_retries: 4,
            retry_initial_delay_ms: 500,
            retry_max_delay_ms: 15_000,
            max_json_response_bytes: DEFAULT_JSON_LIMIT_BYTES,
            max_document_bytes: DEFAULT_DOCUMENT_LIMIT_BYTES,
            contact_address: None,
            host_concurrency: DEFAULT_HOST_CONCURRENCY,
            host_request_budget: DEFAULT_HOST_REQUEST_BUDGET,
        }
    }
}

/// Identifies the application, its version, and how to reach whoever is running it. A portal
/// operator who wants to talk to a client instead of blocking it needs all three.
fn user_agent_value(contact_address: Option<&str>) -> Result<HeaderValue, AppError> {
    let contact = match contact_address {
        Some(value) => {
            let value = value.trim();
            if value.len() > 128
                || !(value.starts_with("mailto:") || value.starts_with("https://"))
                || value.bytes().any(|byte| !(0x20..0x7f).contains(&byte))
            {
                return Err(AppError::DataParsing(
                    "The crawler contact address must be a printable ASCII 'mailto:' or 'https://' value of at most 128 characters".to_string(),
                ));
            }
            format!("+{value}")
        }
        None => "no contact address configured".to_string(),
    };
    HeaderValue::from_str(&format!(
        "{ROBOTS_PRODUCT_TOKEN_DISPLAY}/{APPLICATION_VERSION} (public-records archival client; {contact})"
    ))
    .map_err(|error| AppError::DataParsing(format!("Failed to build the User-Agent header: {error}")))
}

#[derive(Clone)]
pub struct ApiClient {
    client: Client,
    config: HttpClientConfig,
    dns: Arc<PinnedDnsResolver>,
    governor: Arc<HostGovernor>,
}

struct ResponsePayload {
    final_url: Url,
    content_type: Option<String>,
    bytes: Vec<u8>,
    acquisition: AcquisitionRecord,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicResourceLink {
    pub url: String,
    pub label: String,
    pub download_candidate: bool,
    pub follow_candidate: bool,
    /// The link sits in the page's navigation chrome. Reported rather than dropped: the crawl
    /// frontier refuses these and counts the refusals, and guided repair still shows the operator
    /// every link on the page, including one this rule would have eaten on a portal whose index
    /// lives in a sidebar.
    pub navigation_chrome: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicResourceKind {
    HtmlPage,
    Download,
}

#[derive(Debug, Clone)]
pub struct PublicResourceInspection {
    pub final_url: String,
    pub content_type: Option<String>,
    pub title: String,
    pub kind: PublicResourceKind,
    pub links: Vec<PublicResourceLink>,
    pub body: Option<Vec<u8>>,
    pub embedded_connectors: Vec<PublicEmbeddedConnector>,
    // The server's declared `Content-Length`, read from the response headers before the body is
    // consumed (or, for a non-HTML resource, before it is discarded unread). `None` when the
    // header is absent or unparseable, which is a genuine "not known", not zero: a size predicate
    // must treat it as contributing nothing rather than as a document of size zero.
    pub content_length: Option<u64>,
    /// The fetch that produced `body`; `None` when nothing was fetched.
    pub acquisition: Option<AcquisitionRecord>,
}

#[derive(Debug, Clone)]
pub enum PublicEmbeddedConnector {
    WordPressGoogleDrive(WordPressGoogleDriveConnector),
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WordPressGoogleDriveConnector {
    pub endpoint: String,
    pub shortcode_id: String,
    pub nonce: String,
    pub folder_id: String,
    pub account_id: String,
    pub folder_name: String,
}

#[derive(Debug, Clone)]
pub struct WordPressGoogleDriveItem {
    pub id: String,
    pub account_id: String,
    pub name: String,
    pub media_type: String,
    pub is_folder: bool,
    pub web_view_link: Option<String>,
    pub web_content_link: Option<String>,
}

#[derive(Debug)]
pub struct ArcGisFeaturePage {
    pub request_url: String,
    pub bytes: Vec<u8>,
    pub acquisition: AcquisitionRecord,
    pub feature_count: usize,
    pub exceeded_transfer_limit: bool,
}

/// What a re-fetch of a collected document received.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceRecheck {
    Retrieved(DownloadedPayload),
    /// The source answered 404 or 410.
    Gone(AcquisitionRecord),
}

/// What [`ApiClient::get_direct`] received.
#[derive(Debug, Clone)]
pub struct DirectResponse {
    pub status: u16,
    pub content_range: Option<String>,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadedPayload {
    pub path: PathBuf,
    pub byte_count: u64,
    pub sha256: String,
    pub acquisition: AcquisitionRecord,
}

/// What the network said when it delivered a payload, retained as an immutable fetch event so a
/// later fetch of the same document never erases what an earlier one received.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcquisitionRecord {
    pub requested_url: String,
    pub resolved_url: String,
    /// Every URL requested before the one that answered, in order.
    pub redirect_chain: Vec<String>,
    pub http_status: u16,
    pub content_type: Option<String>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    /// The response's `Date` header: the server's own clock when it answered. A `Last-Modified`
    /// equal to it is the request time, not a statement about the file.
    pub response_date: Option<String>,
    /// Local clock when the response headers arrived; not a trusted timestamp.
    pub retrieved_at: String,
}

const MAXIMUM_RECORDED_HEADER_CHARACTERS: usize = 512;

fn acquisition_record(
    requested_url: &Url,
    response: &reqwest::Response,
    redirect_chain: Vec<String>,
) -> AcquisitionRecord {
    let headers = response.headers();
    AcquisitionRecord {
        requested_url: requested_url.to_string(),
        resolved_url: response.url().to_string(),
        redirect_chain,
        http_status: response.status().as_u16(),
        content_type: recorded_header(headers, reqwest::header::CONTENT_TYPE),
        etag: recorded_header(headers, reqwest::header::ETAG),
        last_modified: recorded_header(headers, reqwest::header::LAST_MODIFIED),
        response_date: recorded_header(headers, reqwest::header::DATE),
        retrieved_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true),
    }
}

fn recorded_header(headers: &HeaderMap, name: reqwest::header::HeaderName) -> Option<String> {
    let value = headers.get(name)?.to_str().ok()?.trim();
    (!value.is_empty()).then(|| {
        value
            .chars()
            .take(MAXIMUM_RECORDED_HEADER_CHARACTERS)
            .collect()
    })
}

struct DiscoveredLayout {
    requests_pointer: String,
    metadata_pointer: Option<String>,
}

impl ApiClient {
    pub fn new(config: HttpClientConfig) -> Result<Self, AppError> {
        if config.connect_timeout_seconds == 0 || config.request_timeout_seconds == 0 {
            return Err(AppError::DataParsing(
                "HTTP timeouts must be greater than zero".to_string(),
            ));
        }
        if config.max_json_response_bytes == 0 || config.max_document_bytes == 0 {
            return Err(AppError::DataParsing(
                "HTTP response limits must be greater than zero".to_string(),
            ));
        }
        if config.max_retries > MAX_RETRIES {
            return Err(AppError::DataParsing(format!(
                "HTTP retry count must not exceed {MAX_RETRIES}"
            )));
        }
        if config.retry_initial_delay_ms == 0
            || config.retry_max_delay_ms == 0
            || config.retry_initial_delay_ms > config.retry_max_delay_ms
        {
            return Err(AppError::DataParsing(
                "HTTP retry delays must be positive and the initial delay must not exceed the maximum delay"
                    .to_string(),
            ));
        }
        let mut headers = HeaderMap::new();
        headers.insert(
            reqwest::header::USER_AGENT,
            user_agent_value(config.contact_address.as_deref())?,
        );
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("application/json, text/html;q=0.5, */*;q=0.1"),
        );
        let dns = Arc::new(PinnedDnsResolver::default());
        let client = Client::builder()
            .default_headers(headers)
            // Every direct connection resolves through the addresses `validate_remote_url`
            // already approved; see `PinnedDnsResolver`. A configured system proxy resolves the
            // host itself, so behind one the pin covers only the validation lookup.
            .dns_resolver(Arc::clone(&dns))
            .connect_timeout(Duration::from_secs(config.connect_timeout_seconds))
            // An idle limit, not a total one: a total deadline would also bound the body, so a
            // large document on a slow link could never finish and each retry would re-download
            // it from byte 0. The total per request is set from its size cap; see
            // `transfer_deadline`.
            .read_timeout(Duration::from_secs(config.request_timeout_seconds))
            .pool_idle_timeout(Duration::from_secs(90))
            .pool_max_idle_per_host(8)
            // Redirects stay disabled here so every hop passes through `validate_remote_url`
            // before a socket is opened; enabling reqwest's automatic policy would bypass it.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| AppError::Network(format!("Failed to build HTTP client: {error}")))?;
        let governor = Arc::new(HostGovernor::new(
            config.host_concurrency,
            config.host_request_budget,
            DEFAULT_HOST_INTERVAL,
        ));
        Ok(Self {
            client,
            config,
            dns,
            governor,
        })
    }

    /// The per-host governor, so callers can consult a host's standing before scheduling work
    /// against it and report why a host stopped.
    pub fn host_governor(&self) -> &Arc<HostGovernor> {
        &self.governor
    }

    /// Records that the operator directed this application at `url`, which exempts it from
    /// robots.txt path exclusions. Pacing, concurrency, per-host budgets, and host stops still
    /// apply. See `HostState::user_directed` for why the exemption is drawn here.
    pub fn mark_user_directed(&self, url: &str) {
        if let Ok(url) = Url::parse(url) {
            self.governor.mark_user_directed(&url);
        }
    }

    /// This host's robots.txt rules, fetched at most once per host per process.
    async fn robots_rules(&self, url: &Url) -> Arc<RobotsRules> {
        let cell = self.governor.robots_cell(url);
        Arc::clone(
            cell.get_or_init(|| async { Arc::new(self.fetch_robots(url).await) })
                .await,
        )
    }

    /// Fetches and parses `/robots.txt`, deliberately bypassing the governor: the file that
    /// grants the crawl allowance cannot itself require one, and a 403 on robots.txt must not
    /// trip the host stop that a 403 on a content URL does.
    ///
    /// Per RFC 9309 an unavailable (4xx) robots.txt permits everything, while a server that
    /// answers 5xx withdraws the allowance. A transport failure is neither: nothing was answered,
    /// so nothing was withdrawn, and the site request that follows will fail on its own. Treating
    /// a DNS hiccup as a withdrawal would stop the host for the whole run off one cached error.
    async fn fetch_robots(&self, url: &Url) -> RobotsRules {
        let Ok(robots_url) = url.join("/robots.txt") else {
            return RobotsRules::unavailable("the robots.txt URL could not be constructed");
        };
        let outcome = self
            .execute_with_retries(&robots_url, "robots.txt", || async {
                let mut current = robots_url.clone();
                let mut visited = BTreeSet::new();
                for hop in 0..=MAX_REDIRECTS {
                    current.set_fragment(None);
                    if !visited.insert(current.as_str().to_string()) {
                        return Err(RetryDisposition::Terminal(AppError::PortalSecurity(
                            format!("robots.txt redirect loop detected at '{current}'"),
                        )));
                    }
                    validate_remote_url(&self.dns, &current)
                        .await
                        .map_err(RetryDisposition::Terminal)?;
                    let response =
                        self.client
                            .get(current.clone())
                            .send()
                            .await
                            .map_err(|error| {
                                RetryDisposition::Retry(
                                    AppError::Network(format!("GET '{current}' failed: {error}")),
                                    None,
                                )
                            })?;
                    let status = response.status();
                    if status.is_redirection() && hop < MAX_REDIRECTS {
                        let Some(location) = response
                            .headers()
                            .get(LOCATION)
                            .and_then(|value| value.to_str().ok())
                            .and_then(|value| next_redirect_url(&current, value).ok())
                        else {
                            return Ok(RobotsRules::unrestricted());
                        };
                        current = location;
                        continue;
                    }
                    if status.is_client_error() {
                        return Ok(RobotsRules::unrestricted());
                    }
                    if !status.is_success() {
                        return Err(RetryDisposition::Retry(
                            AppError::PortalSecurity(format!(
                                "robots.txt at '{current}' returned HTTP {status}"
                            )),
                            None,
                        ));
                    }
                    let (_, bytes) =
                        read_response_bytes(response, MAXIMUM_ROBOTS_BYTES, "robots.txt")
                            .await
                            .map_err(RetryDisposition::Terminal)?;
                    return Ok(RobotsRules::parse(&String::from_utf8_lossy(&bytes)));
                }
                Ok(RobotsRules::unrestricted())
            })
            .await;
        outcome.unwrap_or_else(|error| match error {
            // The server answered, and its answer withheld the allowance.
            AppError::PortalSecurity(detail) => RobotsRules::unavailable(detail),
            // Nothing answered. The host is unreachable, not restrictive.
            _ => RobotsRules::unrestricted(),
        })
    }

    /// Clears the request for dispatch and holds one of the host's concurrency slots until the
    /// returned permit is dropped, which is why every caller keeps it alive across the body read.
    async fn reserve_host_slot(&self, url: &Url) -> Result<HostPermit, RetryDisposition> {
        let rules = self.robots_rules(url).await;
        if let Some(detail) = rules.unavailable_detail() {
            self.governor.stop_host(
                url,
                HostStopReason::RobotsUnavailable {
                    detail: detail.to_string(),
                },
            );
        }
        let refuse = |refusal: RequestRefusal| RetryDisposition::Terminal(AppError::from(refusal));
        self.governor.admits(url, Some(&rules)).map_err(refuse)?;
        self.governor
            .reserve(url, Some(&rules))
            .await
            .map_err(refuse)
    }

    pub async fn discover_portal(&self, portal_url: &str) -> Result<PortalRegistration, AppError> {
        let submitted_url = normalize_submitted_url(portal_url)?;
        if super::regional::is_publication_site(&submitted_url) {
            return Err(AppError::PortalDiscovery(
                "San Diego City document pages and regional publication sites use public document discovery; request portals use their own NextRequest endpoints".to_string(),
            ));
        }
        validate_remote_url(&self.dns, &submitted_url).await?;
        let origin = origin_url(&submitted_url)?;
        if let Some(portal) = self
            .discover_hosted_nextrequest(&submitted_url, &origin)
            .await
        {
            return Ok(portal);
        }
        let mut candidates = initial_api_candidates(&submitted_url, &origin)?;
        // Discovery probes the publisher's own documented API endpoints at the operator's
        // direction; robots.txt path exclusions govern link-following, not this.
        self.governor.mark_user_directed(&submitted_url);
        for candidate in &candidates {
            self.governor.mark_user_directed(candidate);
        }
        let mut visited = BTreeSet::new();
        let parameter_layouts = [
            ("page", "per_page"),
            ("page", "limit"),
            ("page", "page_size"),
            ("offset", "limit"),
        ];
        let mut attempts = 0_usize;
        let mut last_error = String::new();

        while let Some(candidate) = candidates.pop_front() {
            if attempts >= MAX_DISCOVERY_CANDIDATES {
                break;
            }
            let candidate_key = candidate.as_str().to_string();
            if !visited.insert(candidate_key) {
                continue;
            }
            attempts += 1;
            validate_remote_url(&self.dns, &candidate).await?;

            for (page_parameter, page_size_parameter) in parameter_layouts {
                let query = if page_parameter == "offset" {
                    vec![
                        ("offset", "0".to_string()),
                        (page_size_parameter, "1".to_string()),
                    ]
                } else {
                    vec![
                        (page_parameter, "1".to_string()),
                        (page_size_parameter, "1".to_string()),
                    ]
                };
                match self
                    .get_bounded(
                        candidate.clone(),
                        &query,
                        self.config.max_json_response_bytes,
                    )
                    .await
                {
                    Ok(payload) => {
                        if let Ok(json) = serde_json::from_slice::<Value>(&payload.bytes) {
                            if let Some(layout) = detect_json_layout(&json) {
                                validate_response_records(
                                    &json,
                                    &layout.requests_pointer,
                                    layout.metadata_pointer.as_deref(),
                                )?;
                                let record_count =
                                    response_records(&json, &layout.requests_pointer)
                                        .map_or(0, |records| records.len());
                                if record_count > 1 {
                                    last_error = format!(
                                        "{} did not honor the '{}' page-size probe",
                                        payload.final_url, page_size_parameter
                                    );
                                    continue;
                                }
                                if !has_discovery_evidence(&json, &layout) {
                                    last_error = format!(
                                        "{} returned an ambiguous empty JSON collection",
                                        payload.final_url
                                    );
                                    continue;
                                }
                                let canonical_url = canonical_url(&submitted_url);
                                let requests_endpoint = without_query_parameters(
                                    &payload.final_url,
                                    &[page_parameter, page_size_parameter],
                                );
                                match self
                                    .pagination_probe_advances(
                                        &requests_endpoint,
                                        &layout,
                                        &json,
                                        page_parameter,
                                        page_size_parameter,
                                    )
                                    .await
                                {
                                    Ok(true) => {}
                                    Ok(false) => {
                                        last_error = format!(
                                            "{} did not advance when probed with '{}'",
                                            payload.final_url, page_parameter
                                        );
                                        continue;
                                    }
                                    Err(error) => {
                                        last_error = error.to_string();
                                        continue;
                                    }
                                }
                                let requests_endpoint = requests_endpoint.to_string();
                                return Ok(PortalRegistration {
                                    portal_key: portal_key(&submitted_url, &requests_endpoint),
                                    display_name: discover_display_name(&json, &submitted_url),
                                    canonical_url,
                                    origin_url: origin.to_string(),
                                    api_layout: PortalApiLayout {
                                        requests_endpoint,
                                        requests_pointer: layout.requests_pointer,
                                        metadata_pointer: layout.metadata_pointer,
                                        page_parameter: page_parameter.to_string(),
                                        page_size_parameter: page_size_parameter.to_string(),
                                    },
                                    verified_at: Utc::now().to_rfc3339(),
                                    source_kind: SourceKind::NextRequest,
                                });
                            }
                        } else if payload
                            .content_type
                            .as_deref()
                            .is_some_and(|value| value.to_ascii_lowercase().contains("html"))
                        {
                            for discovered in extract_api_links(&payload.final_url, &payload.bytes)?
                            {
                                if candidates.len() + visited.len() < MAX_DISCOVERY_CANDIDATES {
                                    candidates.push_back(discovered);
                                }
                            }
                            last_error = format!(
                                "{} returned an HTML discovery page rather than JSON",
                                payload.final_url
                            );
                            break;
                        }
                        last_error = format!(
                            "{} returned no recognizable public-request collection",
                            payload.final_url
                        );
                    }
                    Err(error) => {
                        last_error = error.to_string();
                    }
                }
            }
        }

        Err(AppError::PortalDiscovery(format!(
            "No compatible NextRequest JSON collection was discovered from '{}'; inspected {attempts} endpoint candidates. Last result: {last_error}",
            submitted_url
        )))
    }

    pub async fn discover_public_source(
        &self,
        source_url: &str,
    ) -> Result<(PortalRegistration, PublicResourceInspection), AppError> {
        let submitted_url = normalize_submitted_url(source_url)?;
        validate_remote_url(&self.dns, &submitted_url).await?;
        self.governor.mark_user_directed(&submitted_url);
        let inspection = self.inspect_public_resource(submitted_url.as_str()).await?;
        let final_url = Url::parse(&inspection.final_url).map_err(|error| {
            AppError::PortalDiscovery(format!("Discovered source URL is invalid: {error}"))
        })?;
        let origin = origin_url(&final_url)?;
        let source_kind =
            if inspection.embedded_connectors.iter().any(|connector| {
                matches!(connector, PublicEmbeddedConnector::WordPressGoogleDrive(_))
            }) {
                SourceKind::GoogleDrive
            } else {
                classify_source_kind(&final_url, &inspection.kind)
            };
        let identity = format!("{}|{}", source_kind.as_str(), inspection.final_url);
        let digest = sha256_hex(identity.as_bytes());
        let host = final_url.host_str().unwrap_or("public source");
        let display_name = if inspection.title.trim().is_empty() {
            host.to_string()
        } else {
            inspection.title.chars().take(160).collect()
        };
        let registration = PortalRegistration {
            portal_key: format!("source-{}", &digest[..24]),
            display_name,
            canonical_url: inspection.final_url.clone(),
            origin_url: origin.to_string(),
            api_layout: PortalApiLayout {
                requests_endpoint: inspection.final_url.clone(),
                requests_pointer: "/resources".to_string(),
                metadata_pointer: None,
                page_parameter: "crawl_page".to_string(),
                page_size_parameter: "crawl_limit".to_string(),
            },
            verified_at: Utc::now().to_rfc3339(),
            source_kind,
        };
        Ok((registration, inspection))
    }

    pub async fn inspect_public_resource(
        &self,
        source_url: &str,
    ) -> Result<PublicResourceInspection, AppError> {
        self.inspect_public_resource_with_selectors(source_url, None, None)
            .await
    }

    pub async fn inspect_public_resource_with_selectors(
        &self,
        source_url: &str,
        link_selector: Option<&str>,
        download_selector: Option<&str>,
    ) -> Result<PublicResourceInspection, AppError> {
        let url = normalize_submitted_url(source_url)?;
        validate_remote_url(&self.dns, &url).await?;
        let (response, _host_permit, acquisition) = self
            .response_with_retries(url, self.config.max_document_bytes)
            .await?;
        let final_url = response.url().clone();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(|value| {
                value
                    .split(';')
                    .next()
                    .unwrap_or(value)
                    .trim()
                    .to_ascii_lowercase()
            });
        let content_length = response
            .headers()
            .get(reqwest::header::CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok());
        if !is_html_resource(&final_url, content_type.as_deref()) {
            return Ok(PublicResourceInspection {
                title: file_name_from_url(&final_url),
                final_url: final_url.to_string(),
                content_type,
                kind: PublicResourceKind::Download,
                links: Vec::new(),
                body: None,
                embedded_connectors: Vec::new(),
                content_length,
                acquisition: None,
            });
        }
        let (_, bytes) = read_response_bytes(
            response,
            self.config.max_json_response_bytes,
            "HTML discovery page",
        )
        .await?;
        // A meeting shell has no substantive text. Follow the exact document route it
        // publishes in place, so a bounded source run does not spend every page on shells.
        // Release the host permit before the next governed request (concurrency may be 1).
        drop(_host_permit);
        if link_selector.is_none() && download_selector.is_none() {
            if let Some(companion) = super::regional::hyland_document_companion(
                &final_url,
                &String::from_utf8_lossy(&bytes),
            ) {
                return self.inspect_hyland_packet(&companion).await;
            }
        }
        let (title, links, embedded_connectors) =
            extract_public_resource_links(&final_url, &bytes, link_selector, download_selector)?;
        Ok(PublicResourceInspection {
            final_url: final_url.to_string(),
            content_type,
            title,
            kind: PublicResourceKind::HtmlPage,
            links,
            body: Some(bytes),
            embedded_connectors,
            content_length,
            acquisition: Some(acquisition),
        })
    }

    /// Public OnBase packets use JavaScript item links. Read their literal references,
    /// then fetch a bounded number of supporting-document indexes through the same
    /// retry, robots, pacing and host budget as every other request. The source text
    /// remains the published agenda; attachments retain their own original URLs.
    async fn inspect_hyland_packet(
        &self,
        companion: &str,
    ) -> Result<PublicResourceInspection, AppError> {
        let companion =
            Url::parse(companion).map_err(|error| AppError::DataParsing(error.to_string()))?;
        let payload = self
            .get_bounded(companion.clone(), &[], self.config.max_json_response_bytes)
            .await?;
        if payload.final_url != companion
            || !is_html_resource(&payload.final_url, payload.content_type.as_deref())
        {
            return Err(AppError::DataParsing(
                "The published Council agenda route did not return its expected HTML document."
                    .into(),
            ));
        }
        let (title, mut links, embedded_connectors) =
            extract_public_resource_links(&payload.final_url, &payload.bytes, None, None)?;
        let items = super::regional::hyland_agenda_items(
            &payload.final_url,
            &String::from_utf8_lossy(&payload.bytes),
        );
        let mut seen = links
            .iter()
            .map(|link| link.url.clone())
            .collect::<BTreeSet<_>>();
        for (item_url, item_title) in items.into_iter().take(MAX_HYLAND_ITEMS_PER_MEETING) {
            let item_url = Url::parse(&item_url).expect("adapter emits validated URLs");
            let item = self
                .get_bounded(item_url.clone(), &[], self.config.max_json_response_bytes)
                .await;
            merge_hyland_item_attachments(&item_url, &item_title, item, &mut links, &mut seen);
        }
        Ok(PublicResourceInspection {
            final_url: payload.final_url.to_string(),
            content_type: payload.content_type,
            title,
            kind: PublicResourceKind::HtmlPage,
            links,
            content_length: Some(payload.bytes.len() as u64),
            acquisition: Some(payload.acquisition),
            body: Some(payload.bytes),
            embedded_connectors,
        })
    }

    pub async fn list_wordpress_google_drive_folder(
        &self,
        connector: &WordPressGoogleDriveConnector,
    ) -> Result<Vec<WordPressGoogleDriveItem>, AppError> {
        let endpoint = Url::parse(&connector.endpoint).map_err(|error| {
            AppError::DataParsing(format!(
                "Stored Google Drive connector endpoint is invalid: {error}"
            ))
        })?;
        validate_remote_url(&self.dns, &endpoint).await?;
        validate_connector_token("shortcode ID", &connector.shortcode_id, 128)?;
        validate_connector_token("nonce", &connector.nonce, 256)?;
        validate_connector_token("folder ID", &connector.folder_id, 256)?;
        validate_connector_token("account ID", &connector.account_id, 256)?;
        let form = [
            ("action", "igd_get_files"),
            ("shortcodeId", connector.shortcode_id.as_str()),
            ("data[folder][id]", connector.folder_id.as_str()),
            ("data[folder][accountId]", connector.account_id.as_str()),
            ("data[folder][name]", connector.folder_name.as_str()),
            ("data[folder][type]", "application/vnd.google-apps.folder"),
            ("nonce", connector.nonce.as_str()),
            ("skipBreadcrumbs", "1"),
        ];
        let (response, _host_permit) = self
            .execute_with_retries(&endpoint, "Google Drive connector request", || {
                self.send_wordpress_google_drive_request(endpoint.clone(), &form)
            })
            .await?;
        let (_, bytes) = read_response_bytes(
            response,
            self.config.max_json_response_bytes,
            "Google Drive folder listing",
        )
        .await?;
        drop(_host_permit);
        decode_wordpress_google_drive_listing(&bytes)
    }

    pub fn wordpress_google_drive_download_url(
        connector: &WordPressGoogleDriveConnector,
        item: &WordPressGoogleDriveItem,
    ) -> Result<String, AppError> {
        if let Some(url) = item
            .web_content_link
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            let parsed = Url::parse(url).map_err(|error| {
                AppError::DataParsing(format!("Google Drive download URL is invalid: {error}"))
            })?;
            if parsed.scheme() == "https" {
                return Ok(parsed.to_string());
            }
        }
        let mut endpoint = Url::parse(&connector.endpoint).map_err(|error| {
            AppError::DataParsing(format!(
                "Google Drive connector endpoint is invalid: {error}"
            ))
        })?;
        endpoint.query_pairs_mut().extend_pairs([
            ("action", "igd_download"),
            ("id", item.id.as_str()),
            ("accountId", item.account_id.as_str()),
            ("shortcodeId", connector.shortcode_id.as_str()),
            ("nonce", connector.nonce.as_str()),
        ]);
        Ok(endpoint.to_string())
    }

    pub async fn fetch_arcgis_feature_page(
        &self,
        layer_url: &str,
        offset: usize,
        page_size: usize,
    ) -> Result<ArcGisFeaturePage, AppError> {
        if !(1..=2_000).contains(&page_size) || offset > 100_000_000 {
            return Err(AppError::DataParsing(
                "ArcGIS pagination parameters exceed safe bounds".to_string(),
            ));
        }
        let mut url = Url::parse(layer_url).map_err(|error| {
            AppError::DataParsing(format!("ArcGIS layer URL is invalid: {error}"))
        })?;
        validate_arcgis_layer_url(&url)?;
        url.set_query(None);
        url.set_fragment(None);
        if !url.path().ends_with("/query") {
            let path = format!("{}/query", url.path().trim_end_matches('/'));
            url.set_path(&path);
        }
        let offset_value = offset.to_string();
        let page_size_value = page_size.to_string();
        url.query_pairs_mut().extend_pairs([
            ("where", "1=1"),
            ("outFields", "*"),
            ("returnGeometry", "true"),
            ("f", "geojson"),
            ("resultOffset", offset_value.as_str()),
            ("resultRecordCount", page_size_value.as_str()),
        ]);
        validate_remote_url(&self.dns, &url).await?;
        // This body is buffered whole and then parsed as JSON, so it takes the JSON ceiling. Under
        // the 512 MB document ceiling a ~500 KB compressed response could expand to 512 MB in
        // memory and then again into a `serde_json::Value`.
        let payload = self
            .get_bounded(url, &[], self.config.max_json_response_bytes)
            .await?;
        let value: Value = serde_json::from_slice(&payload.bytes).map_err(|error| {
            AppError::DataParsing(format!(
                "ArcGIS layer query '{}' returned invalid GeoJSON: {error}",
                payload.final_url
            ))
        })?;
        if let Some(error) = value.get("error") {
            return Err(AppError::DataParsing(format!(
                "ArcGIS layer query '{}' failed: {}",
                payload.final_url, error
            )));
        }
        let features = value
            .get("features")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                AppError::DataParsing(format!(
                    "ArcGIS layer query '{}' omitted its GeoJSON feature collection",
                    payload.final_url
                ))
            })?;
        Ok(ArcGisFeaturePage {
            request_url: payload.final_url.to_string(),
            acquisition: payload.acquisition,
            bytes: payload.bytes,
            feature_count: features.len(),
            exceeded_transfer_limit: value
                .get("exceededTransferLimit")
                .or_else(|| value.pointer("/properties/exceededTransferLimit"))
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })
    }

    /// Hosted NextRequest pages at a fixed size the generic page-size probe rejects, so it is
    /// recognized by its own listing shape before the generic layouts are tried.
    async fn discover_hosted_nextrequest(
        &self,
        submitted_url: &Url,
        origin: &Url,
    ) -> Option<PortalRegistration> {
        let endpoint = origin.join("client/requests").ok()?;
        self.governor.mark_user_directed(&endpoint);
        let payload = self
            .get_bounded(
                endpoint.clone(),
                &[(hosted_nextrequest::PAGE_PARAMETER, "1".to_string())],
                self.config.max_json_response_bytes,
            )
            .await
            .ok()?;
        let json: Value = serde_json::from_slice(&payload.bytes).ok()?;
        if !hosted_nextrequest::is_listing(&json) {
            return None;
        }
        let requests_endpoint = endpoint.to_string();
        Some(PortalRegistration {
            portal_key: portal_key(submitted_url, &requests_endpoint),
            display_name: discover_display_name(&json, submitted_url),
            canonical_url: canonical_url(submitted_url),
            origin_url: origin.to_string(),
            api_layout: PortalApiLayout {
                requests_endpoint,
                requests_pointer: "/requests".to_string(),
                metadata_pointer: None,
                page_parameter: hosted_nextrequest::PAGE_PARAMETER.to_string(),
                page_size_parameter: "per_page".to_string(),
            },
            verified_at: Utc::now().to_rfc3339(),
            source_kind: SourceKind::NextRequest,
        })
    }

    fn hosted_urls(portal: &PortalRegistration) -> Result<(Url, Url), AppError> {
        let endpoint = Url::parse(&portal.api_layout.requests_endpoint).map_err(|error| {
            AppError::PortalDiscovery(format!(
                "Stored endpoint for portal '{}' is invalid: {error}",
                portal.portal_key
            ))
        })?;
        let origin = Url::parse(&portal.origin_url).map_err(|error| {
            AppError::PortalDiscovery(format!(
                "Stored origin for portal '{}' is invalid: {error}",
                portal.portal_key
            ))
        })?;
        Ok((endpoint, origin))
    }

    /// One listing page of a hosted NextRequest portal, optionally searched for a subject. The
    /// listing carries no dates or documents; [`Self::fetch_hosted_request`] reads those.
    pub async fn list_hosted_nextrequest_page(
        &self,
        portal: &PortalRegistration,
        page: usize,
        search_term: Option<&str>,
    ) -> Result<hosted_nextrequest::ListedPage, AppError> {
        if page == 0 {
            return Err(AppError::DataParsing(
                "Request page must be greater than zero".to_string(),
            ));
        }
        let (endpoint, _) = Self::hosted_urls(portal)?;
        validate_remote_url(&self.dns, &endpoint).await?;
        let mut query = vec![(hosted_nextrequest::PAGE_PARAMETER, page.to_string())];
        if let Some(term) = search_term {
            query.push((hosted_nextrequest::SEARCH_PARAMETER, term.to_string()));
        }
        self.governor.mark_user_directed(&endpoint);
        let listing = self.hosted_json(endpoint, &query).await?;
        hosted_nextrequest::decode_listing(&listing, page)
    }

    /// A listed request with its opening/closing dates and public documents, the text of the
    /// agency's closing response when the request is closed, and the timeline's release events.
    pub async fn fetch_hosted_request(
        &self,
        portal: &PortalRegistration,
        request: hosted_nextrequest::ListedRequest,
    ) -> Result<(PublicRequest, Option<String>, Vec<ReleaseEvent>), AppError> {
        if !hosted_nextrequest::is_safe_request_identifier(&request.id) {
            return Err(AppError::DataParsing(format!(
                "NextRequest listed a request identifier '{}' that cannot be addressed",
                request.id
            )));
        }
        let (_, origin) = Self::hosted_urls(portal)?;
        let timeline_url = origin
            .join(&format!("client/requests/{}/timeline", request.id))
            .map_err(|error| AppError::PortalDiscovery(error.to_string()))?;
        // The timeline pages newest first, so the opening event is on its last page.
        let mut entries = Vec::new();
        for timeline_page in 1..=hosted_nextrequest::MAXIMUM_TIMELINE_PAGES {
            let value = self
                .hosted_json(
                    timeline_url.clone(),
                    &[(
                        hosted_nextrequest::PAGE_PARAMETER,
                        timeline_page.to_string(),
                    )],
                )
                .await?;
            let page_entries = value
                .get("timeline")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let reported = value.get("total_count").and_then(Value::as_u64);
            let listed_here = page_entries.len();
            entries.extend(page_entries);
            if listed_here == 0 || reported.is_none_or(|total| entries.len() as u64 >= total) {
                break;
            }
        }
        let mut timeline =
            hosted_nextrequest::decode_timeline(&serde_json::json!({ "timeline": entries }))?;
        let fallback_created_at = timeline.opened_at.clone().unwrap_or_default();
        let documents_url = origin
            .join("client/request_documents")
            .map_err(|error| AppError::PortalDiscovery(error.to_string()))?;
        let mut documents = Vec::new();
        for document_page in 1..=hosted_nextrequest::MAXIMUM_DOCUMENT_PAGES {
            let value = self
                .hosted_json(
                    documents_url.clone(),
                    &[
                        ("request_id", request.id.clone()),
                        (
                            hosted_nextrequest::PAGE_PARAMETER,
                            document_page.to_string(),
                        ),
                    ],
                )
                .await?;
            let (page_documents, total) = hosted_nextrequest::decode_documents(
                &value,
                origin.as_str(),
                &fallback_created_at,
            )?;
            let listed_here = value
                .get("documents")
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            documents.extend(page_documents);
            let seen = (document_page - 1) * hosted_nextrequest::PAGE_SIZE + listed_here;
            if listed_here == 0 || total.is_none_or(|total| seen as u64 >= total) {
                break;
            }
        }
        let closure_notice = timeline.closure_notice.clone();
        let releases = std::mem::take(&mut timeline.releases);
        Ok((
            hosted_nextrequest::to_public_request(request, timeline, documents)?,
            closure_notice,
            releases,
        ))
    }

    async fn hosted_json(&self, url: Url, query: &[(&str, String)]) -> Result<Value, AppError> {
        let payload = self
            .get_bounded(url, query, self.config.max_json_response_bytes)
            .await?;
        serde_json::from_slice(&payload.bytes).map_err(|error| {
            AppError::DataParsing(format!(
                "NextRequest returned invalid JSON from '{}': {error}",
                payload.final_url
            ))
        })
    }

    pub async fn fetch_requests(
        &self,
        portal: &PortalRegistration,
        page: usize,
        per_page: usize,
    ) -> Result<NextRequestResponse, AppError> {
        if page == 0 {
            return Err(AppError::DataParsing(
                "Request page must be greater than zero".to_string(),
            ));
        }
        if !(1..=100).contains(&per_page) {
            return Err(AppError::DataParsing(
                "Request page size must be between 1 and 100".to_string(),
            ));
        }
        let endpoint = Url::parse(&portal.api_layout.requests_endpoint).map_err(|error| {
            AppError::PortalDiscovery(format!(
                "Stored endpoint for portal '{}' is invalid: {error}",
                portal.portal_key
            ))
        })?;
        validate_pagination_parameters(
            &portal.api_layout.page_parameter,
            &portal.api_layout.page_size_parameter,
        )?;
        validate_remote_url(&self.dns, &endpoint).await?;
        let page_value = if portal.api_layout.page_parameter == "offset" {
            page.checked_sub(1)
                .and_then(|index| index.checked_mul(per_page))
                .ok_or_else(|| {
                    AppError::DataParsing(format!(
                        "Request offset overflowed for page {page} and page size {per_page}"
                    ))
                })?
                .to_string()
        } else {
            page.to_string()
        };
        let endpoint = with_query_parameters(
            &endpoint,
            &[
                (portal.api_layout.page_parameter.as_str(), page_value),
                (
                    portal.api_layout.page_size_parameter.as_str(),
                    per_page.to_string(),
                ),
            ],
        );
        let query = Vec::new();
        self.governor.mark_user_directed(&endpoint);
        let payload = self
            .get_bounded(endpoint, &query, self.config.max_json_response_bytes)
            .await?;
        let json: Value = serde_json::from_slice(&payload.bytes).map_err(|error| {
            AppError::DataParsing(format!(
                "Portal '{}' returned invalid JSON from '{}': {error}",
                portal.portal_key, payload.final_url
            ))
        })?;
        decode_nextrequest_response(
            &json,
            &portal.api_layout.requests_pointer,
            portal.api_layout.metadata_pointer.as_deref(),
            page,
            per_page,
        )
    }

    pub async fn download_document_to_path(
        &self,
        portal: &PortalRegistration,
        download_url: &str,
        destination: &Path,
    ) -> Result<DownloadedPayload, AppError> {
        let url = normalize_document_url(&portal.origin_url, download_url)?;
        self.download_url_to_path(url, destination).await
    }

    /// Downloads an absolute URL the app itself names, with no portal origin to resolve against.
    pub async fn download_url_to_path(
        &self,
        url: Url,
        destination: &Path,
    ) -> Result<DownloadedPayload, AppError> {
        validate_remote_url(&self.dns, &url).await?;
        self.execute_with_retries(&url, "Download", || async {
            let result = self
                .stream_response_to_path(url.clone(), destination, self.config.max_document_bytes)
                .await;
            if result.is_err() {
                let _ = tokio::fs::remove_file(destination).await;
            }
            result
        })
        .await
    }

    /// A GET that carries the caller's headers and follows no redirect, so a `Range` or an API
    /// key is answered by the host it was addressed to or not at all. Header values never appear
    /// in an error.
    pub async fn get_direct(
        &self,
        url: Url,
        headers: &[(&'static str, &str)],
        maximum_bytes: usize,
    ) -> Result<DirectResponse, AppError> {
        validate_remote_url(&self.dns, &url).await?;
        self.execute_with_retries(&url, "Request", || async {
            let _host_permit = self.reserve_host_slot(&url).await?;
            let mut request = self
                .client
                .get(url.clone())
                .timeout(self.transfer_deadline(maximum_bytes));
            for (name, value) in headers {
                request = request.header(*name, *value);
            }
            let response = request.send().await.map_err(|error| {
                RetryDisposition::Retry(
                    AppError::Network(format!("GET '{url}' failed: {}", error.without_url())),
                    None,
                )
            })?;
            let status = response.status();
            self.governor.record_status(
                &url,
                status.as_u16(),
                parse_retry_after(response.headers()),
            );
            if status.is_redirection() {
                return Err(RetryDisposition::Terminal(AppError::PortalDiscovery(
                    format!("GET '{url}' was redirected, which this request does not follow"),
                )));
            }
            validate_success_status("GET", &url, &response)?;
            let content_range = response
                .headers()
                .get(reqwest::header::CONTENT_RANGE)
                .and_then(|value| value.to_str().ok())
                .map(str::to_string);
            let (_, bytes) = read_response_bytes(response, maximum_bytes, "Response")
                .await
                .map_err(|error| {
                    if error.is_retryable() {
                        RetryDisposition::Retry(error, None)
                    } else {
                        RetryDisposition::Terminal(error)
                    }
                })?;
            Ok(DirectResponse {
                status: status.as_u16(),
                content_range,
                bytes,
            })
        })
        .await
    }

    async fn pagination_probe_advances(
        &self,
        endpoint: &Url,
        layout: &DiscoveredLayout,
        first_page: &Value,
        page_parameter: &str,
        page_size_parameter: &str,
    ) -> Result<bool, AppError> {
        if !pagination_metadata_reports_more(first_page, layout.metadata_pointer.as_deref()) {
            return Ok(true);
        }
        let first_identifier = response_records(first_page, &layout.requests_pointer)
            .and_then(|records| records.first())
            .and_then(|record| record.get("id"))
            .map(Value::to_string);
        let page_value = if page_parameter == "offset" { "1" } else { "2" };
        let query = [
            (page_parameter, page_value.to_string()),
            (page_size_parameter, "1".to_string()),
        ];
        let payload = self
            .get_bounded(
                endpoint.clone(),
                &query,
                self.config.max_json_response_bytes,
            )
            .await?;
        let second_page: Value = serde_json::from_slice(&payload.bytes).map_err(|error| {
            AppError::DataParsing(format!(
                "Pagination probe from '{}' returned invalid JSON: {error}",
                payload.final_url
            ))
        })?;
        let second_identifier = response_records(&second_page, &layout.requests_pointer)
            .and_then(|records| records.first())
            .and_then(|record| record.get("id"))
            .map(Value::to_string);
        Ok(first_identifier.is_some()
            && second_identifier.is_some()
            && first_identifier != second_identifier)
    }

    async fn get_bounded(
        &self,
        initial_url: Url,
        query: &[(&str, String)],
        maximum_bytes: usize,
    ) -> Result<ResponsePayload, AppError> {
        self.execute_with_retries(&initial_url, "Request", || {
            self.send_get_following_safe_redirects(initial_url.clone(), query, maximum_bytes)
        })
        .await
    }

    /// The permit rides along with the response because the host's concurrency slot is not free
    /// until the body has been read, not merely until the headers arrive.
    async fn response_with_retries(
        &self,
        initial_url: Url,
        maximum_bytes: usize,
    ) -> Result<(reqwest::Response, HostPermit, AcquisitionRecord), AppError> {
        self.execute_with_retries(&initial_url, "Request", || async {
            self.send_response_following_safe_redirects(
                initial_url.clone(),
                &[],
                maximum_bytes,
                false,
            )
            .await
            .map(|(response, permit, redirect_chain)| {
                let acquisition = acquisition_record(&initial_url, &response, redirect_chain);
                (response, permit, acquisition)
            })
        })
        .await
    }

    async fn execute_with_retries<T, Operation, OperationFuture>(
        &self,
        initial_url: &Url,
        operation_name: &str,
        mut operation: Operation,
    ) -> Result<T, AppError>
    where
        Operation: FnMut() -> OperationFuture,
        OperationFuture: Future<Output = Result<T, RetryDisposition>>,
    {
        let maximum_attempts = self.config.max_retries.saturating_add(1);
        let retry_base = Duration::from_millis(self.config.retry_initial_delay_ms.max(1));
        let retry_cap = Duration::from_millis(self.config.retry_max_delay_ms.max(1));
        let mut last_error = None;
        for attempt in 0..maximum_attempts {
            if attempt > 0 {
                // The backoff can outlast `DNS_PIN_TTL`, which would fail the retry closed in the
                // resolver rather than retrying. Re-validating also rechecks an address that was
                // approved minutes ago, which is the right thing to do on its own.
                validate_remote_url(&self.dns, initial_url).await?;
            }
            match operation().await {
                Ok(value) => return Ok(value),
                Err(RetryDisposition::Terminal(error)) => return Err(error),
                Err(RetryDisposition::Retry(error, retry_after)) => {
                    last_error = Some(error);
                    if attempt + 1 >= maximum_attempts {
                        break;
                    }
                    let wait = retry_after.unwrap_or_else(|| {
                        backoff::exponential_delay(attempt as u64, retry_base, retry_cap)
                    });
                    tokio::time::sleep(wait.min(Duration::from_secs(60))).await;
                }
            }
        }
        Err(last_error.unwrap_or_else(|| {
            AppError::Network(format!(
                "{operation_name} to '{initial_url}' exhausted retry attempts"
            ))
        }))
    }

    async fn send_wordpress_google_drive_request(
        &self,
        endpoint: Url,
        form: &[(&str, &str)],
    ) -> Result<(reqwest::Response, HostPermit), RetryDisposition> {
        let host_permit = self.reserve_host_slot(&endpoint).await?;
        let response = self
            .client
            .post(endpoint.clone())
            .form(form)
            .send()
            .await
            .map_err(|error| {
                RetryDisposition::Retry(
                    AppError::Network(format!(
                        "Google Drive connector request to '{endpoint}' failed: {error}"
                    )),
                    None,
                )
            })?;
        let status = response.status();
        self.governor.record_status(
            &endpoint,
            status.as_u16(),
            parse_retry_after(response.headers()),
        );
        if status.is_redirection() {
            return Err(RetryDisposition::Terminal(AppError::PortalSecurity(
                format!(
                    "Google Drive connector endpoint '{endpoint}' attempted an unverified redirect"
                ),
            )));
        }
        validate_success_status("Google Drive connector endpoint", &endpoint, &response)?;
        Ok((response, host_permit))
    }

    async fn send_get_following_safe_redirects(
        &self,
        initial_url: Url,
        query: &[(&str, String)],
        maximum_bytes: usize,
    ) -> Result<ResponsePayload, RetryDisposition> {
        let requested_url = materialize_request_url(&initial_url, Some(query));
        let (response, _host_permit, redirect_chain) = self
            .send_response_following_safe_redirects(initial_url, query, maximum_bytes, false)
            .await?;
        let acquisition = acquisition_record(&requested_url, &response, redirect_chain);
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let (final_url, bytes) = read_response_bytes(response, maximum_bytes, "Response")
            .await
            .map_err(|error| {
                if error.is_retryable() {
                    RetryDisposition::Retry(error, None)
                } else {
                    RetryDisposition::Terminal(error)
                }
            })?;
        Ok(ResponsePayload {
            final_url,
            content_type,
            bytes,
            acquisition,
        })
    }

    /// Total time a request may take: the configured request timeout plus the time to transfer
    /// `maximum_bytes` at a floor throughput. Keeps a trickling server from holding a worker
    /// indefinitely while still letting the largest permitted document arrive over a slow link.
    fn transfer_deadline(&self, maximum_bytes: usize) -> Duration {
        const MINIMUM_THROUGHPUT_BYTES_PER_SECOND: u64 = 64 * 1024;
        let transfer_seconds = (maximum_bytes as u64).div_ceil(MINIMUM_THROUGHPUT_BYTES_PER_SECOND);
        Duration::from_secs(
            self.config
                .request_timeout_seconds
                .saturating_add(transfer_seconds),
        )
    }

    async fn send_response_following_safe_redirects(
        &self,
        initial_url: Url,
        query: &[(&str, String)],
        maximum_bytes: usize,
        answer_gone: bool,
    ) -> Result<(reqwest::Response, HostPermit, Vec<String>), RetryDisposition> {
        let mut current_url = initial_url;
        let mut current_query = Some(query);
        let mut visited_requests = BTreeSet::new();
        let mut redirect_chain = Vec::new();
        // SECURITY(security): The loop bound is the redirect limit. After pure URL/query and loop
        // bookkeeping, validation is the first operation that can resolve or contact anything,
        // so the initial URL and every subsequent hop are validated before a socket is opened.
        // Hoisting validation out of the loop, or moving it below `send()`, would reintroduce the
        // standard SSRF hole where a permitted host redirects to a private address.
        //
        // Query parameters supplied by the caller apply to the first request only. They are first
        // materialized into `request_url` so loop detection distinguishes that request from a
        // legitimate redirect back to the same path without its pagination query.
        for redirect_count in 0..=MAX_REDIRECTS {
            let request_url = materialize_request_url(&current_url, current_query.take());
            if !visited_requests.insert(request_url.as_str().to_string()) {
                return Err(RetryDisposition::Terminal(AppError::PortalSecurity(
                    format!("Redirect loop detected at '{request_url}'"),
                )));
            }
            validate_remote_url(&self.dns, &request_url)
                .await
                .map_err(RetryDisposition::Terminal)?;
            // Every hop is governed independently: a redirect can cross hosts, and the host that
            // answers is the one whose robots.txt, pacing, budget, and stop state apply.
            let host_permit = self.reserve_host_slot(&request_url).await?;
            let request = self
                .client
                .get(request_url.clone())
                .timeout(self.transfer_deadline(maximum_bytes));
            let response = request.send().await.map_err(|error| {
                RetryDisposition::Retry(
                    AppError::Network(format!("GET '{}' failed: {error}", request_url)),
                    None,
                )
            })?;
            let status = response.status();
            // Recorded before the status is classified so a refusal stops the host even though
            // the disposition below is terminal for this one URL.
            self.governor.record_status(
                &request_url,
                status.as_u16(),
                parse_retry_after(response.headers()),
            );
            if status.is_redirection() {
                if redirect_count == MAX_REDIRECTS {
                    return Err(RetryDisposition::Terminal(AppError::PortalSecurity(
                        format!("Redirect limit exceeded for '{request_url}'"),
                    )));
                }
                current_url =
                    redirect_url_from_header(&request_url, response.headers().get(LOCATION))
                        .map_err(RetryDisposition::Terminal)?;
                redirect_chain.push(request_url.to_string());
                drop(host_permit);
                continue;
            }
            if answer_gone && matches!(status, StatusCode::NOT_FOUND | StatusCode::GONE) {
                return Ok((response, host_permit, redirect_chain));
            }
            validate_success_status("GET", &request_url, &response)?;
            if let Some(content_length) = response
                .headers()
                .get(CONTENT_LENGTH)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<u64>().ok())
            {
                if content_length > maximum_bytes as u64 {
                    return Err(RetryDisposition::Terminal(AppError::PortalSecurity(
                        format!(
                            "Response from '{request_url}' declared {content_length} bytes, exceeding the {maximum_bytes}-byte limit"
                        ),
                    )));
                }
            }
            return Ok((response, host_permit, redirect_chain));
        }
        Err(RetryDisposition::Terminal(AppError::PortalSecurity(
            "Redirect processing reached an invalid terminal state".to_string(),
        )))
    }

    async fn stream_response_to_path(
        &self,
        initial_url: Url,
        destination: &Path,
        maximum_bytes: usize,
    ) -> Result<DownloadedPayload, RetryDisposition> {
        let (response, _host_permit, redirect_chain) = self
            .send_response_following_safe_redirects(initial_url.clone(), &[], maximum_bytes, false)
            .await?;
        let acquisition = acquisition_record(&initial_url, &response, redirect_chain);
        write_response_to_path(response, acquisition, destination, maximum_bytes).await
    }

    /// Fetches a document again to compare it with what was collected. 404 and 410 come back as
    /// [`SourceRecheck::Gone`] rather than an error, because they are the only answers that say
    /// the source no longer serves the record; every other failure stays an error and says only
    /// that this check did not reach it.
    pub async fn recheck_document_to_path(
        &self,
        portal: &PortalRegistration,
        download_url: &str,
        destination: &Path,
    ) -> Result<SourceRecheck, AppError> {
        let url = normalize_document_url(&portal.origin_url, download_url)?;
        validate_remote_url(&self.dns, &url).await?;
        let maximum_bytes = self.config.max_document_bytes;
        self.execute_with_retries(&url, "Recheck", || async {
            let (response, _host_permit, redirect_chain) = self
                .send_response_following_safe_redirects(url.clone(), &[], maximum_bytes, true)
                .await?;
            let acquisition = acquisition_record(&url, &response, redirect_chain);
            if !response.status().is_success() {
                return Ok(SourceRecheck::Gone(acquisition));
            }
            let result =
                write_response_to_path(response, acquisition, destination, maximum_bytes).await;
            if result.is_err() {
                let _ = tokio::fs::remove_file(destination).await;
            }
            result.map(SourceRecheck::Retrieved)
        })
        .await
    }
}

async fn write_response_to_path(
    response: reqwest::Response,
    acquisition: AcquisitionRecord,
    destination: &Path,
    maximum_bytes: usize,
) -> Result<DownloadedPayload, RetryDisposition> {
    let final_url = response.url().clone();
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .await
        .map_err(|error| {
            RetryDisposition::Terminal(AppError::Storage(format!(
                "Failed to create streaming download '{}': {error}",
                destination.display()
            )))
        })?;
    let mut digest = Sha256::new();
    let mut byte_count = 0_u64;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| {
            RetryDisposition::Retry(
                AppError::Network(format!(
                    "Failed while streaming response from '{final_url}': {error}"
                )),
                None,
            )
        })?;
        byte_count = byte_count.checked_add(chunk.len() as u64).ok_or_else(|| {
            RetryDisposition::Terminal(AppError::PortalSecurity(
                "Document byte count overflowed".to_string(),
            ))
        })?;
        if byte_count > maximum_bytes as u64 {
            return Err(RetryDisposition::Terminal(AppError::PortalSecurity(
                format!("Response from '{final_url}' exceeded the {maximum_bytes}-byte limit"),
            )));
        }
        file.write_all(&chunk).await.map_err(|error| {
            RetryDisposition::Terminal(AppError::Storage(format!(
                "Failed to stream download '{}': {error}",
                destination.display()
            )))
        })?;
        digest.update(&chunk);
    }
    file.sync_all().await.map_err(|error| {
        RetryDisposition::Terminal(AppError::Storage(format!(
            "Failed to sync download '{}': {error}",
            destination.display()
        )))
    })?;
    drop(file);
    Ok(DownloadedPayload {
        path: destination.to_path_buf(),
        byte_count,
        sha256: digest_hex(digest.finalize()),
        acquisition,
    })
}

// ---------------------------------------------------------------------------
// Open-data catalog adapters
//
// A catalog portal publishes an index of its own holdings. Reading that index costs the host one
// request per hundred assets; crawling the HTML that renders it costs a request per page, per
// facet combination, and per navigation link on every one of them. data.ca.gov answers
// `/api/3/action/package_search` with all 4,570 of its datasets, which is why platform detection
// runs before the crawler rather than as a fallback from it.
// ---------------------------------------------------------------------------

/// Catalog families with a documented index API this client can page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DataCatalogPlatform {
    Ckan,
    Socrata,
}

impl DataCatalogPlatform {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ckan => "CKAN",
            Self::Socrata => "Socrata",
        }
    }
}

/// A detected catalog index, resolved to the endpoint that pages it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DataCatalogEndpoint {
    pub platform: DataCatalogPlatform,
    pub api_url: String,
    /// The portal's own host. Socrata's catalog API federates across domains by default, and the
    /// scoping parameters are advisory, so every returned asset is checked against this.
    pub domain: String,
    pub asset_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataCatalogDownload {
    pub url: String,
    pub label: String,
    pub media_type: Option<String>,
    pub byte_count: Option<u64>,
}

/// One page of a catalog index.
///
/// `assets_without_downloads` and `foreign_domain_assets` are not diagnostics. An index page whose
/// downloads are reported without them states a coverage this run did not achieve.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DataCatalogPage {
    pub downloads: Vec<DataCatalogDownload>,
    pub next_cursor: Option<String>,
    /// The catalog's own total, when it reported one. `None` is "not reported", never zero: a
    /// page count is not a corpus count, and paging must not stop on a total the index withheld.
    pub asset_count: Option<usize>,
    pub assets_without_downloads: BTreeMap<String, u64>,
    pub foreign_domain_assets: u64,
}

/// Assets requested per index page. Kept well under CKAN's default `ckan.search.rows_max` of
/// 1,000 and Socrata's `limit` ceiling, because a page holds full metadata for every asset on it.
const CATALOG_PAGE_SIZE: usize = 100;
/// A catalog index page is metadata; a body larger than this is not one.
const CATALOG_PAGE_LIMIT_BYTES: usize = 32 * 1024 * 1024;
/// Socrata asset types that resolve to a retrievable tabular export. Every other type is counted
/// rather than guessed at: a `href` asset is a link to somebody else's server, and a `blob` needs
/// a file identifier the catalog does not carry.
const SOCRATA_TABULAR_TYPES: &[&str] = &["dataset", "table", "map", "chart", "filter"];

impl ApiClient {
    /// Identifies the catalog family behind a URL by asking each platform's index endpoint for a
    /// single record. Returns `None` when no platform answers, which leaves the crawler as the
    /// fallback for genuinely static HTML.
    pub async fn detect_data_catalog(&self, source_url: &str) -> Option<DataCatalogEndpoint> {
        let url = normalize_submitted_url(source_url).ok()?;
        if !super::regional::is_catalog_root(&url) {
            return None;
        }
        let origin = origin_url(&url).ok()?;
        let domain = url.host_str()?.to_ascii_lowercase();
        if super::regional::socrata_host(&domain) {
            return self.detect_socrata(&origin, &domain).await;
        }
        if super::regional::is_publication_site(&url) && domain != "data.ca.gov" {
            return None;
        }
        if let Some(endpoint) = self.detect_ckan(&origin, &domain).await {
            return Some(endpoint);
        }
        self.detect_socrata(&origin, &domain).await
    }

    async fn detect_ckan(&self, origin: &Url, domain: &str) -> Option<DataCatalogEndpoint> {
        let api_url = origin.join("/api/3/action/package_search").ok()?;
        // The publisher's own documented index, reached because the operator named this portal.
        // See `ApiClient::mark_user_directed` for why robots path exclusions do not govern it.
        self.governor.mark_user_directed(&api_url);
        let payload = self
            .get_bounded(
                api_url.clone(),
                &[("rows", "1".to_string()), ("start", "0".to_string())],
                self.config.max_json_response_bytes,
            )
            .await
            .ok()?;
        let value: Value = serde_json::from_slice(&payload.bytes).ok()?;
        if value.get("success") != Some(&Value::Bool(true)) {
            return None;
        }
        let result = value.get("result")?;
        result.get("results")?.as_array()?;
        Some(DataCatalogEndpoint {
            platform: DataCatalogPlatform::Ckan,
            api_url: api_url.to_string(),
            domain: domain.to_string(),
            asset_count: result.get("count").and_then(Value::as_u64).unwrap_or(0) as usize,
        })
    }

    async fn detect_socrata(&self, origin: &Url, domain: &str) -> Option<DataCatalogEndpoint> {
        let api_url = origin.join("/api/catalog/v1").ok()?;
        self.governor.mark_user_directed(&api_url);
        let payload = self
            .get_bounded(
                api_url.clone(),
                &[
                    ("limit", "1".to_string()),
                    ("domains", domain.to_string()),
                    ("search_context", domain.to_string()),
                ],
                self.config.max_json_response_bytes,
            )
            .await
            .ok()?;
        let value: Value = serde_json::from_slice(&payload.bytes).ok()?;
        value.get("results")?.as_array()?;
        let asset_count = value
            .get("resultSetSize")
            .and_then(Value::as_u64)
            .unwrap_or(0) as usize;
        Some(DataCatalogEndpoint {
            platform: DataCatalogPlatform::Socrata,
            api_url: api_url.to_string(),
            domain: domain.to_string(),
            asset_count,
        })
    }

    /// Reads one page of a catalog index. `cursor` is opaque and comes from the previous page's
    /// `next_cursor`; `None` starts at the beginning.
    pub async fn fetch_data_catalog_page(
        &self,
        endpoint: &DataCatalogEndpoint,
        cursor: Option<&str>,
    ) -> Result<DataCatalogPage, AppError> {
        let api_url = Url::parse(&endpoint.api_url).map_err(|error| {
            AppError::PortalDiscovery(format!(
                "Stored {} catalog endpoint is invalid: {error}",
                endpoint.platform.as_str()
            ))
        })?;
        validate_remote_url(&self.dns, &api_url).await?;
        self.governor.mark_user_directed(&api_url);
        let query = match endpoint.platform {
            DataCatalogPlatform::Ckan => vec![
                ("rows", CATALOG_PAGE_SIZE.to_string()),
                (
                    "start",
                    cursor
                        .and_then(|value| value.parse::<usize>().ok())
                        .unwrap_or(0)
                        .to_string(),
                ),
            ],
            // Socrata's `offset` paging refuses `offset + limit > 10000` outright, so this uses
            // the deep-scrolling cursor instead. Paging by offset would silently stop at ten
            // thousand assets on any portal larger than that and report the first ten thousand as
            // the whole catalog.
            DataCatalogPlatform::Socrata => vec![
                ("limit", CATALOG_PAGE_SIZE.to_string()),
                ("scroll_id", cursor.unwrap_or_default().to_string()),
                ("domains", endpoint.domain.clone()),
                ("search_context", endpoint.domain.clone()),
            ],
        };
        let payload = self
            .get_bounded(api_url.clone(), &query, CATALOG_PAGE_LIMIT_BYTES)
            .await?;
        let value: Value = serde_json::from_slice(&payload.bytes).map_err(|error| {
            AppError::DataParsing(format!(
                "{} catalog page from '{}' was not JSON: {error}",
                endpoint.platform.as_str(),
                payload.final_url
            ))
        })?;
        match endpoint.platform {
            DataCatalogPlatform::Ckan => decode_ckan_page(&value, cursor),
            DataCatalogPlatform::Socrata => decode_socrata_page(&value, &endpoint.domain),
        }
    }
}

fn decode_ckan_page(value: &Value, cursor: Option<&str>) -> Result<DataCatalogPage, AppError> {
    if value.get("success") != Some(&Value::Bool(true)) {
        return Err(AppError::DataParsing(
            "The CKAN catalog reported an unsuccessful search".to_string(),
        ));
    }
    let result = value.get("result").ok_or_else(|| {
        AppError::DataParsing("The CKAN catalog page omitted its result object".to_string())
    })?;
    let packages = result
        .get("results")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            AppError::DataParsing("The CKAN catalog page omitted its result list".to_string())
        })?;
    let asset_count = result
        .get("count")
        .and_then(Value::as_u64)
        .map(|count| count as usize);
    let start = cursor
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    let mut page = DataCatalogPage {
        asset_count,
        ..DataCatalogPage::default()
    };
    for package in packages {
        let resources = package
            .get("resources")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let mut retrievable = 0_usize;
        for resource in resources {
            let Some(url) = resource.get("url").and_then(Value::as_str) else {
                continue;
            };
            let Ok(parsed) = Url::parse(url) else {
                continue;
            };
            if parsed.scheme() != "https" {
                continue;
            }
            retrievable += 1;
            page.downloads.push(DataCatalogDownload {
                label: resource
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|value| !value.trim().is_empty())
                    .map(|value| value.chars().take(256).collect())
                    .unwrap_or_else(|| file_name_from_url(&parsed)),
                url: parsed.to_string(),
                media_type: resource
                    .get("mimetype")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or_else(|| {
                        resource
                            .get("format")
                            .and_then(Value::as_str)
                            .and_then(catalog_format_media_type)
                            .map(str::to_string)
                    }),
                byte_count: resource.get("size").and_then(Value::as_u64),
            });
        }
        if retrievable == 0 {
            *page
                .assets_without_downloads
                .entry("ckan-package-without-retrievable-resource".to_string())
                .or_insert(0) += 1;
        }
    }
    let consumed = start.saturating_add(packages.len());
    // Without a reported total, a full page is the only sign that more may follow.
    let more = match asset_count {
        Some(count) => consumed < count,
        None => packages.len() >= CATALOG_PAGE_SIZE,
    };
    page.next_cursor = (!packages.is_empty() && more).then(|| consumed.to_string());
    Ok(page)
}

fn decode_socrata_page(value: &Value, domain: &str) -> Result<DataCatalogPage, AppError> {
    let results = value
        .get("results")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            AppError::DataParsing("The Socrata catalog page omitted its result list".to_string())
        })?;
    let mut page = DataCatalogPage {
        asset_count: value
            .get("resultSetSize")
            .and_then(Value::as_u64)
            .map(|count| count as usize),
        ..DataCatalogPage::default()
    };
    let mut last_identifier = None;
    let mut unidentified = 0_u64;
    for entry in results {
        let Some(identifier) = entry
            .get("resource")
            .and_then(|resource| resource.get("id"))
            .and_then(Value::as_str)
        else {
            unidentified += 1;
            continue;
        };
        let resource = &entry["resource"];
        last_identifier = Some(identifier.to_string());
        // `domains` and `search_context` are advisory. A portal that ignores them answers with
        // another domain's assets, which is how a scoped catalog read turns back into an
        // unbounded one; those are refused here rather than downloaded.
        let asset_domain = entry
            .get("metadata")
            .and_then(|metadata| metadata.get("domain"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_ascii_lowercase();
        if asset_domain != domain {
            page.foreign_domain_assets = page.foreign_domain_assets.saturating_add(1);
            continue;
        }
        let asset_type = resource
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_ascii_lowercase();
        if !SOCRATA_TABULAR_TYPES.contains(&asset_type.as_str()) {
            *page
                .assets_without_downloads
                .entry(format!("socrata-{asset_type}-has-no-tabular-export"))
                .or_insert(0) += 1;
            continue;
        }
        if !identifier
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            *page
                .assets_without_downloads
                .entry("socrata-unusable-asset-identifier".to_string())
                .or_insert(0) += 1;
            continue;
        }
        page.downloads.push(DataCatalogDownload {
            url: format!("https://{domain}/api/views/{identifier}/rows.csv?accessType=DOWNLOAD"),
            label: resource
                .get("name")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(|value| value.chars().take(256).collect())
                .unwrap_or_else(|| identifier.to_string()),
            media_type: Some("text/csv".to_string()),
            byte_count: None,
        });
    }
    if unidentified > 0 {
        page.assets_without_downloads
            .insert("socrata-entry-without-identifier".to_string(), unidentified);
    }
    // The scroll cursor is the last identifier on the page. A non-empty page whose entries carry
    // none cannot be advanced past, and ending the scroll there would report the catalog as
    // exhausted when it was not read.
    if !results.is_empty() && last_identifier.is_none() {
        return Err(AppError::DataParsing(
            "The Socrata catalog page carried no asset identifiers, so the scroll cannot continue"
                .to_string(),
        ));
    }
    page.next_cursor = last_identifier;
    Ok(page)
}

/// Catalog `format` strings are free text. Only the unambiguous ones are mapped; an unrecognized
/// format leaves the media type absent so extraction reads the bytes rather than a guess.
pub(crate) fn catalog_format_media_type(format: &str) -> Option<&'static str> {
    match format.trim().to_ascii_uppercase().as_str() {
        "CSV" => Some("text/csv"),
        "TSV" => Some("text/tab-separated-values"),
        "JSON" => Some("application/json"),
        "GEOJSON" => Some("application/geo+json"),
        "XML" => Some("application/xml"),
        "PDF" => Some("application/pdf"),
        "XLSX" => Some("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"),
        "XLS" => Some("application/vnd.ms-excel"),
        "ZIP" => Some("application/zip"),
        "TXT" => Some("text/plain"),
        _ => None,
    }
}

enum RetryDisposition {
    Retry(AppError, Option<Duration>),
    Terminal(AppError),
}

fn validate_success_status(
    operation: &str,
    url: &Url,
    response: &reqwest::Response,
) -> Result<(), RetryDisposition> {
    let status = response.status();
    if status == StatusCode::REQUEST_TIMEOUT
        || status == StatusCode::TOO_MANY_REQUESTS
        || status.is_server_error()
    {
        return Err(RetryDisposition::Retry(
            AppError::Network(format!("{operation} '{url}' returned HTTP {status}")),
            parse_retry_after(response.headers()),
        ));
    }
    if !status.is_success() {
        return Err(RetryDisposition::Terminal(AppError::PortalDiscovery(
            format!("{operation} '{url}' returned HTTP {status}"),
        )));
    }
    Ok(())
}

async fn read_response_bytes(
    response: reqwest::Response,
    maximum_bytes: usize,
    label: &str,
) -> Result<(Url, Vec<u8>), AppError> {
    let final_url = response.url().clone();
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| {
            AppError::Network(format!(
                "Failed while reading {label} '{final_url}': {error}"
            ))
        })?;
        if bytes.len().saturating_add(chunk.len()) > maximum_bytes {
            return Err(AppError::PortalSecurity(format!(
                "{label} '{final_url}' exceeded the {maximum_bytes}-byte discovery limit"
            )));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok((final_url, bytes))
}

fn validate_connector_token(
    label: &str,
    value: &str,
    maximum_length: usize,
) -> Result<(), AppError> {
    if value.is_empty()
        || value.len() > maximum_length
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b' '))
    {
        return Err(AppError::PortalSecurity(format!(
            "Google Drive connector {label} is malformed"
        )));
    }
    Ok(())
}

fn validate_arcgis_layer_url(url: &Url) -> Result<(), AppError> {
    if url.scheme() != "https" {
        return Err(AppError::PortalSecurity(
            "ArcGIS layer queries require HTTPS".to_string(),
        ));
    }
    let segments = url
        .path_segments()
        .map(|segments| segments.collect::<Vec<_>>())
        .unwrap_or_default();
    let valid = segments.windows(2).any(|window| {
        matches!(window[0], "FeatureServer" | "MapServer")
            && window[1].bytes().all(|byte| byte.is_ascii_digit())
            && !window[1].is_empty()
    });
    if !valid {
        return Err(AppError::DataParsing(format!(
            "ArcGIS layer URL '{}' does not identify a numbered FeatureServer or MapServer layer",
            url
        )));
    }
    Ok(())
}

fn decode_wordpress_google_drive_listing(
    bytes: &[u8],
) -> Result<Vec<WordPressGoogleDriveItem>, AppError> {
    let response: Value = serde_json::from_slice(bytes).map_err(|error| {
        AppError::DataParsing(format!(
            "Google Drive connector returned invalid JSON: {error}"
        ))
    })?;
    if response.get("success").and_then(Value::as_bool) != Some(true) {
        return Err(AppError::DataParsing(
            "Google Drive connector rejected the public folder listing request".to_string(),
        ));
    }
    let files = response
        .pointer("/data/files")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            AppError::DataParsing(
                "Google Drive connector response omitted its file collection".to_string(),
            )
        })?;
    if files.len() > 10_000 {
        return Err(AppError::PortalCapacity(
            "Google Drive connector returned more than 10000 entries in one folder".to_string(),
        ));
    }
    files
        .iter()
        .map(|file| {
            let required = |name: &str| {
                file.get(name)
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
                    .ok_or_else(|| {
                        AppError::DataParsing(format!(
                            "Google Drive connector file omitted '{name}'"
                        ))
                    })
            };
            let id = required("id")?;
            let account_id = required("accountId")?;
            validate_connector_token("file ID", &id, 256)?;
            validate_connector_token("account ID", &account_id, 256)?;
            Ok(WordPressGoogleDriveItem {
                id,
                account_id,
                name: required("name")?.chars().take(512).collect(),
                media_type: required("type")?,
                is_folder: file
                    .get("isFolder")
                    .and_then(Value::as_bool)
                    .unwrap_or_else(|| {
                        file.get("type").and_then(Value::as_str)
                            == Some("application/vnd.google-apps.folder")
                    }),
                web_view_link: file
                    .get("webViewLink")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string),
                web_content_link: file
                    .get("webContentLink")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string),
            })
        })
        .collect()
}

fn is_html_resource(url: &Url, content_type: Option<&str>) -> bool {
    if content_type.is_some_and(|value| value == "text/html" || value == "application/xhtml+xml") {
        return true;
    }
    if content_type.is_some_and(|value| {
        value.starts_with("application/")
            || value.starts_with("image/")
            || value.starts_with("audio/")
            || value.starts_with("video/")
            || value == "text/csv"
            || value == "text/plain"
            || value == "text/xml"
    }) {
        return false;
    }
    !has_download_extension(url.path())
}

fn classify_source_kind(url: &Url, resource_kind: &PublicResourceKind) -> SourceKind {
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    if matches!(resource_kind, PublicResourceKind::Download) {
        return SourceKind::DirectFile;
    }
    if host.contains("arcgis.com") || host.contains("arcgisonline.com") {
        SourceKind::ArcGis
    } else if host.contains("tableau.com") {
        SourceKind::Tableau
    } else if host == "drive.google.com" || host == "docs.google.com" {
        SourceKind::GoogleDrive
    } else if looks_like_open_data_catalog(url) {
        SourceKind::OpenDataCatalog
    } else {
        SourceKind::WebCrawl
    }
}

fn looks_like_open_data_catalog(url: &Url) -> bool {
    let path = url.path().to_ascii_lowercase();
    path.contains("/dataset") || path.contains("/datasets") || path.contains("/catalog")
}

/// An optional item failure leaves its original follow link intact for the ordinary
/// crawl frontier. Successful agenda text and other item attachments remain usable.
fn merge_hyland_item_attachments(
    item_url: &Url,
    item_title: &str,
    result: Result<ResponsePayload, AppError>,
    links: &mut Vec<PublicResourceLink>,
    seen: &mut BTreeSet<String>,
) {
    let Ok(item) = result else {
        return;
    };
    if item.final_url != *item_url
        || !is_html_resource(&item.final_url, item.content_type.as_deref())
    {
        return;
    }
    let Ok((_, attachments, _)) =
        extract_public_resource_links(&item.final_url, &item.bytes, None, None)
    else {
        return;
    };
    for mut attachment in attachments {
        if links.len() >= MAX_PUBLIC_LINKS_PER_PAGE {
            break;
        }
        if !attachment.download_candidate
            || !Url::parse(&attachment.url)
                .is_ok_and(|url| super::regional::hyland_item_attachment(item_url, &url))
            || !seen.insert(attachment.url.clone())
        {
            continue;
        }
        attachment.label = format!(
            "{} — {}",
            item_title.chars().take(160).collect::<String>(),
            attachment.label.chars().take(160).collect::<String>()
        );
        links.push(attachment);
    }
}

/// Snapshot references declared by this successfully retrieved body only. Packet
/// expansion may add links from optional network responses to `inspection.links`;
/// those must not be mistaken for changes to this body's publication listing.
pub(crate) fn publication_listing_links(
    source_url: &str,
    body: &[u8],
) -> Result<Vec<String>, AppError> {
    let base = Url::parse(source_url).map_err(|error| AppError::DataParsing(error.to_string()))?;
    let (_, links, _) = extract_public_resource_links(&base, body, None, None)?;
    Ok(links
        .into_iter()
        .filter(|link| {
            !link.navigation_chrome
                && (link.download_candidate
                    || Url::parse(&link.url)
                        .is_ok_and(|url| super::regional::meeting_identity(&url).is_some()))
        })
        .map(|link| link.url)
        .collect())
}

fn extract_public_resource_links(
    base: &Url,
    body: &[u8],
    custom_link_selector: Option<&str>,
    custom_download_selector: Option<&str>,
) -> Result<
    (
        String,
        Vec<PublicResourceLink>,
        Vec<PublicEmbeddedConnector>,
    ),
    AppError,
> {
    let text = String::from_utf8_lossy(body);
    let document = Html::parse_document(&text);
    let title_selector = Selector::parse("title, h1").map_err(|error| {
        AppError::Internal(format!("Failed to initialize HTML title selector: {error}"))
    })?;
    let title = document
        .select(&title_selector)
        .find_map(|element| {
            let value = element.text().collect::<Vec<_>>().join(" ");
            let value = value.split_whitespace().collect::<Vec<_>>().join(" ");
            (!value.is_empty()).then(|| value.chars().take(256).collect())
        })
        .unwrap_or_else(|| base.host_str().unwrap_or("Public source").to_string());
    let mut combined_selector =
        "a[href], link[href], iframe[src], frame[src], embed[src], script[src], object[data]"
            .to_string();
    for selector in [custom_link_selector, custom_download_selector]
        .into_iter()
        .flatten()
    {
        combined_selector.push_str(", ");
        combined_selector.push_str(selector);
    }
    let link_selector = Selector::parse(&combined_selector).map_err(|error| {
        AppError::DataParsing(format!(
            "Failed to initialize HTML resource selector '{combined_selector}': {error}"
        ))
    })?;
    let download_selector = custom_download_selector
        .map(Selector::parse)
        .transpose()
        .map_err(|error| {
            AppError::DataParsing(format!(
                "Failed to initialize custom download selector: {error}"
            ))
        })?;
    let forced_download_urls = download_selector
        .as_ref()
        .map(|selector| {
            document
                .select(selector)
                .filter_map(|element| resolved_public_url(base, &element))
                .map(|mut url| {
                    strip_tracking_parameters(&mut url);
                    url.to_string()
                })
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default();
    let base_host = base.host_str().unwrap_or_default().to_ascii_lowercase();
    let mut seen = BTreeSet::new();
    let mut links = Vec::new();
    for element in document.select(&link_selector) {
        let Some(mut url) = resolved_public_url(base, &element) else {
            continue;
        };
        strip_tracking_parameters(&mut url);
        let canonical = url.to_string();
        let forced_download = forced_download_urls.contains(&canonical);
        if !forced_download && should_ignore_crawl_resource(&element, &url) {
            continue;
        }
        if !seen.insert(canonical.clone()) {
            continue;
        }
        let board_attachment = super::regional::sdhc_board_attachment(base, &url, &element);
        let label = element
            .text()
            .collect::<Vec<_>>()
            .join(" ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let download_candidate = forced_download
            || board_attachment.is_some()
            || super::regional::is_datagov_metadata_url(&url)
            || element.value().attr("download").is_some()
            || has_download_extension(url.path())
            || url.path().to_ascii_lowercase().contains("/download")
            || url.query().is_some_and(|query| {
                let query = query.to_ascii_lowercase();
                query.contains("download=") || query.contains("format=csv")
            });
        let target_host = url.host_str().unwrap_or_default().to_ascii_lowercase();
        let follow_candidate = target_host == base_host || is_supported_platform_host(&target_host);
        let navigation_chrome =
            is_navigation_chrome(&element) && !is_san_diego_policy_page_link(base, &url, &element);
        if !download_candidate && !follow_candidate {
            continue;
        }
        links.push(PublicResourceLink {
            url: canonical,
            label: if let Some(context) = board_attachment {
                context
            } else if label.is_empty() {
                file_name_from_url(&url)
            } else {
                label.chars().take(256).collect()
            },
            download_candidate,
            follow_candidate,
            navigation_chrome,
        });
        if links.len() >= MAX_PUBLIC_LINKS_PER_PAGE {
            break;
        }
    }
    if super::regional::is_sdhc_board_index(base) {
        // Resource limits must reach the shelter agreements and their evidence before
        // unrelated navigation or presentation slides consume the collection budget.
        links.sort_by_key(|link| {
            let label = link.label.to_lowercase();
            let consequential = [
                "shelter",
                "homeless",
                "budget",
                "audit",
                "financial",
                "performance",
            ]
            .iter()
            .any(|term| label.contains(term));
            let report = label.starts_with("hcr");
            (
                !link.download_candidate,
                !consequential,
                !report,
                link.navigation_chrome,
            )
        });
    }
    let embedded_connectors = extract_embedded_connectors(base, &document)?;
    for (url, label) in super::regional::hyland_agenda_items(base, &text) {
        if seen.insert(url.clone()) {
            links.push(PublicResourceLink {
                url,
                label,
                download_candidate: false,
                follow_candidate: true,
                navigation_chrome: false,
            });
        }
    }
    if let Some(url) = super::regional::hyland_document_companion(base, &text) {
        if seen.insert(url.clone()) {
            links.push(PublicResourceLink {
                url,
                label: "Published meeting document and attachments".into(),
                download_candidate: false,
                follow_candidate: true,
                navigation_chrome: false,
            });
        }
    }
    for url in super::queue::arcgis_hub_companion_urls(base.as_str(), body) {
        if seen.insert(url.clone()) {
            links.push(PublicResourceLink {
                url,
                label: "ArcGIS published catalog metadata".into(),
                download_candidate: true,
                follow_candidate: false,
                navigation_chrome: false,
            });
        }
    }
    if base.host_str() == Some("sandiego.granicus.com") {
        // Granicus archive buttons use a literal window.open URL. Read only that literal;
        // never evaluate inline JavaScript or fetch media files as documents.
        let buttons = Selector::parse("a[onclick]").expect("static selector");
        let literal = regex::Regex::new(r#"window\.open\(\s*['"]([^'"]+)['"]"#)
            .expect("static popup pattern");
        for button in document.select(&buttons).take(50_000) {
            let Some(capture) = button
                .value()
                .attr("onclick")
                .and_then(|value| literal.captures(value))
            else {
                continue;
            };
            let Ok(mut url) = base.join(&capture[1]) else {
                continue;
            };
            if url.host_str() != base.host_str() || !matches!(url.scheme(), "http" | "https") {
                continue;
            }
            strip_tracking_parameters(&mut url);
            if super::regional::meeting_identity(&url).is_some() && seen.insert(url.to_string()) {
                links.push(PublicResourceLink {
                    url: url.to_string(),
                    label: button.text().collect::<Vec<_>>().join(" "),
                    download_candidate: false,
                    follow_candidate: true,
                    navigation_chrome: false,
                });
            }
        }
    }
    for (url, label) in super::regional::dataset_companions(base) {
        if seen.insert(url.clone()) {
            links.push(PublicResourceLink {
                url,
                label,
                download_candidate: true,
                follow_candidate: false,
                navigation_chrome: false,
            });
        }
    }
    for (url, label) in super::regional::opengov_companions(base, &text) {
        if seen.insert(url.clone()) {
            links.push(PublicResourceLink {
                url,
                label,
                download_candidate: true,
                follow_candidate: false,
                navigation_chrome: false,
            });
        }
    }
    // The County's Open Budget app publishes these identifiers in its public configuration.
    // Resolve those declarations, rather than enumerating the unrelated county-wide catalog.
    if base.host_str() == Some("sandiegocounty.budget.socrata.com") {
        let pattern = regex::Regex::new(
            r#"\"((?:operating|revenue)_budget_dataset_id)\"\s*:\s*\"([a-z0-9]{4}-[a-z0-9]{4})\""#,
        )
        .expect("static pattern");
        for capture in pattern.captures_iter(&text).take(8) {
            let id = &capture[2];
            for (suffix, label) in [
                (".json", "Budget schema and attribution"),
                ("/rows.csv?accessType=DOWNLOAD", "Published budget CSV"),
            ] {
                let url = format!("https://data.sandiegocounty.gov/api/views/{id}{suffix}");
                if seen.insert(url.clone()) {
                    links.push(PublicResourceLink {
                        url,
                        label: label.into(),
                        download_candidate: true,
                        follow_candidate: false,
                        navigation_chrome: false,
                    });
                }
            }
        }
    }
    Ok((title, links, embedded_connectors))
}

fn extract_embedded_connectors(
    base: &Url,
    document: &Html,
) -> Result<Vec<PublicEmbeddedConnector>, AppError> {
    let selector = Selector::parse("[data-shortcode-data]").map_err(|error| {
        AppError::Internal(format!(
            "Failed to initialize embedded connector selector: {error}"
        ))
    })?;
    let endpoint = base.join("/wp-admin/admin-ajax.php").map_err(|error| {
        AppError::DataParsing(format!(
            "Failed to construct WordPress connector endpoint: {error}"
        ))
    })?;
    let mut connectors = Vec::new();
    for element in document.select(&selector).take(32) {
        let Some(encoded) = element.value().attr("data-shortcode-data") else {
            continue;
        };
        if encoded.is_empty() || encoded.len() > 1_000_000 {
            continue;
        }
        let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(encoded) else {
            continue;
        };
        let Ok(configuration) = serde_json::from_slice::<Value>(&decoded) else {
            continue;
        };
        let Some(shortcode_id) = json_token(configuration.get("id")) else {
            continue;
        };
        let Some(nonce) = configuration
            .get("nonce")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty() && value.len() <= 256)
        else {
            continue;
        };
        let account = configuration.get("account");
        let account_id = account
            .and_then(|value| json_token(value.get("id")))
            .unwrap_or_default();
        let folders = account
            .and_then(|value| value.get("specific_folders"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten();
        for folder in folders.take(256) {
            let Some(folder_id) = folder.get("id").and_then(Value::as_str) else {
                continue;
            };
            let resolved_account_id = folder
                .get("accountId")
                .and_then(|value| json_token(Some(value)))
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| account_id.clone());
            if folder_id.is_empty()
                || folder_id.len() > 256
                || resolved_account_id.is_empty()
                || resolved_account_id.len() > 256
            {
                continue;
            }
            connectors.push(PublicEmbeddedConnector::WordPressGoogleDrive(
                WordPressGoogleDriveConnector {
                    endpoint: endpoint.to_string(),
                    shortcode_id: shortcode_id.clone(),
                    nonce: nonce.to_string(),
                    folder_id: folder_id.to_string(),
                    account_id: resolved_account_id,
                    folder_name: folder
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("Public Google Drive")
                        .chars()
                        .take(256)
                        .collect(),
                },
            ));
        }
    }
    Ok(connectors)
}

fn json_token(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn resolved_public_url(base: &Url, element: &scraper::ElementRef<'_>) -> Option<Url> {
    let reference = element
        .value()
        .attr("href")
        .or_else(|| element.value().attr("src"))
        .or_else(|| element.value().attr("data"))?;
    if reference.len() > 4_096 {
        return None;
    }
    let reference_lower = reference.to_ascii_lowercase();
    if reference.starts_with('#')
        || reference_lower.starts_with("mailto:")
        || reference_lower.starts_with("tel:")
        || reference_lower.starts_with("javascript:")
        || reference_lower.starts_with("data:")
    {
        return None;
    }
    let mut url = base.join(reference).ok()?;
    if url.host_str() == Some("view.officeapps.live.com")
        && matches!(url.path(), "/op/view.aspx" | "/op/embed.aspx")
    {
        if let Some(target) = url
            .query_pairs()
            .find(|(key, _)| key == "src")
            .and_then(|(_, value)| Url::parse(&value).ok())
        {
            if target.scheme() == "https"
                && target.username().is_empty()
                && target.password().is_none()
            {
                url = target;
            }
        }
    }
    url.set_fragment(None);
    (url.scheme() == "https").then_some(url)
}

fn should_ignore_crawl_resource(element: &scraper::ElementRef<'_>, url: &Url) -> bool {
    if element.value().name() == "script" {
        return true;
    }
    if element.value().name() == "link"
        && element.value().attr("rel").is_some_and(|value| {
            value.split_ascii_whitespace().any(|relation| {
                matches!(
                    relation.to_ascii_lowercase().as_str(),
                    "stylesheet"
                        | "icon"
                        | "preload"
                        | "prefetch"
                        | "modulepreload"
                        | "dns-prefetch"
                )
            })
        })
    {
        return true;
    }
    let path = url.path().to_ascii_lowercase();
    [
        ".js", ".mjs", ".css", ".map", ".svg", ".ico", ".woff", ".woff2", ".ttf", ".eot", ".otf",
    ]
    .iter()
    .any(|extension| path.ends_with(extension))
}

/// Whether a link sits in a page's navigation chrome rather than its content.
///
/// Chrome is the same on every page of a site, so following it re-enqueues the whole shell once
/// per referring page and buys nothing: `/contact?ref=<referrer>` on data.ca.gov is a distinct
/// URL for every page that links to it. Faceted sidebars are the same shape and worse, since the
/// facet set is combinatorial. Both live under `<nav>`, `<aside>`, `<header>`, or `<footer>` on
/// every catalog checked, so the landmark is the discriminator rather than a path denylist.
///
/// This flags a link rather than dropping it. The crawl frontier refuses to follow flagged links
/// and counts what it refused; a direct download sitting in a footer is still a document.
fn is_navigation_chrome(element: &scraper::ElementRef<'_>) -> bool {
    element
        .ancestors()
        .filter_map(scraper::ElementRef::wrap)
        .any(|ancestor| {
            let value = ancestor.value();
            matches!(value.name(), "nav" | "aside" | "header" | "footer")
                || value
                    .classes()
                    .any(|class| matches!(class, "navbar" | "navbar-collapse"))
                || value.attr("role").is_some_and(|role| {
                    role.split_ascii_whitespace().any(|token| {
                        matches!(
                            token.to_ascii_lowercase().as_str(),
                            "navigation" | "banner" | "contentinfo" | "search"
                        )
                    })
                })
        })
}

// San Diego publishes these as document pages. Probing each for unrelated platform APIs first
// spends the City's request budget before any records are collected.
#[cfg(test)]
fn is_san_diego_document_source(url: &Url) -> bool {
    matches!(url.host_str(), Some("www.sandiego.gov" | "sandiego.gov"))
        && [
            "/cpp",
            "/police/data-transparency/policies-procedures",
            "/city-clerk/officialdocs/legisdocs/brown-act",
            "/sites/default/files",
        ]
        .iter()
        .any(|prefix| url.path() == *prefix || url.path().starts_with(&format!("{prefix}/")))
}

// The SDPD Drupal catalog puts pagination inside a navigation landmark. Treating that landmark
// like the site menu silently leaves the policy library on its first twenty documents.
fn is_san_diego_policy_page_link(
    base: &Url,
    target: &Url,
    element: &scraper::ElementRef<'_>,
) -> bool {
    san_diego_policy_listing_identity(base).is_some()
        && san_diego_policy_listing_identity(target).is_some()
        && target.origin() == base.origin()
        && target.path() == base.path()
        && target
            .query_pairs()
            .any(|(key, value)| key == "page" && value.parse::<u32>().is_ok())
        && element
            .ancestors()
            .filter_map(scraper::ElementRef::wrap)
            .any(|ancestor| {
                ancestor.value().name() == "nav"
                    && ancestor.value().classes().any(|class| class == "pager")
            })
}

// The City's category and page parameters identify different sets of documents, not optional
// search facets. Keep only those parameters in the crawl identity. Both the page range and the
// normal crawl budgets remain bounded, even if a page emits arbitrary query combinations.
pub(super) fn san_diego_policy_listing_identity(url: &Url) -> Option<String> {
    if !matches!(url.host_str(), Some("www.sandiego.gov" | "sandiego.gov")) {
        return None;
    }
    let path = url.path().trim_end_matches('/');
    let root = "/police/data-transparency/policies-procedures";
    let pairs = url.query_pairs().collect::<Vec<_>>();
    let category = if path == format!("{root}/category") {
        let category = pairs.iter().find(|(key, _)| key == "category")?.1.as_ref();
        if ![
            "Department Orders",
            "Policies",
            "Education and Training Material",
            "Operations Manuals",
            "Training Bulletins",
        ]
        .contains(&category)
        {
            return None;
        }
        Some(category)
    } else if path == format!("{root}/procedures") {
        None
    } else {
        return None;
    };
    let page = match pairs.iter().find(|(key, _)| key == "page") {
        Some((_, value)) => value.parse::<u32>().ok()?,
        None => 0,
    };
    if page >= 100 {
        return None;
    }
    let mut identity = url.clone();
    identity.set_path(path);
    identity.set_query(None);
    identity.set_fragment(None);
    if let Some(category) = category {
        identity.query_pairs_mut().append_pair("category", category);
    }
    if page > 0 {
        identity
            .query_pairs_mut()
            .append_pair("page", &page.to_string());
    }
    Some(identity.to_string())
}

fn strip_tracking_parameters(url: &mut Url) {
    let mut retained = url
        .query_pairs()
        .filter(|(name, _)| {
            let name = name.to_ascii_lowercase();
            !name.starts_with("utm_")
                && !matches!(name.as_str(), "fbclid" | "gclid" | "mc_cid" | "mc_eid")
        })
        .map(|(name, value)| (name.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    retained.sort();
    url.set_query(None);
    if !retained.is_empty() {
        url.query_pairs_mut().extend_pairs(retained);
    }
}

fn is_supported_platform_host(host: &str) -> bool {
    [
        "arcgis.com",
        "arcgisonline.com",
        "tableau.com",
        "socrata.com",
        "granicus.com",
        "hylandcloud.com",
        "netfile.com",
    ]
    .iter()
    .any(|platform| host == *platform || host.ends_with(&format!(".{platform}")))
        || host == "drive.google.com"
        || host == "docs.google.com"
}

fn has_download_extension(path: &str) -> bool {
    let path = path.to_ascii_lowercase();
    [
        ".pdf", ".csv", ".tsv", ".json", ".jsonl", ".ndjson", ".geojson", ".xml", ".xlsx", ".xls",
        ".ods", ".docx", ".doc", ".odt", ".pptx", ".ppt", ".rtf", ".txt", ".md", ".zip", ".tar",
        ".gz", ".bz2", ".xz", ".zst", ".7z", ".rar", ".sqlite", ".sqlite3", ".db", ".dbf", ".shp",
        ".parquet", ".avro", ".orc", ".eml", ".msg", ".mbox", ".warc", ".warc.gz", ".png", ".jpg",
        ".jpeg", ".tif", ".tiff", ".webp",
    ]
    .iter()
    .any(|extension| path.ends_with(extension))
}

fn file_name_from_url(url: &Url) -> String {
    url.path_segments()
        .and_then(|mut segments| segments.next_back())
        .filter(|value| !value.is_empty())
        .map(percent_decode_file_name)
        .unwrap_or_else(|| url.host_str().unwrap_or("download").to_string())
}

fn percent_decode_file_name(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let (Some(high), Some(low)) =
                (hex_nibble(bytes[index + 1]), hex_nibble(bytes[index + 2]))
            {
                output.push(char::from((high << 4) | low));
                index += 3;
                continue;
            }
        }
        output.push(char::from(bytes[index]));
        index += 1;
    }
    output
}

fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

pub(crate) fn normalize_submitted_url(value: &str) -> Result<Url, AppError> {
    let value = value.trim();
    if value.is_empty() || value.len() > 2_048 {
        return Err(AppError::PortalDiscovery(
            "Portal URL must contain between 1 and 2048 characters".to_string(),
        ));
    }
    let candidate = if value.contains("://") {
        value.to_string()
    } else {
        format!("https://{value}")
    };
    let mut url = Url::parse(&candidate).map_err(|error| {
        AppError::PortalDiscovery(format!("Portal URL '{value}' is invalid: {error}"))
    })?;
    if let Some(selected) = super::regional::normalize_opengov_selection(&url)
        .map_err(|message| AppError::PortalDiscovery(message.to_string()))?
    {
        return Ok(selected);
    }
    url.set_fragment(None);
    Ok(url)
}

fn normalize_document_url(origin: &str, value: &str) -> Result<Url, AppError> {
    let value = value.trim();
    if value.is_empty() || value.len() > 4_096 {
        return Err(AppError::DataParsing(
            "Document URL must contain between 1 and 4096 characters".to_string(),
        ));
    }
    if value.starts_with("//") {
        return Url::parse(&format!("https:{value}")).map_err(|error| {
            AppError::DataParsing(format!("Document URL '{value}' is invalid: {error}"))
        });
    }
    if let Ok(absolute) = Url::parse(value) {
        return Ok(absolute);
    }
    Url::parse(origin)
        .and_then(|base| base.join(value))
        .map_err(|error| {
            AppError::DataParsing(format!("Document URL '{value}' is invalid: {error}"))
        })
}

fn validate_pagination_parameters(page: &str, page_size: &str) -> Result<(), AppError> {
    for (label, value) in [("page", page), ("page-size", page_size)] {
        if value.is_empty()
            || value.len() > MAX_QUERY_PARAMETER_LENGTH
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        {
            return Err(AppError::PortalDiscovery(format!(
                "Stored {label} query parameter '{value}' is invalid"
            )));
        }
    }
    if page == page_size {
        return Err(AppError::PortalDiscovery(
            "Stored page and page-size query parameters must be distinct".to_string(),
        ));
    }
    Ok(())
}

fn without_query_parameters(url: &Url, names: &[&str]) -> Url {
    let retained = url
        .query_pairs()
        .filter(|(name, _)| !names.iter().any(|candidate| name == *candidate))
        .map(|(name, value)| (name.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    let mut sanitized = url.clone();
    sanitized.set_query(None);
    if !retained.is_empty() {
        sanitized.query_pairs_mut().extend_pairs(retained);
    }
    sanitized
}

fn with_query_parameters(url: &Url, values: &[(&str, String)]) -> Url {
    let names = values.iter().map(|(name, _)| *name).collect::<Vec<_>>();
    let mut updated = without_query_parameters(url, &names);
    updated
        .query_pairs_mut()
        .extend_pairs(values.iter().map(|(name, value)| (*name, value.as_str())));
    updated
}

fn materialize_request_url(current: &Url, query: Option<&[(&str, String)]>) -> Url {
    let mut url = current.clone();
    // Opening query_pairs_mut on an empty slice appends a bare '?'. That changes
    // acquisition identity and causes stable file URLs to fail reference retention.
    if let Some(values) = query.filter(|values| !values.is_empty()) {
        url.query_pairs_mut()
            .extend_pairs(values.iter().map(|(key, value)| (*key, value.as_str())));
    }
    url.set_fragment(None);
    url
}

fn next_redirect_url(current: &Url, location: &str) -> Result<Url, AppError> {
    let mut target = current.join(location).map_err(|error| {
        AppError::Network(format!(
            "Redirect target '{location}' from '{current}' is invalid: {error}"
        ))
    })?;
    // Fragments are not sent in an HTTP request. Removing them prevents a server from evading
    // exact loop detection by alternating `#one` and `#two` on the same request target.
    target.set_fragment(None);
    Ok(target)
}

fn redirect_url_from_header(
    current: &Url,
    location: Option<&HeaderValue>,
) -> Result<Url, AppError> {
    let location = location.ok_or_else(|| {
        AppError::Network(format!(
            "Redirect from '{current}' omitted the Location header"
        ))
    })?;
    let location = location.to_str().map_err(|error| {
        AppError::Network(format!(
            "Redirect from '{current}' contained an invalid Location header: {error}"
        ))
    })?;
    next_redirect_url(current, location)
}

async fn validate_remote_url(dns: &PinnedDnsResolver, url: &Url) -> Result<(), AppError> {
    if url.scheme() != "https" {
        return Err(AppError::PortalSecurity(format!(
            "URL '{url}' must use HTTPS"
        )));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(AppError::PortalSecurity(format!(
            "URL '{url}' must not contain credentials"
        )));
    }
    let host = url
        .host_str()
        .ok_or_else(|| AppError::PortalSecurity(format!("URL '{url}' has no host")))?;
    let normalized_host = host.trim_end_matches('.').to_ascii_lowercase();
    if normalized_host == "localhost"
        || normalized_host.ends_with(".localhost")
        || normalized_host.ends_with(".local")
        || normalized_host.ends_with(".internal")
    {
        return Err(AppError::PortalSecurity(format!(
            "URL '{url}' targets a local host"
        )));
    }
    let port = url.port_or_known_default().ok_or_else(|| {
        AppError::PortalSecurity(format!("URL '{url}' does not resolve to a known port"))
    })?;
    let addresses = tokio::time::timeout(DNS_LOOKUP_TIMEOUT, tokio::net::lookup_host((host, port)))
        .await
        .map_err(|_| AppError::PortalDiscovery(format!("DNS lookup for '{host}' timed out")))?
        .map_err(|error| {
            AppError::PortalDiscovery(format!("DNS lookup for '{host}' failed: {error}"))
        })?;
    let resolved = addresses
        .map(|address| address.ip())
        .collect::<BTreeSet<_>>();
    if resolved.is_empty() {
        return Err(AppError::PortalDiscovery(format!(
            "DNS lookup for '{host}' returned no addresses"
        )));
    }
    if let Some(address) = resolved.iter().find(|address| !is_public_ip(address)) {
        return Err(AppError::PortalSecurity(format!(
            "URL '{url}' resolved to prohibited address {address}"
        )));
    }
    // Hand the approved answer to the connector. Validating here and letting reqwest resolve
    // again at connect time is what makes DNS rebinding work; see `PinnedDnsResolver`.
    dns.pin(host, &normalized_host, resolved.into_iter().collect());
    Ok(())
}

/// The DNS answers `validate_remote_url` approved, keyed by hostname.
///
/// `validate_remote_url` resolves a hostname and rejects any answer containing a non-public
/// address, but reqwest resolves that hostname a second time when it opens the socket. A
/// resolver that answers with a public address the first time and `127.0.0.1` the second
/// redirects an already-approved URL onto the loopback or LAN interface, and nothing in the
/// validation path ever sees it. Installing this as the client's resolver makes the address
/// reqwest connects to the address that was validated. Only the address is fixed: the hostname
/// still drives TLS SNI and the `Host` header, so virtual hosting keeps working.
///
/// A name with no pin is refused rather than resolved. Every request this client issues runs
/// through `validate_remote_url` first, so an unpinned name means some path skipped validation.
#[derive(Default)]
struct PinnedDnsResolver {
    entries: Mutex<HashMap<String, PinnedAddresses>>,
}

struct PinnedAddresses {
    addresses: Vec<IpAddr>,
    pinned_at: Instant,
}

impl PinnedDnsResolver {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, PinnedAddresses>> {
        // A panic while holding this lock would otherwise fail every later request closed.
        self.entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Pins under both the URL's host and its normalized form, because a trailing-dot host
    /// reaches the connector spelled the way the URL spelled it.
    fn pin(&self, host: &str, normalized_host: &str, addresses: Vec<IpAddr>) {
        let now = Instant::now();
        let mut entries = self.lock();
        entries.retain(|_, entry| now.duration_since(entry.pinned_at) < DNS_PIN_TTL);
        while entries.len() >= MAX_DNS_PINS {
            let Some(oldest) = entries
                .iter()
                .min_by_key(|(_, entry)| entry.pinned_at)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            entries.remove(&oldest);
        }
        for key in BTreeSet::from([
            host.to_ascii_lowercase(),
            normalized_host.to_ascii_lowercase(),
        ]) {
            entries.insert(
                key,
                PinnedAddresses {
                    addresses: addresses.clone(),
                    pinned_at: now,
                },
            );
        }
    }

    fn pinned(&self, host: &str) -> Option<Vec<IpAddr>> {
        let key = host.to_ascii_lowercase();
        let mut entries = self.lock();
        let entry = entries.get(&key)?;
        if Instant::now().duration_since(entry.pinned_at) >= DNS_PIN_TTL {
            entries.remove(&key);
            return None;
        }
        entries.get(&key).map(|entry| entry.addresses.clone())
    }
}

impl reqwest::dns::Resolve for PinnedDnsResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_string();
        let pinned = self.pinned(&host);
        Box::pin(async move {
            match pinned {
                // Port 0 tells reqwest to use the port from the URL.
                Some(addresses) => Ok(Box::new(
                    addresses
                        .into_iter()
                        .map(|address| SocketAddr::new(address, 0)),
                ) as reqwest::dns::Addrs),
                None => Err(format!(
                    "host '{host}' was not validated before connecting, so it will not be resolved"
                )
                .into()),
            }
        })
    }
}

fn is_public_ip(address: &IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => is_public_ipv4(*address),
        IpAddr::V6(address) => is_public_ipv6(*address),
    }
}

fn is_public_ipv4(address: Ipv4Addr) -> bool {
    let [a, b, _, _] = address.octets();
    !(address.is_private()
        || address.is_loopback()
        || address.is_link_local()
        || address.is_broadcast()
        || address.is_unspecified()
        || address.is_multicast()
        || a == 0
        || (a == 100 && (64..=127).contains(&b))
        || (a == 192 && b == 0 && address.octets()[2] == 0)
        || (a == 192 && b == 0 && address.octets()[2] == 2)
        || (a == 198 && (b == 18 || b == 19))
        || (a == 198 && b == 51 && address.octets()[2] == 100)
        || (a == 203 && b == 0 && address.octets()[2] == 113)
        || a >= 240)
}

fn is_public_ipv6(address: Ipv6Addr) -> bool {
    if let Some(mapped) = address.to_ipv4_mapped() {
        return is_public_ipv4(mapped);
    }
    let segments = address.segments();
    let embedded_v4 = || {
        Ipv4Addr::new(
            (segments[6] >> 8) as u8,
            segments[6] as u8,
            (segments[7] >> 8) as u8,
            segments[7] as u8,
        )
    };
    // RFC 8215 reserves 64:ff9b:1::/48 for local use and explicitly forbids assuming an embedded
    // IPv4 layout. The IANA registry marks it not globally reachable, so a public-only client
    // rejects the entire prefix rather than guessing which bytes a local translator interprets.
    if segments[0] == 0x0064 && segments[1] == 0xff9b && segments[2] == 0x0001 {
        return false;
    }
    // RFC 6052's distinct well-known 64:ff9b::/96 format carries IPv4 in the last 32 bits.
    if segments[0] == 0x0064
        && segments[1] == 0xff9b
        && segments[2..6].iter().all(|segment| *segment == 0)
    {
        return is_public_ipv4(embedded_v4());
    }
    // Teredo's 2001::/32 transition addresses route through relays and encode obfuscated endpoint
    // state. They are not accepted as ordinary public destinations by this direct-fetch client.
    if segments[0] == 0x2001 && segments[1] == 0 {
        return false;
    }
    // 6to4: the IPv4 address sits in the second and third groups.
    if segments[0] == 0x2002 {
        return is_public_ipv4(Ipv4Addr::new(
            (segments[1] >> 8) as u8,
            segments[1] as u8,
            (segments[2] >> 8) as u8,
            segments[2] as u8,
        ));
    }
    // Deprecated IPv4-compatible form `::a.b.c.d`, which `to_ipv4_mapped` does not cover.
    if segments[..6].iter().all(|segment| *segment == 0) && !address.is_unspecified() {
        return is_public_ipv4(embedded_v4());
    }
    !(address.is_loopback()
        || address.is_unspecified()
        || address.is_multicast()
        || (segments[0] & 0xfe00) == 0xfc00
        || (segments[0] & 0xffc0) == 0xfe80
        || (segments[0] == 0x2001 && segments[1] == 0x0db8))
}

fn initial_api_candidates(submitted: &Url, origin: &Url) -> Result<VecDeque<Url>, AppError> {
    let mut candidates = VecDeque::new();
    candidates.push_back(submitted.clone());
    for path in [
        // NextRequest, which is the platform behind most United States municipal portals, serves
        // its request index from `/client/requests` and answers `/api/requests` with a 404. It was
        // absent from this list, so every `*.nextrequest.com` portal failed discovery outright.
        // Probed against https://sandiego.nextrequest.com on 2026-08-25: `/client/requests` returns
        // `{"total_count":43517,"requests":[...]}` while `/requests.json` returns an empty body.
        "/client/requests",
        "/requests.json",
        "/requests",
        "/api/requests",
        "/api/v1/requests",
        "/api/v2/requests",
        "/public/requests.json",
    ] {
        candidates.push_back(origin.join(path).map_err(|error| {
            AppError::PortalDiscovery(format!("Failed to construct API candidate: {error}"))
        })?);
    }
    let base_path = submitted.path().trim_end_matches('/');
    if !base_path.is_empty() {
        for suffix in [".json", "/requests.json", "/api/requests"] {
            let path = format!("{base_path}{suffix}");
            candidates.push_back(origin.join(&path).map_err(|error| {
                AppError::PortalDiscovery(format!("Failed to construct API candidate: {error}"))
            })?);
        }
    }
    Ok(candidates)
}

fn extract_api_links(base: &Url, body: &[u8]) -> Result<Vec<Url>, AppError> {
    let text = String::from_utf8_lossy(body);
    let document = Html::parse_document(&text);
    let selector = Selector::parse("a[href], link[href], script[src]").map_err(|error| {
        AppError::Internal(format!("Failed to initialize HTML link selector: {error}"))
    })?;
    let mut links = BTreeSet::new();
    for element in document.select(&selector) {
        let Some(value) = element
            .value()
            .attr("href")
            .or_else(|| element.value().attr("src"))
        else {
            continue;
        };
        let signal = value.to_ascii_lowercase();
        if !signal.contains("api") && !signal.contains("request") {
            continue;
        }
        if let Ok(mut url) = base.join(value) {
            url.set_fragment(None);
            if url.scheme() == "https" {
                links.insert(url.to_string());
            }
        }
    }
    links
        .into_iter()
        .take(8)
        .map(|value| {
            Url::parse(&value).map_err(|error| {
                AppError::PortalDiscovery(format!("Discovered API URL is invalid: {error}"))
            })
        })
        .collect()
}

fn detect_json_layout(value: &Value) -> Option<DiscoveredLayout> {
    if value.is_array() {
        return Some(DiscoveredLayout {
            requests_pointer: String::new(),
            metadata_pointer: None,
        });
    }
    let candidates = [
        ("/requests", Some("/meta")),
        ("/requests", Some("/pagination")),
        ("/data/requests", Some("/data/meta")),
        ("/data", Some("/meta")),
        ("/results", Some("/pagination")),
        ("/records", Some("/meta")),
        ("/public_requests", Some("/meta")),
    ];
    candidates.into_iter().find_map(|(requests, metadata)| {
        value
            .pointer(requests)
            .filter(|candidate| candidate.is_array())
            .map(|_| DiscoveredLayout {
                requests_pointer: requests.to_string(),
                metadata_pointer: metadata
                    .filter(|pointer| value.pointer(pointer).is_some())
                    .map(str::to_string),
            })
    })
}

fn validate_response_records(
    value: &Value,
    pointer: &str,
    metadata_pointer: Option<&str>,
) -> Result<(), AppError> {
    let records = response_records(value, pointer).ok_or_else(|| {
        AppError::PortalDiscovery(format!(
            "Discovered JSON pointer '{}' is missing or is not an array",
            if pointer.is_empty() { "/" } else { pointer }
        ))
    })?;
    if let Some(first) = records.first() {
        let object = first.as_object().ok_or_else(|| {
            AppError::PortalDiscovery("Request collection contains non-object entries".to_string())
        })?;
        if !object.contains_key("id")
            || !(object.contains_key("created_at") || object.contains_key("createdAt"))
        {
            return Err(AppError::PortalDiscovery(
                "Request collection does not expose stable identifiers and creation timestamps"
                    .to_string(),
            ));
        }
    } else {
        let metadata = metadata_pointer
            .and_then(|metadata_pointer| value.pointer(metadata_pointer))
            .and_then(parse_pagination_meta)
            .filter(|metadata| metadata.total_count == 0)
            .ok_or_else(|| {
                AppError::PortalDiscovery(
                    "An empty JSON array could not be verified as a public-request collection"
                        .to_string(),
                )
            })?;
        validate_pagination_meta(&metadata, 0)?;
    }
    Ok(())
}

fn response_records<'a>(value: &'a Value, pointer: &str) -> Option<&'a [Value]> {
    let records = if pointer.is_empty() {
        value
    } else {
        value.pointer(pointer)?
    };
    records.as_array().map(Vec::as_slice)
}

fn has_discovery_evidence(value: &Value, layout: &DiscoveredLayout) -> bool {
    response_records(value, &layout.requests_pointer).is_some_and(|records| {
        !records.is_empty()
            || layout
                .requests_pointer
                .to_ascii_lowercase()
                .contains("request")
            || layout
                .metadata_pointer
                .as_deref()
                .and_then(|pointer| value.pointer(pointer))
                .and_then(parse_pagination_meta)
                .is_some()
    })
}

fn pagination_metadata_reports_more(value: &Value, metadata_pointer: Option<&str>) -> bool {
    metadata_pointer
        .and_then(|pointer| value.pointer(pointer))
        .and_then(parse_pagination_meta)
        .is_some_and(|meta| {
            meta.next_page.is_some() || meta.total_pages > meta.current_page || meta.total_count > 1
        })
}

fn decode_nextrequest_response(
    value: &Value,
    requests_pointer: &str,
    metadata_pointer: Option<&str>,
    requested_page: usize,
    per_page: usize,
) -> Result<NextRequestResponse, AppError> {
    validate_response_records(value, requests_pointer, metadata_pointer)?;
    let records = if requests_pointer.is_empty() {
        value.clone()
    } else {
        value.pointer(requests_pointer).cloned().ok_or_else(|| {
            AppError::DataParsing("Stored requests pointer is missing".to_string())
        })?
    };
    let requests: Vec<PublicRequest> = serde_json::from_value(records).map_err(|error| {
        AppError::DataParsing(format!("Failed to decode public request records: {error}"))
    })?;
    validate_javascript_safe_request_identifiers(&requests)?;
    let metadata = metadata_pointer.and_then(|pointer| value.pointer(pointer));
    let meta = if let Some(metadata) = metadata {
        let meta = parse_pagination_meta(metadata).ok_or_else(|| {
            AppError::DataParsing("Portal returned malformed pagination metadata".to_string())
        })?;
        validate_pagination_meta(&meta, requests.len())?;
        Some(meta)
    } else {
        let has_possible_next_page = requests.len() == per_page;
        let estimated_count = requested_page
            .saturating_sub(1)
            .saturating_mul(per_page)
            .saturating_add(requests.len());
        Some(PaginationMeta {
            current_page: requested_page,
            next_page: has_possible_next_page.then_some(requested_page.saturating_add(1)),
            prev_page: (requested_page > 1).then_some(requested_page - 1),
            total_pages: if has_possible_next_page {
                requested_page.saturating_add(1)
            } else {
                requested_page
            },
            total_count: estimated_count,
        })
    };
    Ok(NextRequestResponse { requests, meta })
}

fn validate_javascript_safe_request_identifiers(
    requests: &[PublicRequest],
) -> Result<(), AppError> {
    for request in requests {
        validate_javascript_safe_identifier("request", request.id)?;
        if let Some(status_id) = request.current_status_id {
            validate_javascript_safe_identifier("request status", status_id)?;
        }
        if let Some(agency) = request.agency.as_ref() {
            validate_javascript_safe_identifier("agency", agency.id)?;
        }
        if let Some(category) = request.category.as_ref() {
            validate_javascript_safe_identifier("category", category.id)?;
        }
        for document in &request.documents {
            validate_javascript_safe_identifier("document", document.id)?;
        }
    }
    Ok(())
}

fn validate_javascript_safe_identifier(kind: &str, identifier: u64) -> Result<(), AppError> {
    if identifier > JAVASCRIPT_MAX_SAFE_INTEGER {
        return Err(AppError::DataParsing(format!(
            "Portal returned {kind} identifier {identifier}, which cannot cross the JavaScript bridge without precision loss"
        )));
    }
    Ok(())
}

fn validate_pagination_meta(meta: &PaginationMeta, records_on_page: usize) -> Result<(), AppError> {
    let valid_next = meta
        .next_page
        .is_none_or(|next| next > meta.current_page && next <= meta.total_pages);
    let valid_previous = meta
        .prev_page
        .is_none_or(|previous| previous > 0 && previous < meta.current_page);
    if meta.current_page == 0
        || meta.total_pages < meta.current_page
        || !valid_next
        || !valid_previous
        || meta.total_count < records_on_page
    {
        return Err(AppError::DataParsing(format!(
            "Portal returned inconsistent pagination metadata for page {}",
            meta.current_page
        )));
    }
    Ok(())
}

fn parse_pagination_meta(value: &Value) -> Option<PaginationMeta> {
    if let Ok(meta) = serde_json::from_value::<PaginationMeta>(value.clone()) {
        return Some(meta);
    }
    let current_page = integer_field(value, &["current_page", "currentPage", "page"])?;
    let next_page = integer_field(value, &["next_page", "nextPage"]);
    let prev_page = integer_field(value, &["prev_page", "previous_page", "prevPage"]);
    let total_pages =
        integer_field(value, &["total_pages", "totalPages", "pages"]).unwrap_or(current_page);
    let total_count = integer_field(value, &["total_count", "totalCount", "count"]).unwrap_or(0);
    Some(PaginationMeta {
        current_page,
        next_page,
        prev_page,
        total_pages,
        total_count,
    })
}

fn integer_field(value: &Value, keys: &[&str]) -> Option<usize> {
    keys.iter().find_map(|key| {
        let field = value.get(*key)?;
        field
            .as_u64()
            .and_then(|number| usize::try_from(number).ok())
            .or_else(|| field.as_str()?.parse::<usize>().ok())
    })
}

fn discover_display_name(value: &Value, submitted: &Url) -> String {
    ["/portal/name", "/agency/name", "/meta/portal_name", "/name"]
        .into_iter()
        .find_map(|pointer| value.pointer(pointer).and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| submitted.host_str().map(str::to_string))
        .unwrap_or_else(|| "Verified NextRequest Portal".to_string())
}

fn origin_url(url: &Url) -> Result<Url, AppError> {
    let mut origin = url.clone();
    origin.set_path("/");
    origin.set_query(None);
    origin.set_fragment(None);
    Ok(origin)
}

fn canonical_url(url: &Url) -> String {
    let mut canonical = url.clone();
    canonical.set_fragment(None);
    canonical.to_string()
}

fn portal_key(url: &Url, endpoint: &str) -> String {
    let host = url.host_str().unwrap_or("portal").to_ascii_lowercase();
    let label = host
        .strip_suffix(".nextrequest.com")
        .unwrap_or(&host)
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .chars()
        .take(48)
        .collect::<String>();
    let identity = format!(
        "{}://{}:{}|{endpoint}",
        url.scheme(),
        host,
        url.port_or_known_default().unwrap_or(443)
    );
    let digest = sha256_hex(identity.as_bytes());
    format!(
        "{}-{}",
        if label.is_empty() { "portal" } else { &label },
        &digest[..12]
    )
}

fn parse_retry_after(headers: &HeaderMap) -> Option<Duration> {
    let value = headers.get(RETRY_AFTER)?.to_str().ok()?.trim();
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    let deadline = chrono::DateTime::parse_from_rfc2822(value).ok()?;
    Some(
        (deadline.with_timezone(&Utc) - Utc::now())
            .to_std()
            .unwrap_or(Duration::ZERO),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recorded_source_shapes_replay_without_network_access() {
        let ckan: Value =
            serde_json::from_str(include_str!("../../fixtures/open-data/ckan-page.json"))
                .expect("CKAN replay fixture must parse");
        let ckan_page = decode_ckan_page(&ckan, None).expect("CKAN replay must decode");
        assert_eq!(ckan_page.downloads.len(), 1);
        assert_eq!(ckan_page.asset_count, Some(2));

        let socrata: Value =
            serde_json::from_str(include_str!("../../fixtures/open-data/socrata-page.json"))
                .expect("Socrata replay fixture must parse");
        let socrata_page =
            decode_socrata_page(&socrata, "data.example.gov").expect("Socrata replay must decode");
        assert_eq!(socrata_page.downloads.len(), 1);

        let nextrequest: Value = serde_json::from_str(include_str!(
            "../../fixtures/open-data/nextrequest-page.json"
        ))
        .expect("NextRequest replay fixture must parse");
        let response = decode_nextrequest_response(&nextrequest, "/requests", Some("/meta"), 1, 50)
            .expect("NextRequest replay must decode");
        assert_eq!(response.requests.len(), 1);

        let base = Url::parse("https://records.example.gov/reading-room").unwrap();
        let (_, links, _) = extract_public_resource_links(
            &base,
            include_bytes!("../../fixtures/open-data/reading-room.html"),
            None,
            None,
        )
        .expect("reading-room replay must decode");
        assert!(links.iter().any(|link| link.download_candidate));
        assert!(links
            .iter()
            .any(|link| link.url.contains("public.tableau.com")));
    }

    fn sample_request(id: u64) -> Value {
        serde_json::json!({
            "id": id,
            "rpid": null,
            "title": "Records request",
            "description": null,
            "status": "open",
            "current_status_id": null,
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T01:00:00Z",
            "closed_at": null,
            "due_date": null,
            "documents": [],
            "agency": null,
            "category": null
        })
    }

    #[test]
    fn blocks_non_public_network_ranges() {
        assert!(!is_public_ipv4(Ipv4Addr::new(127, 0, 0, 1)));
        assert!(!is_public_ipv4(Ipv4Addr::new(169, 254, 1, 1)));
        assert!(!is_public_ipv4(Ipv4Addr::new(100, 64, 0, 1)));
        assert!(is_public_ipv4(Ipv4Addr::new(8, 8, 8, 8)));
    }

    #[test]
    fn detects_nested_request_collections() {
        let value = serde_json::json!({"data": {"requests": []}, "meta": {}});
        let layout = detect_json_layout(&value).unwrap();
        assert_eq!(layout.requests_pointer, "/data/requests");
    }

    #[test]
    fn requires_semantic_evidence_for_empty_discovery_collections() {
        let ambiguous = serde_json::json!([]);
        let ambiguous_layout = detect_json_layout(&ambiguous).unwrap();
        assert!(!has_discovery_evidence(&ambiguous, &ambiguous_layout));

        let requests = serde_json::json!({"requests": []});
        let requests_layout = detect_json_layout(&requests).unwrap();
        assert!(has_discovery_evidence(&requests, &requests_layout));

        let paginated = serde_json::json!({
            "data": [],
            "meta": {
                "current_page": 1,
                "next_page": null,
                "prev_page": null,
                "total_pages": 1,
                "total_count": 0
            }
        });
        let paginated_layout = detect_json_layout(&paginated).unwrap();
        assert!(has_discovery_evidence(&paginated, &paginated_layout));
    }

    #[test]
    fn probes_page_advancement_only_when_metadata_reports_more_records() {
        let exhausted = serde_json::json!({
            "meta": {"page": 1, "pages": 1, "count": 1}
        });
        assert!(!pagination_metadata_reports_more(&exhausted, Some("/meta")));

        let additional = serde_json::json!({
            "meta": {"page": 1, "pages": 2, "count": 2}
        });
        assert!(pagination_metadata_reports_more(&additional, Some("/meta")));
    }

    #[test]
    fn replaces_probe_pagination_without_losing_endpoint_query_state() {
        let url = Url::parse(
            "https://records.example.test/api/requests?tenant=public&page=1&per_page=1&page=2",
        )
        .unwrap();
        let canonical = without_query_parameters(&url, &["page", "per_page"]);
        assert_eq!(
            canonical.as_str(),
            "https://records.example.test/api/requests?tenant=public"
        );

        let requested = with_query_parameters(
            &canonical,
            &[("page", "7".to_string()), ("per_page", "50".to_string())],
        );
        let pairs = requested.query_pairs().collect::<Vec<_>>();
        assert_eq!(pairs.len(), 3);
        assert_eq!(
            pairs,
            vec![
                ("tenant".into(), "public".into()),
                ("page".into(), "7".into()),
                ("per_page".into(), "50".into())
            ]
        );
    }

    #[test]
    fn rejects_ambiguous_or_unsafe_pagination_parameter_names() {
        assert!(validate_pagination_parameters("page", "page").is_err());
        assert!(validate_pagination_parameters("page[]", "limit").is_err());
        assert!(validate_pagination_parameters("offset", "page-size").is_ok());
    }

    #[test]
    fn estimates_total_count_across_fallback_pages() {
        let value = serde_json::json!([sample_request(1), sample_request(2)]);
        let decoded = decode_nextrequest_response(&value, "", None, 3, 10).unwrap();
        let meta = decoded.meta.unwrap();
        assert_eq!(meta.current_page, 3);
        assert_eq!(meta.total_pages, 3);
        assert_eq!(meta.total_count, 22);
        assert_eq!(meta.prev_page, Some(2));
        assert_eq!(meta.next_page, None);
    }

    #[test]
    fn rejects_inconsistent_pagination_metadata() {
        let value = serde_json::json!({
            "requests": [sample_request(1)],
            "meta": {
                "current_page": 2,
                "next_page": 2,
                "prev_page": 3,
                "total_pages": 2,
                "total_count": 0
            }
        });
        let error =
            decode_nextrequest_response(&value, "/requests", Some("/meta"), 2, 50).unwrap_err();
        assert!(error.to_string().contains("inconsistent pagination"));
    }

    #[test]
    fn rejects_identifiers_that_would_lose_precision_in_the_frontend_bridge() {
        let value = serde_json::json!([sample_request(JAVASCRIPT_MAX_SAFE_INTEGER + 1)]);
        let error = decode_nextrequest_response(&value, "", None, 1, 50).unwrap_err();
        assert!(error.to_string().contains("precision loss"));
    }

    #[test]
    fn validates_retry_configuration_bounds() {
        let excessive_retries = HttpClientConfig {
            max_retries: MAX_RETRIES + 1,
            ..HttpClientConfig::default()
        };
        assert!(ApiClient::new(excessive_retries).is_err());

        let inverted_delays = HttpClientConfig {
            retry_initial_delay_ms: 2_000,
            retry_max_delay_ms: 1_000,
            ..HttpClientConfig::default()
        };
        assert!(ApiClient::new(inverted_delays).is_err());
    }

    #[test]
    fn parses_both_retry_after_wire_formats() {
        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, HeaderValue::from_static("17"));
        assert_eq!(parse_retry_after(&headers), Some(Duration::from_secs(17)));

        headers.insert(
            RETRY_AFTER,
            HeaderValue::from_static("Wed, 21 Oct 2015 07:28:00 GMT"),
        );
        assert_eq!(parse_retry_after(&headers), Some(Duration::ZERO));
    }

    #[test]
    fn a_ckan_page_yields_resource_downloads_and_counts_packages_that_have_none() {
        // Shape taken from https://data.ca.gov/api/3/action/package_search fetched 2026-08-26,
        // which reported 4,570 packages. The whole catalog is four dozen of these pages; crawling
        // the HTML that renders it is what produced the faceted queue this replaces.
        let body = serde_json::json!({
            "success": true,
            "result": {
                "count": 4570,
                "results": [
                    {"name": "hazardous-waste", "resources": [
                        {"url": "https://data.ca.gov/dataset/bd2c/resource/3fb5/download/hazardous-waste-id-numbers.csv",
                         "name": "Hazardous Waste ID Numbers", "format": "CSV", "size": 136203124, "mimetype": null}
                    ]},
                    {"name": "documentation-only", "resources": [
                        {"url": "http://legacy.example.gov/insecure.csv", "format": "CSV"}
                    ]},
                    {"name": "empty-package", "resources": []}
                ]
            }
        });
        let page = decode_ckan_page(&body, Some("100")).unwrap();
        assert_eq!(page.asset_count, Some(4570));
        assert_eq!(page.downloads.len(), 1);
        assert_eq!(page.downloads[0].media_type.as_deref(), Some("text/csv"));
        assert_eq!(page.downloads[0].byte_count, Some(136_203_124));
        // A package the index could not resolve to a retrievable file is counted, not dropped:
        // the plain-HTTP resource and the resourceless package are both holes in coverage.
        assert_eq!(
            page.assets_without_downloads
                .get("ckan-package-without-retrievable-resource"),
            Some(&2)
        );
        assert_eq!(page.next_cursor.as_deref(), Some("103"));

        // The last page stops rather than paging past the reported count.
        let tail = serde_json::json!({
            "success": true,
            "result": {"count": 4570, "results": [{"name": "x", "resources": []}]}
        });
        assert_eq!(
            decode_ckan_page(&tail, Some("4569")).unwrap().next_cursor,
            None
        );

        // A catalog that withholds its total has not said the corpus is exhausted: a full page
        // keeps paging, and only a short page ends it.
        let package = serde_json::json!({"name": "x", "resources": []});
        let full = serde_json::json!({
            "success": true,
            "result": {"results": vec![package.clone(); CATALOG_PAGE_SIZE]}
        });
        let page = decode_ckan_page(&full, Some("200")).unwrap();
        assert_eq!(page.asset_count, None);
        assert_eq!(page.next_cursor.as_deref(), Some("300"));
        let short = serde_json::json!({
            "success": true,
            "result": {"results": [package]}
        });
        assert_eq!(
            decode_ckan_page(&short, Some("300")).unwrap().next_cursor,
            None
        );
    }

    #[test]
    fn a_socrata_page_refuses_federated_assets_and_scrolls_by_identifier() {
        // `domains` and `search_context` are advisory: the live catalog at
        // data.cityofnewyork.us answered an unscoped query with dallasopendata.com rows on
        // 2026-08-26, which is why the domain is re-checked per asset here.
        let body = serde_json::json!({
            "resultSetSize": 3016,
            "results": [
                {"metadata": {"domain": "data.cityofnewyork.us"},
                 "resource": {"id": "8wbx-tsch", "type": "dataset", "name": "For Hire Vehicles (FHV) - Active"}},
                {"metadata": {"domain": "www.dallasopendata.com"},
                 "resource": {"id": "9fxf-t2tr", "type": "dataset", "name": "Dallas Police Active Calls"}},
                {"metadata": {"domain": "data.cityofnewyork.us"},
                 "resource": {"id": "vx8i-nprf", "type": "href", "name": "Linked elsewhere"}}
            ]
        });
        let page = decode_socrata_page(&body, "data.cityofnewyork.us").unwrap();
        assert_eq!(page.asset_count, Some(3016));
        assert_eq!(
            page.downloads
                .iter()
                .map(|download| download.url.as_str())
                .collect::<Vec<_>>(),
            vec!["https://data.cityofnewyork.us/api/views/8wbx-tsch/rows.csv?accessType=DOWNLOAD"]
        );
        assert_eq!(page.foreign_domain_assets, 1);
        assert_eq!(
            page.assets_without_downloads
                .get("socrata-href-has-no-tabular-export"),
            Some(&1)
        );
        // The cursor is the last identifier seen, including ones no download came from, so the
        // scroll advances past them instead of re-reading the same page forever.
        assert_eq!(page.next_cursor.as_deref(), Some("vx8i-nprf"));

        let exhausted = serde_json::json!({"resultSetSize": 3016, "results": []});
        assert_eq!(
            decode_socrata_page(&exhausted, "data.cityofnewyork.us")
                .unwrap()
                .next_cursor,
            None
        );

        // An entry without an identifier is counted, not dropped; a page made only of them
        // cannot be scrolled past and is an error rather than a silent end of the catalog.
        let mixed = serde_json::json!({
            "results": [
                {"metadata": {"domain": "data.cityofnewyork.us"}, "resource": {"type": "dataset"}},
                {"metadata": {"domain": "data.cityofnewyork.us"},
                 "resource": {"id": "8wbx-tsch", "type": "dataset", "name": "Named"}}
            ]
        });
        let page = decode_socrata_page(&mixed, "data.cityofnewyork.us").unwrap();
        assert_eq!(page.asset_count, None);
        assert_eq!(
            page.assets_without_downloads
                .get("socrata-entry-without-identifier"),
            Some(&1)
        );
        assert_eq!(page.next_cursor.as_deref(), Some("8wbx-tsch"));
        let unidentified = serde_json::json!({"results": [{"resource": {"type": "dataset"}}]});
        assert!(decode_socrata_page(&unidentified, "data.cityofnewyork.us").is_err());
    }

    #[test]
    fn san_diego_policy_pagination_is_followed_without_admitting_site_menus() {
        let url = |value: &str| Url::parse(value).unwrap();
        let base =
            url("https://www.sandiego.gov/police/data-transparency/policies-procedures/procedures");
        let body = br#"<main>
          <nav><a href="/police">Police home</a></nav>
          <a href="/sites/default/files/104.pdf">Use of Force</a>
          <nav class="pager" role="navigation" aria-label="Pagination">
            <a href="?page=1" rel="next">Next page</a>
            <a href="https://example.test/?page=2">External</a>
            <a href="/other?page=3">Other section</a>
          </nav>
        </main>"#;
        let (_, links, _) = extract_public_resource_links(&base, body, None, None).unwrap();
        let next = links
            .iter()
            .find(|link| link.url.ends_with("?page=1"))
            .unwrap();
        assert!(next.follow_candidate && !next.navigation_chrome);
        assert!(
            links
                .iter()
                .find(|link| link.url.ends_with("/police"))
                .unwrap()
                .navigation_chrome
        );
        assert!(
            links
                .iter()
                .find(|link| link.url.ends_with("/other?page=3"))
                .unwrap()
                .navigation_chrome
        );
        assert!(
            links
                .iter()
                .find(|link| link.url.ends_with("104.pdf"))
                .unwrap()
                .download_candidate
        );
        assert!(!is_san_diego_document_source(&url(
            "https://www.sandiego.gov.example.test/cpp"
        )));
        assert!(!is_san_diego_document_source(&url(
            "https://www.sandiego.gov/cpp-unrelated"
        )));
        assert!(!is_san_diego_document_source(&url(
            "https://sandiego.nextrequest.com/"
        )));
    }

    #[test]
    fn navigation_chrome_is_not_followed_but_a_download_in_it_is_kept() {
        // Structure taken from https://data.ca.gov/dataset fetched 2026-08-25: the account bar
        // links `/contact?ref=<referrer>`, which is a distinct URL on every page of the site, and
        // the facet sidebar links ten `?tags=` variants of the listing the crawler is already on.
        let body = br#"<html><body>
            <div class="account-masthead"><nav class="account"><ul>
              <li><a href="/contact?ref=https%3A%2F%2Fdata.ca.gov%2Fdataset">Contact</a></li>
            </ul></nav></div>
            <aside class="secondary"><div class="filters"><nav><ul>
              <li><a href="/dataset/?tags=species">species</a></li>
              <li><a href="/dataset/?tags=wildlife">wildlife</a></li>
            </ul></nav></aside>
            <section id="content">
              <a href="/dataset/fish-hatchery-annual-report">Fish Hatchery Annual Report</a>
            </section>
            <footer class="site-footer">
              <a href="/reports/2026-annual.pdf">2026 Annual Report</a>
            </footer>
          </body></html>"#;
        let base = Url::parse("https://data.ca.gov/dataset").unwrap();
        let (_, links, _) = extract_public_resource_links(&base, body, None, None).unwrap();
        let followable = links
            .iter()
            .filter(|link| link.follow_candidate && !link.navigation_chrome)
            .map(|link| link.url.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            followable,
            vec!["https://data.ca.gov/dataset/fish-hatchery-annual-report"]
        );
        // Every chrome link is still reported, so the frontier can count what it refused and
        // guided repair can still show the operator a link this rule would have eaten.
        for suppressed in [
            "https://data.ca.gov/contact?ref=https%3A%2F%2Fdata.ca.gov%2Fdataset",
            "https://data.ca.gov/dataset/?tags=species",
            "https://data.ca.gov/dataset/?tags=wildlife",
        ] {
            let link = links
                .iter()
                .find(|link| link.url == suppressed)
                .unwrap_or_else(|| panic!("{suppressed} was dropped instead of flagged"));
            assert!(
                link.navigation_chrome,
                "{suppressed} was not flagged as chrome"
            );
        }
        // A direct download inside the footer is still a document; chrome suppresses following,
        // not the link itself.
        let footer_download = links
            .iter()
            .find(|link| link.url.ends_with("2026-annual.pdf"))
            .expect("the footer download survived");
        assert!(footer_download.download_candidate);
        assert!(footer_download.navigation_chrome);
    }

    #[test]
    fn discovers_repair_selectors_and_embedded_google_drive_connectors() {
        let connector_json = serde_json::json!({
            "id": 4,
            "nonce": "abc123_nonce",
            "account": {
                "id": "account_42",
                "specific_folders": [{
                    "id": "folder_99",
                    "accountId": "account_42",
                    "name": "Public evidence"
                }]
            }
        });
        let encoded = base64::engine::general_purpose::STANDARD
            .encode(serde_json::to_vec(&connector_json).unwrap());
        let html = format!(
            r#"<html><head><title>Evidence</title></head><body>
                <a class="api-export" href="/api/export?id=7">Records API</a>
                <div data-shortcode-data="{encoded}"></div>
            </body></html>"#
        );
        let base = Url::parse("https://records.example.gov/documents").unwrap();
        let (title, links, connectors) = extract_public_resource_links(
            &base,
            html.as_bytes(),
            Some("a.api-export"),
            Some("a.api-export"),
        )
        .unwrap();

        assert_eq!(title, "Evidence");
        assert_eq!(links.len(), 1);
        assert!(links[0].download_candidate);
        let PublicEmbeddedConnector::WordPressGoogleDrive(connector) = &connectors[0];
        assert_eq!(connector.folder_id, "folder_99");
        assert_eq!(connector.account_id, "account_42");
        assert_eq!(
            connector.endpoint,
            "https://records.example.gov/wp-admin/admin-ajax.php"
        );
    }

    #[test]
    fn validates_google_drive_listing_shape_and_limits() {
        let response = serde_json::json!({
            "success": true,
            "data": {"files": [{
                "id": "file_1",
                "accountId": "account_1",
                "name": "Annual Report.pdf",
                "type": "application/pdf",
                "isFolder": false,
                "webViewLink": "https://drive.google.com/file/d/file_1/view",
                "webContentLink": "https://drive.usercontent.google.com/download?id=file_1"
            }]}
        });
        let items = decode_wordpress_google_drive_listing(
            &serde_json::to_vec(&response).expect("fixture should encode"),
        )
        .expect("listing should decode");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "Annual Report.pdf");
        assert!(!items[0].is_folder);
    }
    #[test]
    fn ipv6_wrappers_do_not_launder_a_private_destination() {
        // Each of these reaches an IPv4 address that `is_public_ipv4` rejects, spelled so that
        // `to_ipv4_mapped` returns None and the old prefix checks all pass.
        for candidate in [
            "64:ff9b::7f00:1",                      // NAT64 well-known prefix, 127.0.0.1
            "64:ff9b::a00:1",                       // NAT64 well-known prefix, 10.0.0.1
            "64:ff9b:1::808:808", // RFC 8215 local-use prefix, regardless of tail bytes
            "2001:0:4136:e378:8000:63bf:3fff:fdd2", // Teredo transition address
            "2002:c0a8:1::",      // 6to4, 192.168.0.1
            "::7f00:1",           // deprecated IPv4-compatible, 127.0.0.1
            "::ffff:169.254.169.254", // link-local metadata service, for contrast
        ] {
            let address: Ipv6Addr = candidate.parse().expect("fixture address should parse");
            assert!(
                !is_public_ipv6(address),
                "{candidate} must not be treated as public"
            );
        }
        assert!(is_public_ipv6(
            "64:ff9b::5db8:d822".parse().expect("fixture should parse")
        ));
        assert!(is_public_ipv6(
            "2606:4700:4700::1111"
                .parse()
                .expect("fixture should parse")
        ));
    }

    #[tokio::test]
    async fn validation_pins_the_addresses_it_approved() {
        let dns = PinnedDnsResolver::default();
        // An address literal keeps this offline: `lookup_host` parses it without a DNS query.
        let url = Url::parse("https://93.184.216.34/records").expect("fixture URL should parse");
        validate_remote_url(&dns, &url)
            .await
            .expect("a public address must validate");
        assert_eq!(
            dns.pinned("93.184.216.34"),
            Some(vec![IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34))])
        );
        assert_eq!(dns.pinned("records.example.gov"), None);
    }

    #[tokio::test]
    async fn a_rejected_url_leaves_no_pin_behind() {
        let dns = PinnedDnsResolver::default();
        let url = Url::parse("https://127.0.0.1/records").expect("fixture URL should parse");
        validate_remote_url(&dns, &url)
            .await
            .expect_err("a loopback address must be rejected");
        assert_eq!(dns.pinned("127.0.0.1"), None);
    }

    #[tokio::test]
    async fn the_client_refuses_to_resolve_a_host_that_skipped_validation() {
        // This is the property that defeats DNS rebinding: the connector cannot consult the
        // system resolver at all, so it cannot receive a second, different answer. The request
        // fails before any socket is opened, which is also why this test needs no network.
        let client = ApiClient::new(HttpClientConfig::default()).expect("client should build");
        let error = client
            .client
            .get("https://records.example.gov/")
            .send()
            .await
            .expect_err("an unvalidated host must not connect");
        let rendered = format!("{error:?}");
        assert!(
            rendered.contains("was not validated before connecting"),
            "expected the pinned resolver to refuse the name, got: {rendered}"
        );
    }
}

/// Security regression tests for the URL/redirect boundary.
///
/// Every case uses an address literal or a name that is never resolved, so the whole module runs
/// offline. `lookup_host` parses a literal without issuing a DNS query, which is what lets
/// `validate_remote_url` be exercised directly.
#[cfg(test)]
mod url_security_tests {
    use super::*;

    fn url(value: &str) -> Url {
        Url::parse(value).expect("fixture URL should parse")
    }

    async fn rejects(value: &str) -> AppError {
        let dns = PinnedDnsResolver::default();
        match validate_remote_url(&dns, &url(value)).await {
            Ok(()) => panic!("'{value}' must be rejected"),
            Err(error) => error,
        }
    }

    // SECURITY(security): https only. Every other scheme is a different trust model, and `file:`
    // in particular turns a portal-supplied URL into a local file read.
    #[tokio::test]
    async fn only_https_is_accepted() {
        for value in [
            "http://93.184.216.34/records",
            "file:///etc/passwd",
            "ftp://93.184.216.34/records",
            "gopher://93.184.216.34/",
        ] {
            let error = rejects(value).await;
            assert!(
                matches!(error, AppError::PortalSecurity(_)),
                "'{value}' rejected with the wrong variant: {error:?}",
            );
        }

        let dns = PinnedDnsResolver::default();
        validate_remote_url(&dns, &url("https://93.184.216.34/records"))
            .await
            .expect("https to a public address must be accepted");
    }

    // SECURITY(security): credentials in a URL would be sent to whatever host the URL names, and
    // a redirect chain could carry them somewhere else entirely.
    #[tokio::test]
    async fn credentials_embedded_in_a_url_are_refused() {
        for value in [
            "https://user:password@93.184.216.34/records",
            "https://user@93.184.216.34/records",
            "https://:password@93.184.216.34/records",
        ] {
            let error = rejects(value).await;
            assert!(
                matches!(error, AppError::PortalSecurity(_)),
                "'{value}' rejected with the wrong variant: {error:?}",
            );
        }
    }

    // SECURITY(security): loopback, link-local, and RFC 1918 addresses are the SSRF targets that
    // matter. These must be refused whichever spelling reaches them.
    #[tokio::test]
    async fn private_and_loopback_destinations_are_refused() {
        for value in [
            "https://127.0.0.1/",
            "https://127.0.0.1:8080/",
            "https://10.0.0.1/",
            "https://192.168.1.1/",
            "https://172.16.0.1/",
            "https://169.254.169.254/latest/meta-data/",
            "https://100.64.0.1/",
            "https://0.0.0.0/",
        ] {
            let error = rejects(value).await;
            assert!(
                matches!(error, AppError::PortalSecurity(_)),
                "'{value}' rejected with the wrong variant: {error:?}",
            );
        }
    }

    // LIMIT(security): a bracketed IPv6 literal is refused, but by DNS failure rather than by the
    // address check: `Url::host_str` keeps the brackets, and `lookup_host("[::1]", _)` cannot
    // parse that. The outcome is fail-closed, and it applies to PUBLIC IPv6 literals too, so this
    // client cannot address any IPv6 literal URL at all. `is_public_ipv6` still guards names that
    // resolve to AAAA records, which is where it actually does its work.
    #[tokio::test]
    async fn ipv6_literal_urls_are_refused_before_the_address_check() {
        for value in [
            "https://[::1]/",
            "https://[fc00::1]/",
            "https://[fe80::1]/",
            "https://[2606:4700:4700::1111]/",
        ] {
            let error = rejects(value).await;
            assert!(
                matches!(
                    error,
                    AppError::PortalDiscovery(_) | AppError::PortalSecurity(_)
                ),
                "'{value}' rejected with an unexpected variant: {error:?}",
            );
        }
    }

    // The name-suffix check is defense in depth. `is_public_ip` on the resolved address is the
    // actual boundary, which is why adding more suffixes here would not make the app safer.
    #[tokio::test]
    async fn internal_name_suffixes_are_refused_before_resolution() {
        for value in [
            "https://localhost/",
            "https://LocalHost./",
            "https://api.localhost/",
            "https://printer.local/",
            "https://vault.internal/",
        ] {
            let error = rejects(value).await;
            assert!(
                matches!(error, AppError::PortalSecurity(_)),
                "'{value}' rejected with the wrong variant: {error:?}",
            );
        }
    }

    // LIMIT(security): there is no port allowlist. Pinned as current behavior so that adding one
    // is a deliberate decision rather than an accident, and so that removing one is noticed.
    #[tokio::test]
    async fn any_port_on_a_public_address_is_currently_permitted() {
        let dns = PinnedDnsResolver::default();
        for value in [
            "https://93.184.216.34:9200/",
            "https://93.184.216.34:22/",
            "https://93.184.216.34:6379/",
        ] {
            validate_remote_url(&dns, &url(value))
                .await
                .unwrap_or_else(|error| panic!("'{value}' unexpectedly rejected: {error:?}"));
        }
    }

    // SECURITY(security): rejection must leave no pin, or a refused host would become reachable
    // by a later unvalidated call inside the pin TTL.
    #[tokio::test]
    async fn every_rejected_url_leaves_the_resolver_untouched() {
        for (value, host) in [
            ("http://93.184.216.34/", "93.184.216.34"),
            ("https://user@93.184.216.34/", "93.184.216.34"),
            ("https://127.0.0.1/", "127.0.0.1"),
            ("https://localhost/", "localhost"),
        ] {
            let dns = PinnedDnsResolver::default();
            let _ = validate_remote_url(&dns, &url(value)).await;
            assert_eq!(dns.pinned(host), None, "'{value}' left a pin behind");
        }
    }

    // SECURITY(security): IPv4 ranges that are not globally routable must all be refused. The
    // documentation and benchmark ranges are included because they are reachable inside some
    // corporate networks.
    #[test]
    fn non_routable_ipv4_ranges_are_not_public() {
        for octets in [
            [0, 0, 0, 0],
            [10, 0, 0, 1],
            [100, 64, 0, 1],
            [127, 0, 0, 1],
            [169, 254, 169, 254],
            [172, 16, 0, 1],
            [172, 31, 255, 254],
            [192, 0, 0, 1],
            [192, 0, 2, 1],
            [192, 168, 1, 1],
            [198, 18, 0, 1],
            [198, 51, 100, 1],
            [203, 0, 113, 1],
            [224, 0, 0, 1],
            [240, 0, 0, 1],
            [255, 255, 255, 255],
        ] {
            let address = Ipv4Addr::from(octets);
            assert!(!is_public_ipv4(address), "{address} must not be public");
        }

        for octets in [
            [8, 8, 8, 8],
            [93, 184, 216, 34],
            [1, 1, 1, 1],
            [172, 32, 0, 1],
        ] {
            let address = Ipv4Addr::from(octets);
            assert!(is_public_ipv4(address), "{address} should be public");
        }
    }

    // SECURITY(security): only 192.0.0.0/24 (IETF Protocol Assignments) and 192.0.2.0/24
    // (TEST-NET-1) are reserved inside 192.0.0.0/16; every other address in that block is
    // ordinary allocated unicast space. Over-blocking the whole /16 would refuse a legitimate
    // portal hosted there, which is exactly as much a correctness bug as under-blocking is a
    // security one.
    #[test]
    fn only_the_reserved_192_0_slash24s_are_non_public() {
        assert!(!is_public_ipv4(Ipv4Addr::new(192, 0, 0, 1)));
        assert!(!is_public_ipv4(Ipv4Addr::new(192, 0, 2, 1)));
        assert!(is_public_ipv4(Ipv4Addr::new(192, 0, 1, 1)));
        assert!(is_public_ipv4(Ipv4Addr::new(192, 0, 3, 1)));
        assert!(is_public_ipv4(Ipv4Addr::new(192, 0, 255, 1)));
    }

    // SECURITY(security): an IPv6 wrapper must not launder a private IPv4 destination. Each of
    // these spellings reaches the same place as the bare IPv4 address inside it.
    #[test]
    fn ipv6_wrappers_cannot_launder_a_private_destination() {
        let private: &[&str] = &[
            "::1",
            "::",
            "fc00::1",
            "fd00::1",
            "fe80::1",
            "ff02::1",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "::127.0.0.1",
            "64:ff9b::127.0.0.1",
            "64:ff9b::169.254.169.254",
            "2002:7f00:0001::",
        ];
        for value in private {
            let address: Ipv6Addr = value.parse().expect("fixture address should parse");
            assert!(!is_public_ipv6(address), "{value} must not be public");
        }

        for value in ["2606:4700:4700::1111", "2001:4860:4860::8888"] {
            let address: Ipv6Addr = value.parse().expect("fixture address should parse");
            assert!(is_public_ipv6(address), "{value} should be public");
        }
    }

    // INVARIANT(security): the loop runs `0..=MAX_REDIRECTS` and errors when `redirect_count`
    // reaches `MAX_REDIRECTS`, so at most MAX_REDIRECTS redirects are followed across at most
    // MAX_REDIRECTS + 1 requests. This pins the arithmetic; the loop itself needs a live socket.
    #[test]
    fn the_redirect_budget_is_bounded_and_small() {
        assert_eq!(MAX_REDIRECTS, 5);
        assert_eq!((0..=MAX_REDIRECTS).count(), MAX_REDIRECTS + 1);
    }

    #[test]
    fn redirect_targets_resolve_relative_and_protocol_relative_locations() {
        let current = url("https://93.184.216.34/records/pages/2?old=yes");
        assert_eq!(
            next_redirect_url(&current, "../3?new=yes#not-sent")
                .expect("relative redirect")
                .as_str(),
            "https://93.184.216.34/records/3?new=yes"
        );
        assert_eq!(
            next_redirect_url(&current, "//1.1.1.1/landing")
                .expect("protocol-relative redirect")
                .as_str(),
            "https://1.1.1.1/landing"
        );
        assert_eq!(
            next_redirect_url(&current, "/landing")
                .expect("path redirect")
                .as_str(),
            "https://93.184.216.34/landing",
            "a redirect Location owns the next query rather than retaining caller pagination"
        );
    }

    #[test]
    fn redirect_header_errors_are_explicit_and_offline_testable() {
        let current = url("https://93.184.216.34/records");
        let missing = redirect_url_from_header(&current, None).expect_err("header is required");
        assert!(missing.to_string().contains("omitted the Location header"));

        let non_utf8 = HeaderValue::from_bytes(b"/records/\xff").expect("opaque header bytes");
        let invalid = redirect_url_from_header(&current, Some(&non_utf8))
            .expect_err("Location must be visible text");
        assert!(invalid.to_string().contains("invalid Location header"));

        let malformed = HeaderValue::from_static("https://[");
        assert!(redirect_url_from_header(&current, Some(&malformed)).is_err());
    }

    #[test]
    fn empty_request_parameters_preserve_stable_file_identity() {
        let file = url("https://seshat.datasd.org/operating_budget/budget_operating_datasd.csv");
        assert_eq!(materialize_request_url(&file, Some(&[])), file);
        assert!(materialize_request_url(&file, Some(&[])).query().is_none());
        let params = [("page", "2".to_string())];
        let paginated = materialize_request_url(
            &url("https://example.test/records?kind=budget#section"),
            Some(&params),
        );
        assert_eq!(
            paginated.as_str(),
            "https://example.test/records?kind=budget&page=2"
        );
        let landing = next_redirect_url(&paginated, "/download.csv").unwrap();
        assert!(materialize_request_url(&landing, None).query().is_none());
    }

    #[test]
    fn redirect_request_identity_detects_cycles_without_conflating_query_removal() {
        let mut visited = BTreeSet::new();
        let paginated = url("https://93.184.216.34/records?page=2");
        let landing = next_redirect_url(&paginated, "/records").expect("landing redirect");
        let alternate = next_redirect_url(&landing, "/alternate").expect("alternate redirect");
        let cycle =
            next_redirect_url(&alternate, "/records#different-fragment").expect("cycle redirect");

        assert!(visited.insert(paginated.as_str().to_string()));
        assert!(visited.insert(landing.as_str().to_string()));
        assert!(visited.insert(alternate.as_str().to_string()));
        assert!(!visited.insert(cycle.as_str().to_string()));
    }
}
#[test]
fn regional_budget_hub_and_meeting_buttons_expose_their_public_resources() {
    let budget = Url::parse("https://sandiegocounty.budget.socrata.com").unwrap();
    let html = br#"<script>var settings={"operating_budget_dataset_id":"u4wm-r3vc","revenue_budget_dataset_id":"sucv-y5vk"};</script>"#;
    let (_, links, _) = extract_public_resource_links(&budget, html, None, None).unwrap();
    assert_eq!(
        links.iter().filter(|l| l.url.contains("rows.csv")).count(),
        2
    );
    assert!(links.iter().all(|l| l
        .url
        .starts_with("https://data.sandiegocounty.gov/api/views/")));
    let hub = Url::parse("https://sdgis-sandag.opendata.arcgis.com").unwrap();
    let (_, links, _) = extract_public_resource_links(&hub, br#"<meta content='%7B%22contentGroupId%22%3A%22e1a39cc8334f4aa0b7970f7dd268a8ab%22%2C%22ownerId%22%3A%226e08ea455575ef20877c0ef11251959f%22%7D'>"#, None, None).unwrap();
    assert_eq!(
        links.len(),
        1,
        "owner hashes must not be treated as content items"
    );
    assert!(links[0]
        .url
        .contains("group%3Ae1a39cc8334f4aa0b7970f7dd268a8ab"));
    let granicus = Url::parse("https://sandiego.granicus.com/ViewPublisher.php?view_id=3").unwrap();
    let (_, links, _) = extract_public_resource_links(&granicus, br#"<a href="javascript:void(0)" onclick="window.open('//sandiego.granicus.com/MediaPlayer.php?view_id=3&amp;clip_id=9484','player')">Video</a><a onclick="window.open('https://other.test/MediaPlayer.php?clip_id=1')">External</a>"#, None, None).unwrap();
    assert_eq!(links.len(), 1);
    assert!(links[0].url.contains("clip_id=9484"));
    assert!(links[0].follow_candidate);
    assert!(!is_supported_platform_host("arcgis.com.evil.test"));
    let (_, links, _) = extract_public_resource_links(&budget, br#"<a href="https://view.officeapps.live.com/op/view.aspx?src=https%3A%2F%2Fwww.sandiego.gov%2Fcomments.xlsx">Comments</a>"#, None, None).unwrap();
    assert_eq!(links[0].url, "https://www.sandiego.gov/comments.xlsx");
    assert!(links[0].download_candidate);
}

#[test]
fn sdhc_board_extensionless_reports_are_downloads_with_usable_context() {
    let base = Url::parse(
        "https://sdhc.org/governance-legislative-affairs/sdhc-board-of-commissioners/meetings/",
    )
    .unwrap();
    let body = br#"<nav><a href='/housing-opportunities/'>Housing opportunities</a></nav>
      <table><tr><td>April 16, 2026</td><td><a href='/4-16-2026-agenda/'>Agenda</a></td></tr>
      <tr><td></td><td>Item 103 <a href='/103_hcr26-014-imperial-shelter_final/'>HCR26-014</a>
      Approval of the Operating Agreement with Alpha Project for the Imperial Shelter
      <a href='/103_imperial-shelter_final/'>Presentation</a></td><td>Approved 6-0</td></tr>
      <tr><td><a href='/contact/'>Contact us</a><a href='https://unrelated.test/report/'>HCR26-900</a>
      <a href='/account/?download=private'>Account</a></td></tr></table>"#;
    let (_, links, _) = extract_public_resource_links(&base, body, None, None).unwrap();
    let report = &links[0];
    assert!(report
        .url
        .ends_with("/103_hcr26-014-imperial-shelter_final/"));
    assert!(report.download_candidate && !report.navigation_chrome);
    assert!(report.label.contains("Alpha Project") && report.label.contains("Imperial Shelter"));
    assert!(links
        .iter()
        .any(|l| l.label.starts_with("Presentation") && l.download_candidate));
    assert!(
        !links
            .iter()
            .find(|l| l.url.ends_with("/contact/"))
            .unwrap()
            .download_candidate
    );
    let elsewhere = Url::parse("https://sdhc.org/news-release/example/").unwrap();
    let (_, generic, _) = extract_public_resource_links(&elsewhere, body, None, None).unwrap();
    assert!(
        !generic
            .iter()
            .find(|l| l.url == report.url)
            .unwrap()
            .download_candidate
    );
    let listing = publication_listing_links(base.as_str(), body).unwrap();
    assert!(listing.contains(&report.url));
}

#[test]
fn council_agendas_expose_item_indexes_without_spending_frontier_on_bootstrap_navigation() {
    let agenda = Url::parse("https://sandiego.hylandcloud.com/211agendaonlinecouncil/Documents/ViewAgenda?meetingId=7192&type=agenda&doctype=1").unwrap();
    let (_, links, _) = extract_public_resource_links(&agenda, br#"
        <div class="navbar-collapse"><a href="/211agendaonlinecouncil/Meetings">Past Meeting Search</a></div>
        <a href="javascript:loadAgendaItem(265583,false);">Riverbed encampment funding</a>
        <a href="javascript:loadAgendaItem(265584,false);">Loan documents</a>
    "#, None, None).unwrap();
    assert_eq!(links.len(), 3);
    assert!(
        links
            .iter()
            .find(|link| link.label == "Past Meeting Search")
            .unwrap()
            .navigation_chrome
    );
    let items = links
        .iter()
        .filter(|link| link.url.contains("ViewMeetingAgendaItem"))
        .collect::<Vec<_>>();
    assert_eq!(items.len(), 2);
    assert!(items
        .iter()
        .all(|link| link.follow_candidate && !link.download_candidate && !link.navigation_chrome));
}

#[cfg(test)]
fn unused_acquisition() -> AcquisitionRecord {
    AcquisitionRecord {
        requested_url: String::new(),
        resolved_url: String::new(),
        redirect_chain: Vec::new(),
        http_status: 200,
        content_type: None,
        etag: None,
        last_modified: None,
        response_date: None,
        retrieved_at: String::new(),
    }
}

#[test]
fn council_optional_item_failure_preserves_other_attachments_and_retry_links() {
    let agenda = Url::parse("https://sandiego.hylandcloud.com/211agendaonlinecouncil/Documents/ViewAgenda?meetingId=7192&type=agenda&doctype=1").unwrap();
    let (_, mut links, _) = extract_public_resource_links(
        &agenda,
        br#"
        <a href="javascript:loadAgendaItem(265583,false);">Riverbed encampment funding</a>
        <a href="javascript:loadAgendaItem(265584,false);">Loan documents</a>
    "#,
        None,
        None,
    )
    .unwrap();
    let original_links = links.clone();
    let mut seen = links
        .iter()
        .map(|link| link.url.clone())
        .collect::<BTreeSet<_>>();
    let item = Url::parse(&links[0].url).unwrap();
    let unavailable = Url::parse(&links[1].url).unwrap();
    let successful = ResponsePayload { final_url: item.clone(), content_type: Some("text/html".into()), bytes: br#"<a href="/211agendaonlinecouncil/Documents/DownloadFile/grant.pdf?meetingId=7192&amp;itemId=265583&amp;publishId=1">Grant agreement</a>"#.to_vec(), acquisition: unused_acquisition() };
    merge_hyland_item_attachments(&item, "Riverbed", Ok(successful), &mut links, &mut seen);
    merge_hyland_item_attachments(
        &unavailable,
        "Loan documents",
        Err(AppError::DataParsing("temporarily unavailable".into())),
        &mut links,
        &mut seen,
    );
    let redirected = ResponsePayload {
        final_url: agenda.clone(),
        content_type: Some("text/html".into()),
        bytes: Vec::new(),
        acquisition: unused_acquisition(),
    };
    merge_hyland_item_attachments(
        &unavailable,
        "Loan documents",
        Ok(redirected),
        &mut links,
        &mut seen,
    );
    assert_eq!(links.len(), 3);
    assert_eq!(&links[..2], original_links.as_slice());
    assert!(
        links[1].follow_candidate,
        "A failed item must remain eligible for ordinary frontier retry"
    );
    assert!(links[2].download_candidate && links[2].label.contains("Grant agreement"));
}

#[cfg(test)]
mod hosted_nextrequest_live {
    use super::*;

    /// Network-dependent, so ignored by default. Run with
    /// `cargo test --lib hosted_nextrequest_live -- --ignored` to replay discovery and one
    /// subject-searched page against the live San Diego portal.
    #[tokio::test]
    #[ignore = "contacts sandiego.nextrequest.com"]
    async fn discovers_and_reads_a_searched_page_of_the_live_portal() {
        let client = ApiClient::new(HttpClientConfig::default()).expect("client should build");
        let portal = client
            .discover_portal("https://sandiego.nextrequest.com/")
            .await
            .expect("the hosted portal must be discovered as NextRequest");
        assert_eq!(portal.source_kind, SourceKind::NextRequest);
        assert_eq!(
            portal.api_layout.page_parameter,
            hosted_nextrequest::PAGE_PARAMETER
        );
        let listed = client
            .list_hosted_nextrequest_page(&portal, 2, Some("encampment"))
            .await
            .expect("a searched page must decode");
        assert!(
            listed.meta.total_count < 10_000,
            "the search must narrow the portal"
        );
        assert!(!listed.requests.is_empty());
        let mut documents = 0;
        let mut read = 0;
        for listed_request in listed.requests.into_iter().take(15) {
            let (request, _, _) = client
                .fetch_hosted_request(&portal, listed_request)
                .await
                .expect("a listed request must be read");
            assert!(chrono::DateTime::parse_from_rfc3339(&request.created_at).is_ok());
            if request.status == "closed" {
                assert!(
                    request.closed_at.is_some(),
                    "{:?} closed without a date",
                    request.rpid
                );
            }
            documents += request.documents.len();
            read += 1;
        }
        eprintln!("{read} requests read from page 2, {documents} public documents");
    }
}
