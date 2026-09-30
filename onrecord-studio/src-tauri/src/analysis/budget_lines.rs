//! Budget-line comparisons over the imported budget and actuals tables.
//!
//! Three measures, each computed only from grouped rows that bind to a fiscal year, a fund and,
//! for the per-department measures, a department: a department's actual spending against its
//! previous fiscal year, an overtime account's actual spending against its adopted line, and a
//! fund's actual spending against its adopted amount. A role read from a partial snapshot, a
//! truncated query, or rows that name no year, fund, department, basis or parsable amount leave
//! every total they could belong to unreconciled: no number and no finding.
//!
//! Adopted is the original appropriation, not the revised legal authority, so none of these is an
//! appropriation test; a finding states a counted difference between two imported totals.

use super::{
    financial::{decimal, display},
    finding::{
        finding_id, Finding, FindingConfidence, FindingKind, FindingSeverity, FindingSubject,
    },
    phrase_match::{has_phrase, words, Phrases},
};
use crate::types::FinancialRecord;
use chrono::Utc;
use std::collections::{BTreeMap, BTreeSet};

/// A department's actual spending is reported when it moved by at least 10% of the prior year's,
/// in either direction.
pub const YEAR_OVER_YEAR_FINDING_BASIS_POINTS: i128 = 1_000;
/// Actual spending is reported when it is at least 5% above the adopted amount it is compared with.
pub const ABOVE_ADOPTED_FINDING_BASIS_POINTS: i128 = 500;
/// Both totals are read in full from complete snapshots; 10000 is withheld because a fiscal-year
/// export does not certify a closed, final ledger.
pub const LEDGER_CONFIDENCE: u16 = 9_000;
/// The overtime line is identified by its account label, which the agency chose, not by a code
/// whose meaning is documented.
pub const OVERTIME_CONFIDENCE: u16 = 8_000;
const SCALE: i128 = 10_000;
const OVERTIME: Phrases = &[&["overtime"]];

/// A distinct `(account, account_label)` pair; either may be absent in the source.
pub type AccountLabel = (Option<String>, Option<String>);
/// Accounts that are overtime lines, and `(account, why not)` for those that cannot be settled.
pub type OvertimeAccounts = (BTreeSet<String>, Vec<(String, String)>);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Measure {
    DepartmentYearOverYear,
    OvertimeAgainstAdopted,
    FundAgainstAdopted,
}

impl Measure {
    fn kind(self) -> FindingKind {
        match self {
            Self::DepartmentYearOverYear => FindingKind::DepartmentSpendingYearOverYear,
            Self::OvertimeAgainstAdopted => FindingKind::OvertimeAboveAdoptedLine,
            Self::FundAgainstAdopted => FindingKind::FundActualAboveAdopted,
        }
    }

