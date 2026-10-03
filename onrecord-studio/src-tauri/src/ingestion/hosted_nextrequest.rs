//! The hosted NextRequest client API (`<agency>.nextrequest.com`), as measured against
//! sandiego.nextrequest.com on 2026-10-02:
//!
//! - `/client/requests?page_number=N[&search_term=T]` returns `{"total_count": N, "requests": [...]}`
//!   at a fixed 100 records per page. Any page-size parameter is ignored, `page` is ignored, and
//!   `search_term` filters on request text server-side.
//! - Records carry string identifiers (`"24-5408"`) and `MM/DD/YYYY` dates, and no documents.
//! - `/client/request_documents?request_id=<id>` lists a request's documents, and
//!   `/documents/<document id>/download` redirects to the file.
//! - `/client/requests/<id>/timeline` dates the request's events in the portal's local wall-clock
//!   time, with no zone given anywhere in the API.

use chrono::{Datelike, FixedOffset, NaiveDate, NaiveDateTime, TimeZone, Weekday};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::error::AppError;
use crate::types::nextrequest::AgencyMetadata;
use crate::types::{DocumentMetadata, PaginationMeta, PublicRequest};

pub(crate) const PAGE_PARAMETER: &str = "page_number";
pub(crate) const SEARCH_PARAMETER: &str = "search_term";
pub(crate) const PAGE_SIZE: usize = 100;
/// Documents are listed per request; a request with more pages than this is truncated, not
/// fetched without bound.
pub(crate) const MAXIMUM_DOCUMENT_PAGES: usize = 20;
/// A request's timeline pages 25 events at a time; one longer than this is read in part, and a
/// request whose opening falls beyond it is refused rather than dated from a later event.
pub(crate) const MAXIMUM_TIMELINE_PAGES: usize = 20;

const JAVASCRIPT_SAFE_MASK: u64 = (1 << 53) - 1;
const TITLE_CHARACTERS: usize = 140;

/// A listing row, also persisted as the payload of the task that reads its details.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListedRequest {
    pub id: String,
    #[serde(default)]
    request_state: Option<String>,
    #[serde(default)]
    request_text: Option<String>,
    #[serde(default)]
    department_names: Option<String>,
}

#[derive(Debug)]
pub struct ListedPage {
    pub requests: Vec<ListedRequest>,
    pub meta: PaginationMeta,
}

/// When the request was opened and, if the timeline records it, closed.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct RequestTimeline {
    pub opened_at: Option<String>,
    pub closed_at: Option<String>,
    /// The text of the closing event: the agency's response, such as "No responsive documents
    /// The City of San Diego has no responsive documents." Present only when `closed_at` is.
    pub closure_notice: Option<String>,
}

pub(crate) fn is_listing(value: &Value) -> bool {
    value.get("total_count").is_some_and(Value::is_u64)
        && value
            .get("requests")
            .and_then(Value::as_array)
            .is_some_and(|records| {
                records.iter().all(|record| {
                    record.get("id").is_some_and(Value::is_string)
                        && record.get("request_path").is_some_and(Value::is_string)
                })
            })
}

/// A request identifier safe to place in a URL path. NextRequest's are `YY-N`; anything else is
/// refused rather than interpolated.
pub(crate) fn is_safe_request_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

pub(crate) fn decode_listing(value: &Value, page: usize) -> Result<ListedPage, AppError> {
    if !is_listing(value) {
        return Err(AppError::DataParsing(
            "NextRequest returned a request listing in an unrecognized shape".to_string(),
        ));
    }
    let total_count = value
        .get("total_count")
        .and_then(Value::as_u64)
        .and_then(|count| usize::try_from(count).ok())
        .unwrap_or(0);
    let requests: Vec<ListedRequest> = serde_json::from_value(
        value.get("requests").cloned().unwrap_or(Value::Null),
    )
    .map_err(|error| {
        AppError::DataParsing(format!("Failed to decode NextRequest requests: {error}"))
    })?;
    let total_pages = total_count.div_ceil(PAGE_SIZE).max(1);
    Ok(ListedPage {
        requests,
        meta: PaginationMeta {
            current_page: page,
            next_page: (page < total_pages).then_some(page + 1),
            prev_page: (page > 1).then_some(page - 1),
            total_pages,
            total_count,
        },
    })
}

#[derive(Deserialize)]
struct TimelinePage {
    #[serde(default)]
    timeline: Vec<TimelineEntry>,
}

