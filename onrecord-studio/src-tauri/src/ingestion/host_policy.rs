//! Per-host crawl governance: robots.txt, pacing, concurrency, request budgets, and the
//! run-scoped host stop that a refusal status has to produce.
//!
//! The defect this module exists to close: a crawl that reads HTTP 403 as "this one URL failed"
//! keeps issuing requests to a host that has already refused it, which is how an operator gets
//! their address blocked by a portal. A refusal is a statement about the client, not about the
//! URL, so it stops the host for the remainder of the run.
//!
//! Everything here is run-scoped and in memory. Nothing persists across a restart, because a
//! block recorded now is evidence about this run's traffic and says nothing about the next one.

use crate::error::AppError;
use reqwest::Url;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{OnceCell, OwnedSemaphorePermit, Semaphore};
use tokio::time::Instant;

/// Two in-flight requests per host is the ceiling a public agency portal should ever see from a
/// single desktop client. It is not tuned for throughput; throughput comes from platform APIs.
pub const DEFAULT_HOST_CONCURRENCY: usize = 2;
/// A per-run ceiling on requests to one host. This is a governor, not a coverage policy: reaching
/// it stops the host and is reported as a stop, never rendered as though the host were exhausted.
pub const DEFAULT_HOST_REQUEST_BUDGET: usize = 2_000;
/// Applied when robots.txt declares no `Crawl-delay`. Deliberately small, since it is a floor for
/// well-behaved hosts rather than a substitute for a declared delay.
pub const DEFAULT_HOST_INTERVAL: Duration = Duration::from_millis(400);
/// A hostile or misconfigured `Crawl-delay: 86400` must not park a worker for a day.
pub const MAXIMUM_HOST_INTERVAL: Duration = Duration::from_secs(30);
/// Bounds a `Retry-After` a host can name; longer than this stops the host instead of sleeping.
pub const MAXIMUM_RETRY_AFTER: Duration = Duration::from_secs(120);
/// How many consecutive HTTP 429 responses a host may issue before the run stops talking to it.
/// Below this the client backs off and retries, which is what `Retry-After` asks for. A success
/// resets the count: a long collection against a rate-limited portal meets 429 many times, and
/// counting them over the whole run stopped every such collection partway.
pub const THROTTLE_STRIKES_BEFORE_STOP: u32 = 5;
/// The first back-off after a 429 that names no `Retry-After`, doubling per consecutive strike.
/// Measured against sandiego.nextrequest.com on 2026-10-02: 20 requests in 12 s drew a 429 with no
/// `Retry-After`, and the host answered again 5 s later.
pub const THROTTLE_BACKOFF_FLOOR: Duration = Duration::from_secs(5);
/// The widest pacing 429s alone can push a host to. The same measurement found roughly one
/// request per 1.5 s sustainable; unbounded doubling reached 30 s per request and turned a
/// 400-request collection into hours.
pub const MAXIMUM_THROTTLED_INTERVAL: Duration = Duration::from_secs(5);
/// robots.txt is a text file; anything larger is not one.
pub const MAXIMUM_ROBOTS_BYTES: usize = 512 * 1024;

/// The product token this client matches against robots.txt user-agent groups.
pub const ROBOTS_PRODUCT_TOKEN: &str = "onrecordstudio";

/// Why a host will not be contacted again.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum HostStopReason {
    /// The host answered a request with 403. Continuing is what gets an operator blocked.
    Refused { status: u16 },
    /// The host answered 429 more times than the strike budget allows.
    Throttled { strikes: u32 },
    /// The host named a `Retry-After` longer than a single run should wait out.
    RetryAfterTooLong { seconds: u64 },
    /// robots.txt could not be retrieved, so no allowance can be demonstrated.
    RobotsUnavailable { detail: String },
    /// The per-run request budget for this host was spent.
    BudgetSpent { budget: usize },
}

impl HostStopReason {
    pub fn describe(&self) -> String {
        match self {
            Self::Refused { status } => {
                format!("the host answered HTTP {status}, so this run stopped contacting it")
            }
            Self::Throttled { strikes } => {
                format!("the host answered HTTP 429 {strikes} times, so this run stopped contacting it")
            }
            Self::RetryAfterTooLong { seconds } => format!(
                "the host asked for a {seconds}-second wait, longer than a single run will hold"
            ),
            Self::RobotsUnavailable { detail } => {
                format!("robots.txt could not be read ({detail}), so no crawl allowance is proven")
            }
            Self::BudgetSpent { budget } => format!(
                "this run spent its {budget}-request budget for the host; the host was not exhausted"
            ),
        }
    }
}

