//! Descriptive facility coverage over a single 24-hour day. Only rows whose hours were recorded
//! and interpretable contribute to the open-minute arithmetic; blank or unreadable hours are
//! counted separately and never read as open or closed.

use crate::types::{
    FacilityAreaCoverage, FacilityCoverageFacility, FacilityHoursStatus, FacilityOpenInterval,
};

pub const MINUTES_PER_DAY: u32 = 1440;
pub const MAX_LISTED_FACILITIES: usize = 500;

/// Parses a day-independent daily schedule. Accepted forms: `closed`, `24 hours`, `24/7`, and
/// comma- or semicolon-separated ranges whose endpoints are `HH:MM` (24-hour), `h[:mm]am|pm`,
/// `noon`, or `midnight`. A range ending at `00:00`, `24:00`, `12am`, or `midnight` ends at the
/// end of the day; a range whose end precedes its start wraps past midnight. Anything else,
/// including weekday-qualified schedules and a range whose start equals its end, is
/// uninterpretable rather than guessed.
pub fn parse_daily_hours(text: Option<&str>) -> (FacilityHoursStatus, Vec<FacilityOpenInterval>) {
    let Some(text) = text.map(str::trim).filter(|t| !t.is_empty()) else {
        return (FacilityHoursStatus::NotRecorded, Vec::new());
    };
    let lower = text.to_lowercase();
    let compact: String = lower.chars().filter(|c| !c.is_whitespace()).collect();
    if compact == "closed" {
        return (FacilityHoursStatus::Recorded, Vec::new());
    }
    if matches!(
        compact.as_str(),
        "24hours" | "24hrs" | "24/7" | "open24hours"
    ) {
        return (
            FacilityHoursStatus::Recorded,
            vec![FacilityOpenInterval {
                start_minute: 0,
                end_minute: MINUTES_PER_DAY,
            }],
        );
    }
    let mut intervals = Vec::new();
    for segment in compact.split([',', ';']) {
        match parse_range(segment) {
            Some(mut parsed) => intervals.append(&mut parsed),
            None => return (FacilityHoursStatus::Uninterpretable, Vec::new()),
        }
    }
    (FacilityHoursStatus::Recorded, intervals)
}

fn parse_range(segment: &str) -> Option<Vec<FacilityOpenInterval>> {
    let normalized = segment.replace(['\u{2013}', '\u{2014}'], "-");
    let (start, end) = normalized
        .split_once('-')
        .or_else(|| normalized.split_once("to"))?;
    let start = parse_time(start)?;
    let end = parse_time(end)?;
    if start == end || start >= MINUTES_PER_DAY {
        return None;
    }
    let end = if end == 0 { MINUTES_PER_DAY } else { end };
    Some(if start < end {
        vec![FacilityOpenInterval {
            start_minute: start,
            end_minute: end,
        }]
    } else {
        vec![
            FacilityOpenInterval {
                start_minute: start,
                end_minute: MINUTES_PER_DAY,
            },
            FacilityOpenInterval {
                start_minute: 0,
                end_minute: end,
            },
        ]
    })
}

/// Returns minutes since midnight, 0..=1440. `24:00` is 1440; `midnight` and `12am` are 0.
fn parse_time(value: &str) -> Option<u32> {
    let value = value.replace('.', "");
    match value.as_str() {
        "noon" => return Some(720),
        "midnight" => return Some(0),
        _ => {}
    }
    let (clock, meridiem) = if let Some(rest) = value.strip_suffix("am") {
        (rest, Some(false))
    } else if let Some(rest) = value.strip_suffix("pm") {
        (rest, Some(true))
    } else {
        (value.as_str(), None)
    };
    let (hour, minute) = match clock.split_once(':') {
        Some((h, m)) => (h, Some(m)),
        None => (clock, None),
    };
    let has_minutes = minute.is_some();
    let digits = |s: &str, max_len: usize| {
        (!s.is_empty() && s.len() <= max_len && s.bytes().all(|b| b.is_ascii_digit()))
            .then(|| s.parse::<u32>().ok())
            .flatten()
    };
    let hour = digits(hour, 2)?;
    let minute = match minute {
        Some(m) if m.len() == 2 => digits(m, 2)?,
        Some(_) => return None,
        None => 0,
    };
    if minute > 59 {
        return None;
    }
    match meridiem {
        Some(pm) => {
            if !(1..=12).contains(&hour) {
                return None;
            }
            Some(((hour % 12) + if pm { 12 } else { 0 }) * 60 + minute)
        }
        // A bare hour such as "8-5" has no stated clock convention.
        None if has_minutes => match hour {
            0..=23 => Some(hour * 60 + minute),
            24 if minute == 0 => Some(MINUTES_PER_DAY),
            _ => None,
        },
        None => None,
    }
}