    fn roles(self) -> &'static [&'static str] {
        match self {
            Self::DepartmentYearOverYear => &["actual"],
            Self::OvertimeAgainstAdopted | Self::FundAgainstAdopted => &["budget", "actual"],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// `reference` is always positive: a percentage of a zero or negative total is not defined.
    Compared {
        reference: i128,
        observed: i128,
        citations: Vec<String>,
    },
    Unreconciled {
        detail: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comparison {
    pub measure: Measure,
    pub scope: String,
    pub currency: String,
    pub outcome: Outcome,
}

impl Comparison {
    /// Signed change against the reference in basis points, truncated toward zero.
    pub fn change_basis_points(&self) -> Option<i128> {
        match &self.outcome {
            Outcome::Compared {
                reference,
                observed,
                ..
            } => observed
                .checked_sub(*reference)?
                .checked_mul(SCALE)?
                .checked_div(*reference),
            Outcome::Unreconciled { .. } => None,
        }
    }

    /// Whether the exact difference reaches the measure's threshold. Compared in integers without
    /// truncation, so a change of 9.9999% never reads as 10%.
    pub fn crosses_threshold(&self) -> bool {
        let Outcome::Compared {
            reference,
            observed,
            ..
        } = &self.outcome
        else {
            return false;
        };
        let Some(difference) = observed.checked_sub(*reference) else {
            return false;
        };
        let (difference, threshold) = match self.measure {
            Measure::DepartmentYearOverYear => {
                (difference.abs(), YEAR_OVER_YEAR_FINDING_BASIS_POINTS)
            }
            _ => (difference, ABOVE_ADOPTED_FINDING_BASIS_POINTS),
        };
        match (
            difference.checked_mul(SCALE),
            threshold.checked_mul(*reference),
        ) {
            (Some(scaled), Some(limit)) => scaled >= limit,
            _ => false,
        }
    }
}

/// One role's reads: department-grain totals, overtime-account totals, and the distinct
/// `(account, label)` pairs that identify overtime.
#[derive(Debug, Clone)]
pub struct RoleData {
    pub dataset: String,
    pub source_sha256: String,
    /// Why the role's rows cannot be totalled; `None` when read in full from a complete snapshot.
    pub incomplete: Option<String>,
    pub department_mapped: bool,
    /// `Err` when labels are unmapped or their read was cut short.
    pub labels: Result<Vec<AccountLabel>, String>,
    pub totals: Vec<FinancialRecord>,
    pub overtime: Vec<FinancialRecord>,
    pub query_ids: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct LineData {
    pub roles: BTreeMap<String, RoleData>,
}

struct Line<'a> {
    agency: Option<&'a str>,
    currency: Option<&'a str>,
    fy: Option<i32>,
    fund: Option<&'a str>,
    department: Option<&'a str>,
    account: Option<&'a str>,
    basis: Option<&'a str>,
    amount: Option<i128>,
    rows: u64,
    citation: &'a str,
}

fn field<'a>(record: &'a FinancialRecord, key: &str) -> Option<&'a str> {
    record
        .values
        .get(key)
        .and_then(Option::as_deref)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn line(record: &FinancialRecord) -> Line<'_> {
    Line {
        agency: field(record, "agency"),
        currency: field(record, "currency"),
        fy: field(record, "fy").and_then(|v| v.parse().ok()),
        fund: field(record, "fund"),
        department: field(record, "department"),
        account: field(record, "account"),
        basis: field(record, "basis"),
        amount: field(record, "amount").and_then(decimal),
        rows: record.source_rows,
        citation: &record.citation_id,
    }
}

impl Line<'_> {
    fn bound(&self) -> bool {
        self.agency.is_some() && self.currency.is_some() && self.fy.is_some() && self.fund.is_some()
    }
}

/// Why no total of `role` can be trusted, or `None`.
fn role_blocker(data: &LineData, role: &str) -> Option<String> {
    let Some(role_data) = data.roles.get(role) else {
        return Some(format!("no imported dataset is mapped to the {role} role"));
    };
    if let Some(reason) = &role_data.incomplete {
        return Some(reason.clone());
    }
    let unbound: u64 = role_data
        .totals
        .iter()
        .map(line)
        .filter(|l| !l.bound())
        .map(|l| l.rows)
        .sum();
    (unbound > 0).then(|| {
        format!(
            "{unbound} {role} rows name no fiscal year, fund, agency or currency, so no {role} total is known to be complete"
        )
    })
}

fn first_blocker(data: &LineData, roles: &[&str]) -> Option<String> {
    roles.iter().find_map(|role| role_blocker(data, role))
}

fn total(lines: &[&Line]) -> Option<i128> {
    lines
        .iter()
        .try_fold(0_i128, |sum, l| sum.checked_add(l.amount?))
}

fn citations(lines: &[&Line]) -> Vec<String> {
    lines
        .iter()
        .map(|l| l.citation.to_string())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// Actual rows against the adopted rows of the same line. Budget rows with no basis could be
/// adopted rows, so they leave the adopted total unknown rather than smaller.
fn against_adopted(budget: &[&Line], actual: &[&Line]) -> Outcome {
    let unreconciled = |detail: String| Outcome::Unreconciled { detail };
    let no_basis: u64 = budget
        .iter()
        .filter(|l| l.basis.is_none())
        .map(|l| l.rows)
        .sum();
    if no_basis > 0 {
        return unreconciled(format!("{no_basis} budget rows state no budget basis"));
    }
    let adopted: Vec<&Line> = budget
        .iter()
        .copied()
        .filter(|l| l.basis.is_some_and(|b| b.eq_ignore_ascii_case("adopted")))
        .collect();
    if adopted.is_empty() {
        return unreconciled("no adopted budget rows".into());
    }
    if actual.is_empty() {
        return unreconciled("no actual rows".into());
    }
    let (Some(reference), Some(observed)) = (total(&adopted), total(actual)) else {
        return unreconciled("an amount is missing or does not parse as a decimal".into());
    };
    if reference <= 0 {
        return unreconciled(format!(
            "the adopted total is {}, so no percentage is defined",
            display(reference)
        ));
    }
    let mut cited: Vec<&Line> = adopted;
    cited.extend(actual.iter().copied());
    Outcome::Compared {
        reference,
        observed,
        citations: citations(&cited),
    }
}

type FundKey<'a> = (&'a str, &'a str, i32, &'a str);

