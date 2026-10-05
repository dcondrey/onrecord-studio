//! Posting time of a meeting document, read from what the server said when the file was fetched.
//!
//! A `Last-Modified` header is the server's statement about the file. It is shown as that and
//! nothing more: it does not prove when the public could first read the file, and no notice
//! requirement is evaluated against it. A header that is the request time is never shown as a
//! time; each way of having no posting time is a distinct [`PostingAbsence`].
use crate::types::meetings::{PostingAbsence, PostingCounts, PostingTime};
use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime, Utc, Weekday};
use regex::Regex;
use std::sync::LazyLock;

/// Hosts measured to answer every request with `Last-Modified` set to the request time
/// (OnBase Agenda Online pages; 2026-10-04 and 2026-10-05, header 0 to 2 seconds before `Date`).
const REQUEST_TIME_HOSTS: &[&str] = &["sandiego.hylandcloud.com"];

/// A generated page stamps `Last-Modified` up to two seconds before its `Date` (measured on
/// sandiego.gov HTML and OnBase). A file really modified this close to the fetch is shown as not
/// recorded, which is the safe direction.
const SAME_RESPONSE_SECONDS: i64 = 5;

/// Without the response `Date`, the only clock to compare with is the local one, which can be
/// wrong. A header must precede the fetch by this much before it is taken as a file time.
const LOCAL_CLOCK_MARGIN_SECONDS: i64 = 3_600;

pub(crate) const NOTICE_COUNT_THRESHOLD_MINUTES: i64 = 72 * 60;

/// What one fetch recorded, as stored in `document_acquisitions`.
#[derive(Debug, Clone, Default)]
pub(crate) struct FetchHeaders {
    pub last_modified: Option<String>,
    pub response_date: Option<String>,
    pub retrieved_at: String,
}

fn http_date(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc2822(value.trim())
        .ok()
        .map(|time| time.with_timezone(&Utc))
}

/// The file time a fetch supports, or why it supports none.
pub(crate) fn posted_at(
    url: Option<&str>,
    fetch: Option<&FetchHeaders>,
) -> Result<DateTime<Utc>, PostingAbsence> {
    let Some(fetch) = fetch else {
        return Err(PostingAbsence::NoFetchRecord);
    };
    let host = url
        .and_then(|url| reqwest::Url::parse(url).ok())
        .and_then(|url| url.host_str().map(str::to_ascii_lowercase));
    if host.is_some_and(|host| REQUEST_TIME_HOSTS.contains(&host.as_str())) {
        return Err(PostingAbsence::RequestTimeHost);
    }
    let Some(header) = fetch.last_modified.as_deref() else {
        return Err(PostingAbsence::NoHeader);
    };
    let Some(modified) = http_date(header) else {
        return Err(PostingAbsence::UnreadableHeader);
    };
    match fetch.response_date.as_deref().and_then(http_date) {
        Some(answered) => {
            if modified + Duration::seconds(SAME_RESPONSE_SECONDS) >= answered {
                return Err(PostingAbsence::HeaderIsRequestTime);
            }
        }
        None => {
            let retrieved = DateTime::parse_from_rfc3339(&fetch.retrieved_at)
                .ok()
                .map(|time| time.with_timezone(&Utc));
            if retrieved.is_none_or(|retrieved| {
                modified + Duration::seconds(LOCAL_CLOCK_MARGIN_SECONDS) >= retrieved
            }) {
                return Err(PostingAbsence::RequestTimeNotRuledOut);
            }
        }
    }
    Ok(modified)
}

pub(crate) fn posting_time(posted: &Result<DateTime<Utc>, PostingAbsence>) -> PostingTime {
    match posted {
        Ok(time) => PostingTime::Recorded {
            last_modified: time.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        },
        Err(reason) => PostingTime::NotRecorded { reason: *reason },
    }
}

static START_TIME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^[\s,–—-]*(?:at\s+)?(?P<hour>[0-9]{1,2})(?::(?P<minute>[0-5][0-9]))?\s*(?P<half>[ap])\.?\s?m\b")
        .unwrap()
});

