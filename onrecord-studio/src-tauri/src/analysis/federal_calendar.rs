//! The federal working-day calendar: what "excepting Saturdays, Sundays, and legal public
//! holidays" in 5 U.S.C. § 552(a)(6) excludes.
//!
//! Legal public holidays are those of 5 U.S.C. § 6103(a). A holiday on a Saturday is observed the
//! Friday before (§ 6103(b)(1)); one on a Sunday is observed the Monday after (Executive Order
//! 11582 § 3(a), 1971). Inauguration Day (§ 6103(c)), a holiday only for employees in the
//! Washington area, is included.
//!
//! LENIENCY(statute): as in `court_calendar`, every uncertainty resolves toward a later deadline.
//! Juneteenth (added 2021) is applied to every modeled year, Inauguration Day to every agency, and
//! a holiday falling on a weekend closes both the weekend day and its observed weekday. Closures
//! ordered ad hoc by the President are not modeled; that is stated on every finding through
//! [`FEDERAL_CALENDAR_BASIS`].

use crate::analysis::court_calendar::{FIRST_MODELED_YEAR, LAST_MODELED_YEAR};
use chrono::{Datelike, Days, NaiveDate, Weekday};

pub const FEDERAL_CALENDAR_BASIS: &str = "working days exclude Saturdays, Sundays and the legal \
     public holidays of 5 U.S.C. § 6103 with their observed days; closures ordered ad hoc are not modeled";

const FIXED_DATE_HOLIDAYS: [(u32, u32); 5] = [(1, 1), (6, 19), (7, 4), (11, 11), (12, 25)];

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
    let mut days = Vec::with_capacity(16);
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
    days.extend(nth_weekday(year, 10, Weekday::Mon, 2));
    days.extend(nth_weekday(year, 11, Weekday::Thu, 4));
    // § 6103(c): 20 January following a presidential election, or the 21st when the 20th is a
    // Sunday.
    if year.rem_euclid(4) == 1 {
        if let Some(inauguration) = NaiveDate::from_ymd_opt(year, 1, 20) {
            days.push(if inauguration.weekday() == Weekday::Sun {
                inauguration + Days::new(1)
            } else {
                inauguration
            });
        }
    }
    days
}

pub(crate) fn is_closed(date: NaiveDate) -> bool {
    matches!(date.weekday(), Weekday::Sat | Weekday::Sun)
        || holidays_of_year(date.year()).contains(&date)
        || holidays_of_year(date.year() + 1).contains(&date)
}

/// Every weekday holiday in the modeled span, for the SQL rendering.
pub fn modeled_weekday_holidays() -> Vec<NaiveDate> {
    let mut days = (FIRST_MODELED_YEAR..=LAST_MODELED_YEAR + 1)
        .flat_map(holidays_of_year)
        .filter(|date| {
            (FIRST_MODELED_YEAR..=LAST_MODELED_YEAR).contains(&date.year())
                && !matches!(date.weekday(), Weekday::Sat | Weekday::Sun)
        })
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
    fn federal_closures_follow_section_6103_and_its_observed_days() {
        for (day, closed) in [
            // Columbus Day is a federal holiday and not a California judicial one.
            ("2026-10-12", true),
            // Cesar Chavez Day and the day after Thanksgiving are California closures only.
            ("2026-03-31", false),
            ("2026-11-27", false),
            ("2026-11-26", true),
            // Independence Day 2026 is a Saturday, observed Friday 3 July.
            ("2026-07-03", true),
            // Juneteenth 2022 was a Sunday, observed Monday 20 June.
            ("2022-06-20", true),
            // New Year's Day 2022 was a Saturday: Friday 31 December 2021 is closed.
            ("2021-12-31", true),
            // Inauguration Day 2025 coincided with Martin Luther King Jr. Day.
            ("2025-01-20", true),
            ("2029-01-22", false),
            ("2026-03-11", false),
        ] {
            assert_eq!(is_closed(date(day)), closed, "{day}");
        }
    }

    #[test]
    fn rendered_holiday_list_agrees_with_the_rule_on_every_modeled_day() {
        let listed = modeled_weekday_holidays();
        let mut day = date("1990-01-01");
        while day < date("2099-12-25") {
            let weekend = matches!(day.weekday(), Weekday::Sat | Weekday::Sun);
            assert_eq!(
                is_closed(day),
                weekend || listed.binary_search(&day).is_ok(),
                "{day}"
            );
            day = day.succ_opt().unwrap();
        }
    }
}