fn fund_comparisons(data: &LineData, out: &mut Vec<Comparison>) {
    let blocker = first_blocker(data, Measure::FundAgainstAdopted.roles());
    let parsed: BTreeMap<&str, Vec<Line>> = ["budget", "actual"]
        .into_iter()
        .map(|role| {
            let lines = data
                .roles
                .get(role)
                .map(|r| r.totals.iter().map(line).collect())
                .unwrap_or_default();
            (role, lines)
        })
        .collect();
    let mut keys: BTreeMap<FundKey, [Vec<&Line>; 2]> = BTreeMap::new();
    for (index, role) in ["budget", "actual"].into_iter().enumerate() {
        for l in parsed[role].iter().filter(|l| l.bound()) {
            let key = (
                l.agency.unwrap_or_default(),
                l.currency.unwrap_or_default(),
                l.fy.unwrap_or_default(),
                l.fund.unwrap_or_default(),
            );
            keys.entry(key).or_default()[index].push(l);
        }
    }
    for ((agency, currency, fy, fund), [budget, actual]) in keys {
        out.push(Comparison {
            measure: Measure::FundAgainstAdopted,
            scope: format!("FY {fy} fund {fund} ({agency})"),
            currency: currency.into(),
            outcome: match &blocker {
                Some(detail) => Outcome::Unreconciled {
                    detail: detail.clone(),
                },
                None => against_adopted(&budget, &actual),
            },
        });
    }
}

fn department_comparisons(data: &LineData, out: &mut Vec<Comparison>) {
    let Some(actual) = data.roles.get("actual") else {
        return;
    };
    if !actual.department_mapped {
        out.push(Comparison {
            measure: Measure::DepartmentYearOverYear,
            scope: "every department".into(),
            currency: String::new(),
            outcome: Outcome::Unreconciled {
                detail: "the actual dataset has no mapped department column".into(),
            },
        });
        return;
    }
    let blocker = role_blocker(data, "actual");
    let lines: Vec<Line> = actual.totals.iter().map(line).collect();
    type Fund<'a> = (&'a str, &'a str, &'a str);
    let mut fund_years: BTreeMap<Fund, BTreeSet<i32>> = BTreeMap::new();
    let mut no_department: BTreeMap<(Fund, i32), u64> = BTreeMap::new();
    let mut departments: BTreeMap<(Fund, &str), BTreeMap<i32, Vec<&Line>>> = BTreeMap::new();
    for l in lines.iter().filter(|l| l.bound()) {
        let fund = (
            l.agency.unwrap_or_default(),
            l.currency.unwrap_or_default(),
            l.fund.unwrap_or_default(),
        );
        let fy = l.fy.unwrap_or_default();
        fund_years.entry(fund).or_default().insert(fy);
        match l.department {
            Some(department) => departments
                .entry((fund, department))
                .or_default()
                .entry(fy)
                .or_default()
                .push(l),
            None => *no_department.entry((fund, fy)).or_default() += l.rows,
        }
    }
    for ((fund, department), years) in &departments {
        let (agency, currency, fund_name) = *fund;
        for &fy in &fund_years[fund] {
            if !fund_years[fund].contains(&(fy - 1)) {
                continue;
            }
            let (current, prior) = (years.get(&fy), years.get(&(fy - 1)));
            if current.is_none() && prior.is_none() {
                continue;
            }
            let unbound = [fy - 1, fy]
                .iter()
                .map(|year| no_department.get(&(*fund, *year)).copied().unwrap_or(0))
                .sum::<u64>();
            let outcome = if let Some(detail) = &blocker {
                Outcome::Unreconciled {
                    detail: detail.clone(),
                }
            } else if unbound > 0 {
                Outcome::Unreconciled {
                    detail: format!(
                        "{unbound} actual rows in fund {fund_name} for FY {} or FY {fy} name no department",
                        fy - 1
                    ),
                }
            } else {
                match (prior, current) {
                    (None, _) => Outcome::Unreconciled {
                        detail: format!("no actual rows for this department in FY {}", fy - 1),
                    },
                    (_, None) => Outcome::Unreconciled {
                        detail: format!("no actual rows for this department in FY {fy}"),
                    },
                    (Some(prior), Some(current)) => match (total(prior), total(current)) {
                        (Some(reference), Some(observed)) if reference > 0 => {
                            let mut cited = prior.clone();
                            cited.extend(current.iter().copied());
                            Outcome::Compared {
                                reference,
                                observed,
                                citations: citations(&cited),
                            }
                        }
                        (Some(reference), Some(_)) => Outcome::Unreconciled {
                            detail: format!(
                                "the FY {} total is {}, so no percentage is defined",
                                fy - 1,
                                display(reference)
                            ),
                        },
                        _ => Outcome::Unreconciled {
                            detail: "an amount is missing or does not parse as a decimal".into(),
                        },
                    },
                }
            };
            out.push(Comparison {
                measure: Measure::DepartmentYearOverYear,
                scope: format!(
                    "department {department}, fund {fund_name}, FY {} to FY {fy} ({agency})",
                    fy - 1
                ),
                currency: currency.into(),
                outcome,
            });
        }
    }
}