/// Summarizes the facilities already selected into one area.
pub fn summarize_area(
    area_name: String,
    boundary: String,
    facilities: Vec<FacilityCoverageFacility>,
    unmatched_facility_ids: Vec<String>,
) -> FacilityAreaCoverage {
    let mut open_counts = vec![0u32; MINUTES_PER_DAY as usize];
    let (mut recorded, mut uninterpretable, mut not_recorded) = (0u64, 0u64, 0u64);
    for facility in &facilities {
        match facility.hours_status {
            FacilityHoursStatus::Recorded => recorded += 1,
            FacilityHoursStatus::Uninterpretable => uninterpretable += 1,
            FacilityHoursStatus::NotRecorded => not_recorded += 1,
        }
        let mut open = vec![false; MINUTES_PER_DAY as usize];
        for interval in &facility.open_intervals {
            for slot in &mut open[interval.start_minute as usize..interval.end_minute as usize] {
                *slot = true;
            }
        }
        for (count, is_open) in open_counts.iter_mut().zip(open) {
            *count += u32::from(is_open);
        }
    }
    let covered = open_counts.iter().filter(|count| **count > 0).count() as u32;
    let measured = recorded > 0;
    let total = facilities.len();
    let facilities_omitted = total.saturating_sub(MAX_LISTED_FACILITIES) as u64;
    FacilityAreaCoverage {
        area_name,
        boundary,
        public_restrooms: total as u64,
        with_recorded_hours: recorded,
        hours_uninterpretable: uninterpretable,
        hours_not_recorded: not_recorded,
        open_minutes_covered: measured.then_some(covered),
        minutes_with_none_open: measured.then_some(MINUTES_PER_DAY - covered),
        unmatched_facility_ids,
        excluded_facility_ids: Vec::new(),
        facilities: facilities.into_iter().take(MAX_LISTED_FACILITIES).collect(),
        facilities_omitted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minutes(text: &str) -> Vec<(u32, u32)> {
        let (status, intervals) = parse_daily_hours(Some(text));
        assert_eq!(status, FacilityHoursStatus::Recorded, "{text}");
        intervals
            .into_iter()
            .map(|i| (i.start_minute, i.end_minute))
            .collect()
    }

    fn status(text: Option<&str>) -> FacilityHoursStatus {
        parse_daily_hours(text).0
    }

    #[test]
    fn midnight_endings_and_wraps_stay_inside_one_day() {
        assert_eq!(minutes("22:00-24:00"), vec![(1320, 1440)]);
        assert_eq!(minutes("22:00-00:00"), vec![(1320, 1440)]);
        assert_eq!(minutes("10pm-12am"), vec![(1320, 1440)]);
        assert_eq!(minutes("20:00-02:00"), vec![(1200, 1440), (0, 120)]);
        assert_eq!(minutes("06:00-23:59"), vec![(360, 1439)]);
        assert_eq!(minutes("12am-12pm"), vec![(0, 720)]);
        assert_eq!(
            minutes("6:30 am - 9 pm; noon-midnight"),
            vec![(390, 1260), (720, 1440)]
        );
        assert_eq!(minutes("24/7"), vec![(0, 1440)]);
        assert_eq!(minutes("Closed"), vec![]);
    }

    #[test]
    fn unreadable_and_blank_hours_are_distinct_from_closed() {
        assert_eq!(status(None), FacilityHoursStatus::NotRecorded);
        assert_eq!(status(Some("  ")), FacilityHoursStatus::NotRecorded);
        for text in [
            "00:00-00:00",
            "08:00-08:00",
            "8-5",
            "dawn to dusk",
            "Mon-Fri 8am-5pm",
            "varies",
            "24:00-06:00",
            "13pm-2pm",
            "08:60-09:00",
        ] {
            assert_eq!(
                status(Some(text)),
                FacilityHoursStatus::Uninterpretable,
                "{text}"
            );
        }
    }

    fn facility(hours: Option<&str>) -> FacilityCoverageFacility {
        let (hours_status, open_intervals) = parse_daily_hours(hours);
        FacilityCoverageFacility {
            table_row: 0,
            facility_id: None,
            name: None,
            address: None,
            hours_text: hours.map(String::from),
            hours_status,
            open_intervals,
        }
    }

    #[test]
    fn coverage_counts_only_recorded_hours_and_absence_is_not_zero() {
        let area = summarize_area(
            "Downtown".into(),
            String::new(),
            vec![
                facility(Some("06:00-22:00")),
                facility(Some("20:00-02:00")),
                facility(Some("Closed")),
                facility(Some("dawn to dusk")),
                facility(None),
            ],
            vec![],
        );
        assert_eq!(area.public_restrooms, 5);
        assert_eq!(area.with_recorded_hours, 3);
        assert_eq!(area.hours_uninterpretable, 1);
        assert_eq!(area.hours_not_recorded, 1);
        // Open 00:00-02:00 and 06:00-24:00; none open 02:00-06:00.
        assert_eq!(area.open_minutes_covered, Some(1200));
        assert_eq!(area.minutes_with_none_open, Some(240));

        let unrecorded = summarize_area(
            "Unknown".into(),
            String::new(),
            vec![facility(None), facility(Some("sunrise-sunset"))],
            vec![],
        );
        assert_eq!(unrecorded.public_restrooms, 2);
        assert_eq!(unrecorded.open_minutes_covered, None);
        assert_eq!(unrecorded.minutes_with_none_open, None);

        let empty = summarize_area("Empty".into(), String::new(), vec![], vec![]);
        assert_eq!(empty.public_restrooms, 0);
        assert_eq!(empty.minutes_with_none_open, None);
    }
}