/// Why one URL will not be requested. Distinct from `HostStopReason`: a robots exclusion is a
/// statement about a path, and the host stays available for paths it does allow.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RequestRefusal {
    HostStopped {
        host: String,
        reason: HostStopReason,
    },
    RobotsExcluded {
        host: String,
        rule: String,
    },
}

impl RequestRefusal {
    pub fn describe(&self) -> String {
        match self {
            Self::HostStopped { host, reason } => format!("'{host}': {}", reason.describe()),
            Self::RobotsExcluded { host, rule } => {
                format!("'{host}' excludes this path through robots.txt rule '{rule}'")
            }
        }
    }
}

impl From<RequestRefusal> for AppError {
    fn from(value: RequestRefusal) -> Self {
        AppError::PortalSecurity(value.describe())
    }
}

/// A single robots.txt rule. `allow` distinguishes `Allow:` from `Disallow:`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RobotsRule {
    allow: bool,
    pattern: String,
}

/// The rules that apply to this client, already reduced to the single matching user-agent group.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RobotsRules {
    rules: Vec<RobotsRule>,
    crawl_delay: Option<Duration>,
    /// Set when robots.txt could not be read at all. Per RFC 9309 an unreachable robots.txt
    /// withdraws the crawl allowance rather than granting it by default.
    unavailable: Option<String>,
}

impl RobotsRules {
    /// robots.txt answered 404 or another 4xx: the host publishes no restrictions.
    pub fn unrestricted() -> Self {
        Self::default()
    }

    /// robots.txt could not be retrieved. The host is stopped rather than crawled on a guess.
    pub fn unavailable(detail: impl Into<String>) -> Self {
        Self {
            rules: Vec::new(),
            crawl_delay: None,
            unavailable: Some(detail.into()),
        }
    }

    pub fn unavailable_detail(&self) -> Option<&str> {
        self.unavailable.as_deref()
    }

    pub fn crawl_delay(&self) -> Option<Duration> {
        self.crawl_delay
    }

    /// The longest matching pattern decides, and an `Allow` wins a tie. This is the RFC 9309
    /// precedence rule; shortest-first or first-match orderings give the opposite answer on the
    /// `Disallow: /dataset/` plus `Allow: /dataset/foo.csv` shape that real catalogs publish.
    pub fn evaluate(&self, path_and_query: &str) -> Option<&str> {
        let mut verdict: Option<(&RobotsRule, usize)> = None;
        for rule in &self.rules {
            let Some(length) = pattern_match_length(&rule.pattern, path_and_query) else {
                continue;
            };
            let replaces = match verdict {
                None => true,
                Some((current, current_length)) => {
                    length > current_length
                        || (length == current_length && rule.allow && !current.allow)
                }
            };
            if replaces {
                verdict = Some((rule, length));
            }
        }
        match verdict {
            Some((rule, _)) if !rule.allow => Some(rule.pattern.as_str()),
            _ => None,
        }
    }

