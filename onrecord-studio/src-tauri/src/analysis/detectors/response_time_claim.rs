//! Response-time claim detector.
//!
//! A released document (or an official statement) says how quickly an agency answers records
//! requests: "the average response time was 10 days", "requests are closed in a median of 8
//! days". The portal's own request rows record when each request was opened and closed, so the
//! stated figure can be set against the closures the agency actually recorded for the stated
//! period. A sentence disagreeing with the agency's own closure log is a pattern signal for a
//! reader to check; it decides nothing about which side is wrong.
//!
//! Only calendar days are measured (opened-at to closed-at), so a sentence stated in business
//! days is left alone rather than converted by a guess. A claim with fewer than
//! `MIN_SAMPLE` recorded closures in its window is not evaluated: a mean of three requests is
//! not a record of practice.

use super::public_documents::{load_public_documents, PublicDocument};
use super::text_shared::split_units;
use crate::{
    analysis::{
        document_class::{classify, DocumentClass},
        finding::{
            finding_id, Finding, FindingCitation, FindingConfidence, FindingKind, FindingSeverity,
            FindingSubject,
        },
        phrase_match::{has_phrase, words, Phrases, Word},
    },
    error::AppError,
    storage::DuckDbRepository,
};
use chrono::Utc;
use duckdb::params;

pub const SLUG: &str = "response_time_claim";

