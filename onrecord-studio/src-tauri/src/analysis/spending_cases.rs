//! Spending cases: rows of the imported ledgers set against a limit a document or ledger states.
//!
//! Two evidence classes, never mixed. A `counted_comparison` compares recorded rows to a written
//! limit or ceiling whose every input is recorded: a signed contract ceiling, a contract term, an
//! appropriation, a restriction register, or a rule the agency's own policy states. A `pattern`
//! reports an observation that needs an explanation (payments with no contract row, an approval
//! cell left blank, payments clustered under a limit unless the policy itself forbids splitting).
//!
//! A missing role or field is never skipped silently and never weakened into a finding: it
//! becomes a `FinancialGap` naming the table that would allow the check. Confidence is derived
//! from what the evaluation observed (see the constants), never from how the result reads.
//!
//! The records are the `FinancialRecord`s the financial investigation already cites, so every
//! case resolves to `FinancialCitation` rows and the query ids that produced them.

use super::financial::{amount, display, finding, get, sum};
use super::spending_rules::{ExtractedRule, RuleKind};
use crate::types::{FinancialAuditFinding, FinancialCalculation, FinancialGap, FinancialRecord};
use chrono::NaiveDate;
use std::collections::{BTreeMap, BTreeSet};

pub const CEILING_EXCEEDED: &str = "ceiling_exceeded";
pub const PAYMENT_OUTSIDE_TERM: &str = "payment_outside_contract_term";
pub const PAYMENT_WITHOUT_CONTRACT: &str = "payment_without_contract_record";
pub const APPROPRIATION_EXCEEDED: &str = "appropriation_exceeded";
pub const RESTRICTED_OFF_PROGRAM: &str = "restricted_fund_off_program";
pub const APPROVAL_THRESHOLD_EXCEEDED: &str = "approval_threshold_exceeded";
pub const APPROVAL_RECORD_ABSENT: &str = "approval_record_absent";
pub const SPLIT_PURCHASE_PATTERN: &str = "split_purchase_pattern";
pub const SPLIT_PURCHASE_PROHIBITED: &str = "split_purchase_prohibited";

pub const COUNTED: &str = "counted_comparison";
pub const PATTERN: &str = "pattern";

/// A comparison against a ledger confined to one fund and the retrieved years. Other funds,
/// earlier omitted payments and later credits can move the true balance, so no ledger-scoped case
/// reaches 10000.
pub const LEDGER_CONFIDENCE: u16 = 9_000;
/// Open encumbrances can still carry an amount already paid, so adding them to actuals may
/// double count.
pub const ENCUMBERED_CONFIDENCE: u16 = 7_000;
/// A rule in force from its own stated effective date.
pub const STATED_EFFECTIVE_CONFIDENCE: u16 = 8_000;
/// The policy states no effective date; its upload date stands in for it.
pub const UPLOAD_DATE_CONFIDENCE: u16 = 6_000;
pub const ABSENCE_PATTERN_CONFIDENCE: u16 = 5_000;
pub const APPROVAL_BLANK_CONFIDENCE: u16 = 4_000;
const SPLIT_BASE_CONFIDENCE: u16 = 4_000;
const SPLIT_CONFIDENCE_PER_EXTRA_PAYMENT: u16 = 500;
const SPLIT_CONFIDENCE_CAP: u16 = 6_500;

/// Payments are clustered only when each sits within this share of the limit, below it.
pub const SPLIT_MARGIN_BASIS_POINTS: i128 = 1_000;
/// Payments to one vendor count as one cluster when they fall within this many days of the first.
pub const SPLIT_WINDOW_DAYS: i64 = 14;
/// Fewer payments than this is not a cluster.
pub const SPLIT_MIN_PAYMENTS: usize = 3;
/// Rows named in one finding; the observation states the full count.
pub const MAX_CITED_ROWS: usize = 50;

#[derive(Debug, Clone)]
pub struct RuleDocument {
    pub portal_key: String,
    pub document_id: i64,
    pub name: String,
    pub quote: String,
    pub char_start: usize,
    pub char_end: usize,
    pub effective: NaiveDate,
    pub effective_stated: bool,
}

#[derive(Debug, Clone)]
pub struct PolicyRule {
    pub rule: ExtractedRule,
    pub document: RuleDocument,
}

#[derive(Debug, Clone)]
pub struct SpendingCase {
    pub finding: FinancialAuditFinding,
    /// Indices into the rules the case was evaluated against.
    pub rules: Vec<usize>,
}

#[derive(Debug, Clone, Default)]
pub struct SpendingOutcome {
    pub cases: Vec<SpendingCase>,
    pub gaps: Vec<FinancialGap>,
}

impl SpendingOutcome {
    fn gap(&mut self, title: &str, records_needed: &str, why: &str) {
        if self.gaps.iter().all(|gap| gap.title != title) {
            self.gaps.push(FinancialGap {
                title: title.into(),
                records_needed: records_needed.into(),
                why: why.into(),
                citations: vec![],
            });
        }
    }
}

fn date(record: &FinancialRecord, field: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(get(record, field), "%Y-%m-%d").ok()
}

fn approved(record: &FinancialRecord) -> bool {
    ["executed", "approved", "signed"].contains(&get(record, "status").to_lowercase().as_str())
}

fn role_rows<'a>(records: &'a [FinancialRecord], role: &str) -> Vec<&'a FinancialRecord> {
    records.iter().filter(|r| r.role == role).collect()
}

fn has_value(rows: &[&FinancialRecord], field: &str) -> bool {
    rows.iter().any(|row| !get(row, field).is_empty())
}

fn capped<'a>(rows: &[&'a FinancialRecord]) -> Vec<&'a FinancialRecord> {
    rows.iter().copied().take(MAX_CITED_ROWS).collect()
}

#[allow(clippy::too_many_arguments)]
fn counted(
    kind: &str,
    title: &str,
    observation: String,
    rows: &[&FinancialRecord],
    alternatives: &[&str],
    next_test: &str,
    calculations: Vec<FinancialCalculation>,
    confidence: u16,
) -> FinancialAuditFinding {
    classified(
        COUNTED,
        kind,
        title,
        observation,
        rows,
        alternatives,
        next_test,
        calculations,
        confidence,
    )
}

#[allow(clippy::too_many_arguments)]
fn classified(
    basis: &str,
    kind: &str,
    title: &str,
    observation: String,
    rows: &[&FinancialRecord],
    alternatives: &[&str],
    next_test: &str,
    calculations: Vec<FinancialCalculation>,
    confidence: u16,
) -> FinancialAuditFinding {
    let mut item = finding(kind, title, observation, rows, alternatives, next_test);
    item.calculations = calculations;
    item.basis = basis.into();
    item.confidence_basis_points = Some(confidence);
    item
}

fn calc(label: &str, formula: String, value: i128, unit: &str) -> FinancialCalculation {
    FinancialCalculation {
        label: label.into(),
        formula,
        result: display(value),
        unit: unit.into(),
    }
}

/// Transactions a payment total may count: one row per transaction id, identical repeats counted
/// once, posted statuses only. Rows of a transaction id that is blank, conflicting or carries an
/// unrecognized status are excluded and counted, never guessed at. Mirrors the contract chain.
fn settled_payments<'a>(rows: &[&'a FinancialRecord]) -> (Vec<&'a FinancialRecord>, usize) {
    let mut transactions: BTreeMap<&str, Vec<&'a FinancialRecord>> = BTreeMap::new();
    for row in rows {
        transactions
            .entry(get(row, "transaction_id"))
            .or_default()
            .push(row);
    }
    let mut clean = Vec::new();
    let mut excluded = 0;
    for (transaction, group) in transactions {
        let status = get(group[0], "status").to_lowercase();
        let conflicting = group
            .windows(2)
            .any(|pair| pair[0].values != pair[1].values);
        if transaction.is_empty() || conflicting {
            excluded += group.len();
        } else if ["paid", "posted", "settled"].contains(&status.as_str()) {
            clean.push(group[0]);
        } else if !["void", "cancelled", "canceled"].contains(&status.as_str()) {
            excluded += group.len();
        }
    }
    (clean, excluded)
}

fn agency_key(record: &FinancialRecord, id_field: &str) -> (String, String) {
    (get(record, "agency").into(), get(record, id_field).into())
}