    /// Parses a robots.txt body, keeping only the group that applies to this client.
    ///
    /// Group selection follows RFC 9309: the most specific matching user-agent token wins, and
    /// `*` is the fallback. Records are collected per group first because a file may repeat
    /// `User-agent: *` in several places, and every one of those records applies.
    pub fn parse(body: &str) -> Self {
        let mut groups: HashMap<String, Vec<RobotsRule>> = HashMap::new();
        let mut delays: HashMap<String, Duration> = HashMap::new();
        let mut active: Vec<String> = Vec::new();
        // A user-agent line following a rule line opens a new group rather than extending the
        // current one; without this a file's later groups silently inherit earlier rules.
        let mut accepting_agents = false;
        for line in body.lines() {
            let line = line.split('#').next().unwrap_or_default().trim();
            if line.is_empty() {
                continue;
            }
            let Some((field, value)) = line.split_once(':') else {
                continue;
            };
            let field = field.trim().to_ascii_lowercase();
            let value = value.trim();
            match field.as_str() {
                "user-agent" => {
                    if !accepting_agents {
                        active.clear();
                        accepting_agents = true;
                    }
                    active.push(value.to_ascii_lowercase());
                }
                "allow" | "disallow" => {
                    accepting_agents = false;
                    if active.is_empty() {
                        continue;
                    }
                    // An empty `Disallow:` is the documented way to permit everything, and it
                    // carries no pattern, so it contributes no rule.
                    if value.is_empty() {
                        continue;
                    }
                    let rule = RobotsRule {
                        allow: field == "allow",
                        pattern: value.to_string(),
                    };
                    for agent in &active {
                        groups.entry(agent.clone()).or_default().push(rule.clone());
                    }
                }
                "crawl-delay" => {
                    accepting_agents = false;
                    let Ok(seconds) = value.parse::<f64>() else {
                        continue;
                    };
                    if !seconds.is_finite() || seconds <= 0.0 {
                        continue;
                    }
                    let delay = Duration::from_secs_f64(seconds.min(86_400.0));
                    for agent in &active {
                        delays.insert(agent.clone(), delay);
                    }
                }
                _ => {}
            }
        }
        let selected = select_agent_group(&groups, &delays);
        Self {
            rules: selected
                .as_deref()
                .and_then(|agent| groups.get(agent).cloned())
                .unwrap_or_default(),
            crawl_delay: selected
                .as_deref()
                .and_then(|agent| delays.get(agent).copied()),
            unavailable: None,
        }
    }
}

/// Picks the user-agent group that governs this client: an exact or prefix match on the product
/// token first, then `*`. A group named only in `Crawl-delay` still counts, because a host may
/// pace a client without restricting its paths.
fn select_agent_group(
    groups: &HashMap<String, Vec<RobotsRule>>,
    delays: &HashMap<String, Duration>,
) -> Option<String> {
    let named = groups
        .keys()
        .chain(delays.keys())
        .filter(|agent| *agent != "*")
        .filter(|agent| {
            ROBOTS_PRODUCT_TOKEN.starts_with(agent.as_str()) || agent.contains(ROBOTS_PRODUCT_TOKEN)
        })
        .max_by_key(|agent| agent.len())
        .cloned();
    named
        .or_else(|| (groups.contains_key("*") || delays.contains_key("*")).then(|| "*".to_string()))
}

/// Returns the number of pattern characters consumed on a match, which is what RFC 9309's
/// longest-match precedence compares. `*` matches any run and `$` anchors the end.
fn pattern_match_length(pattern: &str, target: &str) -> Option<usize> {
    if !pattern.starts_with('/') {
        return None;
    }
    let anchored = pattern.ends_with('$');
    let body = if anchored {
        &pattern[..pattern.len() - 1]
    } else {
        pattern
    };
    let segments = body.split('*').collect::<Vec<_>>();
    let mut cursor = 0_usize;
    let last = segments.len() - 1;
    for (index, segment) in segments.iter().enumerate() {
        if segment.is_empty() {
            continue;
        }
        if index == 0 {
            if !target[cursor..].starts_with(segment) {
                return None;
            }
            cursor += segment.len();
            continue;
        }
        if anchored && index == last {
            // The leftmost occurrence may sit mid-target while a later one ends it, as in
            // `/*.php$` against `/a.php/b.php`; the final literal has to be tested as a suffix.
            if target.len() < cursor + segment.len() || !target.ends_with(segment) {
                return None;
            }
            cursor = target.len();
            continue;
        }
        let found = target[cursor..].find(segment)?;
        cursor += found + segment.len();
    }
    if anchored && cursor != target.len() {
        // A trailing `*` before `$` may still consume the remainder.
        if !segments.last().is_some_and(|segment| segment.is_empty()) {
            return None;
        }
    }
    Some(body.len())
}

struct HostState {
    permits: Arc<Semaphore>,
    robots: Arc<OnceCell<Arc<RobotsRules>>>,
    next_allowed: Option<Instant>,
    requests_spent: usize,
    throttle_strikes: u32,
    /// Pacing learned from this host's 429s, which every later request to it keeps.
    throttled_interval: Option<Duration>,
    /// The end of the current 429 back-off. Requests already in flight when a host throttles
    /// answer 429 too; those belong to the same episode and are not further strikes.
    backoff_until: Option<Instant>,
    stop: Option<HostStopReason>,
    /// URLs the operator pointed the application at, or endpoints a platform adapter identified
    /// as the publisher's own documented API. Robots path exclusions govern link-following
    /// discovery; they do not govern retrieval of the resource the operator named, and reading a
    /// catalog's API costs the host far less than the crawl it replaces. Pacing, concurrency,
    /// budgets, and host stops still apply to every entry here.
    user_directed: HashSet<String>,
}

