//! How much of an agency's releasing falls late on the last working day before a weekend or
//! court holiday, against the other releases on the same portal.
//!
//! **Which timestamp.** `documents.created_at` holds different things by ingestion path: the
//! portal's upload time of the file (NextRequest `document_scan.upload_date`), the request's
//! opening time when the portal gave no upload date, the request's closing time (the closure
//! notice), or the moment this application first saw the file (local, pasted and open-data
//! sources). Only the first is read: a public file with a download route on a NextRequest portal,
//! whose time differs from its request's opening time and carries a time of day and a local
//! offset. Everything else is counted as not measurable. `request_release_events.released_at` is
//! not read: it is per request, to the minute, and its offset is assumed rather than recorded.
//!
//! **What it is not.** A count of upload times. Staff often finish and post work at the end of a
//! week, and requests that fall due then are released then. The portal does not report when a
//! file uploaded earlier was made public.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use chrono::{DateTime, Duration, FixedOffset, NaiveDate, NaiveTime, Timelike, Utc};
use duckdb::params;

use crate::{
    analysis::{
        court_calendar::court_day_on_or_after,
        finding::{
            finding_id, Finding, FindingCitation, FindingConfidence, FindingKind, FindingSeverity,
            FindingSubject,
        },
    },
    error::AppError,
    parsers::{
        ooxml_docprops::read_ooxml_docprops, pdf_metadata_signals::read_pdf_metadata_signals,
    },
    storage::DuckDbRepository,
};

pub const SLUG: &str = "release_timing";

/// The window opens at 3:00 p.m. local time: the last two hours of an 8-to-5 office day. Measured
/// on 215 releases of sandiego.nextrequest.com (2026-10-04), 39 (18%) began at or after 3 p.m.
/// and one after 5 p.m., so the window is the tail of the working day, not a rare hour.
pub const WINDOW_START_HOUR: u32 = 15;
/// Files uploaded to one request less than this long after the previous file are one release. On
/// the same collection 397 of 505 gaps between successive files of a request are under a minute,
/// 17 fall between one minute and an hour, and 6 between an hour and a day.
pub const RELEASE_GAP_MINUTES: i64 = 60;
/// Below this many releases an agency's share moves by more than five points per release. Applied
/// after late releases on one date are counted once.
pub const MINIMUM_AGENCY_RELEASES: u64 = 20;
/// The portal's other releases, which the agency is compared with.
pub const MINIMUM_BASELINE_RELEASES: u64 = 40;
/// Late releases must fall on at least this many calendar dates, and each date counts once in
/// the comparison, so one large batch cannot produce the finding.
pub const MINIMUM_LATE_DATES: u64 = 3;
/// The agency's share must be at least this multiple of the baseline share.
pub const MINIMUM_SHARE_RATIO: u128 = 2;
/// And exceed it by at least this many percentage points.
pub const MINIMUM_SHARE_DIFFERENCE_POINTS: u128 = 10;

const MAX_LISTED_FILES: usize = 500_000;
const MAX_FILES_AGED: usize = 50;
const MAX_CITATIONS: usize = 5;

/// Requires the aliases `d` (documents) and `p` (portals). True for a file a NextRequest portal
/// lists for download, which is the only row whose `created_at` the portal supplied for the file.
pub(super) const PORTAL_LISTED_FILE: &str =
    "p.source_kind = 'NEXT_REQUEST' AND d.download_url IS NOT NULL";

/// Which recorded time a per-document comparison used as the release time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ReleaseBasis {
    DocumentUpload,
    RequestDetermination,
}

impl ReleaseBasis {
    pub(super) const fn describe(self) -> &'static str {
        match self {
            Self::DocumentUpload => "the portal's recorded upload time of this file",
            Self::RequestDetermination => {
                "the request's determination time; this file's own upload time is not recorded"
            }
        }
    }
}

/// The portal's upload time of a listed file, with the offset it is recorded at. `None` when the
/// value is the request's opening time (what ingestion stores when the portal gave no upload
/// date), has no time of day or no offset, is written in UTC without a local offset, or is
/// exactly midnight, which is how a date without a time is stored.
pub(super) fn portal_upload_time(
    created_at: &str,
    request_created_at: &str,
) -> Option<DateTime<FixedOffset>> {
    let value = created_at.trim();
    if value == request_created_at.trim()
        || value.ends_with(['Z', 'z'])
        || value.ends_with("-00:00")
    {
        return None;
    }
    let at = DateTime::parse_from_rfc3339(value).ok()?;
    (at.time() != NaiveTime::MIN).then_some(at)
}

/// A file's internal modification time against its upload time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileAge {
    /// Last modified this long before upload.
    Before(Duration),
    /// Modified after it was uploaded, by this much: a contradiction, not an age.
    AfterUpload(Duration),
    NotRecorded,
}