/// A start time written directly after the heading date ("…, 2026, AT 10:00 AM"). A time
/// anywhere else in the heading is not taken as the start.
pub(crate) fn start_time_after_date(after_date: &str) -> Option<NaiveTime> {
    let found = START_TIME.captures(after_date)?;
    let hour: u32 = found["hour"].parse().ok()?;
    if !(1..=12).contains(&hour) {
        return None;
    }
    let minute: u32 = found
        .name("minute")
        .map_or(Some(0), |m| m.as_str().parse().ok())?;
    let afternoon = found["half"].eq_ignore_ascii_case("p");
    NaiveTime::from_hms_opt(hour % 12 + if afternoon { 12 } else { 0 }, minute, 0)
}

fn nth_sunday(year: i32, month: u32, nth: u32) -> Option<NaiveDate> {
    NaiveDate::from_weekday_of_month_opt(year, month, Weekday::Sun, nth as u8)
}

/// US Pacific wall-clock time as UTC, by the federal rule in force since 2007 (daylight time from
/// 02:00 on the second Sunday of March to 02:00 on the first Sunday of November). The app links
/// no time-zone database.
pub(crate) fn pacific_to_utc(local: NaiveDateTime) -> Option<DateTime<Utc>> {
    let year = local.date().year();
    let two = NaiveTime::from_hms_opt(2, 0, 0)?;
    let begins = nth_sunday(year, 3, 2)?.and_time(two);
    let ends = nth_sunday(year, 11, 1)?.and_time(two);
    let offset_hours = if local >= begins && local < ends {
        7
    } else {
        8
    };
    Some((local + Duration::hours(offset_hours)).and_utc())
}

/// Whether a document's address belongs to a publisher whose meeting times are Pacific time:
/// every public agency the source table classifies is in San Diego County or is the State.
pub(crate) fn pacific_publisher(url: Option<&str>) -> bool {
    url.is_some_and(|url| {
        !matches!(
            crate::ingestion::regional::source_jurisdiction(url),
            "Unclassified source"
                | "Multi-publisher catalogs"
                | "News publishers"
                | "Research and policy organizations"
                | "Independent dashboards"
        )
    })
}

/// Whole minutes from the file time to the meeting start; negative when the file time is later.
pub(crate) fn minutes_before(posted: DateTime<Utc>, meeting_start: DateTime<Utc>) -> i64 {
    (meeting_start - posted).num_seconds().div_euclid(60)
}