impl HostState {
    fn new(concurrency: usize) -> Self {
        Self {
            permits: Arc::new(Semaphore::new(concurrency.max(1))),
            robots: Arc::new(OnceCell::new()),
            next_allowed: None,
            requests_spent: 0,
            throttle_strikes: 0,
            throttled_interval: None,
            backoff_until: None,
            stop: None,
            user_directed: HashSet::new(),
        }
    }
}

/// A held slot against one host's concurrency limit. Dropping it releases the slot.
#[derive(Debug)]
pub struct HostPermit {
    _permit: OwnedSemaphorePermit,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostSnapshot {
    pub host: String,
    pub requests_spent: usize,
    pub throttle_strikes: u32,
    pub stopped: Option<HostStopReason>,
}

pub struct HostGovernor {
    hosts: Mutex<HashMap<String, HostState>>,
    concurrency: usize,
    request_budget: usize,
    minimum_interval: Duration,
}

impl Default for HostGovernor {
    fn default() -> Self {
        Self::new(
            DEFAULT_HOST_CONCURRENCY,
            DEFAULT_HOST_REQUEST_BUDGET,
            DEFAULT_HOST_INTERVAL,
        )
    }
}

impl HostGovernor {
    pub fn new(concurrency: usize, request_budget: usize, minimum_interval: Duration) -> Self {
        Self {
            hosts: Mutex::new(HashMap::new()),
            concurrency: concurrency.max(1),
            request_budget: request_budget.max(1),
            minimum_interval: minimum_interval.min(MAXIMUM_HOST_INTERVAL),
        }
    }

    /// `records.gov.` and `records.gov` are one host, so they share one stop state, budget, and
    /// pacing history; a redirect to the trailing-dot spelling must not start fresh.
    pub fn host_key(url: &Url) -> String {
        url.host_str()
            .unwrap_or_default()
            .trim_end_matches('.')
            .to_ascii_lowercase()
    }

    fn with_host<T>(&self, host: &str, action: impl FnOnce(&mut HostState) -> T) -> T {
        let mut hosts = self.hosts.lock().unwrap_or_else(|error| error.into_inner());
        let state = hosts
            .entry(host.to_string())
            .or_insert_with(|| HostState::new(self.concurrency));
        action(state)
    }

    /// Records that the operator directed the application at this URL, exempting it from robots
    /// path exclusions. Every other governor limit continues to apply.
    pub fn mark_user_directed(&self, url: &Url) {
        let host = Self::host_key(url);
        let canonical = url.as_str().to_string();
        self.with_host(&host, |state| {
            state.user_directed.insert(canonical);
        });
    }

    pub fn is_user_directed(&self, url: &Url) -> bool {
        let host = Self::host_key(url);
        self.with_host(&host, |state| state.user_directed.contains(url.as_str()))
    }

    /// The cell holding this host's robots rules, so the caller can fetch them exactly once.
    pub fn robots_cell(&self, url: &Url) -> Arc<OnceCell<Arc<RobotsRules>>> {
        let host = Self::host_key(url);
        self.with_host(&host, |state| Arc::clone(&state.robots))
    }

    /// The stop recorded against this host, if any.
    pub fn stop_reason(&self, url: &Url) -> Option<HostStopReason> {
        let host = Self::host_key(url);
        self.with_host(&host, |state| state.stop.clone())
    }

    pub fn stop_host(&self, url: &Url, reason: HostStopReason) {
        let host = Self::host_key(url);
        self.with_host(&host, |state| {
            if state.stop.is_none() {
                state.stop = Some(reason);
            }
        });
    }

