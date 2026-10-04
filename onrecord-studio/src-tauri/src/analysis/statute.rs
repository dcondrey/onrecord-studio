//! Declarative statutory determination clocks, one rule per jurisdiction, and the single model
//! both evaluation paths render from.
//!
//! Two evaluation paths consume these rules and neither of them owns them:
//!
//! * [`crate::analysis::timeline`] evaluates one request in Rust. Its
//!   [`StatutoryComplianceAnalysis`](crate::analysis::StatutoryComplianceAnalysis) is what the
//!   legal-brief export and the repository's timeline-audit refresh are built from.
//! * [`crate::analysis::anomaly`] evaluates every request at once inside DuckDB to produce the
//!   per-agency aggregates.
//!
//! The aggregate path cannot call the Rust evaluator row by row without giving up vectorized
//! execution, so it is *generated* from [`CLOCK_RULES`] and the calendars instead: the rules become
//! a `VALUES` table, the holiday calendars a day table with running working-day ordinals, and the
//! SQL fragments below read both. `analysis::anomaly`'s parity test drives both renderings from one
//! table of cases so they cannot diverge silently.
//!
//! A portal names its jurisdiction (`portals.statutory_jurisdiction`). A request whose portal
//! names none is not evaluated by either path: no rule is assumed, CPRA included.
//!
//! # Column contract for the SQL fragments
//!
//! A query that renders [`sql::render`] must open with
//! `WITH statutory_span(first_day, last_day) AS (...), {statutory_ctes},`, where the span is the
//! earliest and latest recorded-offset receipt date in scope, and expose these columns in scope of
//! the fragments:
//!
//! | column | meaning |
//! |---|---|
//! | `created_timestamp` | receipt instant |
//! | `created_local_timestamp` | receipt wall-clock time in its recorded offset, from [`sql::recorded_wall_clock`] |
//! | `updated_timestamp` / `closed_timestamp` / `release_timestamp` | activity instants, nullable |
//! | `created_date` / `updated_date` / `closed_date` / `release_date` | the *recorded-offset* calendar dates |
//! | `determination_timestamp` / `explicit_determination_date` | sourced determination, nullable |
//! | `determination_source` | notice/source identifier required for the explicit date |
//! | `extension_notice_timestamp` | extension notice instant, nullable |
//! | `extension_notice_date` / `extension_expected_date` | extension calendar dates |
//! | `extension_reason` / `extension_notice_source` | extension provenance |
//! | `rule_jurisdiction`, `rule_calendar`, `start_days`, `start_working`, `initial_days`, `initial_working`, `extension_days`, `extension_working`, `extension_grounds` | the joined `clock_rules` row, NULL when the portal names no jurisdiction |
//! | `determination_date` | output of [`sql::determination_calendar_date`] |
//! | `clock_start_date` | output of [`sql::clock_start_date`] |
//! | `initial_deadline_date` | output of [`sql::initial_deadline_date`] |
//! | `has_valid_extension` | output of [`sql::valid_extension_predicate`] |
//! | `allowed_calendar_days` | output of [`sql::allowed_calendar_days`] |
//! | `normalized_status` / `latency_seconds` | lifecycle aggregates |

use crate::analysis::{court_calendar, federal_calendar};
use crate::error::AppError;
use crate::types::StatutoryExtensionReason;
use chrono::{Datelike, Days, NaiveDate};
use serde::{Deserialize, Serialize};

/// The statutory regime a portal's requests are evaluated under.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum StatutoryJurisdiction {
    #[serde(rename = "CA_CPRA")]
    CaliforniaCpra,
    #[serde(rename = "US_FOIA")]
    FederalFoia,
}

impl StatutoryJurisdiction {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CaliforniaCpra => "CA_CPRA",
            Self::FederalFoia => "US_FOIA",
        }
    }

    /// Reads a stored value. `NONE`, NULL and anything unrecognized name no rule.
    pub fn from_stored(value: Option<&str>) -> Option<Self> {
        match value.map(str::trim) {
            Some("CA_CPRA") => Some(Self::CaliforniaCpra),
            Some("US_FOIA") => Some(Self::FederalFoia),
            _ => None,
        }
    }

    pub fn rule(self) -> &'static StatutoryClockRule {
        match self {
            Self::CaliforniaCpra => &CPRA_DETERMINATION_RULE,
            Self::FederalFoia => &FOIA_DETERMINATION_RULE,
        }
    }
}

/// Stored when the operator states that no modeled statute governs a source. It differs from
/// NULL, which means nobody has said, only in that startup never infers a rule over it.
pub const NO_STATUTORY_JURISDICTION: &str = "NONE";

/// How a period's days are counted.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DayKind {
    /// Every day counts; a last day on a closed day carries to the next open day.
    CalendarCarriedPastClosedDays,
    /// Only open days count, starting with the first open day after the period's start.
    WorkingDays,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Period {
    pub days: u32,
    pub kind: DayKind,
}