#[derive(Deserialize)]
struct TimelineEntry {
    #[serde(default)]
    timeline_name: Option<String>,
    #[serde(default)]
    timeline_byline: Option<String>,
    #[serde(default)]
    timeline_display_text: Option<String>,
}

/// The earliest "Request Opened" and the latest "Request Closed" event. A request reopened after
/// its last closure is open, so a closure followed by a reopening is not reported as closed.
pub(crate) fn decode_timeline(value: &Value) -> Result<RequestTimeline, AppError> {
    let page: TimelinePage = serde_json::from_value(value.clone()).map_err(|error| {
        AppError::DataParsing(format!("Failed to decode a NextRequest timeline: {error}"))
    })?;
    let mut opened: Option<NaiveDateTime> = None;
    let mut closed: Option<NaiveDateTime> = None;
    let mut reopened: Option<NaiveDateTime> = None;
    let mut notice: Option<String> = None;
    for entry in page.timeline {
        let Some(at) = entry.timeline_byline.as_deref().and_then(parse_byline) else {
            continue;
        };
        match entry.timeline_name.as_deref().map(str::trim) {
            Some("Request Opened") => opened = Some(opened.map_or(at, |seen| seen.min(at))),
            Some("Request Closed") => {
                if closed.is_none_or(|seen| at >= seen) {
                    closed = Some(at);
                    notice = entry.timeline_display_text.as_deref().map(plain_text);
                }
            }
            Some("Request Reopened") => reopened = Some(reopened.map_or(at, |seen| seen.max(at))),
            _ => {}
        }
    }
    let closed = closed.filter(|closed| reopened.is_none_or(|reopened| reopened < *closed));
    Ok(RequestTimeline {
        opened_at: opened.map(pacific_rfc3339),
        closure_notice: closed.and(notice).filter(|notice| !notice.is_empty()),
        closed_at: closed.map(pacific_rfc3339),
    })
}