/// Accounts whose labels name overtime in both roles and never name anything else, the accounts
/// that cannot be settled, and a reason no account can be identified at all.
pub fn overtime_accounts(data: &LineData) -> Result<OvertimeAccounts, String> {
    let mut per_role = Vec::new();
    for role in ["budget", "actual"] {
        let role_data = data
            .roles
            .get(role)
            .ok_or_else(|| format!("no imported dataset is mapped to the {role} role"))?;
        let labels = role_data.labels.as_ref().map_err(Clone::clone)?;
        let mut accounts: BTreeMap<&str, (bool, bool)> = BTreeMap::new();
        for (account, label) in labels {
            let Some(account) = account.as_deref() else {
                return Err(format!("some {role} rows name no account"));
            };
            let Some(label) = label.as_deref() else {
                continue;
            };
            let entry = accounts.entry(account).or_default();
            if has_phrase(&words(label), OVERTIME) {
                entry.0 = true;
            } else {
                entry.1 = true;
            }
        }
        per_role.push(accounts);
    }
    let mut qualified = BTreeSet::new();
    let mut unresolved = Vec::new();
    let candidates: BTreeSet<&str> = per_role
        .iter()
        .flat_map(|accounts| accounts.iter().filter(|(_, (ot, _))| *ot).map(|(a, _)| *a))
        .collect();
    for account in candidates {
        let [budget, actual] = [&per_role[0], &per_role[1]].map(|r| r.get(account).copied());
        let detail = match (budget, actual) {
            (Some((true, false)), Some((true, false))) => {
                qualified.insert(account.to_string());
                continue;
            }
            (Some((_, true)), _) | (_, Some((_, true))) => {
                "its labels name overtime and something else".to_string()
            }
            (Some((true, false)), _) => "only the budget labels name overtime".to_string(),
            _ => "only the actual labels name overtime".to_string(),
        };
        unresolved.push((account.to_string(), detail));
    }
    Ok((qualified, unresolved))
}

fn overtime_comparisons(data: &LineData, out: &mut Vec<Comparison>) {
    let unreconciled = |scope: String, detail: String| Comparison {
        measure: Measure::OvertimeAgainstAdopted,
        scope,
        currency: String::new(),
        outcome: Outcome::Unreconciled { detail },
    };
    let (_, unresolved) = match overtime_accounts(data) {
        Ok(accounts) => accounts,
        Err(detail) => return out.push(unreconciled("overtime lines".into(), detail)),
    };
    if let Some(role) = ["budget", "actual"]
        .into_iter()
        .find(|role| !data.roles[*role].department_mapped)
    {
        return out.push(unreconciled(
            "overtime lines".into(),
            format!("the {role} dataset has no mapped department column"),
        ));
    }
    for (account, detail) in unresolved {
        out.push(unreconciled(format!("account {account}"), detail));
    }
    let blocker = first_blocker(data, Measure::OvertimeAgainstAdopted.roles());
    let parsed: [Vec<Line>; 2] =
        ["budget", "actual"].map(|role| data.roles[role].overtime.iter().map(line).collect());
    type Key<'a> = (&'a str, &'a str, i32, &'a str, Option<&'a str>, &'a str);
    let mut keys: BTreeMap<Key, [Vec<&Line>; 2]> = BTreeMap::new();
    for (index, lines) in parsed.iter().enumerate() {
        for l in lines.iter().filter(|l| l.bound()) {
            let Some(account) = l.account else { continue };
            let key = (
                l.agency.unwrap_or_default(),
                l.currency.unwrap_or_default(),
                l.fy.unwrap_or_default(),
                l.fund.unwrap_or_default(),
                l.department,
                account,
            );
            keys.entry(key).or_default()[index].push(l);
        }
    }
    for ((agency, currency, fy, fund, department, account), [budget, actual]) in keys {
        let outcome = match (&blocker, department) {
            (Some(detail), _) => Outcome::Unreconciled {
                detail: detail.clone(),
            },
            (None, None) => Outcome::Unreconciled {
                detail: "overtime rows name no department".into(),
            },
            (None, Some(_)) => against_adopted(&budget, &actual),
        };
        out.push(Comparison {
            measure: Measure::OvertimeAgainstAdopted,
            scope: format!(
                "FY {fy} fund {fund} department {} overtime account {account} ({agency})",
                department.unwrap_or("(none)")
            ),
            currency: currency.into(),
            outcome,
        });
    }
}