const MAX_DOCUMENTS: i64 = 2_000;
const MAX_CLAIMS: usize = 200;
const MAX_FINDINGS: usize = 50;
/// Fewer recorded closures than this and the measured figure is not a record of practice.
const MIN_SAMPLE: i64 = 5;
/// Reading `average` as `median` is a category error; each metric compares with its own.
const AVERAGE: Phrases = &[&["average"], &["mean"], &["on", "average"]];
const MEDIAN: Phrases = &[&["median"], &["typical"]];
/// Records requests specifically. A bare "requests" admitted service-request statistics: on a
/// live San Diego collection, "NPD closed 882 requests with an average response time of 14 days"
/// (police calls for service) was set against the records portal's closures.
const REQUEST_CONTEXT: Phrases = &[
    &["records", "request"],
    &["records", "requests"],
    &["public", "records"],
    &["pra"],
    &["cpra"],
    &["foia"],
];
const RESPONSE_CONTEXT: Phrases = &[
    &["response", "time"],
    &["response", "times"],
    &["turnaround"],
    &["processing", "time"],
    &["time", "to", "close"],
    &["time", "to", "respond"],
    &["closed", "in"],
    &["closed", "within"],
    &["fulfilled", "in"],
    &["fulfilled", "within"],
    &["completed", "in"],
    &["completed", "within"],
    &["responded", "in"],
    &["responded", "within"],
    &["responds", "in"],
    &["responds", "within"],
    &["responds", "to"],
    &["respond", "to"],
    &["responded", "to"],
    &["respond", "in"],
    &["respond", "within"],
    &["days", "to", "respond"],
    &["days", "to", "close"],
    &["days", "to", "fulfill"],
];
const BUSINESS_DAYS: Phrases = &[&["business", "days"], &["working", "days"]];
const APPROXIMATE: Phrases = &[&["about"], &["approximately"], &["roughly"], &["around"]];
const AT_MOST: Phrases = &[
    &["under"],
    &["less", "than"],
    &["within"],
    &["at", "most"],
    &["no", "more", "than"],
    &["below"],
    &["fewer", "than"],
];
const AT_LEAST: Phrases = &[
    &["over"],
    &["more", "than"],
    &["at", "least"],
    &["exceeding"],
];
const FISCAL: Phrases = &[&["fiscal", "year"], &["fy"]];
const NUMBER_WORDS: &[(&str, u32)] = &[
    ("one", 1),
    ("two", 2),
    ("three", 3),
    ("four", 4),
    ("five", 5),
    ("six", 6),
    ("seven", 7),
    ("eight", 8),
    ("nine", 9),
    ("ten", 10),
    ("eleven", 11),
    ("twelve", 12),
    ("thirteen", 13),
    ("fourteen", 14),
    ("fifteen", 15),
    ("twenty", 20),
    ("thirty", 30),
    ("forty", 40),
    ("fifty", 50),
    ("sixty", 60),
    ("ninety", 90),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Metric {
    Average,
    Median,
}

impl Metric {
    fn label(self) -> &'static str {
        match self {
            Self::Average => "average",
            Self::Median => "median",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hedge {
    Exact,
    Approximate,
    AtMost,
    AtLeast,
}

/// The window a sentence scopes its figure to. `None` is "no period stated", which is read as
/// every recorded closure, and said so in the finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Period {
    Calendar(i32),
    /// Named by the year it ends in; July to June, the City's fiscal calendar.
    Fiscal(i32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponseTimeClaim {
    pub metric: Metric,
    pub days: u32,
    pub hedge: Hedge,
    pub period: Option<Period>,
    pub char_start: usize,
    pub char_end: usize,
    pub source_text: String,
}

/// Every response-time claim in `text`, one per sentence at most, in document order.
pub fn extract_claims(text: &str) -> Vec<ResponseTimeClaim> {
    sentences(text)
        .into_iter()
        .filter_map(|(start, end)| {
            let sentence = &text[start..end];
            let tokens = words(sentence);
            let metric = if has_phrase(&tokens, MEDIAN) {
                Metric::Median
            } else if has_phrase(&tokens, AVERAGE) {
                Metric::Average
            } else {
                return None;
            };
            if !has_phrase(&tokens, REQUEST_CONTEXT) || !has_phrase(&tokens, RESPONSE_CONTEXT) {
                return None;
            }
            if has_phrase(&tokens, BUSINESS_DAYS) {
                return None;
            }
            let days = days_figure(&tokens)?;
            let hedge = if has_phrase(&tokens, AT_MOST) {
                Hedge::AtMost
            } else if has_phrase(&tokens, AT_LEAST) {
                Hedge::AtLeast
            } else if has_phrase(&tokens, APPROXIMATE) {
                Hedge::Approximate
            } else {
                Hedge::Exact
            };
            Some(ResponseTimeClaim {
                metric,
                days,
                hedge,
                period: period(&tokens),
                char_start: text[..start].chars().count(),
                char_end: text[..end].chars().count(),
                source_text: sentence.trim().to_string(),
            })
        })
        .collect()
}

/// The first `N days` in the sentence, where N is digits or a small number word. A figure that
/// is itself a year (`2024 days`) is nonsense and ignored.
fn days_figure(tokens: &[Word]) -> Option<u32> {
    tokens.windows(2).find_map(|pair| {
        let unit = pair[1].text.as_str();
        if unit != "days" && unit != "day" {
            return None;
        }
        let word = pair[0].text.as_str();
        let value = if word.chars().all(|c| c.is_ascii_digit()) {
            word.parse::<u32>().ok()?
        } else {
            NUMBER_WORDS
                .iter()
                .find(|(name, _)| *name == word)
                .map(|(_, value)| *value)?
        };
        (value > 0 && value < 1000).then_some(value)
    })
}

fn period(tokens: &[Word]) -> Option<Period> {
    let fiscal = has_phrase(tokens, FISCAL);
    let year = tokens.iter().enumerate().find_map(|(index, word)| {
        let year = parse_year(&word.text).or_else(|| parse_fiscal_suffix(&word.text))?;
        // "2023-24" and "2023/2024" name the fiscal year ending in the second year.
        let ranged = matches!(word.followed_by, Some('-' | '/' | '–'));
        let end = ranged
            .then(|| tokens.get(index + 1).and_then(|w| parse_range_end(&w.text)))
            .flatten();
        Some(end.unwrap_or(year))
    })?;
    Some(if fiscal {
        Period::Fiscal(year)
    } else {
        Period::Calendar(year)
    })
}

/// A four-digit year. Two digits are accepted only behind an `fy` prefix (`fy24`), so a
/// day count such as `10 days` is never read as a year.
fn parse_year(word: &str) -> Option<i32> {
    (word.len() == 4)
        .then(|| word.parse::<i32>().ok())
        .flatten()
        .filter(|year| (2005..=2040).contains(year))
}

/// The second half of a year range: two digits (`24`) or four (`2024`).
fn parse_range_end(word: &str) -> Option<i32> {
    match word.len() {
        2 => word
            .parse::<i32>()
            .ok()
            .map(|y| 2000 + y)
            .filter(|y| (2005..=2040).contains(y)),
        _ => parse_year(word),
    }
}

fn parse_fiscal_suffix(word: &str) -> Option<i32> {
    let digits = word.strip_prefix("fy")?;
    match digits.len() {
        2 => digits
            .parse::<i32>()
            .ok()
            .map(|y| 2000 + y)
            .filter(|y| (2005..=2040).contains(y)),
        _ => parse_year(digits),
    }
}

/// Byte ranges of sentences; a period ends one only before whitespace or end of text.
fn sentences(text: &str) -> Vec<(usize, usize)> {
    split_units(text, false, 12)
}

/// The band a stated figure is held to, in days. A whole-number statement carries at least half
/// a day of rounding; beyond that, ten percent of the figure is the tolerance a reader would
/// give a round summary statistic, doubled for "about".
pub fn band(days: u32, hedge: Hedge) -> (f64, f64) {
    let stated = f64::from(days);
    let half = (stated * 0.10).max(0.5);
    match hedge {
        Hedge::Exact => (stated - half, stated + half),
        Hedge::Approximate => (stated - half * 2.0, stated + half * 2.0),
        Hedge::AtMost => (0.0, stated + 0.5),
        Hedge::AtLeast => (stated - 0.5, f64::INFINITY),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RecordedClosures {
    pub sample: i64,
    pub average_days: f64,
    pub median_days: f64,
}

/// Window bounds as RFC 3339 instants, or empty strings for "no bound" (the query's convention).
fn window(period: Option<Period>) -> (String, String) {
    match period {
        None => (String::new(), String::new()),
        Some(Period::Calendar(year)) => (
            format!("{year}-01-01T00:00:00Z"),
            format!("{}-01-01T00:00:00Z", year + 1),
        ),
        Some(Period::Fiscal(year)) => (
            format!("{}-07-01T00:00:00Z", year - 1),
            format!("{year}-07-01T00:00:00Z"),
        ),
    }
}

/// Closure latency for one agency's requests opened in the window: opened-at to closed-at in
/// calendar days, counting only rows where both instants parse and the order is sane.
async fn recorded_closures(
    repository: &DuckDbRepository,
    portal_key: &str,
    agency_id: i64,
    period: Option<Period>,
) -> Result<Option<RecordedClosures>, AppError> {
    let (from, to) = window(period);
    let portal_key = portal_key.to_string();
    repository
        .query_connection("Response-time claim closures", move |conn| {
            let row = conn
                .query_row(
                    "WITH latency AS (
                        SELECT CAST(date_diff('microsecond',
                                    CAST(TRY_CAST(created_at AS TIMESTAMPTZ) AS TIMESTAMP),
                                    CAST(TRY_CAST(closed_at AS TIMESTAMPTZ) AS TIMESTAMP)) AS DOUBLE)
                               / 86400000000.0 AS days
                        FROM requests
                        WHERE portal_subdomain = ?1
                          AND agency_id = ?2
                          AND LOWER(TRIM(status)) <> 'withdrawn'
                          AND TRY_CAST(created_at AS TIMESTAMPTZ) IS NOT NULL
                          AND TRY_CAST(closed_at AS TIMESTAMPTZ) IS NOT NULL
                          AND TRY_CAST(closed_at AS TIMESTAMPTZ) >= TRY_CAST(created_at AS TIMESTAMPTZ)
                          AND (?3 = '' OR TRY_CAST(created_at AS TIMESTAMPTZ) >= TRY_CAST(?3 AS TIMESTAMPTZ))
                          AND (?4 = '' OR TRY_CAST(created_at AS TIMESTAMPTZ) < TRY_CAST(?4 AS TIMESTAMPTZ))
                    )
                    SELECT COUNT(*), AVG(days), MEDIAN(days) FROM latency",
                    params![portal_key, agency_id, from, to],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, Option<f64>>(1)?,
                            row.get::<_, Option<f64>>(2)?,
                        ))
                    },
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to measure recorded closures: {error}"))
                })?;
            Ok(match row {
                (sample, Some(average_days), Some(median_days)) if sample >= MIN_SAMPLE => {
                    Some(RecordedClosures {
                        sample,
                        average_days,
                        median_days,
                    })
                }
                _ => None,
            })
        })
        .await
}

async fn agency_for_request(
    repository: &DuckDbRepository,
    portal_key: &str,
    request_id: i64,
) -> Result<Option<i64>, AppError> {
    let portal_key = portal_key.to_string();
    repository
        .query_connection("Response-time claim agency", move |conn| {
            let mut statement = conn
                .prepare("SELECT agency_id FROM requests WHERE portal_subdomain = ?1 AND id = ?2")
                .map_err(|error| {
                    AppError::Storage(format!("Failed to prepare agency lookup: {error}"))
                })?;
            let mut rows = statement
                .query_map(params![portal_key, request_id], |row| {
                    row.get::<_, Option<i64>>(0)
                })
                .map_err(|error| AppError::Storage(format!("Failed to look up agency: {error}")))?;
            match rows.next() {
                Some(row) => row
                    .map_err(|error| AppError::Storage(format!("Failed to read agency: {error}"))),
                None => Ok(None),
            }
        })
        .await
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Consistent,
    Mismatch { relative: f64 },
}

pub fn compare(claim: &ResponseTimeClaim, recorded: &RecordedClosures) -> (f64, Outcome) {
    let measured = match claim.metric {
        Metric::Average => recorded.average_days,
        Metric::Median => recorded.median_days,
    };
    let (lower, upper) = band(claim.days, claim.hedge);
    if measured >= lower && measured <= upper {
        return (measured, Outcome::Consistent);
    }
    let stated = f64::from(claim.days).max(0.5);
    let relative = ((measured - stated) / stated).abs();
    (measured, Outcome::Mismatch { relative })
}

pub async fn run(repository: &DuckDbRepository) -> Result<Vec<Finding>, AppError> {
    let mut findings = Vec::new();
    let mut checked = 0usize;
    'documents: for document in load_public_documents(repository, MAX_DOCUMENTS).await? {
        let claims = extract_claims(&document.text);
        if claims.is_empty() {
            continue;
        }
        let Some(agency_id) =
            agency_for_request(repository, &document.portal_key, document.request_id).await?
        else {
            continue;
        };
        let source_is_statement = classify(document.file_name.as_deref(), &document.text)
            .is_some_and(DocumentClass::is_statement);
        for claim in claims {
            if checked >= MAX_CLAIMS || findings.len() >= MAX_FINDINGS {
                break 'documents;
            }
            checked += 1;
            let Some(recorded) =
                recorded_closures(repository, &document.portal_key, agency_id, claim.period)
                    .await?
            else {
                continue;
            };
            let (measured, outcome) = compare(&claim, &recorded);
            let Outcome::Mismatch { relative } = outcome else {
                continue;
            };
            findings.push(to_finding(
                &document,
                agency_id,
                source_is_statement,
                &claim,
                &recorded,
                measured,
                relative,
            ));
        }
    }
    Ok(findings)
}