/// Which days are closed for a rule's counting.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum HolidayCalendar {
    /// Saturdays, Sundays and California judicial holidays (`court_calendar`).
    CaliforniaJudicial,
    /// Saturdays, Sundays and federal legal public holidays (`federal_calendar`).
    FederalLegalPublic,
}

impl HolidayCalendar {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CaliforniaJudicial => "CALIFORNIA_JUDICIAL",
            Self::FederalLegalPublic => "FEDERAL_LEGAL_PUBLIC",
        }
    }

    pub const fn basis(self) -> &'static str {
        match self {
            Self::CaliforniaJudicial => court_calendar::COURT_CALENDAR_BASIS,
            Self::FederalLegalPublic => federal_calendar::FEDERAL_CALENDAR_BASIS,
        }
    }

    fn is_closed(self, date: NaiveDate) -> bool {
        match self {
            Self::CaliforniaJudicial => court_calendar::is_closed(date),
            Self::FederalLegalPublic => federal_calendar::is_closed(date),
        }
    }

    pub fn weekday_holidays(self) -> Vec<NaiveDate> {
        match self {
            Self::CaliforniaJudicial => court_calendar::modeled_weekday_holidays(),
            Self::FederalLegalPublic => federal_calendar::modeled_weekday_holidays(),
        }
    }

    fn require_modeled(self, date: NaiveDate) -> Result<(), AppError> {
        if (court_calendar::FIRST_MODELED_YEAR..=court_calendar::LAST_MODELED_YEAR)
            .contains(&date.year())
        {
            Ok(())
        } else {
            Err(AppError::Compliance(format!(
                "No {} calendar is modeled for {date}; supported years are {} to {}",
                self.as_str(),
                court_calendar::FIRST_MODELED_YEAR,
                court_calendar::LAST_MODELED_YEAR
            )))
        }
    }

    /// The first open day on or after `date`.
    pub fn open_day_on_or_after(self, date: NaiveDate) -> Result<NaiveDate, AppError> {
        self.require_modeled(date)?;
        let mut day = date;
        while self.is_closed(day) {
            day = day
                .succ_opt()
                .ok_or_else(|| AppError::Compliance("Calendar overflowed".to_string()))?;
            self.require_modeled(day)?;
        }
        Ok(day)
    }

    /// The day a period of `period` starting on `start` (excluded) ends.
    pub fn period_end(self, start: NaiveDate, period: Period) -> Result<NaiveDate, AppError> {
        match period.kind {
            DayKind::CalendarCarriedPastClosedDays => self.open_day_on_or_after(
                start
                    .checked_add_days(Days::new(u64::from(period.days)))
                    .ok_or_else(|| AppError::Compliance("Statutory period overflowed".into()))?,
            ),
            DayKind::WorkingDays => {
                self.require_modeled(start)?;
                let mut day = start;
                let mut counted = 0;
                while counted < period.days {
                    day = day
                        .succ_opt()
                        .ok_or_else(|| AppError::Compliance("Calendar overflowed".to_string()))?;
                    self.require_modeled(day)?;
                    if !self.is_closed(day) {
                        counted += 1;
                    }
                }
                Ok(day)
            }
        }
    }
}

/// A statute section and where its current official text is published.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Citation {
    pub section: &'static str,
    pub url: &'static str,
}

/// When the clock starts relative to the recorded receipt.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ClockStart {
    pub event: &'static str,
    /// The latest the statute lets the start trail the recorded receipt. The record never shows
    /// the later event, so the clock is started at this latest point, which can only withhold a
    /// violation.
    pub latest_after_receipt: Option<Period>,
    pub citation: Citation,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionRule {
    /// The longest extension, counted from the initial deadline.
    pub maximum: Period,
    pub grounds: &'static [StatutoryExtensionReason],
    pub citation: Citation,
}

/// One jurisdiction's determination clock. Every field the evaluators read is here, and every
/// field carries the section it comes from.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StatutoryClockRule {
    pub id: &'static str,
    pub jurisdiction: StatutoryJurisdiction,
    pub title: &'static str,
    /// The act as a requester's correspondence names it.
    pub act_name: &'static str,
    /// The `legal_statutes` row a stored finding references.
    pub statute_id: &'static str,
    pub start: ClockStart,
    pub stop_event: &'static str,
    pub initial: Period,
    pub initial_citation: Citation,
    pub calendar: HolidayCalendar,
    pub calendar_citation: Citation,
    pub extension: ExtensionRule,
    /// Events the statute lets stop the clock that no source here records. A finding under such a
    /// rule cannot exclude a stopped period, which costs it confidence.
    pub unrecorded_clock_stops: Option<Citation>,
    pub notes: &'static [&'static str],
}

const CPRA_URL: &str =
    "https://leginfo.legislature.ca.gov/faces/codes_displaySection.xhtml?lawCode=GOV&sectionNum=7922.535.";