/// Case 1b and 1c: payments outside the recorded contract term, and payments to a vendor the
/// contract register does not name.
fn contract_cases(
    records: &[FinancialRecord],
    incomplete: &BTreeSet<String>,
    out: &mut SpendingOutcome,
) {
    let contracts = role_rows(records, "contract");
    let payments = role_rows(records, "payment");
    if contracts.is_empty() || payments.is_empty() {
        return;
    }
    if incomplete.contains("contract") || incomplete.contains("payment") {
        out.gap(
            "Complete contract and payment registers",
            "A contract register and posted payment ledger retrieved in full for this fund and fiscal years, not truncated or partially imported.",
            "A truncated or partial register cannot support a comparison of payments with contract terms or with the register's contents.",
        );
        return;
    }
    let (settled, _) = settled_payments(&payments);

    // 1c: vendor and contract id both absent from the register.
    let contract_ids: BTreeSet<(String, String)> = contracts
        .iter()
        .map(|r| agency_key(r, "contract_id"))
        .collect();
    let contract_vendors: BTreeSet<(String, String)> = contracts
        .iter()
        .map(|r| agency_key(r, "vendor_id"))
        .collect();
    // Keyed by currency too: a total is a figure in one currency, never a sum across them.
    let mut orphans: BTreeMap<(String, String, String), Vec<&FinancialRecord>> = BTreeMap::new();
    for payment in &settled {
        let vendor = agency_key(payment, "vendor_id");
        if vendor.1.is_empty()
            || vendor.0.is_empty()
            || contract_vendors.contains(&vendor)
            || contract_ids.contains(&agency_key(payment, "contract_id"))
        {
            continue;
        }
        let currency = get(payment, "currency").to_string();
        orphans
            .entry((vendor.0, vendor.1, currency))
            .or_default()
            .push(payment);
    }
    for ((agency, vendor, currency), rows) in orphans {
        let Some(total) = sum(&rows, "amount").filter(|total| *total > 0) else {
            continue;
        };
        let currency = currency.as_str();
        let mut cited = capped(&rows);
        cited.extend(contracts.iter().copied().take(1));
        out.cases.push(SpendingCase {
            finding: classified(
                PATTERN,
                PAYMENT_WITHOUT_CONTRACT,
                "Payments to a vendor the contract register does not name",
                format!(
                    "{agency}: vendor {vendor} received {} posted payment rows totalling {} {currency} in this fund and these fiscal years, and neither the vendor nor the payments' contract ids appear in the {} retrieved contract register rows. This compares two imported tables; it is not a finding that a contract was required or missing.",
                    rows.len(), display(total), contracts.len()
                ),
                &cited,
                &[
                    "The register may omit purchases below a competitive-bid threshold, purchase orders, grants or cooperative agreements.",
                    "The register may list this vendor under another identifier, or hold the contract under another fund or a later fiscal year.",
                ],
                "Obtain the procurement basis for these payments (purchase order, bid exemption or agreement) and the vendor identifier crosswalk between the payment and contract systems.",
                vec![calc("Posted payments to the vendor", format!("sum of {} cited payment rows", rows.len()), total, currency)],
                ABSENCE_PATTERN_CONFIDENCE,
            ),
            rules: vec![],
        });
    }

    // 1b: payment date outside the contract's recorded term.
    let amendments = role_rows(records, "amendment");
    let mut undated = 0;
    let mut no_term = 0;
    let mut need_amendment_term = false;
    let mut by_contract: BTreeMap<(String, String), Vec<&FinancialRecord>> = BTreeMap::new();
    for payment in &settled {
        by_contract
            .entry(agency_key(payment, "contract_id"))
            .or_default()
            .push(payment);
    }
    for ((agency, contract_id), rows) in by_contract {
        let matching: Vec<&&FinancialRecord> = contracts
            .iter()
            .filter(|c| agency_key(c, "contract_id") == (agency.clone(), contract_id.clone()))
            .collect();
        let [contract] = matching.as_slice() else {
            continue;
        };
        let contract: &FinancialRecord = contract;
        let (Some(start), Some(end)) =
            (date(contract, "period_start"), date(contract, "period_end"))
        else {
            no_term += 1;
            continue;
        };
        if !approved(contract) || end < start {
            continue;
        }
        let dated: Vec<(&FinancialRecord, NaiveDate)> = rows
            .iter()
            .filter_map(|r| date(r, "transaction_date").map(|d| (*r, d)))
            .collect();
        undated += rows.len() - dated.len();
        // An amendment can move the end date. Without a complete register row for this contract
        // that states its own end date, only a payment before the start can be compared.
        let own_amendments: Vec<&&FinancialRecord> = amendments
            .iter()
            .filter(|a| agency_key(a, "contract_id") == (agency.clone(), contract_id.clone()))
            .collect();
        let extensions_known = !amendments.is_empty()
            && !incomplete.contains("amendment")
            && !own_amendments.is_empty()
            && own_amendments
                .iter()
                .all(|a| approved(a) && date(a, "period_end").is_some());
        if !extensions_known {
            need_amendment_term = true;
        }
        let effective_end = own_amendments
            .iter()
            .filter_map(|a| date(a, "period_end"))
            .fold(end, |latest, day| latest.max(day));
        let before: Vec<&(&FinancialRecord, NaiveDate)> =
            dated.iter().filter(|(_, d)| *d < start).collect();
        let after: Vec<&(&FinancialRecord, NaiveDate)> = if extensions_known {
            dated.iter().filter(|(_, d)| *d > effective_end).collect()
        } else {
            vec![]
        };
        if before.is_empty() && after.is_empty() {
            continue;
        }
        let mut offending: Vec<&FinancialRecord> =
            before.iter().chain(&after).map(|(r, _)| *r).collect();
        let currency = get(contract, "currency");
        if offending.iter().any(|r| get(r, "currency") != currency) {
            out.gap(
                "Payments in the contract's currency",
                "Payment rows recorded in the same currency as the contract they are charged to.",
                "Payments in another currency cannot be totalled against a contract stated in one currency.",
            );
            continue;
        }
        let outside = sum(&offending, "amount");
        let mut cited = vec![contract];
        if extensions_known {
            cited.extend(own_amendments.iter().map(|a| **a));
        }
        offending.sort_by_key(|r| get(r, "transaction_date").to_string());
        cited.extend(capped(&offending));
        let term = if extensions_known && effective_end != end {
            format!("{start} through {effective_end} (contract end {end}, extended by executed amendments)")
        } else {
            format!("{start} through {end}")
        };
        let mut calculations = vec![];
        if let Some(total) = outside {
            calculations.push(calc(
                "Posted payments dated outside the term",
                format!("sum of {} cited payment rows", offending.len()),
                total,
                currency,
            ));
        }
        out.cases.push(SpendingCase {
            finding: counted(
                PAYMENT_OUTSIDE_TERM,
                "Payments dated outside the contract's recorded term",
                format!(
                    "Contract {contract_id} ({agency}) records a term of {term}. {} posted payment row(s) carry a transaction date outside it: {} before the start date and {} after the end date. The comparison uses the recorded transaction date and the recorded term only.",
                    offending.len(), before.len(), after.len()
                ),
                &cited,
                &[
                    "The transaction date may be a posting date for a service delivered inside the term.",
                    "A term extension, renewal or change order may exist outside the retrieved amendment register.",
                ],
                "Obtain the invoice service dates and any executed term extension for this contract, and compare them with the payment dates cited.",
                calculations,
                LEDGER_CONFIDENCE,
            ),
            rules: vec![],
        });
    }
    if no_term > 0 {
        out.gap(
            "Contract start and end dates",
            "The contract register with each contract's start and end date (map period_start and period_end on the contract table).",
            "Without a recorded term, a payment date cannot be compared with when the contract was in effect.",
        );
    }
    if undated > 0 {
        out.gap(
            "Payment transaction dates",
            "The payment ledger with each posted payment's date (map transaction_date on the payment table).",
            "Payments without a recorded date cannot be compared with a contract term or a policy window.",
        );
    }
    if need_amendment_term {
        out.gap(
            "Executed amendments stating term changes",
            "An executed amendment register for each contract with the amendment's new end date (map period_end on the amendment table), including an explicit no-change row.",
            "An amendment can extend a contract's term, so payments after the recorded end date are not compared until the register is complete.",
        );
    }
}