fn period_label(period: Option<Period>) -> String {
    match period {
        None => "every recorded closure".to_string(),
        Some(Period::Calendar(year)) => format!("requests opened in {year}"),
        Some(Period::Fiscal(year)) => format!("requests opened in FY {year}"),
    }
}

fn to_finding(
    document: &PublicDocument,
    agency_id: i64,
    source_is_statement: bool,
    claim: &ResponseTimeClaim,
    recorded: &RecordedClosures,
    measured: f64,
    relative: f64,
) -> Finding {
    let subject = FindingSubject {
        portal_key: Some(document.portal_key.clone()),
        agency_id: Some(agency_id),
        request_id: Some(document.request_id),
        document_id: Some(document.document_id),
        entity_id: None,
    };
    let scope = period_label(claim.period);
    let salient = format!(
        "{}:{}:{}:{}",
        claim.metric.label(),
        claim.days,
        scope,
        claim.char_start
    );
    let id = finding_id(
        FindingKind::ClaimContradictsData,
        SLUG,
        &subject,
        &[&salient],
    );
    let prefix = if source_is_statement {
        "Official statement vs recorded closures: "
    } else {
        ""
    };
    let headline = format!(
        "{prefix}Stated {} response time of {} days; the portal records {measured:.1} days over {} closures",
        claim.metric.label(),
        claim.days,
        recorded.sample
    );
    let (lower, upper) = band(claim.days, claim.hedge);
    let upper_label = if upper.is_finite() {
        format!("{upper:.1}")
    } else {
        "no upper bound".to_string()
    };
    let explanation = format!(
        "'{}' states a {} response time of {} days. The portal's own request rows for this agency \
         ({scope}) record {} closures with an average of {:.1} days and a median of {:.1} days, \
         measured from the request's opened-at to its closed-at in calendar days. The stated \
         figure was held to {lower:.1} to {upper_label} days. Which figure is right, and what \
         population or clock the document used, is not decided here: an agency may count business \
         days, a different start event, or requests the portal does not hold.",
        document
            .file_name
            .clone()
            .unwrap_or_else(|| format!("document {}", document.document_id)),
        claim.metric.label(),
        claim.days,
        recorded.sample,
        recorded.average_days,
        recorded.median_days,
    );
    let severity = if relative >= 0.5 {
        FindingSeverity::Serious
    } else {
        FindingSeverity::Notable
    };
    // Derived from what was observed: the size of the closure sample, and whether the sentence
    // itself named the period the sample was drawn for.
    let sample_bp = ((recorded.sample - MIN_SAMPLE) as f64 * 50.0).clamp(0.0, 2500.0) as u16;
    let scope_bp = if claim.period.is_some() { 1000 } else { 0 };
    let confidence = FindingConfidence::measured(5000 + sample_bp + scope_bp);
    let now = Utc::now();
    Finding {
        id,
        kind: FindingKind::ClaimContradictsData,
        severity,
        confidence,
        subject,
        headline,
        explanation,
        citations: vec![FindingCitation {
            portal_key: document.portal_key.clone(),
            document_id: document.document_id,
            character_start: Some(claim.char_start as i64),
            character_end: Some(claim.char_end as i64),
            quote: Some(claim.source_text.chars().take(240).collect()),
            label: document.file_name.clone(),
        }],
        discovered_at: now,
        last_seen_at: now,
        dismissed_at: None,
        source_detector: SLUG.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Expected = Option<(Metric, u32, Hedge, Option<Period>)>;

    #[test]
    fn extracts_average_and_median_day_claims_with_their_scope_and_hedge() {
        let cases: &[(&str, Expected)] = &[
            (
                "The average response time for public records requests was 10 days in 2024.",
                Some((
                    Metric::Average,
                    10,
                    Hedge::Exact,
                    Some(Period::Calendar(2024)),
                )),
            ),
            (
                "Records requests were closed in a median of eight days during fiscal year 2023-24.",
                Some((Metric::Median, 8, Hedge::Exact, Some(Period::Fiscal(2024)))),
            ),
            (
                "On average the department responds to records requests within 12 days.",
                Some((Metric::Average, 12, Hedge::AtMost, None)),
            ),
            (
                "The average response time for PRA requests was about 15 days.",
                Some((Metric::Average, 15, Hedge::Approximate, None)),
            ),
            // Business days are not measured here.
            (
                "Requests are closed in an average of 10 business days.",
                None,
            ),
            // No metric, no request context, no figure.
            ("The response time was 10 days.", None),
            ("The average request took a long time.", None),
            (
                "Case 2024-0412 was closed after the requester withdrew the appeal.",
                None,
            ),
        ];
        for (text, expected) in cases {
            let claims = extract_claims(text);
            let got = claims
                .first()
                .map(|c| (c.metric, c.days, c.hedge, c.period));
            assert_eq!(got, *expected, "{text}");
        }
    }

    #[test]
    fn bands_carry_rounding_and_hedge() {
        assert_eq!(band(10, Hedge::Exact), (9.0, 11.0));
        assert_eq!(band(3, Hedge::Exact), (2.5, 3.5));
        assert_eq!(band(10, Hedge::Approximate), (8.0, 12.0));
        assert_eq!(band(10, Hedge::AtMost), (0.0, 10.5));
        assert_eq!(band(10, Hedge::AtLeast).0, 9.5);
        assert!(band(10, Hedge::AtLeast).1.is_infinite());
    }

    fn claim(metric: Metric, days: u32, hedge: Hedge) -> ResponseTimeClaim {
        ResponseTimeClaim {
            metric,
            days,
            hedge,
            period: None,
            char_start: 0,
            char_end: 10,
            source_text: "x".into(),
        }
    }

    #[test]
    fn compares_each_metric_with_its_own_figure() {
        let recorded = RecordedClosures {
            sample: 20,
            average_days: 21.0,
            median_days: 10.4,
        };
        assert_eq!(
            compare(&claim(Metric::Median, 10, Hedge::Exact), &recorded).1,
            Outcome::Consistent
        );
        let (measured, outcome) = compare(&claim(Metric::Average, 10, Hedge::Exact), &recorded);
        assert_eq!(measured, 21.0);
        assert!(matches!(outcome, Outcome::Mismatch { relative } if (relative - 1.1).abs() < 1e-9));
        assert_eq!(
            compare(&claim(Metric::Average, 25, Hedge::AtMost), &recorded).1,
            Outcome::Consistent
        );
    }

    async fn seeded(sentence: &str, closure_days: &[i64]) -> DuckDbRepository {
        let repository = DuckDbRepository::new(":memory:").expect("open");
        repository
            .run_migrations(include_str!("../../../migrations/0001_initial_schema.sql"))
            .await
            .expect("migrate");
        let mut sql = String::from(
            "INSERT INTO portals (subdomain, display_name, base_url) VALUES ('sd', 'SD', 'https://sd.example');
             INSERT INTO agencies (id, portal_subdomain, name) VALUES (3, 'sd', 'Police Department');",
        );
        for (index, days) in closure_days.iter().enumerate() {
            let id = index as i64 + 10;
            sql.push_str(&format!(
                "INSERT INTO requests (id, portal_subdomain, title, status, created_at, updated_at, closed_at, agency_id)
                   VALUES ({id}, 'sd', 'R{id}', 'closed', '2024-03-01T00:00:00Z', '2024-03-02T00:00:00Z',
                           '2024-03-01T00:00:00'::TIMESTAMP + INTERVAL ({days}) DAY, 3);"
            ));
        }
        sql.push_str(&format!(
            "INSERT INTO requests (id, portal_subdomain, title, status, created_at, updated_at, agency_id)
               VALUES (1, 'sd', 'Report', 'closed', '2025-01-01T00:00:00Z', '2025-01-02T00:00:00Z', 3);
             INSERT INTO documents (id, request_id, portal_subdomain, file_name, visibility, created_at, updated_at, extracted_text_sha256)
               VALUES (1, 1, 'sd', 'annual-report.pdf', 'public', '2025-01-01T00:00:00Z', '2025-01-01T00:00:00Z', 'h1');
             INSERT INTO documents_fts (document_id, portal_subdomain, file_name, extracted_text)
               VALUES (1, 'sd', 'annual-report.pdf', '{sentence}');"
        ));
        repository
            .query_connection("response-time fixture", move |connection| {
                connection
                    .execute_batch(&sql)
                    .map_err(|error| AppError::Storage(format!("fixture: {error}")))
            })
            .await
            .expect("seed");
        repository
    }

    #[tokio::test]
    async fn a_stated_average_outside_the_recorded_closures_is_a_finding() {
        let repository = seeded(
            "In 2024 the average response time for public records requests was 10 days.",
            &[20, 20, 20, 20, 20, 20],
        )
        .await;
        let findings = run(&repository).await.unwrap();
        assert_eq!(findings.len(), 1, "{findings:?}");
        let finding = &findings[0];
        assert_eq!(finding.kind, FindingKind::ClaimContradictsData);
        assert_eq!(finding.subject.agency_id, Some(3));
        assert!(
            finding.headline.contains("20.0 days over 6 closures"),
            "{}",
            finding.headline
        );
        assert!(finding.explanation.contains("requests opened in 2024"));
        assert_eq!(finding.severity, FindingSeverity::Serious);
        for word in ["false", "lied", "misled"] {
            assert!(!finding.headline.to_lowercase().contains(word));
        }
    }

    #[tokio::test]
    async fn a_consistent_figure_a_short_sample_or_another_year_yields_nothing() {
        let consistent = seeded(
            "In 2024 the average response time for public records requests was 20 days.",
            &[20, 20, 20, 20, 20, 20],
        )
        .await;
        assert!(run(&consistent).await.unwrap().is_empty());
        let short = seeded(
            "In 2024 the average response time for public records requests was 10 days.",
            &[20, 20, 20, 20],
        )
        .await;
        assert!(run(&short).await.unwrap().is_empty());
        let other_year = seeded(
            "In 2023 the average response time for public records requests was 10 days.",
            &[20, 20, 20, 20, 20, 20],
        )
        .await;
        assert!(run(&other_year).await.unwrap().is_empty());
    }

    #[test]
    fn a_service_request_statistic_is_not_a_records_response_claim() {
        assert!(extract_claims(
            "The week proceeding, NPD closed 882 requests with an average response time of 14 days."
        )
        .is_empty());
        assert_eq!(
            extract_claims("The average CPRA response time was 14 days in 2024.").len(),
            1
        );
    }
}