const FOIA_URL: &str = "https://www.govinfo.gov/content/pkg/USCODE-2023-title5/html/USCODE-2023-title5-partI-chap5-subchapII-sec552.htm";

/// California Public Records Act determination clock.
pub const CPRA_DETERMINATION_RULE: StatutoryClockRule = StatutoryClockRule {
    id: "CA_CPRA_DETERMINATION",
    jurisdiction: StatutoryJurisdiction::CaliforniaCpra,
    title: "California Public Records Act determination deadline",
    act_name: "the California Public Records Act",
    statute_id: "CPRA_7922.535",
    start: ClockStart {
        event: "receipt of the request",
        latest_after_receipt: None,
        citation: Citation {
            section: "Cal. Gov. Code § 7922.535(a)",
            url: CPRA_URL,
        },
    },
    stop_event: "the agency's determination whether the request seeks disclosable records",
    initial: Period {
        days: 10,
        kind: DayKind::CalendarCarriedPastClosedDays,
    },
    initial_citation: Citation {
        section: "California Government Code § 7922.535",
        url: CPRA_URL,
    },
    calendar: HolidayCalendar::CaliforniaJudicial,
    calendar_citation: Citation {
        section: "Cal. Code Civ. Proc. §§ 12, 12a, 135",
        url: "https://leginfo.legislature.ca.gov/faces/codes_displaySection.xhtml?lawCode=CCP&sectionNum=12a.",
    },
    extension: ExtensionRule {
        maximum: Period {
            days: 14,
            kind: DayKind::CalendarCarriedPastClosedDays,
        },
        grounds: &[
            StatutoryExtensionReason::SeparateFieldFacilities,
            StatutoryExtensionReason::VoluminousRecords,
            StatutoryExtensionReason::InteragencyConsultation,
            StatutoryExtensionReason::DataCompilation,
            StatutoryExtensionReason::CyberattackSystemsUnavailable,
            StatutoryExtensionReason::DeclaredEmergencyOperationalImpact,
        ],
        citation: Citation {
            section: "Cal. Gov. Code § 7922.535(b)-(c)",
            url: CPRA_URL,
        },
    },
    unrecorded_clock_stops: None,
    notes: &[
        "Recodified without substantive change from former Gov. Code § 6253(c) (Stats. 2021, ch. 614, AB 473, operative 2023-01-01); the same 10 and 14 days apply to earlier requests.",
        "The cyberattack ground was added by Stats. 2025, ch. 34 (AB 370), effective 2026-01-01. Every listed ground is accepted for every request, the reading that grants the agency the most time.",
    ],
};

/// Federal Freedom of Information Act determination clock.
pub const FOIA_DETERMINATION_RULE: StatutoryClockRule = StatutoryClockRule {
    id: "US_FOIA_DETERMINATION",
    jurisdiction: StatutoryJurisdiction::FederalFoia,
    title: "Federal FOIA determination deadline",
    act_name: "the Freedom of Information Act, 5 U.S.C. § 552",
    statute_id: "FOIA_552_A_6",
    start: ClockStart {
        event: "receipt by the appropriate component, at the latest ten working days after first receipt by a designated component",
        latest_after_receipt: Some(Period {
            days: 10,
            kind: DayKind::WorkingDays,
        }),
        citation: Citation {
            section: "5 U.S.C. § 552(a)(6)(A) (flush text after clause (ii))",
            url: FOIA_URL,
        },
    },
    stop_event: "the agency's determination whether to comply with the request",
    initial: Period {
        days: 20,
        kind: DayKind::WorkingDays,
    },
    initial_citation: Citation {
        section: "5 U.S.C. § 552(a)(6)(A)(i)",
        url: FOIA_URL,
    },
    calendar: HolidayCalendar::FederalLegalPublic,
    calendar_citation: Citation {
        section: "5 U.S.C. § 6103; Exec. Order 11582 § 3(a)",
        url: "https://www.govinfo.gov/content/pkg/USCODE-2023-title5/html/USCODE-2023-title5-partIII-subpartE-chap61-subchapI-sec6103.htm",
    },
    extension: ExtensionRule {
        maximum: Period {
            days: 10,
            kind: DayKind::WorkingDays,
        },
        grounds: &[
            StatutoryExtensionReason::SeparateFieldFacilities,
            StatutoryExtensionReason::VoluminousRecords,
            StatutoryExtensionReason::InteragencyConsultation,
        ],
        citation: Citation {
            section: "5 U.S.C. § 552(a)(6)(B)(i), (iii)",
            url: FOIA_URL,
        },
    },
    unrecorded_clock_stops: Some(Citation {
        section: "5 U.S.C. § 552(a)(6)(A)(ii)(I)-(II) (tolling for requested information or fee clarification)",
        url: FOIA_URL,
    }),
    notes: &[
        "The record shows first receipt only, so the clock starts ten working days later, the latest start the statute allows (DOJ OIP reads the routing allowance as working days).",
    ],
};