fn file_age(uploaded: DateTime<FixedOffset>, modified: Option<DateTime<Utc>>) -> FileAge {
    match modified {
        None => FileAge::NotRecorded,
        Some(modified) => {
            let age = uploaded.with_timezone(&Utc) - modified;
            if age < Duration::zero() {
                FileAge::AfterUpload(-age)
            } else {
                FileAge::Before(age)
            }
        }
    }
}

#[derive(Debug, Clone)]
struct ListedFile {
    portal_key: String,
    request_id: i64,
    agency_id: Option<i64>,
    agency_name: Option<String>,
    document_id: i64,
    file_name: Option<String>,
    created_at: String,
    request_created_at: String,
    local_path: Option<String>,
}

/// One agency of one portal, with the portal's other releases as its baseline.
#[derive(Debug, Clone, PartialEq, Eq)]
struct AgencyMeasure {
    portal_key: String,
    agency_id: i64,
    agency_name: Option<String>,
    releases: u64,
    late_releases: u64,
    late_dates: u64,
    other_releases: u64,
    other_late_releases: u64,
    /// First and last release date, `None` when no file had a usable time.
    period: Option<(NaiveDate, NaiveDate)>,
    excluded_files: u64,
    portal_excluded_files: u64,
    /// Files of the late releases with their upload time, newest release first.
    late_files: Vec<(usize, DateTime<FixedOffset>)>,
}

impl AgencyMeasure {
    /// Releases with several late releases on one date counted once.
    fn counted_releases(&self) -> u64 {
        self.releases - (self.late_releases - self.late_dates)
    }

    fn qualifies(&self) -> bool {
        if self.counted_releases() < MINIMUM_AGENCY_RELEASES
            || self.other_releases < MINIMUM_BASELINE_RELEASES
            || self.late_dates < MINIMUM_LATE_DATES
        {
            return false;
        }
        let agency = u128::from(self.late_dates) * u128::from(self.other_releases);
        let baseline = u128::from(self.other_late_releases) * u128::from(self.counted_releases());
        let whole = u128::from(self.counted_releases()) * u128::from(self.other_releases);
        agency >= MINIMUM_SHARE_RATIO * baseline
            && agency >= baseline
            && 100 * (agency - baseline) >= MINIMUM_SHARE_DIFFERENCE_POINTS * whole
    }
}

/// Whether a release at this local time falls at or after [`WINDOW_START_HOUR`] on a working day
/// whose next day is a weekend day or court holiday. `None` outside the modeled calendar.
fn in_late_window(at: DateTime<FixedOffset>) -> Option<bool> {
    let date = at.date_naive();
    let closed = |day: NaiveDate| court_day_on_or_after(day).ok().map(|open| open != day);
    let working = !closed(date)?;
    let before_closure = closed(date.succ_opt()?)?;
    Some(working && before_closure && at.hour() >= WINDOW_START_HOUR)
}

#[derive(Default)]
struct Tally<'a> {
    name: Option<&'a str>,
    releases: u64,
    late_dates: BTreeSet<NaiveDate>,
    late_releases: u64,
    period: Option<(NaiveDate, NaiveDate)>,
    excluded_files: u64,
    late_files: Vec<(usize, DateTime<FixedOffset>)>,
}

fn measure(files: &[ListedFile]) -> Vec<AgencyMeasure> {
    type AgencyKey<'a> = (&'a str, Option<i64>);
    let mut tallies: BTreeMap<AgencyKey, Tally> = BTreeMap::new();
    /// Upload times with the index of their file.
    type Uploads = Vec<(DateTime<FixedOffset>, usize)>;
    let mut by_request: BTreeMap<(&str, i64), Uploads> = BTreeMap::new();
    for (index, file) in files.iter().enumerate() {
        let tally = tallies
            .entry((file.portal_key.as_str(), file.agency_id))
            .or_default();
        tally.name = tally.name.or(file.agency_name.as_deref());
        match portal_upload_time(&file.created_at, &file.request_created_at) {
            Some(at) => by_request
                .entry((file.portal_key.as_str(), file.request_id))
                .or_default()
                .push((at, index)),
            None => tally.excluded_files += 1,
        }
    }
    for ((portal, _), mut uploads) in by_request {
        uploads.sort_by_key(|(at, index)| (*at, files[*index].document_id));
        let mut chains: Vec<Uploads> = Vec::new();
        for upload in uploads {
            match chains.last_mut() {
                Some(chain)
                    if chain.last().is_some_and(|previous| {
                        upload.0 - previous.0 < Duration::minutes(RELEASE_GAP_MINUTES)
                    }) =>
                {
                    chain.push(upload);
                }
                _ => chains.push(vec![upload]),
            }
        }
        for chain in chains {
            let (at, first) = chain[0];
            let tally = tallies.entry((portal, files[first].agency_id)).or_default();
            let Some(late) = in_late_window(at) else {
                tally.excluded_files += chain.len() as u64;
                continue;
            };
            let date = at.date_naive();
            tally.releases += 1;
            tally.period = Some(
                tally
                    .period
                    .map_or((date, date), |(from, to)| (from.min(date), to.max(date))),
            );
            if late {
                tally.late_releases += 1;
                tally.late_dates.insert(date);
                tally
                    .late_files
                    .extend(chain.iter().map(|(_, index)| (*index, at)));
            }
        }
    }
    let mut portals: BTreeMap<&str, (u64, u64, u64)> = BTreeMap::new();
    for ((portal, _), tally) in &tallies {
        let totals = portals.entry(portal).or_default();
        totals.0 += tally.releases;
        totals.1 += tally.late_releases;
        totals.2 += tally.excluded_files;
    }
    tallies
        .into_iter()
        .filter_map(|((portal, agency_id), mut tally)| {
            let (releases, late_releases, excluded) = portals.get(portal).copied()?;
            tally
                .late_files
                .sort_by_key(|(index, at)| (std::cmp::Reverse(*at), files[*index].document_id));
            Some(AgencyMeasure {
                portal_key: portal.to_string(),
                agency_id: agency_id?,
                agency_name: tally.name.map(str::to_string),
                releases: tally.releases,
                late_releases: tally.late_releases,
                late_dates: tally.late_dates.len() as u64,
                other_releases: releases - tally.releases,
                other_late_releases: late_releases - tally.late_releases,
                period: tally.period,
                excluded_files: tally.excluded_files,
                portal_excluded_files: excluded,
                late_files: tally.late_files,
            })
        })
        .collect()
}