    /// Whether a URL may be scheduled at all. Called before a task is enqueued so a stopped host
    /// does not accumulate thousands of tasks that will each be refused at dispatch.
    pub fn admits(&self, url: &Url, robots: Option<&RobotsRules>) -> Result<(), RequestRefusal> {
        let host = Self::host_key(url);
        let stop = self.with_host(&host, |state| {
            if state.stop.is_none() && state.requests_spent >= self.request_budget {
                state.stop = Some(HostStopReason::BudgetSpent {
                    budget: self.request_budget,
                });
            }
            state.stop.clone()
        });
        if let Some(reason) = stop {
            return Err(RequestRefusal::HostStopped { host, reason });
        }
        if let Some(rules) = robots {
            if !self.is_user_directed(url) {
                if let Some(rule) = rules.evaluate(&path_and_query(url)) {
                    return Err(RequestRefusal::RobotsExcluded {
                        host,
                        rule: rule.to_string(),
                    });
                }
            }
        }
        Ok(())
    }

    /// Reserves a request slot: spends one unit of the host budget, waits out the host's pacing
    /// interval, and holds one of its concurrency permits until the returned permit is dropped.
    pub async fn reserve(
        &self,
        url: &Url,
        rules: Option<&RobotsRules>,
    ) -> Result<HostPermit, RequestRefusal> {
        let host = Self::host_key(url);
        let interval = rules
            .and_then(RobotsRules::crawl_delay)
            .unwrap_or(self.minimum_interval)
            .min(MAXIMUM_HOST_INTERVAL);
        let permits = self
            .with_host(&host, |state| {
                if state.stop.is_none() && state.requests_spent >= self.request_budget {
                    state.stop = Some(HostStopReason::BudgetSpent {
                        budget: self.request_budget,
                    });
                }
                if let Some(reason) = state.stop.clone() {
                    return Err(reason);
                }
                state.requests_spent = state.requests_spent.saturating_add(1);
                Ok(Arc::clone(&state.permits))
            })
            .map_err(|reason| RequestRefusal::HostStopped {
                host: host.clone(),
                reason,
            })?;
        let permit = permits
            .acquire_owned()
            .await
            .map_err(|_| RequestRefusal::HostStopped {
                host: host.clone(),
                reason: HostStopReason::RobotsUnavailable {
                    detail: "the host permit pool was closed".to_string(),
                },
            })?;
        // Pacing is assigned only once the permit is held: a slot claimed before a queued request
        // gets its permit lets several of them fire back to back when the permit frees. The stop is
        // re-read here and after the wait because a response recorded while this request was
        // queued or sleeping is the host refusing this client.
        let wait = self
            .with_host(&host, |state| {
                if let Some(reason) = state.stop.clone() {
                    return Err(reason);
                }
                let now = Instant::now();
                let ready = state.next_allowed.unwrap_or(now).max(now);
                let interval = state
                    .throttled_interval
                    .map_or(interval, |learned| learned.max(interval));
                state.next_allowed = Some(ready + interval);
                Ok(ready.saturating_duration_since(now))
            })
            .map_err(|reason| RequestRefusal::HostStopped {
                host: host.clone(),
                reason,
            })?;
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
            if let Some(reason) = self.with_host(&host, |state| state.stop.clone()) {
                return Err(RequestRefusal::HostStopped { host, reason });
            }
        }
        Ok(HostPermit { _permit: permit })
    }

    /// Records a response status against the host.
    ///
    /// 403 stops the host outright: a refusal is about the client, and the next URL will be
    /// refused the same way. 429 is a pacing signal, so it backs off first and stops the host
    /// only once the strike budget is spent. The returned duration, when present, is how long
    /// the caller should wait before retrying this request.
    pub fn record_status(
        &self,
        url: &Url,
        status: u16,
        retry_after: Option<Duration>,
    ) -> Option<Duration> {
        let host = Self::host_key(url);
        match status {
            403 | 401 | 451 => {
                self.with_host(&host, |state| {
                    if state.stop.is_none() {
                        state.stop = Some(HostStopReason::Refused { status });
                    }
                });
                None
            }
            429 => {
                if retry_after.is_some_and(|value| value > MAXIMUM_RETRY_AFTER) {
                    let seconds = retry_after.unwrap_or_default().as_secs();
                    self.with_host(&host, |state| {
                        if state.stop.is_none() {
                            state.stop = Some(HostStopReason::RetryAfterTooLong { seconds });
                        }
                    });
                    return None;
                }
                self.with_host(&host, |state| {
                    let now = Instant::now();
                    if let Some(until) = state.backoff_until.filter(|until| *until > now) {
                        return Some(until - now);
                    }
                    state.throttle_strikes = state.throttle_strikes.saturating_add(1);
                    if state.throttle_strikes >= THROTTLE_STRIKES_BEFORE_STOP
                        && state.stop.is_none()
                    {
                        state.stop = Some(HostStopReason::Throttled {
                            strikes: state.throttle_strikes,
                        });
                        return None;
                    }
                    // A host that throttles has told us our pacing is wrong, so widen the
                    // interval for every subsequent request rather than only this one.
                    state.throttled_interval = Some(
                        (state.throttled_interval.unwrap_or(self.minimum_interval) * 2)
                            .min(MAXIMUM_THROTTLED_INTERVAL),
                    );
                    let backoff = retry_after
                        .unwrap_or(
                            THROTTLE_BACKOFF_FLOOR * (1 << (state.throttle_strikes - 1).min(5)),
                        )
                        .min(MAXIMUM_RETRY_AFTER);
                    state.backoff_until = Some(now + backoff);
                    state.next_allowed = Some(state.next_allowed.unwrap_or(now).max(now) + backoff);
                    Some(backoff)
                })
            }
            // Each success eases learned pacing back toward the floor, so one burst of 429s
            // early in a run does not set the pace for the rest of it.
            200..=299 => {
                self.with_host(&host, |state| {
                    state.throttle_strikes = 0;
                    state.throttled_interval = state
                        .throttled_interval
                        .map(|learned| learned.mul_f32(0.99))
                        .filter(|learned| *learned > self.minimum_interval);
                });
                None
            }
            _ => None,
        }
    }