/// Every comparison the imported rows allow, compared or unreconciled, in a stable order.
pub fn compare(data: &LineData) -> Vec<Comparison> {
    let mut out = Vec::new();
    department_comparisons(data, &mut out);
    overtime_comparisons(data, &mut out);
    fund_comparisons(data, &mut out);
    out
}

fn provenance(data: &LineData, measure: Measure) -> String {
    measure
        .roles()
        .iter()
        .filter_map(|role| {
            let r = data.roles.get(*role)?;
            Some(format!(
                "the {role} dataset '{}' (sha256 {}), query id(s) {}",
                r.dataset,
                r.source_sha256.get(..16).unwrap_or(&r.source_sha256),
                r.query_ids.join(", ")
            ))
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// Only a comparison that reaches its threshold is a finding.
pub fn to_finding(
    data: &LineData,
    comparison: &Comparison,
    source_detector: &str,
) -> Option<Finding> {
    if !comparison.crosses_threshold() {
        return None;
    }
    let Outcome::Compared {
        reference,
        observed,
        ..
    } = &comparison.outcome
    else {
        return None;
    };
    let change = comparison.change_basis_points()?;
    let percent = format!(
        "{}{}.{:02}%",
        if change < 0 { "-" } else { "+" },
        change.unsigned_abs() / 100,
        change.unsigned_abs() % 100
    );
    let money = |value: &i128| format!("{} {}", display(*value), comparison.currency);
    let scope = &comparison.scope;
    let sources = provenance(data, comparison.measure);
    let (headline, statement, limits, confidence) = match comparison.measure {
        Measure::DepartmentYearOverYear => (
            format!("Actual spending changed {percent} year over year: {scope}"),
            format!(
                "Imported actual spending for {scope} was {} in the earlier year and {} in the later one, a change of {percent}. The threshold is a change of at least {}% either way.",
                money(reference),
                money(observed),
                YEAR_OVER_YEAR_FINDING_BASIS_POINTS / 100
            ),
            "A reorganization, a transfer of programs between departments, one-time costs or a change in how the export assigns departments can each move a department total; this does not say why it changed.",
            LEDGER_CONFIDENCE,
        ),
        Measure::OvertimeAgainstAdopted => (
            format!("Overtime spending {percent} against its adopted line: {scope}"),
            format!(
                "Imported actual spending on {scope} is {} against an adopted amount of {} for the same account, fund, department and year, {percent}. The threshold is at least {}% above adopted. The account was identified as overtime by its label in both tables.",
                money(observed),
                money(reference),
                ABOVE_ADOPTED_FINDING_BASIS_POINTS / 100
            ),
            "Adopted is the original appropriation, not the revised authority; mid-year transfers, reimbursed overtime (for example, events or grants) and account reclassification can each account for the difference. This is not a finding that an appropriation was exceeded.",
            OVERTIME_CONFIDENCE,
        ),
        Measure::FundAgainstAdopted => (
            format!("Actual spending {percent} against the adopted amount: {scope}"),
            format!(
                "Imported actual spending for {scope} is {} against an adopted amount of {}, {percent}. The threshold is at least {}% above adopted.",
                money(observed),
                money(reference),
                ABOVE_ADOPTED_FINDING_BASIS_POINTS / 100
            ),
            "Adopted is the original appropriation, not the revised authority; council revisions, carried-forward encumbrances and transfers change what a fund may spend. This is not a finding that an appropriation was exceeded.",
            LEDGER_CONFIDENCE,
        ),
    };
    let subject = FindingSubject::default();
    let reference_text = display(*reference);
    let observed_text = display(*observed);
    let kind = comparison.measure.kind();
    let now = Utc::now();
    Some(Finding {
        id: finding_id(
            kind,
            source_detector,
            &subject,
            &[scope, &reference_text, &observed_text, &sources],
        ),
        kind,
        severity: FindingSeverity::Notable,
        confidence: FindingConfidence::measured(confidence),
        subject,
        headline,
        explanation: format!("{statement} {limits} Source: {sources}."),
        citations: vec![],
        discovered_at: now,
        last_seen_at: now,
        dismissed_at: None,
        source_detector: source_detector.into(),
    })
}