pub(crate) fn count(counts: &mut PostingCounts, minutes: i64) {
    counts.agenda_files_with_both_times += 1;
    if minutes < 0 {
        counts.after_meeting_start += 1;
    } else if minutes < NOTICE_COUNT_THRESHOLD_MINUTES {
        counts.under_72_hours_before_start += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PDF: &str = "https://www.sandiego.gov/sites/default/files/2026-10/agenda.pdf";

    fn fetch(last_modified: &str, response_date: Option<&str>, retrieved_at: &str) -> FetchHeaders {
        FetchHeaders {
            last_modified: Some(last_modified.into()),
            response_date: response_date.map(Into::into),
            retrieved_at: retrieved_at.into(),
        }
    }

    #[test]
    fn a_header_that_is_the_request_time_is_never_a_posting_time() {
        // Measured: the 2026-10-06 Council agenda PDF, fetched three days after its header.
        let file = fetch(
            "Fri, 02 Oct 2026 21:31:25 GMT",
            Some("Mon, 05 Oct 2026 07:16:39 GMT"),
            "2026-10-05T07:16:39.100000Z",
        );
        assert_eq!(
            posted_at(Some(PDF), Some(&file)).unwrap().to_rfc3339(),
            "2026-10-02T21:31:25+00:00"
        );
        // Measured on a sandiego.gov page: the header trails the response Date by a second.
        let page = fetch(
            "Mon, 05 Oct 2026 07:14:52 GMT",
            Some("Mon, 05 Oct 2026 07:14:53 GMT"),
            "2026-10-05T07:14:53.400000Z",
        );
        assert_eq!(
            posted_at(Some("https://www.sandiego.gov/pab/meetings"), Some(&page)),
            Err(PostingAbsence::HeaderIsRequestTime)
        );
        // The docket host is refused whatever its header says.
        assert_eq!(
            posted_at(
                Some("https://sandiego.hylandcloud.com/211agendaonlinecouncil/Documents/ViewAgenda?meetingId=7192"),
                Some(&file)
            ),
            Err(PostingAbsence::RequestTimeHost)
        );
        // A fetch stored before the response Date was kept: a wrong local clock must not turn a
        // request-time header into a file time.
        let old_row = fetch(
            "Mon, 05 Oct 2026 07:14:52 GMT",
            None,
            "2026-10-05T07:40:00.000000Z",
        );
        assert_eq!(
            posted_at(Some(PDF), Some(&old_row)),
            Err(PostingAbsence::RequestTimeNotRuledOut)
        );
        assert!(posted_at(
            Some(PDF),
            Some(&fetch(
                &file.last_modified.clone().unwrap(),
                None,
                &file.retrieved_at
            ))
        )
        .is_ok());
        assert_eq!(
            posted_at(Some(PDF), None),
            Err(PostingAbsence::NoFetchRecord)
        );
        assert_eq!(
            posted_at(Some(PDF), Some(&FetchHeaders::default())),
            Err(PostingAbsence::NoHeader)
        );
        assert_eq!(
            posted_at(Some(PDF), Some(&fetch("yesterday", None, ""))),
            Err(PostingAbsence::UnreadableHeader)
        );
    }

    #[test]
    fn the_72_hour_count_includes_71h59m_and_excludes_72h_and_72h01m() {
        let start = pacific_to_utc(
            NaiveDate::from_ymd_opt(2026, 10, 6)
                .unwrap()
                .and_hms_opt(10, 0, 0)
                .unwrap(),
        )
        .unwrap();
        // 10:00 Pacific daylight time is 17:00 UTC.
        assert_eq!(start.to_rfc3339(), "2026-10-06T17:00:00+00:00");
        let mut counts = PostingCounts::default();
        for (before, minutes) in [
            (Duration::minutes(71 * 60 + 59), 4_319),
            (Duration::hours(72), 4_320),
            (Duration::minutes(72 * 60 + 1), 4_321),
        ] {
            let found = minutes_before(start - before, start);
            assert_eq!(found, minutes);
            count(&mut counts, found);
        }
        count(
            &mut counts,
            minutes_before(start + Duration::hours(2), start),
        );
        assert_eq!(
            counts,
            PostingCounts {
                agenda_files_with_both_times: 4,
                under_72_hours_before_start: 1,
                after_meeting_start: 1,
            }
        );
    }

    #[test]
    fn pacific_time_changes_offset_at_the_federal_transition_days() {
        let at = |m, d, h| {
            pacific_to_utc(
                NaiveDate::from_ymd_opt(2026, m, d)
                    .unwrap()
                    .and_hms_opt(h, 0, 0)
                    .unwrap(),
            )
            .unwrap()
            .format("%d %H")
            .to_string()
        };
        assert_eq!(at(3, 8, 1), "08 09"); // standard time until 02:00 on the second Sunday
        assert_eq!(at(3, 8, 3), "08 10");
        assert_eq!(at(11, 1, 3), "01 11"); // standard again after the first Sunday of November
        assert_eq!(at(10, 31, 12), "31 19");
    }

    #[test]
    fn a_start_time_is_read_only_directly_after_the_date() {
        let time = |text| start_time_after_date(text).map(|t| t.format("%H:%M").to_string());
        assert_eq!(
            time(", AT 10:00 AM COUNCIL CHAMBERS").as_deref(),
            Some("10:00")
        );
        assert_eq!(
            time(" AT 1:00 P.M. CITY ADMINISTRATION").as_deref(),
            Some("13:00")
        );
        assert_eq!(time(" – 12:30 p.m.").as_deref(), Some("12:30"));
        assert_eq!(time(" 12 am").as_deref(), Some("00:00"));
        assert_eq!(time(" Closed session begins at 9:00 AM"), None);
        assert_eq!(time(" at 13:00 PM"), None);
        assert_eq!(time(""), None);
    }
}