    pub fn snapshots(&self) -> Vec<HostSnapshot> {
        let hosts = self.hosts.lock().unwrap_or_else(|error| error.into_inner());
        let mut snapshots = hosts
            .iter()
            .map(|(host, state)| HostSnapshot {
                host: host.clone(),
                requests_spent: state.requests_spent,
                throttle_strikes: state.throttle_strikes,
                stopped: state.stop.clone(),
            })
            .collect::<Vec<_>>();
        snapshots.sort_by(|left, right| left.host.cmp(&right.host));
        snapshots
    }
}

/// robots.txt patterns are matched against the path plus query, not the path alone: a rule such
/// as `Disallow: /dataset/?tags=` is meaningless otherwise.
pub fn path_and_query(url: &Url) -> String {
    match url.query() {
        Some(query) => format!("{}?{}", url.path(), query),
        None => url.path().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(value: &str) -> Url {
        Url::parse(value).unwrap()
    }

    #[test]
    fn a_trailing_dot_names_the_same_governed_host() {
        assert_eq!(
            HostGovernor::host_key(&url("https://Records.gov./x")),
            HostGovernor::host_key(&url("https://records.gov/y"))
        );
    }

    #[test]
    fn honors_the_wildcard_group_and_its_crawl_delay() {
        // Abridged from the live https://data.ca.gov/robots.txt fetched 2026-08-25, which is the
        // host that produced the 403s this module exists to stop.
        let rules = RobotsRules::parse(
            "Sitemap: https://data.ca.gov/sitemap_index.xml\n\
             \n\
             User-agent: *\n\
             Disallow: /contact\n\
             Disallow: /dataset/activity/*\n\
             Disallow: /api/\n\
             \n\
             User-agent: *\n\
             Crawl-delay: 10\n",
        );
        assert_eq!(
            rules.evaluate("/contact?ref=https%3A%2F%2Fdata.ca.gov%2Fdataset"),
            Some("/contact")
        );
        assert_eq!(
            rules.evaluate("/dataset/activity/x"),
            Some("/dataset/activity/*")
        );
        assert_eq!(rules.evaluate("/dataset/?tags=species"), None);
        assert_eq!(rules.crawl_delay(), Some(Duration::from_secs(10)));
    }

    #[test]
    fn the_longest_matching_rule_decides_and_allow_wins_a_tie() {
        let rules =
            RobotsRules::parse("User-agent: *\nDisallow: /dataset/\nAllow: /dataset/open.csv\n");
        assert_eq!(rules.evaluate("/dataset/private"), Some("/dataset/"));
        assert_eq!(rules.evaluate("/dataset/open.csv"), None);

        let tied = RobotsRules::parse("User-agent: *\nDisallow: /a\nAllow: /a\n");
        assert_eq!(tied.evaluate("/a"), None);
    }

    #[test]
    fn a_group_for_another_agent_does_not_bind_this_client() {
        let rules = RobotsRules::parse(
            "User-agent: AhrefsBot\nDisallow: /\n\nUser-agent: *\nDisallow: /private\n",
        );
        assert_eq!(rules.evaluate("/dataset"), None);
        assert_eq!(rules.evaluate("/private"), Some("/private"));
    }

    #[test]
    fn an_empty_disallow_permits_everything() {
        let rules = RobotsRules::parse("User-agent: *\nDisallow:\n");
        assert_eq!(rules.evaluate("/anything"), None);
    }

    #[test]
    fn an_end_anchored_pattern_matches_only_the_full_path() {
        let rules = RobotsRules::parse("User-agent: *\nDisallow: /*.php$\n");
        assert_eq!(rules.evaluate("/index.php"), Some("/*.php$"));
        assert_eq!(rules.evaluate("/index.php?a=1"), None);
    }

    #[test]
    fn an_end_anchored_pattern_matches_a_later_occurrence_of_its_suffix() {
        let rules = RobotsRules::parse("User-agent: *\nDisallow: /*.php$\n");
        assert_eq!(rules.evaluate("/a.php/b.php"), Some("/*.php$"));
        assert_eq!(rules.evaluate("/x.php.php"), Some("/*.php$"));
        assert_eq!(rules.evaluate("/x.php/y"), None);
        let exact = RobotsRules::parse("User-agent: *\nDisallow: /a$\n");
        assert_eq!(exact.evaluate("/a"), Some("/a$"));
        assert_eq!(exact.evaluate("/ab"), None);
    }

    #[tokio::test]
    async fn a_queued_request_is_refused_when_its_host_is_stopped_while_it_waits() {
        let governor = Arc::new(HostGovernor::new(1, 100, Duration::ZERO));
        let target = url("https://example.gov/a");
        let held = governor.reserve(&target, None).await.unwrap();
        let queued = {
            let governor = Arc::clone(&governor);
            let target = target.clone();
            tokio::spawn(async move { governor.reserve(&target, None).await.map(|_| ()) })
        };
        tokio::time::sleep(Duration::from_millis(50)).await;
        governor.record_status(&target, 403, None);
        drop(held);
        assert!(matches!(
            queued.await.unwrap().unwrap_err(),
            RequestRefusal::HostStopped {
                reason: HostStopReason::Refused { status: 403 },
                ..
            }
        ));
    }

    #[test]
    fn a_forbidden_status_stops_the_host_rather_than_the_url() {
        let governor = HostGovernor::default();
        let first = url("https://data.ca.gov/dataset/?tags=species");
        let second = url("https://data.ca.gov/dataset/?tags=wildlife");
        assert!(governor.admits(&second, None).is_ok());
        assert_eq!(governor.record_status(&first, 403, None), None);
        let refusal = governor.admits(&second, None).unwrap_err();
        assert_eq!(
            refusal,
            RequestRefusal::HostStopped {
                host: "data.ca.gov".to_string(),
                reason: HostStopReason::Refused { status: 403 },
            }
        );
        // A different host is unaffected: the refusal was this host's statement.
        assert!(governor
            .admits(&url("https://data.cityofnewyork.us/x"), None)
            .is_ok());
    }

    #[test]
    fn throttling_backs_off_before_it_stops_the_host() {
        let governor = HostGovernor::default();
        let target = url("https://example.gov/a");
        for _ in 1..THROTTLE_STRIKES_BEFORE_STOP {
            assert!(governor
                .record_status(&target, 429, Some(Duration::ZERO))
                .is_some());
            assert!(governor.admits(&target, None).is_ok());
        }
        assert_eq!(
            governor.record_status(&target, 429, Some(Duration::ZERO)),
            None
        );
        assert!(matches!(
            governor.admits(&target, None).unwrap_err(),
            RequestRefusal::HostStopped {
                reason: HostStopReason::Throttled {
                    strikes: THROTTLE_STRIKES_BEFORE_STOP
                },
                ..
            }
        ));
    }

    /// A portal that rate-limits a long collection answers 429 many times over the run. Only an
    /// unbroken run of them may stop the host, and a back-off without `Retry-After` must outlast
    /// the measured recovery window rather than retry into it.
    #[test]
    fn only_consecutive_throttles_stop_a_host_and_back_off_outlasts_recovery() {
        let governor = HostGovernor::default();
        let target = url("https://example.gov/a");
        assert_eq!(
            governor.record_status(&target, 429, None),
            Some(THROTTLE_BACKOFF_FLOOR)
        );
        for _ in 0..(THROTTLE_STRIKES_BEFORE_STOP * 3) {
            governor.record_status(&target, 200, None);
            assert!(governor
                .record_status(&target, 429, Some(Duration::ZERO))
                .is_some());
        }
        assert!(governor.admits(&target, None).is_ok());
    }

    /// Measured on sandiego.nextrequest.com: requests in flight when the host first throttles
    /// all answer 429, and counting each as a strike stopped the host within one burst.
    #[test]
    fn a_burst_of_429s_during_one_back_off_is_one_strike() {
        let governor = HostGovernor::default();
        let target = url("https://example.gov/a");
        for _ in 0..(THROTTLE_STRIKES_BEFORE_STOP * 4) {
            assert!(governor.record_status(&target, 429, None).is_some());
        }
        assert!(governor.admits(&target, None).is_ok());
    }

    #[test]
    fn learned_pacing_is_bounded_and_eases_back() {
        let governor = HostGovernor::new(1, 1_000, Duration::from_millis(400));
        let target = url("https://example.gov/a");
        for _ in 0..20 {
            governor.record_status(&target, 429, Some(Duration::ZERO));
            governor.record_status(&target, 200, None);
        }
        let learned = |governor: &HostGovernor| {
            governor.with_host(&HostGovernor::host_key(&target), |state| {
                state.throttled_interval
            })
        };
        assert!(learned(&governor).is_some_and(|interval| interval <= MAXIMUM_THROTTLED_INTERVAL));
        for _ in 0..1_000 {
            governor.record_status(&target, 200, None);
        }
        assert_eq!(
            learned(&governor),
            None,
            "steady success returns to the configured pace"
        );
    }

    #[tokio::test]
    async fn a_throttled_host_keeps_its_wider_pacing() {
        let governor = HostGovernor::new(1, 100, Duration::from_millis(10));
        let target = url("https://example.gov/a");
        governor.record_status(&target, 429, Some(Duration::ZERO));
        governor.record_status(&target, 200, None);
        drop(governor.reserve(&target, None).await.unwrap());
        let started = Instant::now();
        drop(governor.reserve(&target, None).await.unwrap());
        assert!(
            started.elapsed() >= Duration::from_millis(15),
            "the learned interval must outlast one success"
        );
    }

    #[test]
    fn a_robots_exclusion_refuses_a_path_without_stopping_the_host() {
        let governor = HostGovernor::default();
        let rules = RobotsRules::parse("User-agent: *\nDisallow: /contact\n");
        let excluded = url("https://data.ca.gov/contact?ref=x");
        let permitted = url("https://data.ca.gov/dataset");
        assert!(matches!(
            governor.admits(&excluded, Some(&rules)).unwrap_err(),
            RequestRefusal::RobotsExcluded { .. }
        ));
        assert!(governor.admits(&permitted, Some(&rules)).is_ok());
        // The operator pointing the application at a path overrides the crawl exclusion.
        governor.mark_user_directed(&excluded);
        assert!(governor.admits(&excluded, Some(&rules)).is_ok());
    }

    #[tokio::test]
    async fn the_per_host_request_budget_stops_the_host_and_says_so() {
        let governor = HostGovernor::new(2, 2, Duration::ZERO);
        let target = url("https://example.gov/a");
        governor.reserve(&target, None).await.unwrap();
        governor.reserve(&target, None).await.unwrap();
        let refusal = governor.reserve(&target, None).await.unwrap_err();
        assert_eq!(
            refusal,
            RequestRefusal::HostStopped {
                host: "example.gov".to_string(),
                reason: HostStopReason::BudgetSpent { budget: 2 },
            }
        );
        assert!(refusal.describe().contains("the host was not exhausted"));
    }

    #[tokio::test]
    async fn concurrency_is_capped_per_host() {
        let governor = Arc::new(HostGovernor::new(1, 100, Duration::ZERO));
        let target = url("https://example.gov/a");
        let held = governor.reserve(&target, None).await.unwrap();
        let contender = {
            let governor = Arc::clone(&governor);
            let target = target.clone();
            tokio::spawn(async move { governor.reserve(&target, None).await.map(|_| ()) })
        };
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!contender.is_finished());
        drop(held);
        contender.await.unwrap().unwrap();
    }
}