/// Event HTML as plain text: tags become spaces, common entities are decoded, whitespace collapses.
fn plain_text(html: &str) -> String {
    let mut text = String::with_capacity(html.len());
    let mut in_tag = false;
    for character in html.chars() {
        match character {
            '<' => in_tag = true,
            '>' if in_tag => {
                in_tag = false;
                text.push(' ');
            }
            _ if !in_tag => text.push(character),
            _ => {}
        }
    }
    text.replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&#39;", "'")
        .replace("&quot;", "\"")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// `"August  8, 2024, 11:48am by the requester"` as a local wall-clock time.
fn parse_byline(byline: &str) -> Option<NaiveDateTime> {
    let when = byline.split(" by ").next()?;
    let normalized = when.split_whitespace().collect::<Vec<_>>().join(" ");
    NaiveDateTime::parse_from_str(&normalized.to_uppercase(), "%B %d, %Y, %I:%M%p").ok()
}

/// The API gives local wall-clock time and no zone. It is recorded at US Pacific offsets, which is
/// exact for the San Diego portal it was measured on. For a portal elsewhere the instant is off by
/// the zone difference, but the calendar date, which is what statutory counting reads, is the
/// date the portal displayed.
fn pacific_rfc3339(local: NaiveDateTime) -> String {
    let hours = if us_daylight_time(local) { -7 } else { -8 };
    let offset = FixedOffset::east_opt(hours * 3600).expect("a whole-hour offset is valid");
    offset
        .from_local_datetime(&local)
        .single()
        .expect("a fixed offset maps every local time exactly once")
        .to_rfc3339()
}

/// US daylight time since 2007: from 02:00 on the second Sunday in March to 02:00 on the first
/// Sunday in November, local time.
fn us_daylight_time(local: NaiveDateTime) -> bool {
    let year = local.year();
    let start = nth_sunday(year, 3, 2).and_hms_opt(2, 0, 0);
    let end = nth_sunday(year, 11, 1).and_hms_opt(2, 0, 0);
    matches!((start, end), (Some(start), Some(end)) if local >= start && local < end)
}

fn nth_sunday(year: i32, month: u32, nth: u32) -> NaiveDate {
    NaiveDate::from_weekday_of_month_opt(year, month, Weekday::Sun, nth as u8)
        .expect("every month has a first and second Sunday")
}

#[derive(Deserialize)]
struct DocumentsPage {
    #[serde(default)]
    total_documents_count: Option<u64>,
    #[serde(default)]
    documents: Vec<ListedDocument>,
}

#[derive(Deserialize)]
struct ListedDocument {
    id: u64,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    visibility: Option<String>,
    #[serde(default)]
    folder_name: Option<String>,
    #[serde(default)]
    document_scan: Option<DocumentScan>,
}

#[derive(Deserialize)]
struct DocumentScan {
    #[serde(default)]
    upload_date: Option<String>,
}

/// One page of a request's public documents, and the total the portal reports for the request.
/// A document whose upload date is missing or unparseable takes `fallback_created_at`.
pub(crate) fn decode_documents(
    value: &Value,
    origin: &str,
    fallback_created_at: &str,
) -> Result<(Vec<DocumentMetadata>, Option<u64>), AppError> {
    let page: DocumentsPage = serde_json::from_value(value.clone()).map_err(|error| {
        AppError::DataParsing(format!("Failed to decode NextRequest documents: {error}"))
    })?;
    let origin = origin.trim_end_matches('/');
    let documents = page
        .documents
        .into_iter()
        .filter(|document| {
            document
                .visibility
                .as_deref()
                .is_some_and(|visibility| visibility.eq_ignore_ascii_case("public"))
        })
        .map(|document| {
            let created_at = document
                .document_scan
                .as_ref()
                .and_then(|scan| scan.upload_date.as_deref())
                .filter(|value| chrono::DateTime::parse_from_rfc3339(value).is_ok())
                .unwrap_or(fallback_created_at)
                .to_string();
            DocumentMetadata {
                id: document.id,
                file_name: document.title,
                // `document_scan.file_size` is a decimal in an unstated unit (`0.153` on the
                // live portal), so it is not recorded as a byte count.
                file_size: None,
                description: document.folder_name.filter(|name| !name.trim().is_empty()),
                content_type: None,
                download_url: Some(format!("{origin}/documents/{}/download", document.id)),
                updated_at: created_at.clone(),
                created_at,
                visibility: "public".to_string(),
                local_path: None,
                extracted_text_sha256: None,
                was_ocr_processed: false,
            }
        })
        .collect();
    Ok((documents, page.total_documents_count))
}

/// The request as this application stores it. `due_date` stays empty: the portal gives a day, and
/// the deadline detectors read it as an instant, which would put the deadline at that day's start.
pub(crate) fn to_public_request(
    listed: ListedRequest,
    timeline: RequestTimeline,
    documents: Vec<DocumentMetadata>,
) -> Result<PublicRequest, AppError> {
    let created_at = timeline.opened_at.ok_or_else(|| {
        AppError::DataParsing(format!(
            "NextRequest request {} has no dated \"Request Opened\" event",
            listed.id
        ))
    })?;
    let status = listed
        .request_state
        .as_deref()
        .map(str::trim)
        .filter(|state| !state.is_empty())
        .unwrap_or("unreported")
        .to_ascii_lowercase();
    let text = listed
        .request_text
        .as_deref()
        .map(|text| text.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|text| !text.is_empty());
    let title = text.as_deref().map(|text| {
        if text.chars().count() <= TITLE_CHARACTERS {
            text.to_string()
        } else {
            let cut: String = text.chars().take(TITLE_CHARACTERS).collect();
            format!("{}…", cut.trim_end())
        }
    });
    let agency = listed
        .department_names
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(|name| AgencyMetadata {
            id: stable_identifier(&format!("nextrequest-department|{name}")),
            name: name.to_string(),
            department_name: None,
        });
    Ok(PublicRequest {
        id: stable_identifier(&format!("nextrequest-request|{}", listed.id)),
        rpid: Some(listed.id),
        title,
        description: text,
        status,
        current_status_id: None,
        // No other activity is reported; claiming any would invent it.
        updated_at: timeline
            .closed_at
            .clone()
            .unwrap_or_else(|| created_at.clone()),
        created_at,
        closed_at: timeline.closed_at,
        due_date: None,
        determination_at: None,
        determination_source: None,
        neighborhood_block: None,
        statutory_extension: None,
        documents,
        agency,
        category: None,
        timeline_anomalies: Vec::new(),
    })
}

fn stable_identifier(value: &str) -> u64 {
    let digest = Sha256::digest(value.as_bytes());
    u64::from_be_bytes(digest[..8].try_into().expect("SHA-256 prefix is 8 bytes"))
        & JAVASCRIPT_SAFE_MASK
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_listing_pages_by_the_reported_total_at_a_fixed_hundred() {
        let value = json!({"total_count": 142, "requests": [
            {"id": "24-5408", "request_path": "/requests/24-5408", "request_state": "Closed",
             "request_text": "Encampment   citations", "department_names": "Police"}
        ]});
        let first = decode_listing(&value, 1).unwrap();
        assert_eq!(first.meta.total_pages, 2);
        assert_eq!(first.meta.next_page, Some(2));
        assert_eq!(decode_listing(&value, 2).unwrap().meta.next_page, None);
        assert!(!is_listing(&json!({"requests": [{"id": 42}]})));
    }

    #[test]
    fn timeline_times_carry_the_pacific_offset_in_force_on_that_date() {
        let value = json!({"timeline": [
            {"timeline_name": "Request Closed", "timeline_byline": "August 29, 2024,  2:01pm by A (Staff)"},
            {"timeline_name": "Request Opened", "timeline_byline": "August  8, 2024, 11:48am by the requester"},
            {"timeline_name": "Request Opened", "timeline_byline": "January  3, 2024,  9:05am by the requester"}
        ]});
        let timeline = decode_timeline(&value).unwrap();
        assert_eq!(
            timeline.opened_at.as_deref(),
            Some("2024-01-03T09:05:00-08:00")
        );
        assert_eq!(
            timeline.closed_at.as_deref(),
            Some("2024-08-29T14:01:00-07:00")
        );
    }

    #[test]
    fn the_latest_closure_text_is_the_agency_response() {
        let value = json!({"timeline": [
            {"timeline_name": "Request Closed", "timeline_byline": "March 9, 2024, 9:00am",
             "timeline_display_text": "<p>No responsive documents</p><p>The City of San Diego has no responsive documents.</p>"},
            {"timeline_name": "Request Closed", "timeline_byline": "March 5, 2024, 9:00am",
             "timeline_display_text": "<p>02. Released</p>"},
            {"timeline_name": "Request Opened", "timeline_byline": "March 1, 2024, 9:00am"}
        ]});
        assert_eq!(
            decode_timeline(&value).unwrap().closure_notice.as_deref(),
            Some("No responsive documents The City of San Diego has no responsive documents.")
        );
    }

    #[test]
    fn a_request_reopened_after_its_last_closure_is_not_closed() {
        let value = json!({"timeline": [
            {"timeline_name": "Request Opened", "timeline_byline": "March 1, 2024, 9:00am"},
            {"timeline_name": "Request Closed", "timeline_byline": "March 5, 2024, 9:00am"},
            {"timeline_name": "Request Reopened", "timeline_byline": "March 9, 2024, 9:00am"}
        ]});
        assert_eq!(decode_timeline(&value).unwrap().closed_at, None);
    }

    #[test]
    fn daylight_time_begins_and_ends_on_the_us_sundays() {
        let at = |text: &str| NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M").unwrap();
        assert!(!us_daylight_time(at("2024-03-10 01:59")));
        assert!(us_daylight_time(at("2024-03-10 02:00")));
        assert!(us_daylight_time(at("2024-11-03 01:59")));
        assert!(!us_daylight_time(at("2024-11-03 02:00")));
    }

    #[test]
    fn only_public_documents_are_listed_with_a_download_route() {
        let value = json!({"total_documents_count": 2, "documents": [
            {"id": 37009476, "title": "PRA 24-5408.xlsx", "visibility": "Public", "folder_name": "",
             "document_scan": {"file_size": 0.153, "upload_date": "2024-08-14T12:38:53.425-07:00"}},
            {"id": 9, "title": "internal.pdf", "visibility": "Staff"}
        ]});
        let (documents, total) = decode_documents(
            &value,
            "https://city.nextrequest.com/",
            "2024-08-08T11:48:00-07:00",
        )
        .unwrap();
        assert_eq!(total, Some(2));
        assert_eq!(documents.len(), 1);
        assert_eq!(
            documents[0].download_url.as_deref(),
            Some("https://city.nextrequest.com/documents/37009476/download")
        );
        assert_eq!(documents[0].created_at, "2024-08-14T12:38:53.425-07:00");
        assert_eq!(documents[0].description, None);
    }

    #[test]
    fn a_request_without_a_dated_opening_is_refused_rather_than_dated_today() {
        let listed: ListedRequest =
            serde_json::from_value(json!({"id": "24-1", "request_state": "Open"})).unwrap();
        assert!(to_public_request(listed, RequestTimeline::default(), Vec::new()).is_err());
    }
}