/// Case 2: spending against revised appropriation for one reconciled fund, year and account.
/// Adopted is never the limit: council revisions move authority, so an adopted-only ledger is a
/// gap. `comparable` is the reconciliation's own gate; a conflicting crosswalk clears it.
#[allow(clippy::too_many_arguments)]
pub fn appropriation_case(
    dimensions: &[String],
    comparable: bool,
    revised: Option<i128>,
    actual: Option<i128>,
    encumbered: Option<i128>,
    revised_rows: &[&FinancialRecord],
    actual_rows: &[&FinancialRecord],
    encumbered_rows: &[&FinancialRecord],
    crosswalk_ids: &BTreeSet<String>,
) -> Option<FinancialAuditFinding> {
    if !comparable {
        return None;
    }
    let (revised, actual) = (revised?, actual?);
    let currency = &dimensions[6];
    let place = format!(
        "FY{} fund {} account {}{}",
        dimensions[0],
        dimensions[1],
        dimensions[2],
        if dimensions[4].is_empty() {
            if dimensions[3].is_empty() {
                String::new()
            } else {
                format!(" department {}", dimensions[3])
            }
        } else {
            format!(" center {}", dimensions[4])
        }
    );
    let (over, total, confidence, what, used_encumbrance) = if actual > revised {
        (
            actual - revised,
            actual,
            LEDGER_CONFIDENCE,
            "recorded actual spending",
            false,
        )
    } else {
        let committed = actual.checked_add(encumbered?)?;
        if committed <= revised {
            return None;
        }
        (
            committed - revised,
            committed,
            ENCUMBERED_CONFIDENCE,
            "recorded actual spending plus open encumbrances",
            true,
        )
    };
    let mut rows: Vec<&FinancialRecord> = revised_rows.iter().chain(actual_rows).copied().collect();
    if used_encumbrance {
        rows.extend(encumbered_rows);
    }
    let mut item = counted(
        APPROPRIATION_EXCEEDED,
        "Recorded spending is above the recorded revised appropriation",
        format!(
            "{place}: {what} is {} {currency} against a recorded revised appropriation of {} {currency}, {} above it. The ledgers state these amounts; whether a transfer, supplemental appropriation or reclassification covers the difference is not recorded in them.",
            display(total), display(revised), display(over)
        ),
        &rows,
        &[
            "A dated budget transfer or supplemental appropriation may be missing from the revised rows.",
            "An account reclassification after the budget was revised can place the same spending under a different account.",
            "Open encumbrances may already be partly liquidated into actuals, which double counts them.",
        ],
        "Obtain the dated appropriation transfers and supplemental appropriations for this fund and account, and the encumbrance liquidation history.",
        vec![calc(
            "Amount above revised appropriation",
            format!("{} − {}", display(total), display(revised)),
            over,
            currency,
        )],
        confidence,
    );
    item.citations.extend(crosswalk_ids.iter().cloned());
    item.citations.sort();
    item.citations.dedup();
    Some(item)
}

/// Case 3: spending from a fund whose restriction register names the permitted programs, on a
/// program it does not name. Every comparison is an exact identifier.
fn restriction_cases(
    records: &[FinancialRecord],
    incomplete: &BTreeSet<String>,
    out: &mut SpendingOutcome,
) {
    let restrictions = role_rows(records, "restriction");
    let actuals = role_rows(records, "actual");
    if restrictions.is_empty() || actuals.is_empty() {
        return;
    }
    if incomplete.contains("restriction") || incomplete.contains("actual") {
        out.gap(
            "Complete restriction register and actual expenditures",
            "A fund restriction register and actual expenditure ledger retrieved in full for this fund and fiscal year.",
            "A truncated register could omit the permitted program, and truncated actuals would understate the spending compared.",
        );
        return;
    }
    if !has_value(&actuals, "program") {
        out.gap(
            "Program on actual expenditures",
            "The actual expenditure ledger with the program each amount was spent on (map program on the actual table).",
            "Without a program on the spending rows, spending cannot be compared with the programs a restriction permits.",
        );
        return;
    }
    let mut funds: BTreeMap<(String, String, String), Vec<&FinancialRecord>> = BTreeMap::new();
    for row in &restrictions {
        funds
            .entry((
                get(row, "agency").into(),
                get(row, "fy").into(),
                get(row, "fund").into(),
            ))
            .or_default()
            .push(row);
    }
    for ((agency, fy, fund), rules) in funds {
        let classes: BTreeSet<String> = rules
            .iter()
            .map(|r| get(r, "restriction").to_lowercase())
            .collect();
        if classes != BTreeSet::from(["restricted".to_string()]) {
            if classes.contains("restricted") {
                out.gap(
                    "Unambiguous restriction classification",
                    "A restriction register whose rows for one fund and fiscal year all state restricted, with the expressly eligible programs.",
                    "A fund whose register rows mix restricted with unrestricted or unrecognized classifications cannot be tested for a forbidden program.",
                );
            }
            continue;
        }
        let eligible: BTreeSet<&str> = rules
            .iter()
            .map(|r| get(r, "eligible_program"))
            .filter(|p| !p.is_empty())
            .collect();
        if eligible.is_empty() {
            out.gap(
                "Expressly eligible programs for restricted funds",
                "The restriction register with the program each restricted fund may be spent on (map eligible_program on the restriction table).",
                "A restriction that names no permitted program cannot be compared with where the fund was spent.",
            );
            continue;
        }
        let spent: Vec<&FinancialRecord> = actuals
            .iter()
            .copied()
            .filter(|a| get(a, "agency") == agency && get(a, "fy") == fy && get(a, "fund") == fund)
            .collect();
        let offending: Vec<&FinancialRecord> = spent
            .iter()
            .copied()
            .filter(|a| {
                let program = get(a, "program");
                !program.is_empty()
                    && !eligible.contains(program)
                    && amount(a, "amount").is_some_and(|v| v > 0)
            })
            .collect();
        let Some(total) = sum(&offending, "amount").filter(|t| *t > 0) else {
            continue;
        };
        let unclassified = spent
            .iter()
            .filter(|a| get(a, "program").is_empty())
            .count();
        let programs: BTreeSet<&str> = offending.iter().map(|a| get(a, "program")).collect();
        let currency = get(offending[0], "currency");
        if offending.iter().any(|a| get(a, "currency") != currency) {
            out.gap(
                "Restricted-fund spending in one currency",
                "Actual expenditure rows for the restricted fund recorded in a single currency.",
                "Spending recorded in several currencies cannot be summed into one total.",
            );
            continue;
        }
        let mut cited: Vec<&FinancialRecord> = rules.clone();
        cited.extend(capped(&offending));
        out.cases.push(SpendingCase {
            finding: counted(
                RESTRICTED_OFF_PROGRAM,
                "Spending from a restricted fund on a program its restriction does not name",
                format!(
                    "{agency} FY{fy} fund {fund} is recorded as restricted to {}. Actual expenditure rows record {} {currency} on {} other program(s): {}. {} actual row(s) for the fund carry no program and are not counted. The comparison matches program identifiers exactly.",
                    eligible.iter().copied().collect::<Vec<_>>().join(", "), display(total), programs.len(),
                    programs.iter().copied().collect::<Vec<_>>().join(", "), unclassified
                ),
                &cited,
                &[
                    "The expenditure ledger and the restriction register may name the same program differently.",
                    "A restriction may permit a class of programs that the register lists by one example.",
                    "A reimbursement or interfund transfer may carry the program label of the receiving side.",
                ],
                "Obtain the grant agreement or ordinance behind the restriction and the program crosswalk between the expenditure and restriction systems.",
                vec![calc(
                    "Actual spending on programs the restriction does not name",
                    format!("sum of {} cited actual rows", offending.len()),
                    total,
                    currency,
                )],
                LEDGER_CONFIDENCE,
            ),
            rules: vec![],
        });
    }
}