impl DayKind {
    pub const fn unit(self) -> &'static str {
        match self {
            Self::CalendarCarriedPastClosedDays => "calendar days",
            Self::WorkingDays => "working days",
        }
    }
}

impl StatutoryClockRule {
    /// The rule in one paragraph, with its citations, for correspondence and reports.
    pub fn statement(&self) -> String {
        let start = match self.start.latest_after_receipt {
            Some(allowance) => format!(
                "{} ({}; taken here as {} {} after first receipt, the latest the statute allows)",
                self.start.event,
                self.start.citation.section,
                allowance.days,
                allowance.kind.unit()
            ),
            None => self.start.event.to_string(),
        };
        let stops = self
            .unrecorded_clock_stops
            .map(|citation| {
                format!(
                    " The clock may also stop under {}, which this record does not show.",
                    citation.section
                )
            })
            .unwrap_or_default();
        format!(
            "{} requires {} within {} {} of {}. A written notice citing a ground listed in {} may extend the deadline by at most {} {}. Days are counted under {}: {}.{stops} Official text: {}",
            self.initial_citation.section,
            self.stop_event,
            self.initial.days,
            self.initial.kind.unit(),
            start,
            self.extension.citation.section,
            self.extension.maximum.days,
            self.extension.maximum.kind.unit(),
            self.calendar_citation.section,
            self.calendar.basis(),
            self.initial_citation.url,
        )
    }
}

/// Every modeled rule, in the order the SQL renders them.
pub const CLOCK_RULES: [&StatutoryClockRule; 2] =
    [&CPRA_DETERMINATION_RULE, &FOIA_DETERMINATION_RULE];

/// Calendar days an agency has to determine whether a request seeks disclosable records.
pub const CPRA_INITIAL_DETERMINATION_DAYS: u64 = CPRA_DETERMINATION_RULE.initial.days as u64;
/// Calendar days a single written extension may add to the initial determination period.
pub const CPRA_MAXIMUM_EXTENSION_DAYS: u64 = CPRA_DETERMINATION_RULE.extension.maximum.days as u64;
/// Citation recorded on CPRA-specific drafts.
pub const CPRA_STATUTE_CITATION: &str = CPRA_DETERMINATION_RULE.initial_citation.section;
/// Bumped whenever a rule changes what a stored statutory finding says; startup then recomputes
/// every stored finding. 2: last days carried past weekends and judicial holidays. 3: rules per
/// jurisdiction, no rule without a portal jurisdiction, release events in the proxy, activity at
/// the receipt instant no longer counted as activity.
pub const STATUTORY_RULE_VERSION: i64 = 3;
/// A closure this soon after receipt is recorded as a lifecycle anomaly, not a statutory finding.
pub const SUSPICIOUS_CLOSURE_THRESHOLD_SECONDS: i64 = 300;

/// What a compliance evaluation actually measured the elapsed period against.
///
/// The statute runs to the agency's *determination*, which is a distinct, noticed event. Sources
/// may carry that event and its provenance explicitly. When absent, both evaluation paths report
/// which substitute they used, and no consumer may present a proxy as a measured event.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DeterminationBasis {
    /// MEASURED. The source carried a distinct determination timestamp and a non-empty notice or
    /// source identifier, both retained with the request: a confirmed agency notice, or the
    /// portal timeline's first closure when no release preceded it.
    SourcedDeterminationEvent,
    /// PROXY. The earliest recorded activity strictly after receipt (`updated_at`, `closed_at`,
    /// or the first release event) stood in for the determination. Activity is an upper bound on
    /// when the agency acted, not evidence that the act was a determination: a release proves a
    /// determination came no later, and an unrelated touch may predate the real one.
    EarliestRecordedActivityProxy,
    /// PROVISIONAL. Nothing was recorded after receipt, so the elapsed period was measured to the
    /// observation instant. The period is still running; a violation reported on this basis
    /// states only that the deadline has passed with no recorded response.
    ObservationWithoutRecordedActivity,
}

impl DeterminationBasis {
    /// Whether the evaluation measured to something the agency actually did.
    ///
    /// False means the elapsed period was measured to "now" and will keep growing.
    pub const fn is_recorded_activity(self) -> bool {
        matches!(
            self,
            Self::SourcedDeterminationEvent | Self::EarliestRecordedActivityProxy
        )
    }

    pub const fn is_sourced_determination(self) -> bool {
        matches!(self, Self::SourcedDeterminationEvent)
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SourcedDeterminationEvent => "SOURCED_DETERMINATION_EVENT",
            Self::EarliestRecordedActivityProxy => "EARLIEST_RECORDED_ACTIVITY_PROXY",
            Self::ObservationWithoutRecordedActivity => "OBSERVATION_WITHOUT_RECORDED_ACTIVITY",
        }
    }

    pub fn from_stored(value: &str) -> Option<Self> {
        match value {
            "SOURCED_DETERMINATION_EVENT" => Some(Self::SourcedDeterminationEvent),
            "EARLIEST_RECORDED_ACTIVITY_PROXY" => Some(Self::EarliestRecordedActivityProxy),
            "OBSERVATION_WITHOUT_RECORDED_ACTIVITY" => {
                Some(Self::ObservationWithoutRecordedActivity)
            }
            _ => None,
        }
    }
}

