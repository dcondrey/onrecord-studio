//! California's rule for a statutory period whose last day falls on a holiday.
//!
//! Code Civ. Proc. § 12 excludes the first day and includes the last; § 12a(a) carries a last day
//! that is a "holiday" forward to the next day that is not one, and § 12a(b) applies that to every
//! period "provided or required by law", which *DeLeon v. BART* (1983) 33 Cal.3d 456 applied to a
//! Government Code deadline. "Holiday" there means every Saturday, every Sunday, and every
//! judicial holiday under § 135: the full-day holidays of Gov. Code § 6700 other than Lunar New
//! Year, Genocide Remembrance Day, Diwali, Admission Day and Columbus Day, plus the day after
//! Thanksgiving, with the Gov. Code § 6701 observed days.
//!
//! LENIENCY(statute): every uncertainty resolves toward a later deadline, because a later deadline
//! can only withhold a violation, never fabricate one. Hence the current § 135 list applies to
//! every modeled year (some days were added recently), and a fixed-date holiday falling on a
//! Saturday also closes the preceding Friday. Days proclaimed ad hoc by the President or Governor
//! are not modeled; that is stated on every finding through [`COURT_CALENDAR_BASIS`].

use crate::error::AppError;
use chrono::{Datelike, Days, NaiveDate, Weekday};

pub const COURT_CALENDAR_BASIS: &str = "a last day on a Saturday, Sunday or judicial holiday \
     moves to the next court day (Code Civ. Proc. §§ 12, 12a, 135); days proclaimed ad hoc are not modeled";

/// Years the calendar is modeled for. A date outside them has no answer rather than a guess, in
/// both the Rust evaluator and the rendered SQL.
pub const FIRST_MODELED_YEAR: i32 = 1990;
pub const LAST_MODELED_YEAR: i32 = 2099;

/// The longest run of consecutive closed days: Thanksgiving through Sunday.
pub const MAXIMUM_CONSECUTIVE_CLOSED_DAYS: u64 = 4;

const FIXED_DATE_HOLIDAYS: [(u32, u32); 7] = [
    (1, 1),
    (2, 12),
    (3, 31),
    (6, 19),
    (7, 4),
    (11, 11),
    (12, 25),
];

fn nth_weekday(year: i32, month: u32, weekday: Weekday, n: u8) -> Option<NaiveDate> {
    NaiveDate::from_weekday_of_month_opt(year, month, weekday, n)
}

fn last_weekday(year: i32, month: u32, weekday: Weekday) -> Option<NaiveDate> {
    (1..=5)
        .rev()
        .find_map(|n| NaiveDate::from_weekday_of_month_opt(year, month, weekday, n))
}

/// Weekday closures attributable to `year`'s holidays, including an observed Friday that falls
/// on 31 December of the year before.
fn holidays_of_year(year: i32) -> Vec<NaiveDate> {
    let mut days = Vec::with_capacity(20);
    for (month, day) in FIXED_DATE_HOLIDAYS {
        let Some(date) = NaiveDate::from_ymd_opt(year, month, day) else {
            continue;
        };
        days.push(date);
        match date.weekday() {
            Weekday::Sun => days.extend(date.checked_add_days(Days::new(1))),
            Weekday::Sat => days.extend(date.checked_sub_days(Days::new(1))),
            _ => {}
        }
    }
    days.extend(nth_weekday(year, 1, Weekday::Mon, 3));
    days.extend(nth_weekday(year, 2, Weekday::Mon, 3));
    days.extend(last_weekday(year, 5, Weekday::Mon));
    days.extend(nth_weekday(year, 9, Weekday::Mon, 1));
    days.extend(nth_weekday(year, 9, Weekday::Fri, 4));
    if let Some(thanksgiving) = nth_weekday(year, 11, Weekday::Thu, 4) {
        days.push(thanksgiving);
        days.extend(thanksgiving.checked_add_days(Days::new(1)));
    }
    days
}

fn is_modeled(date: NaiveDate) -> bool {
    (FIRST_MODELED_YEAR..=LAST_MODELED_YEAR).contains(&date.year())
}