/// The ledger cases that need no policy document.
pub fn ledger_cases(records: &[FinancialRecord], incomplete: &BTreeSet<String>) -> SpendingOutcome {
    let mut out = SpendingOutcome::default();
    contract_cases(records, incomplete, &mut out);
    restriction_cases(records, incomplete, &mut out);
    if !role_rows(records, "actual").is_empty()
        && !records
            .iter()
            .any(|r| r.role == "budget" && get(r, "basis").eq_ignore_ascii_case("revised"))
    {
        out.gap(
            "Revised appropriations",
            "An appropriation ledger with revised rows (budget cycle 'revised') for the fund and fiscal year.",
            "Spending can be compared with an appropriation only when the ledger states the revised authority; an adopted-only ledger is not a legal limit.",
        );
    }
    out
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Approval {
    Approved,
    Negative,
    Unrecorded,
}

/// Only an explicit statement that no approval was obtained is a negative. A blank or
/// not-applicable cell is unrecorded, never a denial.
fn approval_state(value: &str) -> Approval {
    let words = super::phrase_match::words(value);
    let text: Vec<&str> = words.iter().map(|w| w.text.as_str()).collect();
    match text.as_slice() {
        [] => Approval::Unrecorded,
        ["na"] | ["n", "a"] | ["not", "applicable"] | ["not", "required"] | ["exempt"] => {
            Approval::Unrecorded
        }
        ["no"]
        | ["n"]
        | ["false"]
        | ["none"]
        | ["unapproved"]
        | ["denied"]
        | ["not", "approved"]
        | ["not", "obtained"]
        | ["pending"] => Approval::Negative,
        _ => Approval::Approved,
    }
}

#[derive(Clone, Copy)]
enum InForce {
    Yes,
    Unknown,
    No,
}

fn in_force(policy: &RuleDocument, on: Option<NaiveDate>) -> InForce {
    match on {
        None => InForce::Unknown,
        Some(day) if day >= policy.effective => InForce::Yes,
        Some(_) => InForce::No,
    }
}

fn rule_label(rule: &PolicyRule) -> String {
    format!(
        "'{}' states: \"{}\"",
        rule.document.name, rule.document.quote
    )
}

/// Case 4: rules the agency's own policy states, evaluated against the payment and contract
/// tables. `rules` come from policy documents of the same jurisdiction as the data.
pub fn rule_cases(
    records: &[FinancialRecord],
    incomplete: &BTreeSet<String>,
    rules: &[PolicyRule],
) -> SpendingOutcome {
    let mut out = SpendingOutcome::default();
    for (index, policy) in rules.iter().enumerate() {
        match policy.rule.kind {
            RuleKind::CardLimit => out.gap(
                "Purchasing-card transactions",
                "A transaction ledger that identifies purchasing-card transactions (a card or payment-method indicator) with amount and date.",
                "A policy states a purchasing-card limit, but no mapped table marks which payments were card purchases, so the limit cannot be applied to a row.",
            ),
            RuleKind::OvertimeAuthorization => out.gap(
                "Overtime pay with prior authorization",
                "A payroll or overtime ledger with hours or amounts per employee and pay period, and each overtime authorization with its date.",
                "A policy requires prior authorization for overtime, and no mapped role carries payroll or authorization records.",
            ),
            RuleKind::SplittingProhibited => {}
            RuleKind::ApprovalThreshold => {
                for (role, applies) in [("contract", policy.rule.contracts), ("payment", policy.rule.payments)] {
                    if applies {
                        threshold_cases(records, incomplete, rules, index, role, &mut out);
                    }
                }
                if policy.rule.payments {
                    split_cases(records, incomplete, rules, index, &mut out);
                }
            }
        }
    }
    out
}

fn bucket_total<'a>(bucket: &[(&'a FinancialRecord, i128)]) -> (i128, Vec<&'a FinancialRecord>) {
    let total: i128 = bucket.iter().map(|(_, v)| *v).sum();
    let rows: Vec<&'a FinancialRecord> = bucket.iter().map(|(r, _)| *r).collect();
    (total, capped(&rows))
}

fn threshold_cases(
    records: &[FinancialRecord],
    incomplete: &BTreeSet<String>,
    rules: &[PolicyRule],
    index: usize,
    role: &str,
    out: &mut SpendingOutcome,
) {
    let policy = &rules[index];
    let Some(limit) = policy.rule.limit else {
        return;
    };
    let rows = role_rows(records, role);
    if rows.is_empty() {
        return;
    }
    if incomplete.contains(role) {
        out.gap(
            &format!("Complete {role} register"),
            &format!("The {role} table retrieved in full for this fund and fiscal years."),
            "A truncated table can hide rows above the limit and rows that carry the approval.",
        );
        return;
    }
    if !has_value(&rows, "approval") {
        out.gap(
            &format!("Approval on {role} rows"),
            &format!("The {role} table with each row's approval (map approval on the {role} table: approval status, resolution or approver)."),
            "A policy requires approval above a stated amount, and no row of the mapped table records whether approval was obtained.",
        );
        return;
    }
    let candidates: Vec<&FinancialRecord> = if role == "payment" {
        settled_payments(&rows).0
    } else {
        rows.iter().copied().filter(|r| approved(r)).collect()
    };
    let date_field = if role == "payment" {
        "transaction_date"
    } else {
        "period_start"
    };
    let mut legal = Vec::new();
    let mut unrecorded = Vec::new();
    for row in candidates {
        if !get(row, "currency").eq_ignore_ascii_case("USD") {
            continue;
        }
        let Some(value) = amount(row, "amount").filter(|v| policy.rule.exceeded_by(*v)) else {
            continue;
        };
        let state = approval_state(get(row, "approval"));
        let force = in_force(&policy.document, date(row, date_field));
        match (state, force) {
            (Approval::Approved, _) | (_, InForce::No) => {}
            (Approval::Negative, InForce::Yes) => legal.push((row, value)),
            _ => unrecorded.push((row, value)),
        }
    }
    let comparator = if policy.rule.inclusive {
        "at or above"
    } else {
        "over"
    };
    let limit_text = display(limit);
    let subject = if role == "payment" {
        "posted payment"
    } else {
        "executed contract"
    };
    let date_note = if role == "payment" {
        "transaction date"
    } else {
        "contract start date"
    };
    let confidence = if policy.document.effective_stated {
        STATED_EFFECTIVE_CONFIDENCE
    } else {
        UPLOAD_DATE_CONFIDENCE
    };
    let effective_note = if policy.document.effective_stated {
        format!(
            "the policy's stated effective date {}",
            policy.document.effective
        )
    } else {
        format!(
            "the policy's upload date {} (it states no effective date)",
            policy.document.effective
        )
    };
    if !legal.is_empty() {
        let (total, cited) = bucket_total(&legal);
        out.cases.push(SpendingCase {
            finding: counted(
                APPROVAL_THRESHOLD_EXCEEDED,
                "Amounts above a stated approval threshold are recorded as not approved",
                format!(
                    "{} The {role} table records {} {subject} row(s) {comparator} {limit_text} USD whose approval value states that no approval was obtained, dated on or after {effective_note} ({date_note}). Together they record {} USD.",
                    rule_label(policy), legal.len(), display(total)
                ),
                &cited,
                &[
                    "The approval may have been obtained in a system that is not the mapped approval column.",
                    "The policy may contain an exception, emergency procedure or delegation the extracted sentence does not state.",
                    "The row's date may not be the date the purchase was committed.",
                ],
                "Obtain the approval record (council or board action, resolution or purchase approval) for each cited row and the policy version in force on its date.",
                vec![calc("Recorded amount above the limit without approval", format!("sum of {} cited rows", legal.len()), total, "USD")],
                confidence,
            ),
            rules: vec![index],
        });
    }
    if !unrecorded.is_empty() {
        let (total, cited) = bucket_total(&unrecorded);
        out.cases.push(SpendingCase {
            finding: classified(
                PATTERN,
                APPROVAL_RECORD_ABSENT,
                "Amounts above a stated approval threshold with no approval recorded",
                format!(
                    "{} The {role} table has {} {subject} row(s) {comparator} {limit_text} USD whose approval cell is blank or not applicable, or whose date does not establish that the policy was in force. Together they record {} USD. A blank cell is a missing record, not a statement that approval was not obtained.",
                    rule_label(policy), unrecorded.len(), display(total)
                ),
                &cited,
                &[
                    "The approval may be recorded elsewhere than the mapped column.",
                    "The table may not populate the approval column for this class of spending.",
                ],
                "Request the approval records for the cited rows; if they exist, re-import the table with the approval column populated.",
                vec![calc("Recorded amount above the limit without a recorded approval", format!("sum of {} cited rows", unrecorded.len()), total, "USD")],
                APPROVAL_BLANK_CONFIDENCE,
            ),
            rules: vec![index],
        });
    }
}