/// A share to one decimal place, by integer arithmetic.
fn percent(part: u64, whole: u64) -> String {
    let tenths = (u128::from(part) * 1000 + u128::from(whole) / 2) / u128::from(whole.max(1));
    format!("{}.{}%", tenths / 10, tenths % 10)
}

fn plural(count: u64, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

fn span(duration: Duration) -> String {
    if duration.num_days() >= 1 {
        plural(duration.num_days() as u64, "day", "days")
    } else if duration.num_hours() >= 1 {
        plural(duration.num_hours() as u64, "hour", "hours")
    } else {
        plural(duration.num_minutes() as u64, "minute", "minutes")
    }
}

fn internal_modification(file: &ListedFile) -> Option<DateTime<Utc>> {
    let path = Path::new(file.local_path.as_deref()?);
    read_pdf_metadata_signals(path)
        .ok()
        .and_then(|signals| signals.effective_modification())
        .or_else(|| {
            read_ooxml_docprops(path)
                .ok()
                .and_then(|props| props.modified)
        })
}

fn finding(
    measure: &AgencyMeasure,
    files: &[ListedFile],
    read_modified: &dyn Fn(&ListedFile) -> Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Option<Finding> {
    let (first, last) = measure.period?;
    let counted = measure.counted_releases();
    let share = percent(measure.late_dates, counted);
    let other_share = percent(measure.other_late_releases, measure.other_releases);
    let name = measure
        .agency_name
        .clone()
        .unwrap_or_else(|| format!("Agency {}", measure.agency_id));

    let mut aged: Vec<(usize, DateTime<FixedOffset>, FileAge)> = measure
        .late_files
        .iter()
        .take(MAX_FILES_AGED)
        .map(|(index, at)| (*index, *at, file_age(*at, read_modified(&files[*index]))))
        .collect();
    let count = |test: &dyn Fn(&FileAge) -> bool| {
        aged.iter().filter(|(_, _, age)| test(age)).count() as u64
    };
    let under_a_day = count(&|age| matches!(age, FileAge::Before(d) if d.num_days() < 1));
    let under_a_week =
        count(&|age| matches!(age, FileAge::Before(d) if (1..7).contains(&d.num_days())));
    let a_week_or_more = count(&|age| matches!(age, FileAge::Before(d) if d.num_days() >= 7));
    let after_upload = count(&|age| matches!(age, FileAge::AfterUpload(_)));
    let not_recorded = count(&|age| matches!(age, FileAge::NotRecorded));
    let read = if measure.late_files.len() > aged.len() {
        format!(", of which the {} most recent were read", aged.len())
    } else {
        String::new()
    };
    let ages = format!(
        "The late releases hold {}{read}: {under_a_day} last modified less than a day before \
         upload, {under_a_week} one to six days before, {a_week_or_more} seven or more days \
         before; {after_upload} with an internal modification time later than the upload time, \
         which is a contradiction between the file and the portal's clock (a clock or metadata \
         error, or a file rewritten after upload) and not an age; {not_recorded} with no \
         internal modification time recorded or readable.",
        plural(measure.late_files.len() as u64, "file", "files"),
    );
    let excluded = format!(
        "{} of this agency's requests ({} across the portal) {} left out because no upload time \
         with a time of day and a local offset is recorded.",
        plural(measure.excluded_files, "public file", "public files"),
        measure.portal_excluded_files,
        if measure.excluded_files == 1 {
            "was"
        } else {
            "were"
        },
    );
    let explanation = format!(
        "{name}: {late} of {releases} releases from {first} to {last} began at or after \
         {WINDOW_START_HOUR}:00 local time on the last working day before a weekend or California \
         court holiday, on {dates} separate dates. Counting each such date once, that is {dates} \
         of {counted} ({share}), against {other_late} of {other} ({other_share}) for the other \
         releases on the same portal. A release is the public files the portal records as \
         uploaded to one request with less than {RELEASE_GAP_MINUTES} minutes between one file \
         and the next, timed at its first file by that file's own recorded offset. {excluded} \
         {ages} Staff often finish and post work at the end of a week or before a holiday, and \
         requests that fall due then are released then, so a concentration has ordinary \
         explanations. This counts upload times. It does not show why they fall where they do, \
         and the portal does not report when a file uploaded earlier was made public. Holidays \
         are California judicial holidays (Code Civ. Proc. § 135); days proclaimed ad hoc are \
         not modeled.",
        late = measure.late_releases,
        releases = measure.releases,
        dates = measure.late_dates,
        other_late = measure.other_late_releases,
        other = measure.other_releases,
    );

    // Contradictions first, so a bounded sample always shows them.
    aged.sort_by_key(|(_, _, age)| !matches!(age, FileAge::AfterUpload(_)));
    let citations = aged
        .iter()
        .take(MAX_CITATIONS)
        .map(|(index, at, age)| {
            let file = &files[*index];
            let file_name = file
                .file_name
                .clone()
                .unwrap_or_else(|| format!("document {}", file.document_id));
            let age = match age {
                FileAge::Before(duration) if duration.num_days() < 1 => {
                    "last modified less than a day before upload".to_string()
                }
                FileAge::Before(duration) => {
                    format!("last modified {} before upload", span(*duration))
                }
                FileAge::AfterUpload(duration) => format!(
                    "internal modification time is {} later than the upload time (a \
                     contradiction, not an age)",
                    span(*duration)
                ),
                FileAge::NotRecorded => "internal modification time not recorded".to_string(),
            };
            FindingCitation {
                portal_key: file.portal_key.clone(),
                document_id: file.document_id,
                character_start: None,
                character_end: None,
                quote: None,
                label: Some(format!(
                    "{file_name}: uploaded {} (UTC{}); {age}",
                    at.format("%a %Y-%m-%d %H:%M"),
                    at.format("%:z"),
                )),
            }
        })
        .collect();

    let subject = FindingSubject {
        portal_key: Some(measure.portal_key.clone()),
        agency_id: Some(measure.agency_id),
        ..Default::default()
    };
    // Bucketed like `just_in_time_compliance`, so one more release does not mint a new finding.
    let share_bucket = format!(
        "late_share_bucket={}",
        measure.late_dates * 10 / counted.max(1)
    );
    let sample_bucket = format!("sample_bucket={}", measure.releases / 100);
    Some(Finding {
        id: finding_id(
            FindingKind::ReleaseTimingConcentration,
            SLUG,
            &subject,
            &[&share_bucket, &sample_bucket],
        ),
        kind: FindingKind::ReleaseTimingConcentration,
        severity: FindingSeverity::Info,
        // Both shares are restated from counted upload times; nothing is inferred to score.
        confidence: FindingConfidence::Unassessed,
        subject,
        headline: format!(
            "{name}: {share} of releases began late on the last working day before a weekend \
             or holiday, against {other_share} for the rest of the portal"
        ),
        explanation,
        citations,
        discovered_at: now,
        last_seen_at: now,
        dismissed_at: None,
        source_detector: SLUG.to_string(),
    })
}

async fn load_files(repository: &DuckDbRepository) -> Result<Vec<ListedFile>, AppError> {
    repository
        .query_connection("Release-timing file query", move |conn| {
            let mut statement = conn
                .prepare(&format!(
                    "SELECT d.portal_subdomain, d.request_id, r.agency_id, a.name, d.id,
                        d.file_name, d.created_at, r.created_at, d.local_path
                     FROM documents d
                     JOIN requests r
                        ON r.id = d.request_id
                       AND r.portal_subdomain = d.portal_subdomain
                     JOIN portals p ON p.subdomain = d.portal_subdomain
                     LEFT JOIN agencies a
                        ON a.id = r.agency_id
                       AND a.portal_subdomain = r.portal_subdomain
                     WHERE LOWER(TRIM(d.visibility)) = 'public'
                       AND {PORTAL_LISTED_FILE}
                     ORDER BY d.portal_subdomain, d.request_id, d.id
                     LIMIT ?1"
                ))
                .map_err(|error| {
                    AppError::Storage(format!("Failed to prepare release-timing query: {error}"))
                })?;
            let rows = statement
                .query_map(params![MAX_LISTED_FILES as i64 + 1], |row| {
                    Ok(ListedFile {
                        portal_key: row.get(0)?,
                        request_id: row.get(1)?,
                        agency_id: row.get(2)?,
                        agency_name: row.get(3)?,
                        document_id: row.get(4)?,
                        file_name: row.get(5)?,
                        created_at: row.get(6)?,
                        request_created_at: row.get(7)?,
                        local_path: row.get(8)?,
                    })
                })
                .map_err(|error| {
                    AppError::Storage(format!("Failed to run release-timing query: {error}"))
                })?;
            let files = rows.collect::<Result<Vec<_>, _>>().map_err(|error| {
                AppError::Storage(format!("Failed to decode a release-timing row: {error}"))
            })?;
            // A truncated listing would drop the tail of one portal and bias its shares.
            if files.len() > MAX_LISTED_FILES {
                return Err(AppError::Analysis(format!(
                    "Release timing is not measured above {MAX_LISTED_FILES} listed files"
                )));
            }
            Ok(files)
        })
        .await
}

fn evaluate(
    files: &[ListedFile],
    read_modified: &dyn Fn(&ListedFile) -> Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Vec<Finding> {
    measure(files)
        .iter()
        .filter(|measure| measure.qualifies())
        .filter_map(|measure| finding(measure, files, read_modified, now))
        .collect()
}

pub async fn run(repository: &DuckDbRepository) -> Result<Vec<Finding>, AppError> {
    let files = load_files(repository).await?;
    Ok(evaluate(&files, &internal_modification, Utc::now()))
}

/// A request determined on 5 March 2026 and due the next day, with three public PDFs: document 1
/// uploaded on 4 March, document 2 with no download route, document 3 stamped with the request's
/// opening time.
#[cfg(test)]
pub(super) async fn repository_with_one_dated_upload() -> DuckDbRepository {
    let repository = DuckDbRepository::new(":memory:").expect("open");
    repository
        .run_migrations(include_str!("../../../migrations/0001_initial_schema.sql"))
        .await
        .expect("migrate");
    repository
        .query_connection("release basis fixture", |connection| {
            connection
                .execute_batch(
                    "INSERT INTO portals (subdomain, display_name, base_url) VALUES ('sd', 'SD', 'https://sd.example');
                     INSERT INTO requests (id, portal_subdomain, title, status, created_at, updated_at, due_date, determination_at, determination_source)
                       VALUES (1, 'sd', 'Request', 'closed', '2026-02-20T09:00:00-08:00', '2026-03-05T12:00:00-08:00',
                               '2026-03-06T12:00:00-08:00', '2026-03-05T12:00:00-08:00', 'portal');
                     INSERT INTO documents (id, request_id, portal_subdomain, file_name, content_type, download_url, visibility, created_at, updated_at, local_path) VALUES
                       (1, 1, 'sd', 'a.pdf', 'application/pdf', 'https://sd.example/documents/1/download', 'public', '2026-03-04T15:30:00-08:00', '2026-03-04T15:30:00-08:00', '/absent/a.pdf'),
                       (2, 1, 'sd', 'b.pdf', 'application/pdf', NULL, 'public', '2026-03-04T15:30:00-08:00', '2026-03-04T15:30:00-08:00', '/absent/b.pdf'),
                       (3, 1, 'sd', 'c.pdf', 'application/pdf', 'https://sd.example/documents/3/download', 'public', '2026-02-20T09:00:00-08:00', '2026-02-20T09:00:00-08:00', '/absent/c.pdf');",
                )
                .map_err(|error| AppError::Storage(format!("fixture: {error}")))
        })
        .await
        .expect("seed");
    repository
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Days;

    const OPENED: &str = "2019-01-02T09:00:00-08:00";

    struct Upload {
        portal: &'static str,
        agency: i64,
        request: i64,
        at: String,
    }

    fn upload(portal: &'static str, agency: i64, request: i64, at: &str) -> Upload {
        Upload {
            portal,
            agency,
            request,
            at: at.to_string(),
        }
    }

    fn weekly(start: &str, time: &str, count: usize) -> Vec<String> {
        let start = NaiveDate::parse_from_str(start, "%Y-%m-%d").unwrap();
        (0..count as u64)
            .map(|week| format!("{}T{time}", start + Days::new(7 * week)))
            .collect()
    }

    /// `late` releases at 4 p.m. on successive ordinary Fridays and `other` at 10 a.m. on
    /// successive Tuesdays, each in its own request numbered from `first_request`.
    fn agency(
        portal: &'static str,
        agency: i64,
        first_request: i64,
        late: usize,
        other: usize,
    ) -> Vec<Upload> {
        weekly("2024-01-05", "16:00:00-08:00", late)
            .into_iter()
            .chain(weekly("2024-01-02", "10:00:00-08:00", other))
            .enumerate()
            .map(|(index, at)| upload(portal, agency, first_request + index as i64, &at))
            .collect()
    }

    async fn execute(repository: &DuckDbRepository, sql: String) {
        repository
            .query_connection("release timing fixture", move |connection| {
                connection
                    .execute_batch(&sql)
                    .map_err(|error| AppError::Storage(format!("fixture: {error}")))
            })
            .await
            .expect("seed");
    }

    /// Documents are numbered from 1 in the order given.
    async fn repository_with(uploads: &[Upload]) -> DuckDbRepository {
        let repository = DuckDbRepository::new(":memory:").expect("open");
        repository
            .run_migrations(include_str!("../../../migrations/0001_initial_schema.sql"))
            .await
            .expect("migrate");
        let portals: BTreeSet<&str> = uploads.iter().map(|upload| upload.portal).collect();
        let agencies: BTreeSet<(&str, i64)> = uploads
            .iter()
            .map(|upload| (upload.portal, upload.agency))
            .collect();
        let requests: BTreeSet<(&str, i64, i64)> = uploads
            .iter()
            .map(|upload| (upload.portal, upload.request, upload.agency))
            .collect();
        let rows = |rows: Vec<String>| rows.join(", ");
        let mut sql = String::new();
        if !uploads.is_empty() {
            sql = format!(
                "INSERT INTO portals (subdomain, display_name, base_url) VALUES {};
                 INSERT INTO agencies (id, portal_subdomain, name) VALUES {};
                 INSERT INTO requests (id, portal_subdomain, title, status, created_at, updated_at, agency_id) VALUES {};
                 INSERT INTO documents (id, request_id, portal_subdomain, file_name, download_url, visibility, created_at, updated_at) VALUES {};",
                rows(portals
                    .iter()
                    .map(|portal| format!("('{portal}', '{portal}', 'https://{portal}.example')"))
                    .collect()),
                rows(agencies
                    .iter()
                    .map(|(portal, id)| format!("({id}, '{portal}', 'Agency {id}')"))
                    .collect()),
                rows(requests
                    .iter()
                    .map(|(portal, id, agency)| format!(
                        "({id}, '{portal}', 'Request', 'closed', '{OPENED}', '{OPENED}', {agency})"
                    ))
                    .collect()),
                rows(uploads
                    .iter()
                    .enumerate()
                    .map(|(index, upload)| {
                        let id = index + 1;
                        format!(
                            "({id}, {}, '{}', 'file-{id}.pdf', 'https://{}.example/documents/{id}/download', 'public', '{}', '{}')",
                            upload.request, upload.portal, upload.portal, upload.at, upload.at
                        )
                    })
                    .collect()),
            );
        }
        execute(&repository, sql).await;
        repository
    }

    async fn measures(repository: &DuckDbRepository) -> Vec<AgencyMeasure> {
        measure(&load_files(repository).await.expect("load"))
    }

    fn of<'a>(measures: &'a [AgencyMeasure], portal: &str, agency: i64) -> &'a AgencyMeasure {
        measures
            .iter()
            .find(|measure| measure.portal_key == portal && measure.agency_id == agency)
            .expect("agency measured")
    }

    async fn portals_reported(uploads: &[Upload]) -> Vec<String> {
        let repository = repository_with(uploads).await;
        let mut portals: Vec<String> = run(&repository)
            .await
            .expect("run")
            .into_iter()
            .filter_map(|finding| finding.subject.portal_key)
            .collect();
        portals.sort();
        portals
    }

    #[tokio::test]
    async fn an_empty_corpus_yields_nothing() {
        let repository = repository_with(&[]).await;
        assert!(run(&repository).await.expect("run").is_empty());
    }

    #[tokio::test]
    async fn the_agency_and_the_baseline_each_need_their_minimum_of_releases() {
        let mut uploads = Vec::new();
        for (portal, agency_releases, baseline) in [
            ("agency19", 19, 40),
            ("agency20", 20, 40),
            ("agency21", 21, 40),
            ("base39", 20, 39),
            ("base41", 20, 41),
        ] {
            uploads.extend(agency(portal, 1, 1, 5, agency_releases - 5));
            uploads.extend(agency(portal, 2, 1000, 0, baseline));
        }
        assert_eq!(
            portals_reported(&uploads).await,
            ["agency20", "agency21", "base41"]
        );
    }

    #[tokio::test]
    async fn the_share_must_be_twice_the_baseline_and_ten_points_above_it() {
        let mut uploads = Vec::new();
        // Against a baseline of 0 of 100 the ratio always holds: 4, 5 and 6 of 50 are 8, 10 and
        // 12 points above it.
        for (portal, late) in [("points08", 4), ("points10", 5), ("points12", 6)] {
            uploads.extend(agency(portal, 1, 1, late, 50 - late));
            uploads.extend(agency(portal, 2, 1000, 0, 100));
        }
        // Against 20 of 100 (20%): 19, 20 and 21 of 50 are 38%, 40% and 42%, all more than ten
        // points above, and only the last two are at least twice the baseline.
        for (portal, late) in [("ratio19", 19), ("ratio20", 20), ("ratio21", 21)] {
            uploads.extend(agency(portal, 1, 1, late, 50 - late));
            uploads.extend(agency(portal, 2, 1000, 20, 80));
        }
        assert_eq!(
            portals_reported(&uploads).await,
            ["points10", "points12", "ratio20", "ratio21"]
        );
    }

    #[tokio::test]
    async fn late_means_three_oclock_onward_on_the_last_working_day_before_a_closed_day() {
        let cases = [
            // Wednesday before Lincoln Day, a court holiday on Thursday 12 February 2026.
            ("2026-02-11T15:30:00-08:00", true),
            // An ordinary Friday: the morning, one second before the window, and its first second.
            ("2026-03-13T09:00:00-07:00", false),
            ("2026-03-13T14:59:59-07:00", false),
            ("2026-03-13T15:00:00-07:00", true),
            // An ordinary Thursday afternoon.
            ("2026-03-12T16:00:00-07:00", false),
            // Juneteenth 2026 is a Friday: Thursday is the last working day, Friday is closed.
            ("2026-06-18T16:00:00-07:00", true),
            ("2026-06-19T16:00:00-07:00", false),
            // The day is the stamp's own: Friday evening here is Saturday in UTC, and Saturday
            // 02:00 at +09:00 is Friday 17:00 in UTC.
            ("2026-03-13T23:30:00-07:00", true),
            ("2026-03-14T02:00:00+09:00", false),
        ];
        let uploads: Vec<Upload> = cases
            .iter()
            .enumerate()
            .map(|(index, (at, _))| upload("sd", index as i64 + 1, index as i64 + 1, at))
            .collect();
        let measures = measures(&repository_with(&uploads).await).await;
        for (index, (at, late)) in cases.iter().enumerate() {
            let measure = of(&measures, "sd", index as i64 + 1);
            assert_eq!(measure.releases, 1, "{at}");
            assert_eq!(measure.late_releases, u64::from(*late), "{at}");
        }
    }

    #[tokio::test]
    async fn files_under_an_hour_apart_in_one_request_are_one_release() {
        let mut uploads: Vec<Upload> = (0..50)
            .map(|second| upload("sd", 1, 1, &format!("2024-01-05T16:00:{second:02}-08:00")))
            .collect();
        // 59:59 after the previous file joins it; 60:00 and 61:00 each start a release.
        for at in ["10:00:00", "10:59:59", "11:59:59", "13:00:59"] {
            uploads.push(upload("sd", 1, 2, &format!("2024-01-02T{at}-08:00")));
        }
        let measures = measures(&repository_with(&uploads).await).await;
        let measure = of(&measures, "sd", 1);
        assert_eq!(measure.releases, 4);
        assert_eq!(measure.late_releases, 1);
        assert_eq!(measure.late_files.len(), 50);
    }

    #[tokio::test]
    async fn a_time_that_is_not_the_portals_upload_time_of_day_is_excluded_and_counted() {
        let uploads: Vec<Upload> = [
            "2024-01-05T16:00:00-08:00",
            "2024-01-05",
            "2024-01-05T16:00:00",
            "2024-01-05T16:00:00Z",
            "2024-01-05T00:00:00-08:00",
            OPENED,
            "",
        ]
        .iter()
        .enumerate()
        .map(|(index, at)| upload("sd", 1, index as i64 + 1, at))
        .collect();
        let repository = repository_with(&uploads).await;
        // A closure notice (no download route) and a file this application fetched from a
        // crawled site carry times that are not the portal's upload time: neither is listed.
        execute(
            &repository,
            format!(
                "INSERT INTO documents (id, request_id, portal_subdomain, file_name, visibility, created_at, updated_at)
                   VALUES (90, 1, 'sd', 'closure-notice.txt', 'public', '2024-01-12T16:00:00-08:00', '2024-01-12T16:00:00-08:00');
                 INSERT INTO portals (subdomain, display_name, base_url, source_kind)
                   VALUES ('site', 'Site', 'https://site.example', 'WEB_CRAWL');
                 INSERT INTO agencies (id, portal_subdomain, name) VALUES (1, 'site', 'Agency 1');
                 INSERT INTO requests (id, portal_subdomain, title, status, created_at, updated_at, agency_id)
                   VALUES (1, 'site', 'Page', 'closed', '{OPENED}', '{OPENED}', 1);
                 INSERT INTO documents (id, request_id, portal_subdomain, file_name, download_url, visibility, created_at, updated_at)
                   VALUES (91, 1, 'site', 'page.pdf', 'https://site.example/page.pdf', 'public', '2024-01-12T16:00:00-08:00', '2024-01-12T16:00:00-08:00');"
            ),
        )
        .await;
        let measures = measures(&repository).await;
        assert_eq!(measures.len(), 1);
        let measure = of(&measures, "sd", 1);
        assert_eq!(measure.releases, 1);
        assert_eq!(measure.late_releases, 1);
        assert_eq!(measure.excluded_files, 6);
        assert_eq!(measure.portal_excluded_files, 6);
    }

    #[tokio::test]
    async fn one_agency_id_on_two_portals_is_two_agencies_each_against_the_others_releases() {
        let mut uploads = agency("north", 7, 1, 5, 15);
        uploads.extend(agency("north", 2, 1000, 0, 40));
        uploads.extend(agency("south", 7, 1, 0, 20));
        uploads.extend(agency("south", 2, 1000, 0, 40));
        let repository = repository_with(&uploads).await;
        let measures = measures(&repository).await;
        let north = of(&measures, "north", 7);
        assert_eq!((north.releases, north.late_releases), (20, 5));
        assert_eq!((north.other_releases, north.other_late_releases), (40, 0));
        let south = of(&measures, "south", 7);
        assert_eq!((south.releases, south.late_releases), (20, 0));
        let findings = run(&repository).await.expect("run");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].subject.portal_key.as_deref(), Some("north"));
        assert_eq!(findings[0].subject.agency_id, Some(7));
    }

    #[tokio::test]
    async fn late_releases_on_one_date_count_once_and_three_dates_are_needed() {
        let mut uploads = Vec::new();
        // Thirty requests released in one Friday sitting, and nineteen on other days.
        uploads.extend(
            (0..30).map(|request| upload("batch", 1, request, "2024-01-05T16:00:00-08:00")),
        );
        uploads.extend(agency("batch", 1, 100, 0, 19));
        uploads.extend(agency("batch", 2, 1000, 0, 40));
        // Two late dates of 20 is ten points above a baseline of none, and still not reported.
        uploads.extend(agency("dates2", 1, 1, 2, 18));
        uploads.extend(agency("dates2", 2, 1000, 0, 40));
        uploads.extend(agency("dates3", 1, 1, 3, 17));
        uploads.extend(agency("dates3", 2, 1000, 0, 40));
        let repository = repository_with(&uploads).await;
        let measures = measures(&repository).await;
        let batch = of(&measures, "batch", 1);
        assert_eq!(
            (batch.releases, batch.late_releases, batch.late_dates),
            (49, 30, 1)
        );
        assert_eq!(batch.counted_releases(), 20);
        let portals: Vec<_> = run(&repository)
            .await
            .expect("run")
            .into_iter()
            .filter_map(|finding| finding.subject.portal_key)
            .collect();
        assert_eq!(portals, ["dates3"]);
    }

    #[tokio::test]
    async fn a_modification_time_after_upload_is_reported_as_a_contradiction_not_an_age() {
        let mut uploads = agency("sd", 1, 1, 5, 15);
        uploads.extend(agency("sd", 2, 1000, 0, 40));
        let repository = repository_with(&uploads).await;
        let files = load_files(&repository).await.expect("load");
        // Documents 1 to 5 are the agency's late files.
        let read = |file: &ListedFile| {
            let uploaded = DateTime::parse_from_rfc3339(&file.created_at)
                .unwrap()
                .with_timezone(&Utc);
            match file.document_id {
                1 => Some(uploaded + Duration::hours(3)),
                2 => Some(uploaded - Duration::days(10)),
                3 => Some(uploaded - Duration::hours(2)),
                _ => None,
            }
        };
        let findings = evaluate(&files, &read, Utc::now());
        assert_eq!(findings.len(), 1);
        let finding = &findings[0];
        assert_eq!(finding.kind, FindingKind::ReleaseTimingConcentration);
        assert!(!finding.kind.is_legal_claim());
        assert_eq!(finding.confidence, FindingConfidence::Unassessed);
        for stated in [
            "5 of 20 releases from 2024-01-02 to 2024-04-09",
            "5 of 20 (25.0%), against 0 of 40 (0.0%)",
            "0 public files of this agency's requests (0 across the portal)",
            "1 last modified less than a day before upload, 0 one to six days before, 1 seven or more days before",
            "1 with an internal modification time later than the upload time, which is a contradiction",
            "2 with no internal modification time recorded",
            "ordinary explanations",
        ] {
            assert!(finding.explanation.contains(stated), "{stated}: {}", finding.explanation);
        }
        let shown = format!("{} {}", finding.headline, finding.explanation).to_lowercase();
        for word in ["buried", "dump", "held back", "intent", "violat", "avoid"] {
            assert!(!shown.contains(word), "{word}");
        }
        assert_eq!(finding.citations.len(), 5);
        assert_eq!(finding.citations[0].document_id, 1);
        let contradiction = finding.citations[0].label.as_deref().unwrap();
        assert!(contradiction.contains("3 hours later than the upload time"));
        assert!(!contradiction.contains("before upload"));
        let labels: Vec<&str> = finding
            .citations
            .iter()
            .filter_map(|citation| citation.label.as_deref())
            .collect();
        assert!(labels
            .iter()
            .any(|label| label.contains("last modified 10 days before upload")));
        assert!(labels.iter().any(|label| label.contains("not recorded")));
        assert!(labels
            .iter()
            .all(|label| label.contains("16:00 (UTC-08:00)")));
    }
}