/// Every extension reason any rule recognizes, for parsing stored free text. CPRA's list is the
/// widest; `every_rule_grounds_are_parseable` holds the others to it.
pub const RECOGNIZED_EXTENSION_REASONS: &[StatutoryExtensionReason] =
    CPRA_DETERMINATION_RULE.extension.grounds;

/// SQL renderings of the rules above, for the vectorized aggregate path.
///
/// Every fragment is deliberately expressed over *calendar dates* rather than instants. The
/// statute counts days, and a day is a day in the offset the agency recorded, so the fragments
/// derive dates from the RFC 3339 date prefix rather than converting to a session time zone
/// first. That matches [`chrono::DateTime::date_naive`] on the Rust side and, unlike a
/// `TIMESTAMPTZ` conversion, produces the same answer on every machine.
pub mod sql {
    use super::{DayKind, HolidayCalendar, CLOCK_RULES, SUSPICIOUS_CLOSURE_THRESHOLD_SECONDS};
    use crate::analysis::court_calendar::{FIRST_MODELED_YEAR, LAST_MODELED_YEAR};

    /// Observation date used when nothing was recorded after receipt, read in the receipt's
    /// recorded offset like every other date the clock compares.
    ///
    /// `CURRENT_TIMESTAMP` cast to `TIMESTAMP` is the session's civil time, the same frame as
    /// `created_timestamp`; shifting it by the receipt's offset from that frame gives "now" as the
    /// agency recorded time. Without the shift, an evening observation on a request's last legal
    /// day is already the next day in UTC and reports a violation the agency has not committed.
    const OBSERVATION_CALENDAR_DATE: &str = "CAST(
        CAST(CURRENT_TIMESTAMP AS TIMESTAMP)
            + COALESCE(created_local_timestamp - created_timestamp, INTERVAL 0 SECOND)
        AS DATE)";

    const CALENDAR_DATE_OPEN: &str = "{calendar_date(";
    const RECORDED_WALL_CLOCK_OPEN: &str = "{recorded_wall_clock(";
    const CALENDAR_DATE_CLOSE: &str = ")}";

    /// Days past the latest receipt in scope the calendar extends. Every rule's furthest date
    /// (start allowance, initial period and extension) falls well inside it;
    /// `every_rule_reaches_inside_the_sql_calendar_horizon` holds it there.
    pub const CALENDAR_HORIZON_DAYS: u32 = 366;

    const CALENDARS: [HolidayCalendar; 2] = [
        HolidayCalendar::CaliforniaJudicial,
        HolidayCalendar::FederalLegalPublic,
    ];

    /// The calendar date of an RFC 3339 timestamp string, in the offset it was recorded with.
    ///
    /// Reading the date prefix is what makes this offset-preserving. `CAST(TRY_CAST(x AS
    /// TIMESTAMPTZ) AS TIMESTAMP)` would first shift the instant into the session time zone, so
    /// `2026-01-01T23:00:00-08:00` would land on 2026-01-02 under a UTC session and on 2026-01-01
    /// under a Pacific one. The statute's clock does not move with the reader's machine.
    ///
    /// `TRY_CAST` keeps malformed input NULL, which every consuming predicate already excludes.
    pub fn calendar_date(timestamp_expression: &str) -> String {
        format!("TRY_CAST(substr({timestamp_expression}, 1, 10) AS DATE)")
    }

    /// The wall-clock time of an RFC 3339 timestamp string in the offset it was recorded with,
    /// offset-preserving for the reason given on [`calendar_date`].
    pub fn recorded_wall_clock(timestamp_expression: &str) -> String {
        format!("TRY_CAST(substr({timestamp_expression}, 1, 19) AS TIMESTAMP)")
    }

    /// `statutory_holidays`, `statutory_days` and `clock_rules`, rendered from the calendars and
    /// [`CLOCK_RULES`] so each has one definition. The calendar spans the query's own receipts,
    /// read from a `statutory_span(first_day, last_day)` CTE the query defines first, plus
    /// [`CALENDAR_HORIZON_DAYS`], within the modeled years: a full 1990-2099 calendar cost the
    /// aggregate about a second per run.
    ///
    /// `statutory_days` carries, per calendar, every modeled day, whether it is open, the running
    /// count of open days through it, and the first open day on or after it. The n-th working day
    /// after `d` is then the open day whose ordinal is `ordinal(d) + n`, which holds whether or not
    /// `d` itself is open. Every lookup is an equality on `(calendar, day)` or
    /// `(calendar, open_ordinal)`, so DuckDB joins rather than scanning the calendar per request;
    /// a range predicate here made the aggregate quadratic.
    pub fn statutory_ctes() -> String {
        let holidays = CALENDARS
            .iter()
            .flat_map(|calendar| {
                calendar
                    .weekday_holidays()
                    .into_iter()
                    .map(move |day| format!("('{}', DATE '{day}')", calendar.as_str()))
            })
            .collect::<Vec<_>>()
            .join(", ");
        let calendars = CALENDARS
            .iter()
            .map(|calendar| format!("('{}')", calendar.as_str()))
            .collect::<Vec<_>>()
            .join(", ");
        let rules = CLOCK_RULES
            .iter()
            .map(|rule| {
                let (start_days, start_working) = match rule.start.latest_after_receipt {
                    Some(period) => (
                        period.days.to_string(),
                        (period.kind == DayKind::WorkingDays).to_string(),
                    ),
                    None => ("NULL".to_string(), "FALSE".to_string()),
                };
                let grounds = rule
                    .extension
                    .grounds
                    .iter()
                    .map(|ground| format!("'{}'", ground.as_str()))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "('{}', '{}', CAST({start_days} AS INTEGER), {start_working}, {}, {}, {}, {}, [{grounds}]::VARCHAR[])",
                    rule.jurisdiction.as_str(),
                    rule.calendar.as_str(),
                    rule.initial.days,
                    rule.initial.kind == DayKind::WorkingDays,
                    rule.extension.maximum.days,
                    rule.extension.maximum.kind == DayKind::WorkingDays,
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "statutory_holidays(calendar, day) AS (VALUES {holidays}),
statutory_days AS MATERIALIZED (
    SELECT calendar, day, is_open,
           SUM(CASE WHEN is_open THEN 1 ELSE 0 END)
               OVER (PARTITION BY calendar ORDER BY day) AS open_ordinal,
           MIN(CASE WHEN is_open THEN day END)
               OVER (PARTITION BY calendar ORDER BY day DESC
                     ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW) AS next_open_day
    FROM (
        SELECT calendars.calendar, CAST(days.day AS DATE) AS day,
               dayofweek(days.day) NOT IN (0, 6)
               AND NOT EXISTS (
                   SELECT 1 FROM statutory_holidays
                   WHERE statutory_holidays.calendar = calendars.calendar
                     AND statutory_holidays.day = CAST(days.day AS DATE)
               ) AS is_open
        FROM (VALUES {calendars}) AS calendars(calendar)
        CROSS JOIN (
            SELECT CAST(unnest(generate_series(
                GREATEST(first_day, DATE '{FIRST_MODELED_YEAR}-01-01'),
                LEAST(last_day + INTERVAL {CALENDAR_HORIZON_DAYS} DAY, DATE '{}-01-01')
                    - INTERVAL 1 DAY,
                INTERVAL 1 DAY
            )) AS DATE) AS day
            FROM statutory_span
        ) AS days
    ) AS marked
),
statutory_open_days AS MATERIALIZED (
    SELECT calendar, open_ordinal, day FROM statutory_days WHERE is_open
),
clock_rules(rule_jurisdiction, rule_calendar, start_days, start_working, initial_days,
            initial_working, extension_days, extension_working, extension_grounds)
    AS (VALUES {rules})",
            LAST_MODELED_YEAR + 1
        )
    }

    /// The day a period of `days` (counted as working days when `working`) starting on `start`
    /// ends, under the row's calendar; mirrors `HolidayCalendar::period_end`. NULL outside the
    /// modeled years, as the Rust side returns an error there.
    fn period_end(start: &str, days: &str, working: &str) -> String {
        format!(
            "(CASE WHEN {working} THEN (
                SELECT ends.day FROM statutory_open_days AS ends
                WHERE ends.calendar = rule_calendar
                  AND ends.open_ordinal = (
                      SELECT starts.open_ordinal FROM statutory_days AS starts
                      WHERE starts.calendar = rule_calendar AND starts.day = ({start})
                  ) + {days}
            ) ELSE {} END)",
            open_day_on_or_after(&format!("({start}) + CAST({days} AS INTEGER)"))
        )
    }

    /// The first open day on or after `date_expression` under the row's calendar.
    fn open_day_on_or_after(date_expression: &str) -> String {
        format!(
            "(SELECT opens.next_open_day FROM statutory_days AS opens
              WHERE opens.calendar = rule_calendar AND opens.day = ({date_expression}))"
        )
    }

    /// Calendar date of a sourced determination, or otherwise the earliest recorded activity
    /// strictly after receipt, mirroring `timeline`.
    ///
    /// The *instants* pick the winner and the winner's own date is returned, so the choice is made
    /// on real ordering while the arithmetic stays on calendar dates. Ties go to the first listed,
    /// as `Iterator::min` does on the Rust side.
    pub fn determination_calendar_date() -> String {
        let after =
            |column: &str| format!("(CASE WHEN {column} > created_timestamp THEN {column} END)");
        let (updated, closed, release) = (
            after("updated_timestamp"),
            after("closed_timestamp"),
            after("release_timestamp"),
        );
        let earliest = format!("least({updated}, {closed}, {release})");
        format!(
            "CASE
            WHEN determination_timestamp >= created_timestamp
             AND TRIM(COALESCE(determination_source, '')) <> ''
                THEN explicit_determination_date
            WHEN {earliest} IS NULL THEN NULL
            WHEN {updated} = {earliest} THEN updated_date
            WHEN {closed} = {earliest} THEN closed_date
            ELSE release_date
        END"
        )
    }

    /// The day the clock starts, mirroring `timeline`.
    pub fn clock_start_date() -> String {
        format!(
            "CASE WHEN start_days IS NULL THEN created_date ELSE {} END",
            period_end("created_date", "start_days", "start_working")
        )
    }

    /// Last day of the initial period, mirroring `timeline`.
    pub fn initial_deadline_date() -> String {
        period_end("clock_start_date", "initial_days", "initial_working")
    }

    /// Whether a recorded extension actually moves the deadline, mirroring
    /// `timeline::evaluate_extension`.
    ///
    /// The Rust evaluator distinguishes *why* an extension is defective because a single request's
    /// report names the defect. The aggregate only needs the deadline, and every defective status
    /// leaves the initial deadline in place, so this renders as one predicate.
    pub fn valid_extension_predicate() -> String {
        format!(
            "CASE
            WHEN extension_notice_timestamp >= created_timestamp
             AND extension_notice_date <= initial_deadline_date
             AND extension_expected_date > initial_deadline_date
             AND extension_expected_date <= {}
             AND TRIM(COALESCE(extension_notice_source, '')) <> ''
             AND list_contains(extension_grounds, extension_reason)
            THEN TRUE ELSE FALSE
        END",
            period_end(
                "initial_deadline_date",
                "extension_days",
                "extension_working"
            )
        )
    }

    /// Days the agency was entitled to, given the extension verdict.
    pub fn allowed_calendar_days() -> String {
        format!(
            "CASE
            WHEN has_valid_extension
            THEN date_diff('day', created_date, {})
            ELSE date_diff('day', created_date, initial_deadline_date)
        END",
            open_day_on_or_after("extension_expected_date")
        )
    }

    /// Whether the row was evaluated at all: its portal names a modeled jurisdiction and its dates
    /// fall where the calendar is modeled.
    pub fn statutory_evaluated_predicate() -> String {
        "rule_jurisdiction IS NOT NULL
          AND created_timestamp IS NOT NULL
          AND created_date IS NOT NULL
          AND allowed_calendar_days IS NOT NULL"
            .to_string()
    }

    /// Whether the elapsed period exceeded the entitlement, mirroring `timeline`'s `is_violation`.
    ///
    /// A row with no rule, or no parseable receipt date, is excluded rather than counted either
    /// way: the model has nothing to measure, and asserting compliance would be as wrong as
    /// asserting a violation.
    pub fn statutory_violation_predicate() -> String {
        format!(
            "{}
          AND date_diff(
                'day',
                created_date,
                COALESCE(determination_date, {OBSERVATION_CALENDAR_DATE})
              ) > allowed_calendar_days",
            statutory_evaluated_predicate()
        )
    }

    /// Lifecycle anomaly, not a statutory finding: a closure too fast to represent real review.
    pub fn suspicious_fast_closure_predicate() -> String {
        format!(
            "normalized_status = 'closed'
          AND latency_seconds BETWEEN 0.0 AND {SUSPICIOUS_CLOSURE_THRESHOLD_SECONDS}.0"
        )
    }

    /// Substitutes every statutory placeholder in a query template.
    ///
    /// Templates keep DuckDB's own `{db}` table qualifier untouched; the caller applies that
    /// afterwards.
    pub fn render(template: &str) -> String {
        let rendered = substitute(
            &substitute(template, CALENDAR_DATE_OPEN, calendar_date),
            RECORDED_WALL_CLOCK_OPEN,
            recorded_wall_clock,
        );
        rendered
            .replace("{statutory_ctes}", &statutory_ctes())
            .replace("{clock_start_date}", &clock_start_date())
            .replace("{initial_deadline_date}", &initial_deadline_date())
            .replace(
                "{determination_calendar_date}",
                &determination_calendar_date(),
            )
            .replace("{valid_extension_predicate}", &valid_extension_predicate())
            .replace("{allowed_calendar_days}", &allowed_calendar_days())
            .replace(
                "{statutory_evaluated_predicate}",
                &statutory_evaluated_predicate(),
            )
            .replace(
                "{statutory_violation_predicate}",
                &statutory_violation_predicate(),
            )
            .replace(
                "{suspicious_fast_closure_predicate}",
                &suspicious_fast_closure_predicate(),
            )
    }

    /// Expands every `{<function>(<expression>)}` placeholder that opens with `open`.
    fn substitute(template: &str, open: &str, expand: fn(&str) -> String) -> String {
        let mut rendered = String::with_capacity(template.len());
        let mut remainder = template;
        while let Some(start) = remainder.find(open) {
            let (before, rest) = remainder.split_at(start);
            rendered.push_str(before);
            let rest = &rest[open.len()..];
            let Some(end) = rest.find(CALENDAR_DATE_CLOSE) else {
                // An unterminated placeholder is a template bug, not user input. Emit it verbatim
                // so DuckDB reports the offending text instead of silently dropping a column.
                rendered.push_str(open);
                remainder = rest;
                continue;
            };
            rendered.push_str(&expand(rest[..end].trim()));
            remainder = &rest[end + CALENDAR_DATE_CLOSE.len()..];
        }
        rendered.push_str(remainder);
        rendered
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(value: &str) -> NaiveDate {
        NaiveDate::parse_from_str(value, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn calendar_date_placeholders_expand_and_survive_malformed_templates() {
        assert_eq!(
            sql::render("SELECT {calendar_date(requests.created_at)} AS created_date"),
            "SELECT TRY_CAST(substr(requests.created_at, 1, 10) AS DATE) AS created_date"
        );
        // An unterminated placeholder must reach DuckDB intact rather than vanish.
        assert!(sql::render("SELECT {calendar_date(oops").contains("{calendar_date("));
    }

    #[test]
    fn only_the_recorded_activity_basis_measures_a_completed_period() {
        assert!(DeterminationBasis::EarliestRecordedActivityProxy.is_recorded_activity());
        assert!(!DeterminationBasis::ObservationWithoutRecordedActivity.is_recorded_activity());
    }

    #[test]
    fn every_rule_cites_a_section_and_an_official_url_for_every_part() {
        for rule in CLOCK_RULES {
            let mut citations = vec![
                rule.start.citation,
                rule.initial_citation,
                rule.calendar_citation,
                rule.extension.citation,
            ];
            citations.extend(rule.unrecorded_clock_stops);
            for citation in citations {
                assert!(citation.section.contains('§'), "{}: {citation:?}", rule.id);
                assert!(citation.url.starts_with("https://"), "{}", rule.id);
            }
            assert_eq!(rule.jurisdiction.rule().id, rule.id);
        }
    }

    #[test]
    fn every_rule_reaches_inside_the_sql_calendar_horizon() {
        // Thanksgiving week and year end hold the longest runs of closed days.
        for receipt in ["2025-11-20", "2025-12-19"] {
            let receipt = date(receipt);
            for rule in CLOCK_RULES {
                let start = match rule.start.latest_after_receipt {
                    Some(allowance) => rule.calendar.period_end(receipt, allowance).unwrap(),
                    None => receipt,
                };
                let initial = rule.calendar.period_end(start, rule.initial).unwrap();
                let extended = rule
                    .calendar
                    .period_end(initial, rule.extension.maximum)
                    .unwrap();
                let reach = rule.calendar.open_day_on_or_after(extended).unwrap();
                assert!(
                    (reach - receipt).num_days() < i64::from(sql::CALENDAR_HORIZON_DAYS) / 2,
                    "{}",
                    rule.id
                );
            }
        }
    }

    #[test]
    fn every_rule_grounds_are_parseable() {
        for rule in CLOCK_RULES {
            for ground in rule.extension.grounds {
                assert!(RECOGNIZED_EXTENSION_REASONS.contains(ground), "{}", rule.id);
            }
        }
    }

    #[test]
    fn foia_working_days_skip_weekends_and_federal_holidays_only() {
        let federal = HolidayCalendar::FederalLegalPublic;
        let working = |days| Period {
            days,
            kind: DayKind::WorkingDays,
        };
        // Friday 2 October 2026: one working day later is Monday the 5th.
        assert_eq!(
            federal.period_end(date("2026-10-02"), working(1)).unwrap(),
            date("2026-10-05")
        );
        // From Friday 9 October, Columbus Day (Monday the 12th) is skipped.
        assert_eq!(
            federal.period_end(date("2026-10-09"), working(1)).unwrap(),
            date("2026-10-13")
        );
        // Cesar Chavez Day (31 March) is a California closure, not a federal one.
        assert_eq!(
            federal.period_end(date("2026-03-30"), working(1)).unwrap(),
            date("2026-03-31")
        );
        assert_eq!(
            HolidayCalendar::CaliforniaJudicial
                .open_day_on_or_after(date("2026-03-31"))
                .unwrap(),
            date("2026-04-01")
        );
        // A start on a closed day counts from the next open day.
        assert_eq!(
            federal.period_end(date("2026-10-10"), working(1)).unwrap(),
            date("2026-10-13")
        );
        assert!(federal.period_end(date("2099-12-30"), working(5)).is_err());
    }
}