fn split_cases(
    records: &[FinancialRecord],
    incomplete: &BTreeSet<String>,
    rules: &[PolicyRule],
    index: usize,
    out: &mut SpendingOutcome,
) {
    let policy = &rules[index];
    let Some(limit) = policy.rule.limit else {
        return;
    };
    let payments = role_rows(records, "payment");
    if payments.is_empty() || incomplete.contains("payment") {
        return;
    }
    let (settled, _) = settled_payments(&payments);
    let usable: Vec<(&FinancialRecord, NaiveDate, i128)> = settled
        .iter()
        .filter(|r| {
            get(r, "currency").eq_ignore_ascii_case("USD") && !get(r, "vendor_id").is_empty()
        })
        .filter_map(|r| Some((*r, date(r, "transaction_date")?, amount(r, "amount")?)))
        .collect();
    if usable.is_empty() {
        out.gap(
            "Payment dates and vendors for split-purchase check",
            "The payment ledger with a transaction date, vendor identifier and amount on each posted payment.",
            "Clustering payments under a stated limit needs each payment's date and vendor.",
        );
        return;
    }
    let floor = limit - limit * SPLIT_MARGIN_BASIS_POINTS / 10_000;
    type Dated<'a> = (&'a FinancialRecord, NaiveDate, i128);
    let mut by_vendor: BTreeMap<(String, String), Vec<Dated>> = BTreeMap::new();
    for entry in usable {
        if !policy.rule.exceeded_by(entry.2) && entry.2 >= floor && entry.2 > 0 {
            by_vendor
                .entry(agency_key(entry.0, "vendor_id"))
                .or_default()
                .push(entry);
        }
    }
    let forbidden = rules.iter().enumerate().find(|(_, other)| {
        other.rule.kind == RuleKind::SplittingProhibited
            && other.document.document_id == policy.document.document_id
            && other.document.portal_key == policy.document.portal_key
    });
    for ((agency, vendor), mut entries) in by_vendor {
        entries.sort_by_key(|(r, d, _)| (*d, r.citation_id.clone()));
        let mut start = 0;
        while start < entries.len() {
            let anchor = entries[start].1;
            let end = entries[start..]
                .iter()
                .take_while(|(_, d, _)| (*d - anchor).num_days() <= SPLIT_WINDOW_DAYS)
                .count()
                + start;
            let cluster = &entries[start..end];
            let total: i128 = cluster.iter().map(|(_, _, v)| *v).sum();
            if cluster.len() < SPLIT_MIN_PAYMENTS || !policy.rule.exceeded_by(total) {
                start += 1;
                continue;
            }
            start = end;
            let last = cluster[cluster.len() - 1].1;
            let in_force_all = matches!(in_force(&policy.document, Some(anchor)), InForce::Yes);
            let rows: Vec<&FinancialRecord> = cluster.iter().map(|(r, _, _)| *r).collect();
            let comparator = if policy.rule.inclusive {
                "at or above"
            } else {
                "over"
            };
            let observation = format!(
                "{} {agency} vendor {vendor}: {} posted payments dated {anchor} through {last} each fall within {}% below the limit and sum to {} USD, {comparator} the stated {} USD. The limit, window of {SPLIT_WINDOW_DAYS} days and margin are fixed in this detector, not taken from the policy.",
                rule_label(policy), cluster.len(), SPLIT_MARGIN_BASIS_POINTS / 100, display(total), display(limit),
            );
            let calculations = vec![calc(
                "Cluster total",
                format!("sum of {} cited payments", cluster.len()),
                total,
                "USD",
            )];
            match forbidden.filter(|_| in_force_all) {
                Some((forbid_index, forbid)) => {
                    let item = counted(
                        SPLIT_PURCHASE_PROHIBITED,
                        "Payments just under an approval limit sum to more than it, and the policy forbids splitting",
                        format!("{observation} The same policy states: \"{}\". Whether these payments were one purchase divided is not recorded in the ledger.", forbid.document.quote),
                        &rows,
                        &[
                            "The payments may be separate legitimate purchases, installments or recurring services that happen to fall near the limit.",
                            "Each may have been approved separately, which the ledger does not show.",
                        ],
                        "Obtain the purchase requisitions, quotes and approvals for the cited payments and ask whether they were one need.",
                        calculations,
                        if policy.document.effective_stated { STATED_EFFECTIVE_CONFIDENCE } else { UPLOAD_DATE_CONFIDENCE },
                    );
                    out.cases.push(SpendingCase {
                        finding: item,
                        rules: vec![index, forbid_index],
                    });
                }
                None => {
                    let extra =
                        u16::try_from(cluster.len() - SPLIT_MIN_PAYMENTS).unwrap_or(u16::MAX);
                    let confidence = SPLIT_BASE_CONFIDENCE
                        .saturating_add(extra.saturating_mul(SPLIT_CONFIDENCE_PER_EXTRA_PAYMENT))
                        .min(SPLIT_CONFIDENCE_CAP);
                    out.cases.push(SpendingCase {
                        finding: classified(
                            PATTERN,
                            SPLIT_PURCHASE_PATTERN,
                            "Payments clustered just under a stated approval limit",
                            format!("{observation} The policy sentence cited does not forbid splitting, so this is a pattern, not a rule breach."),
                            &rows,
                            &[
                                "The payments may be separate legitimate purchases, installments or recurring services.",
                                "Payments near a round limit are common when prices are set just under it.",
                            ],
                            "Obtain the purchase requisitions and approvals for the cited payments and check whether the vendor's invoices describe one need.",
                            calculations,
                            confidence,
                        ),
                        rules: vec![index],
                    });
                }
            }
        }
    }
}