/// Every holiday of the modeled span and the two years after it, built once. A date's holidays
/// are those of its own year and the next (a January 1 on Saturday is observed the Friday
/// before), and a deadline can roll a few days past the last modeled year.
fn modeled_holidays() -> &'static std::collections::HashSet<NaiveDate> {
    static HOLIDAYS: std::sync::OnceLock<std::collections::HashSet<NaiveDate>> =
        std::sync::OnceLock::new();
    HOLIDAYS.get_or_init(|| {
        (FIRST_MODELED_YEAR..=LAST_MODELED_YEAR + 2)
            .flat_map(holidays_of_year)
            .collect()
    })
}

fn is_closed(date: NaiveDate) -> bool {
    matches!(date.weekday(), Weekday::Sat | Weekday::Sun) || modeled_holidays().contains(&date)
}

/// The day a period whose nominal last day is `date` actually ends.
pub fn court_day_on_or_after(date: NaiveDate) -> Result<NaiveDate, AppError> {
    if !is_modeled(date) {
        return Err(AppError::Compliance(format!(
            "No California court calendar is modeled for {date}; supported years are \
             {FIRST_MODELED_YEAR} to {LAST_MODELED_YEAR}"
        )));
    }
    let mut day = date;
    for _ in 0..=MAXIMUM_CONSECUTIVE_CLOSED_DAYS {
        if !is_closed(day) {
            return Ok(day);
        }
        day = day
            .succ_opt()
            .ok_or_else(|| AppError::Compliance("Court calendar overflowed".to_string()))?;
    }
    Err(AppError::Internal(format!(
        "Court calendar found more than {MAXIMUM_CONSECUTIVE_CLOSED_DAYS} closed days after {date}"
    )))
}

/// Every weekday holiday in the modeled span, for the SQL rendering. Weekends are tested there by
/// day of week, so they are not listed.
pub fn modeled_weekday_holidays() -> Vec<NaiveDate> {
    let mut days = (FIRST_MODELED_YEAR..=LAST_MODELED_YEAR + 1)
        .flat_map(holidays_of_year)
        .filter(|date| is_modeled(*date) && !matches!(date.weekday(), Weekday::Sat | Weekday::Sun))
        .collect::<Vec<_>>();
    days.sort_unstable();
    days.dedup();
    days
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(value: &str) -> NaiveDate {
        NaiveDate::parse_from_str(value, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn closed_days_carry_the_deadline_to_the_next_court_day() {
        for (nominal, expected) in [
            // An open Wednesday stays put.
            ("2026-03-11", "2026-03-11"),
            // Saturday and Sunday.
            ("2026-01-10", "2026-01-12"),
            ("2026-01-11", "2026-01-12"),
            // Thanksgiving through Sunday: the longest closure.
            ("2026-11-26", "2026-11-30"),
            // Lincoln Day on a Thursday.
            ("2026-02-12", "2026-02-13"),
            // Christmas on a Sunday is observed Monday.
            ("2022-12-25", "2022-12-27"),
            // New Year's Day on a Saturday closes the Friday before, across the year boundary.
            ("2021-12-31", "2022-01-03"),
            // Admission Day and Columbus Day are not judicial holidays.
            ("2026-09-09", "2026-09-09"),
            ("2026-10-12", "2026-10-12"),
        ] {
            assert_eq!(
                court_day_on_or_after(date(nominal)).unwrap(),
                date(expected),
                "{nominal}"
            );
        }
        assert!(court_day_on_or_after(date("2100-01-04")).is_err());
    }

    #[test]
    fn the_memoized_calendar_closes_exactly_the_days_the_rule_does() {
        let mut day = date("1990-01-01");
        while day <= date("2100-12-31") {
            let rule = matches!(day.weekday(), Weekday::Sat | Weekday::Sun)
                || holidays_of_year(day.year()).contains(&day)
                || holidays_of_year(day.year() + 1).contains(&day);
            assert_eq!(is_closed(day), rule, "{day}");
            day = day.succ_opt().unwrap();
        }
    }

    #[test]
    fn rendered_holiday_list_agrees_with_the_rule_on_every_modeled_day() {
        let listed = modeled_weekday_holidays();
        let mut day = date("1990-01-01");
        // The final days of the span roll into an unmodeled year, so they stop short of it.
        while day < date("2099-12-25") {
            let weekend = matches!(day.weekday(), Weekday::Sat | Weekday::Sun);
            assert_eq!(
                is_closed(day),
                weekend || listed.binary_search(&day).is_ok(),
                "{day}"
            );
            assert!(court_day_on_or_after(day).is_ok(), "{day}");
            day = day.succ_opt().unwrap();
        }
    }
}