/// The financial role whose table would resolve a gap this module raised, by exact title.
pub fn gap_role(title: &str) -> Option<&'static str> {
    Some(match title {
        "Contract start and end dates"
        | "Complete contract register"
        | "Approval on contract rows" => "contract",
        "Executed amendments stating term changes" => "amendment",
        "Payments in the contract's currency" | "Restricted-fund spending in one currency" => {
            "payment"
        }
        "Complete restriction register and actual expenditures"
        | "Unambiguous restriction classification"
        | "Expressly eligible programs for restricted funds" => "restriction",
        "Revised appropriations" => "budget",
        "Complete contract and payment registers"
        | "Payment transaction dates"
        | "Program on actual expenditures"
        | "Purchasing-card transactions"
        | "Overtime pay with prior authorization"
        | "Complete payment register"
        | "Approval on payment rows"
        | "Payment dates and vendors for split-purchase check"
        | "Jurisdiction of the policy and the ledger"
        | "Origin document for cited ledger rows" => "payment",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::financial::{analyze, decimal};
    use crate::analysis::spending_rules::extract_rules;
    use crate::types::{FinancialReport, FinancialScope};

    fn rec(role: &str, id: &str, extra: &[(&str, &str)]) -> FinancialRecord {
        let mut values: BTreeMap<String, Option<String>> = [
            ("fy", "2025"),
            ("fund", "100000"),
            ("currency", "USD"),
            ("agency", "City"),
        ]
        .into_iter()
        .map(|(k, v)| (k.into(), Some(v.into())))
        .collect();
        for (k, v) in extra {
            values.insert((*k).into(), Some((*v).into()));
        }
        FinancialRecord {
            role: role.into(),
            values,
            source_rows: 1,
            citation_id: id.into(),
        }
    }

    fn contract() -> FinancialRecord {
        rec(
            "contract",
            "contract",
            &[
                ("contract_id", "C1"),
                ("vendor_id", "V1"),
                ("amount", "100000"),
                ("status", "executed"),
                ("period_start", "2025-01-01"),
                ("period_end", "2025-06-30"),
            ],
        )
    }

    fn amendment(end: &str) -> FinancialRecord {
        rec(
            "amendment",
            "amend",
            &[
                ("contract_id", "C1"),
                ("amendment_id", "A1"),
                ("amount", "0"),
                ("status", "executed"),
                ("period_end", end),
            ],
        )
    }

    fn payment(id: &str, date: &str, amount: &str) -> FinancialRecord {
        rec(
            "payment",
            id,
            &[
                ("contract_id", "C1"),
                ("vendor_id", "V1"),
                ("transaction_id", id),
                ("invoice_id", id),
                ("amount", amount),
                ("status", "paid"),
                ("transaction_date", date),
            ],
        )
    }

    fn kinds(outcome: &SpendingOutcome) -> Vec<&str> {
        outcome
            .cases
            .iter()
            .map(|c| c.finding.kind.as_str())
            .collect()
    }

    fn none() -> BTreeSet<String> {
        BTreeSet::new()
    }

    #[test]
    fn a_payment_outside_the_term_is_flagged_at_one_day_and_not_at_the_boundary() {
        let cases = |payments: Vec<FinancialRecord>| {
            let mut records = vec![contract(), amendment("2025-06-30")];
            records.extend(payments);
            ledger_cases(&records, &none())
        };
        let on_edges = cases(vec![
            payment("p1", "2025-01-01", "10"),
            payment("p2", "2025-06-30", "10"),
        ]);
        assert!(kinds(&on_edges).is_empty());
        let late = cases(vec![payment("p1", "2025-07-01", "10")]);
        assert_eq!(kinds(&late), [PAYMENT_OUTSIDE_TERM]);
        assert_eq!(late.cases[0].finding.basis, COUNTED);
        assert!(late.cases[0].finding.citations.contains(&"p1".to_string()));
        assert!(late.cases[0]
            .finding
            .citations
            .contains(&"contract".to_string()));
        let early = cases(vec![payment("p1", "2024-12-31", "10")]);
        assert_eq!(kinds(&early), [PAYMENT_OUTSIDE_TERM]);
    }

    #[test]
    fn an_amendment_that_extends_the_end_date_moves_the_boundary() {
        let records = vec![
            contract(),
            amendment("2025-12-31"),
            payment("p1", "2025-09-15", "10"),
        ];
        assert!(kinds(&ledger_cases(&records, &none())).is_empty());
        let records = vec![
            contract(),
            amendment("2025-12-31"),
            payment("p1", "2026-01-01", "10"),
        ];
        assert_eq!(
            kinds(&ledger_cases(&records, &none())),
            [PAYMENT_OUTSIDE_TERM]
        );
    }

    #[test]
    fn without_a_complete_amendment_register_a_late_payment_is_a_gap_not_a_finding() {
        let records = vec![contract(), payment("p1", "2025-09-01", "10")];
        let outcome = ledger_cases(&records, &none());
        assert!(kinds(&outcome).is_empty());
        assert!(outcome
            .gaps
            .iter()
            .any(|g| g.title == "Executed amendments stating term changes"));
        // A payment before the start needs no amendment register.
        let records = vec![contract(), payment("p1", "2024-12-31", "10")];
        assert_eq!(
            kinds(&ledger_cases(&records, &none())),
            [PAYMENT_OUTSIDE_TERM]
        );
        // An amendment with no stated end date does not establish the term either.
        let mut blank = amendment("");
        blank.values.insert("period_end".into(), None);
        let records = vec![contract(), blank, payment("p1", "2025-09-01", "10")];
        assert!(kinds(&ledger_cases(&records, &none())).is_empty());
    }

    #[test]
    fn missing_dates_are_named_gaps() {
        let mut undated = payment("p1", "2025-09-01", "10");
        undated.values.insert("transaction_date".into(), None);
        let outcome = ledger_cases(&[contract(), amendment("2025-06-30"), undated], &none());
        assert!(kinds(&outcome).is_empty());
        assert!(outcome
            .gaps
            .iter()
            .any(|g| g.title == "Payment transaction dates"));
        let mut no_term = contract();
        no_term.values.insert("period_end".into(), None);
        let outcome = ledger_cases(&[no_term, payment("p1", "2025-09-01", "10")], &none());
        assert!(outcome
            .gaps
            .iter()
            .any(|g| g.title == "Contract start and end dates"));
    }

    /// A total is a figure in one currency. Payments to one vendor in two currencies are two
    /// totals, and payments outside a contract's term in another currency are not summed against it.
    #[test]
    fn spending_totals_never_add_amounts_across_currencies() {
        let stray = |id: &str, currency: &str| {
            let mut row = payment(id, "2025-03-01", "500");
            row.values.insert("vendor_id".into(), Some("V9".into()));
            row.values.insert("contract_id".into(), Some("C9".into()));
            row.values.insert("currency".into(), Some(currency.into()));
            row
        };
        let orphans = ledger_cases(
            &[contract(), stray("p8", "USD"), stray("p9", "CAD")],
            &none(),
        );
        let mut totals = orphans
            .cases
            .iter()
            .map(|case| {
                let calculation = &case.finding.calculations[0];
                (calculation.unit.clone(), calculation.result.clone())
            })
            .collect::<Vec<_>>();
        totals.sort();
        let single = ledger_cases(&[contract(), stray("p8", "USD")], &none());
        let one_payment = single.cases[0].finding.calculations[0].result.clone();
        assert_eq!(
            totals,
            [
                ("CAD".to_string(), one_payment.clone()),
                ("USD".to_string(), one_payment)
            ]
        );

        let mut late = payment("p1", "2025-07-01", "10");
        late.values.insert("currency".into(), Some("CAD".into()));
        let mixed = ledger_cases(&[contract(), amendment("2025-06-30"), late], &none());
        assert!(kinds(&mixed).is_empty());
        assert!(mixed
            .gaps
            .iter()
            .any(|gap| gap.title == "Payments in the contract's currency"));
    }

    #[test]
    fn a_vendor_absent_from_a_complete_register_is_a_labelled_pattern() {
        let mut stray = payment("p9", "2025-03-01", "500");
        stray.values.insert("vendor_id".into(), Some("V9".into()));
        stray.values.insert("contract_id".into(), Some("C9".into()));
        let outcome = ledger_cases(
            &[contract(), payment("p1", "2025-03-01", "10"), stray.clone()],
            &none(),
        );
        assert_eq!(kinds(&outcome), [PAYMENT_WITHOUT_CONTRACT]);
        assert_eq!(outcome.cases[0].finding.basis, PATTERN);
        // Same vendor under another contract id is not a missing vendor.
        let mut known = stray.clone();
        known.values.insert("vendor_id".into(), Some("V1".into()));
        assert!(kinds(&ledger_cases(&[contract(), known], &none())).is_empty());
        // A voided payment and a credit that nets to zero are not payments to a vendor.
        let mut voided = stray.clone();
        voided.values.insert("status".into(), Some("void".into()));
        assert!(kinds(&ledger_cases(&[contract(), voided], &none())).is_empty());
        // An incomplete register is a gap, never a finding.
        let incomplete = BTreeSet::from(["contract".to_string()]);
        let outcome = ledger_cases(&[contract(), stray], &incomplete);
        assert!(kinds(&outcome).is_empty());
        assert!(!outcome.gaps.is_empty());
    }

    fn report(records: Vec<FinancialRecord>) -> FinancialReport {
        analyze(
            FinancialScope {
                fiscal_year: Some(2025),
                fund: Some("100000".into()),
                department: None,
            },
            records,
            vec![],
            BTreeMap::new(),
            vec![],
            &none(),
        )
    }

    fn ledger(role: &str, id: &str, amount: &str, extra: &[(&str, &str)]) -> FinancialRecord {
        let mut fields = vec![("account", "5001"), ("center", "001"), ("amount", amount)];
        fields.extend_from_slice(extra);
        rec(role, id, &fields)
    }

    fn appropriation_kinds(report: &FinancialReport) -> Vec<&str> {
        report
            .findings
            .iter()
            .map(|f| f.kind.as_str())
            .filter(|k| *k == APPROPRIATION_EXCEEDED)
            .collect()
    }

    #[test]
    fn spending_above_revised_appropriation_is_flagged_one_cent_over_and_not_at_the_limit() {
        let revised = ledger("budget", "rev", "1000.00", &[("basis", "revised")]);
        for (actual, expected) in [("999.99", 0), ("1000.00", 0), ("1000.01", 1)] {
            let r = report(vec![revised.clone(), ledger("actual", "act", actual, &[])]);
            assert_eq!(appropriation_kinds(&r).len(), expected, "{actual}");
        }
        let r = report(vec![revised, ledger("actual", "act", "1000.01", &[])]);
        let case = r
            .findings
            .iter()
            .find(|f| f.kind == APPROPRIATION_EXCEEDED)
            .unwrap();
        assert_eq!(case.basis, COUNTED);
        assert_eq!(case.confidence_basis_points, Some(LEDGER_CONFIDENCE));
        assert_eq!(case.calculations[0].result, "0.01");
        assert!(
            case.citations.contains(&"rev".to_string())
                && case.citations.contains(&"act".to_string())
        );
    }

    #[test]
    fn other_funds_and_years_are_not_compared() {
        let r = report(vec![
            ledger("budget", "rev", "1000", &[("basis", "revised")]),
            ledger("actual", "other-fund", "5000", &[("fund", "200000")]),
            ledger("actual", "other-year", "5000", &[("fy", "2024")]),
            ledger("actual", "other-account", "5000", &[("account", "5002")]),
        ]);
        assert!(appropriation_kinds(&r).is_empty());
    }

    #[test]
    fn an_adopted_only_ledger_is_a_gap_and_a_conflicting_crosswalk_blocks() {
        let r = report(vec![
            ledger("budget", "adopted", "1000", &[("basis", "adopted")]),
            ledger("actual", "act", "9000", &[]),
        ]);
        assert!(appropriation_kinds(&r).is_empty());
        assert!(r.gaps.iter().any(|g| g.title == "Revised appropriations"));
        let crosswalk = |id: &str, target: &str| {
            rec(
                "crosswalk",
                id,
                &[
                    ("account", "5001"),
                    ("canonical_account", target),
                    ("end_fy", "2025"),
                ],
            )
        };
        let records = |walks: Vec<FinancialRecord>| {
            let mut all = vec![
                ledger("budget", "rev", "1000", &[("basis", "revised")]),
                ledger("actual", "act", "2000", &[]),
            ];
            all.extend(walks);
            all
        };
        assert_eq!(
            appropriation_kinds(&report(records(vec![crosswalk("w1", "A")]))).len(),
            1
        );
        assert!(appropriation_kinds(&report(records(vec![
            crosswalk("w1", "A"),
            crosswalk("w2", "B")
        ])))
        .is_empty());
    }

    #[test]
    fn encumbrances_add_to_actuals_at_lower_confidence() {
        let r = report(vec![
            ledger("budget", "rev", "1000", &[("basis", "revised")]),
            ledger("actual", "act", "600", &[]),
            ledger("encumbrance", "enc", "400.01", &[]),
        ]);
        let case = r
            .findings
            .iter()
            .find(|f| f.kind == APPROPRIATION_EXCEEDED)
            .unwrap();
        assert_eq!(case.confidence_basis_points, Some(ENCUMBERED_CONFIDENCE));
        assert!(case.citations.contains(&"enc".to_string()));
        let r = report(vec![
            ledger("budget", "rev", "1000", &[("basis", "revised")]),
            ledger("actual", "act", "600", &[]),
            ledger("encumbrance", "enc", "400", &[]),
        ]);
        assert!(appropriation_kinds(&r).is_empty());
    }

    fn restriction(class: &str, program: &str) -> FinancialRecord {
        rec(
            "restriction",
            &format!("res-{class}-{program}"),
            &[("restriction", class), ("eligible_program", program)],
        )
    }

    fn spent(id: &str, program: &str, amount: &str, extra: &[(&str, &str)]) -> FinancialRecord {
        let mut fields = vec![("program", program), ("amount", amount)];
        fields.extend_from_slice(extra);
        rec("actual", id, &fields)
    }

    #[test]
    fn restricted_spending_is_compared_by_exact_program_in_the_same_fund_and_year() {
        let records = vec![
            restriction("restricted", "Shelter"),
            spent("ok", "Shelter", "100", &[]),
            spent("bad", "Stadium", "250.50", &[]),
            spent("blank", "", "999", &[]),
            spent("other-fund", "Stadium", "999", &[("fund", "200000")]),
            spent("other-year", "Stadium", "999", &[("fy", "2024")]),
            spent("case-differs", "shelter", "1", &[]),
        ];
        let outcome = ledger_cases(&records, &none());
        let restricted: Vec<_> = outcome
            .cases
            .iter()
            .filter(|c| c.finding.kind == RESTRICTED_OFF_PROGRAM)
            .collect();
        assert_eq!(restricted.len(), 1);
        let finding = &restricted[0].finding;
        assert_eq!(finding.calculations[0].result, "251.50");
        assert!(finding.citations.contains(&"bad".to_string()));
        assert!(!finding.citations.contains(&"ok".to_string()));
        assert!(!finding.citations.contains(&"other-fund".to_string()));
        let clean = vec![
            restriction("restricted", "Shelter"),
            spent("ok", "Shelter", "100", &[]),
        ];
        assert!(ledger_cases(&clean, &none()).cases.is_empty());
    }

    #[test]
    fn unclear_restrictions_and_missing_programs_are_gaps() {
        let mixed = vec![
            restriction("restricted", "Shelter"),
            restriction("unrestricted", ""),
            spent("bad", "Stadium", "5", &[]),
        ];
        let outcome = ledger_cases(&mixed, &none());
        assert!(outcome.cases.is_empty());
        assert!(outcome
            .gaps
            .iter()
            .any(|g| g.title == "Unambiguous restriction classification"));
        let no_programs = vec![
            restriction("restricted", ""),
            spent("bad", "Stadium", "5", &[]),
        ];
        assert!(ledger_cases(&no_programs, &none())
            .gaps
            .iter()
            .any(|g| g.title == "Expressly eligible programs for restricted funds"));
        let mut blank = spent("a", "", "5", &[]);
        blank.values.insert("program".into(), None);
        let outcome = ledger_cases(&[restriction("restricted", "Shelter"), blank], &none());
        assert!(outcome
            .gaps
            .iter()
            .any(|g| g.title == "Program on actual expenditures"));
    }

    fn policy(text: &str, stated: bool) -> Vec<PolicyRule> {
        let effective = NaiveDate::from_ymd_opt(2025, 1, 1).unwrap();
        extract_rules(text)
            .into_iter()
            .map(|rule| PolicyRule {
                document: RuleDocument {
                    portal_key: "city".into(),
                    document_id: 7,
                    name: "Purchasing policy".into(),
                    quote: text[rule.byte_start..rule.byte_end].to_string(),
                    char_start: rule.byte_start,
                    char_end: rule.byte_end,
                    effective,
                    effective_stated: stated,
                },
                rule,
            })
            .collect()
    }

    fn approved_payment(
        id: &str,
        date: &str,
        amount: &str,
        approval: Option<&str>,
    ) -> FinancialRecord {
        let mut row = payment(id, date, amount);
        row.values
            .insert("approval".into(), approval.map(String::from));
        row
    }

    const OVER_25K: &str = "Purchases over $25,000 require council approval.";

    #[test]
    fn a_stated_limit_is_held_against_rows_at_limit_minus_one_limit_and_limit_plus_one() {
        let rules = policy(OVER_25K, true);
        let limit = decimal("25000").unwrap();
        for (amount, expect_case) in [("24999.99", false), ("25000", false), ("25000.01", true)] {
            let records = vec![
                approved_payment("p", "2025-03-01", amount, Some("no")),
                approved_payment("witness", "2025-03-02", "1", Some("approved")),
            ];
            let outcome = rule_cases(&records, &none(), &rules);
            assert_eq!(
                kinds(&outcome).contains(&APPROVAL_THRESHOLD_EXCEEDED),
                expect_case,
                "{amount} vs {limit}"
            );
        }
        let records = vec![approved_payment("p", "2025-03-01", "30000", Some("no"))];
        let case = &rule_cases(&records, &none(), &rules).cases[0];
        assert_eq!(case.finding.basis, COUNTED);
        assert_eq!(
            case.finding.confidence_basis_points,
            Some(STATED_EFFECTIVE_CONFIDENCE)
        );
        assert_eq!(case.rules, [0]);
        let unstated = policy(OVER_25K, false);
        let case = &rule_cases(&records, &none(), &unstated).cases[0];
        assert_eq!(
            case.finding.confidence_basis_points,
            Some(UPLOAD_DATE_CONFIDENCE)
        );
    }

    #[test]
    fn only_an_explicit_no_approval_is_a_rule_breach() {
        let rules = policy(OVER_25K, true);
        let run = |approval: Option<&str>, date: &str| {
            rule_cases(
                &[
                    approved_payment("p", date, "30000", approval),
                    approved_payment("witness", "2025-03-02", "1", Some("approved")),
                ],
                &none(),
                &rules,
            )
        };
        assert_eq!(
            kinds(&run(Some("Council Res 2025-14"), "2025-03-01")),
            Vec::<&str>::new()
        );
        assert_eq!(kinds(&run(None, "2025-03-01")), [APPROVAL_RECORD_ABSENT]);
        assert_eq!(
            kinds(&run(Some("N/A"), "2025-03-01")),
            [APPROVAL_RECORD_ABSENT]
        );
        assert_eq!(
            kinds(&run(Some("Not approved"), "2025-03-01")),
            [APPROVAL_THRESHOLD_EXCEEDED]
        );
        assert_eq!(run(None, "2025-03-01").cases[0].finding.basis, PATTERN);
        // Before the policy took effect the rule is not held against the row; an undated row
        // cannot show the policy in force, so an explicit no is only a pattern.
        assert!(kinds(&run(Some("no"), "2024-12-31")).is_empty());
        assert_eq!(
            kinds(&run(Some("no"), "2025-01-01")),
            [APPROVAL_THRESHOLD_EXCEEDED]
        );
        let mut undated = approved_payment("p", "2025-03-01", "30000", Some("no"));
        undated.values.insert("transaction_date".into(), None);
        assert_eq!(
            kinds(&rule_cases(&[undated], &none(), &rules)),
            [APPROVAL_RECORD_ABSENT]
        );
    }

    #[test]
    fn a_table_with_no_approval_values_is_a_gap_and_card_and_overtime_rules_are_gaps() {
        let rules = policy(OVER_25K, true);
        let records = vec![approved_payment("p", "2025-03-01", "30000", None)];
        let outcome = rule_cases(&records, &none(), &rules);
        assert!(outcome.cases.is_empty());
        assert!(outcome
            .gaps
            .iter()
            .any(|g| g.title == "Approval on payment rows"));
        let mut rules = policy("The purchasing card limit is $5,000 per transaction.", true);
        rules.extend(policy("Overtime requires prior authorization.", true));
        let outcome = rule_cases(&[], &none(), &rules);
        let titles: Vec<&str> = outcome.gaps.iter().map(|g| g.title.as_str()).collect();
        assert_eq!(
            titles,
            [
                "Purchasing-card transactions",
                "Overtime pay with prior authorization"
            ]
        );
    }

    #[test]
    fn an_inclusive_contract_threshold_covers_the_stated_amount() {
        let rules = policy(
            "Contracts of $50,000 or more must be approved by the Board.",
            true,
        );
        let contract_at = |amount: &str| {
            let mut row = contract();
            row.values.insert("amount".into(), Some(amount.into()));
            row.values.insert("approval".into(), Some("no".into()));
            row
        };
        assert_eq!(
            kinds(&rule_cases(&[contract_at("49999.99")], &none(), &rules)),
            Vec::<&str>::new()
        );
        assert_eq!(
            kinds(&rule_cases(&[contract_at("50000")], &none(), &rules)),
            [APPROVAL_THRESHOLD_EXCEEDED]
        );
    }

    fn split_payments(amounts: &[(&str, &str)], vendor: &str) -> Vec<FinancialRecord> {
        amounts
            .iter()
            .enumerate()
            .map(|(i, (date, amount))| {
                let mut row = payment(&format!("s{i}"), date, amount);
                row.values.insert("vendor_id".into(), Some(vendor.into()));
                row
            })
            .collect()
    }

    const OVER_5K: &str = "Purchases over $5,000 require council approval.";

    #[test]
    fn clustered_payments_under_a_limit_are_a_pattern_unless_the_policy_forbids_splitting() {
        let rules = policy(OVER_5K, true);
        let cluster = split_payments(
            &[
                ("2025-03-01", "4600"),
                ("2025-03-08", "4700"),
                ("2025-03-15", "4800"),
            ],
            "V2",
        );
        let outcome = rule_cases(&cluster, &none(), &rules);
        assert_eq!(kinds(&outcome), [SPLIT_PURCHASE_PATTERN]);
        assert_eq!(outcome.cases[0].finding.basis, PATTERN);
        assert_eq!(
            outcome.cases[0].finding.confidence_basis_points,
            Some(4_000)
        );
        let mut forbidding = policy(OVER_5K, true);
        forbidding.extend(policy(
            "Splitting a purchase to avoid the approval limit is prohibited.",
            true,
        ));
        let outcome = rule_cases(&cluster, &none(), &forbidding);
        assert_eq!(kinds(&outcome), [SPLIT_PURCHASE_PROHIBITED]);
        assert_eq!(outcome.cases[0].rules, [0, 1]);
        assert_eq!(outcome.cases[0].finding.basis, COUNTED);
    }

    #[test]
    fn the_cluster_window_margin_count_and_sum_are_exact_boundaries() {
        let rules = policy(OVER_5K, true);
        let run = |payments: &[(&str, &str)]| {
            kinds(&rule_cases(
                &split_payments(payments, "V2"),
                &none(),
                &rules,
            ))
            .len()
        };
        // Window: day 14 is inside, day 15 is outside.
        assert_eq!(
            run(&[
                ("2025-03-01", "4600"),
                ("2025-03-08", "4600"),
                ("2025-03-15", "4600")
            ]),
            1
        );
        assert_eq!(
            run(&[
                ("2025-03-01", "4600"),
                ("2025-03-08", "4600"),
                ("2025-03-16", "4600")
            ]),
            0
        );
        // Margin: 10% below is 4500; one cent below is not near the limit.
        assert_eq!(
            run(&[
                ("2025-03-01", "4500"),
                ("2025-03-02", "4500"),
                ("2025-03-03", "4500")
            ]),
            1
        );
        assert_eq!(
            run(&[
                ("2025-03-01", "4499.99"),
                ("2025-03-02", "4500"),
                ("2025-03-03", "4500")
            ]),
            0
        );
        // A payment at the limit is not over it, so it is part of a cluster; over it is not.
        assert_eq!(
            run(&[
                ("2025-03-01", "5000"),
                ("2025-03-02", "4600"),
                ("2025-03-03", "4600")
            ]),
            1
        );
        assert_eq!(
            run(&[
                ("2025-03-01", "5000.01"),
                ("2025-03-02", "4600"),
                ("2025-03-03", "4600")
            ]),
            0
        );
        // Count: two payments are not a cluster.
        assert_eq!(run(&[("2025-03-01", "4600"), ("2025-03-02", "4600")]), 0);
        // Different vendors never cluster together.
        let mut mixed = split_payments(&[("2025-03-01", "4600"), ("2025-03-02", "4600")], "V2");
        mixed.extend(
            split_payments(&[("2025-03-03", "4600")], "V3")
                .into_iter()
                .map(|mut r| {
                    r.citation_id = "other".into();
                    r.values
                        .insert("transaction_id".into(), Some("other".into()));
                    r
                }),
        );
        assert!(rule_cases(&mixed, &none(), &rules).cases.is_empty());
    }

    #[test]
    fn a_clean_ledger_produces_no_cases_and_no_false_gaps_for_present_roles() {
        let records = vec![
            contract(),
            amendment("2025-06-30"),
            payment("p1", "2025-02-01", "100"),
            ledger("budget", "rev", "1000", &[("basis", "revised")]),
            ledger("actual", "act", "900", &[]),
            restriction("restricted", "Shelter"),
            spent("s", "Shelter", "10", &[("account", "5003")]),
        ];
        let outcome = ledger_cases(&records, &none());
        assert!(outcome.cases.is_empty(), "{:?}", kinds(&outcome));
        let r = report(records);
        assert!(
            r.findings
                .iter()
                .all(|f| f.basis == "lead" || f.kind == "deliverables_gap"),
            "{:?}",
            r.findings.iter().map(|f| &f.kind).collect::<Vec<_>>()
        );
    }

    #[test]
    fn the_contract_ceiling_case_carries_its_basis_and_confidence() {
        let mut records = vec![contract(), amendment("2025-06-30")];
        records.extend([
            payment("p1", "2025-02-01", "60000"),
            payment("p2", "2025-03-01", "40000.01"),
        ]);
        let r = report(records);
        let case = r
            .findings
            .iter()
            .find(|f| f.kind == CEILING_EXCEEDED)
            .expect("ceiling case");
        assert_eq!(case.basis, COUNTED);
        assert_eq!(case.confidence_basis_points, Some(LEDGER_CONFIDENCE));
        assert_eq!(case.calculations[0].result, "0.01");
        // At the ceiling exactly there is no case.
        let r = report(vec![
            contract(),
            amendment("2025-06-30"),
            payment("p1", "2025-02-01", "100000"),
        ]);
        assert!(r.findings.iter().all(|f| f.kind != CEILING_EXCEEDED));
    }

    #[test]
    fn every_gap_title_raised_has_a_role() {
        let rules = {
            let mut all = policy("The purchasing card limit is $5,000 per transaction.", true);
            all.extend(policy("Overtime requires prior authorization.", true));
            all
        };
        let mut gaps = rule_cases(&[], &none(), &rules).gaps;
        gaps.extend(ledger_cases(&[ledger("actual", "a", "1", &[])], &none()).gaps);
        let mut late = payment("p1", "2025-07-01", "10");
        late.values.insert("currency".into(), Some("CAD".into()));
        gaps.extend(ledger_cases(&[contract(), amendment("2025-06-30"), late], &none()).gaps);
        let restricted = [
            restriction("restricted", "Shelter"),
            spent("a", "Stadium", "5", &[]),
            spent("b", "Stadium", "5", &[("currency", "CAD")]),
        ];
        gaps.extend(ledger_cases(&restricted, &none()).gaps);
        for title in [
            "Payments in the contract's currency",
            "Restricted-fund spending in one currency",
        ] {
            assert!(gaps.iter().any(|gap| gap.title == title), "{title}");
        }
        for gap in gaps {
            assert!(gap_role(&gap.title).is_some(), "{}", gap.title);
        }
    }
}
