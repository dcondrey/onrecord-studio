//! Deterministic, source-bound financial investigations. Amounts never pass through f64.
use crate::{error::AppError, integrity::sha256_hex, types::*};
use std::collections::{BTreeMap, BTreeSet};
const SCALE: i128 = 10_000;

pub fn decimal(value: &str) -> Option<i128> {
    let value = value.trim();
    let (negative, raw) = value
        .strip_prefix('-')
        .map_or((false, value), |v| (true, v));
    let raw = raw.strip_prefix('+').unwrap_or(raw);
    let mut parts = raw.split('.');
    let whole = parts.next()?;
    let fractional = parts.next().unwrap_or("");
    if parts.next().is_some()
        || whole.is_empty()
        || fractional.len() > 4
        || !whole.chars().all(|c| c.is_ascii_digit())
        || !fractional.chars().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    let amount = whole
        .parse::<i128>()
        .ok()?
        .checked_mul(SCALE)?
        .checked_add(if fractional.is_empty() {
            0
        } else {
            fractional
                .parse::<i128>()
                .ok()?
                .checked_mul(10_i128.pow((4 - fractional.len()) as u32))?
        })?;
    if negative {
        amount.checked_neg()
    } else {
        Some(amount)
    }
}
pub fn display(value: i128) -> String {
    let sign = if value < 0 { "-" } else { "" };
    let value = value.unsigned_abs();
    let mut fraction = format!("{:04}", value % SCALE as u128);
    while fraction.len() > 2 && fraction.ends_with('0') {
        fraction.pop();
    }
    format!("{sign}{}.{fraction}", value / SCALE as u128)
}
fn get<'a>(record: &'a FinancialRecord, key: &str) -> &'a str {
    record
        .values
        .get(key)
        .and_then(Option::as_deref)
        .unwrap_or("")
        .trim()
}
fn amount(record: &FinancialRecord, field: &str) -> Option<i128> {
    decimal(get(record, field))
}
fn funding_duration(record: &FinancialRecord) -> &'static str {
    match get(record, "funding_duration")
        .to_ascii_lowercase()
        .as_str()
    {
        "one_time" | "one-time" | "one time" | "non-recurring" | "nonrecurring" => "one_time",
        "recurring" | "ongoing" => "recurring",
        "mixed" => "mixed",
        _ => "unknown",
    }
}
fn sum(records: &[&FinancialRecord], field: &str) -> Option<i128> {
    if records.is_empty() {
        return None;
    }
    records.iter().try_fold(0_i128, |total, record| {
        total.checked_add(amount(record, field)?)
    })
}
fn ids(records: &[&FinancialRecord]) -> Vec<String> {
    records
        .iter()
        .map(|r| r.citation_id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}
fn formatted(value: Option<i128>) -> Option<String> {
    value.map(display)
}
fn key(values: &[&str]) -> String {
    sha256_hex(values.join("\u{1f}").as_bytes())[..20].into()
}
fn finding(
    kind: &str,
    title: &str,
    observation: String,
    records: &[&FinancialRecord],
    alternatives: &[&str],
    next_test: &str,
) -> FinancialAuditFinding {
    FinancialAuditFinding {
        id: key(&[kind, &observation, &ids(records).join("|")]),
        kind: kind.into(),
        title: title.into(),
        observation,
        citations: ids(records),
        alternatives: alternatives.iter().map(|s| (*s).into()).collect(),
        next_test: next_test.into(),
        calculations: vec![],
    }
}

fn canonical_account(
    record: &FinancialRecord,
    crosswalks: &[&FinancialRecord],
) -> (String, Vec<String>, bool) {
    let fy = get(record, "fy").parse::<i32>().ok();
    let matched = crosswalks
        .iter()
        .filter(|w| {
            get(w, "agency") == get(record, "agency")
                && get(w, "account") == get(record, "account")
                && get(w, "fund") == get(record, "fund")
                && fy
                    .zip(get(w, "fy").parse::<i32>().ok())
                    .is_some_and(|(fy, start)| {
                        fy >= start && fy <= get(w, "end_fy").parse().unwrap_or(start)
                    })
        })
        .copied()
        .collect::<Vec<_>>();
    let targets = matched
        .iter()
        .map(|r| get(r, "canonical_account"))
        .filter(|s| !s.is_empty())
        .collect::<BTreeSet<_>>();
    match targets.len() {
        0 => (get(record, "account").into(), vec![], false),
        1 => (
            targets.into_iter().next().unwrap().into(),
            ids(&matched),
            false,
        ),
        _ => (get(record, "account").into(), ids(&matched), true),
    }
}

fn calculation(label: &str, formula: String, value: i128, unit: &str) -> FinancialCalculation {
    FinancialCalculation {
        label: label.into(),
        formula,
        result: display(value),
        unit: unit.into(),
    }
}
fn percent(numerator: i128, denominator: i128) -> Option<i128> {
    if denominator <= 0 {
        return None;
    }
    numerator.checked_mul(100 * SCALE)?.checked_div(denominator)
}

// Opposite movements within an account family are a falsification test for isolated variance
// stories, not an inferred transfer. The journal/contract bridge remains missing evidence.
fn account_shift_findings(
    lines: &[FinancialReconciliation],
    records: &[FinancialRecord],
) -> Vec<FinancialAuditFinding> {
    let mut groups: BTreeMap<Vec<String>, Vec<&FinancialReconciliation>> = BTreeMap::new();
    for line in lines {
        if line.center.is_empty()
            || line.agency.is_empty()
            || line.currency.is_empty()
            || line.account.len() < 3
            || !line.account.bytes().all(|c| c.is_ascii_digit())
            || line.actual_minus_adopted.is_none()
        {
            continue;
        }
        groups
            .entry(vec![
                line.agency.clone(),
                line.fiscal_year.clone(),
                line.fund.clone(),
                line.center.clone(),
                line.program.clone(),
                line.currency.clone(),
                line.funding_duration.clone(),
                line.account[..3].into(),
            ])
            .or_default()
            .push(line);
    }
    let mut found = vec![];
    for group in groups.values() {
        let mut positive = vec![];
        let mut negative = vec![];
        for line in group {
            let Some(delta) = line.actual_minus_adopted.as_deref().and_then(decimal) else {
                continue;
            };
            if delta > 0 {
                positive.push((*line, delta));
            }
            if delta < 0 {
                negative.push((*line, delta));
            }
        }
        positive.sort_by_key(|(_, value)| std::cmp::Reverse(*value));
        negative.sort_by_key(|(_, value)| *value);
        let mut best = None;
        for (down, reduction) in negative.iter().take(16) {
            for (up, increase) in positive.iter().take(16) {
                let Some(reduction) = reduction.checked_neg() else {
                    continue;
                };
                let (Some(down_budget), Some(up_budget), Some(down_actual), Some(up_actual)) = (
                    down.adopted.as_deref().and_then(decimal),
                    up.adopted.as_deref().and_then(decimal),
                    down.actual.as_deref().and_then(decimal),
                    up.actual.as_deref().and_then(decimal),
                ) else {
                    continue;
                };
                if down_budget <= 0 || up_budget <= 0 || down_actual < 0 || up_actual < 0 {
                    continue;
                }
                let (Some(planned), Some(spent)) = (
                    down_budget.checked_add(up_budget),
                    down_actual.checked_add(up_actual),
                ) else {
                    continue;
                };
                let offset = reduction.min(*increase);
                let Some(overlap) = percent(offset, reduction.max(*increase)) else {
                    continue;
                };
                if overlap < 70 * SCALE || percent(offset, planned).is_none_or(|p| p < 10 * SCALE) {
                    continue;
                }
                if best
                    .as_ref()
                    .is_none_or(|(_, _, score, _, _, _, _)| offset > *score)
                {
                    best = Some((*down, *up, offset, overlap, planned, spent, reduction));
                }
            }
        }
        let Some((down, up, offset, overlap, planned, spent, reduction)) = best else {
            continue;
        };
        let Some(net) = spent.checked_sub(planned) else {
            continue;
        };
        let cited = down
            .citations
            .iter()
            .chain(&up.citations)
            .collect::<BTreeSet<_>>();
        let rows = records
            .iter()
            .filter(|r| {
                cited.contains(&r.citation_id)
                    && (r.role == "actual"
                        || r.role == "crosswalk"
                        || (r.role == "budget" && get(r, "basis").eq_ignore_ascii_case("adopted")))
            })
            .collect::<Vec<_>>();
        let mut lead = finding("account_shift", "Opposite account movements challenge an isolated spending story", format!("{} FY{} fund {} center {}: account {} ({}) is {} {} below adopted while account {} ({}) is {} above adopted. {}% of the larger movement is offset by the opposite movement; together these two accounts have adopted {} and actual {}, a net difference of {} {}. This is a candidate accounting/service shift, not evidence that money actually transferred or that services were cut.",down.department,down.fiscal_year,down.fund,down.center,down.account,down.account_label,display(reduction),down.currency,up.account,up.account_label,up.actual_minus_adopted.as_deref().unwrap(),display(overlap),display(planned),display(spent),display(net),down.currency), &rows, &["A reclassification, reimbursement agreement, transfer or contract-posting change may explain offsetting movements.","The accounts may purchase different services; one service may have declined while an unrelated service expanded.","Revised appropriations, timing and encumbrances may explain either account separately. Similar amounts do not establish a transfer."], &format!("Obtain FY{} fund {} center {} journal entries and budget transfers touching accounts {} and {}, including document/date, offsetting account, vendor/contract/program IDs and revised authority. Reconcile each agreement's invoices and accepted service outputs; reject the reclassification hypothesis if the underlying programs or vendors differ.",down.fiscal_year,down.fund,down.center,down.account,up.account));
        lead.calculations = vec![
            calculation("Offsetting movement", format!("min(abs({}), abs({}))",down.actual_minus_adopted.as_deref().unwrap(),up.actual_minus_adopted.as_deref().unwrap()), offset, &down.currency),
            calculation("Two-account net difference",format!("{} + {} − {} − {}",down.actual.as_deref().unwrap(),up.actual.as_deref().unwrap(),down.adopted.as_deref().unwrap(),up.adopted.as_deref().unwrap()),net,&down.currency),
            calculation("Larger movement offset", "min(abs(downward difference), upward difference) / max(abs(downward difference), upward difference) × 100".into(),overlap,"%")
        ];
        found.push(lead);
    }
    found
}

fn history_findings(
    records: &[FinancialRecord],
    incomplete: &BTreeSet<String>,
) -> Vec<FinancialAuditFinding> {
    if incomplete.contains("actual_history") {
        return vec![];
    }
    let mut groups: BTreeMap<Vec<String>, BTreeMap<i32, Vec<&FinancialRecord>>> = BTreeMap::new();
    for record in records
        .iter()
        .filter(|r| ["budget_history", "actual_history"].contains(&r.role.as_str()))
    {
        if ["agency", "fund", "department", "currency"]
            .iter()
            .any(|f| get(record, f).is_empty())
        {
            continue;
        }
        let Ok(year) = get(record, "fy").parse::<i32>() else {
            continue;
        };
        groups
            .entry(vec![
                get(record, "agency").into(),
                get(record, "fund").into(),
                get(record, "department").into(),
                get(record, "currency").into(),
                funding_duration(record).into(),
            ])
            .or_default()
            .entry(year)
            .or_default()
            .push(record);
    }
    let mut found = vec![];
    for (dimensions, years) in groups {
        let observations = years
            .iter()
            .map(|(year, rows)| {
                let adopted = sum(
                    &rows
                        .iter()
                        .copied()
                        .filter(|r| r.role == "budget_history")
                        .collect::<Vec<_>>(),
                    "amount",
                );
                let actual = sum(
                    &rows
                        .iter()
                        .copied()
                        .filter(|r| r.role == "actual_history")
                        .collect::<Vec<_>>(),
                    "amount",
                );
                (*year, adopted, actual, rows)
            })
            .collect::<Vec<_>>();
        let [.., prior, latest] = observations.as_slice() else {
            continue;
        };
        if latest.0 != prior.0 + 1 {
            continue;
        }
        if let (Some(b0), Some(a0), Some(b1), Some(a1)) = (prior.1, prior.2, latest.1, latest.2) {
            if let (Some(budget_growth), Some(actual_growth), Some(old_rate), Some(new_rate)) = (
                b1.checked_sub(b0),
                a1.checked_sub(a0),
                percent(a0, b0),
                percent(a1, b1),
            ) {
                if !incomplete.contains("budget_history")
                    && percent(budget_growth, b0).is_some_and(|p| p >= 20 * SCALE)
                    && old_rate >= 0
                    && new_rate >= 0
                    && old_rate < 75 * SCALE
                    && new_rate < 75 * SCALE
                    && percent(actual_growth, budget_growth).is_some_and(|p| p < 50 * SCALE)
                {
                    let growth_execution = percent(actual_growth, budget_growth).unwrap();
                    let rows = prior
                        .3
                        .iter()
                        .chain(latest.3.iter())
                        .copied()
                        .collect::<Vec<_>>();
                    let mut lead = finding("execution_capacity_gap", "A larger allocation has not translated into proportional recorded execution", format!("{} fund {}: adopted allocation rose {} {} from FY{} to FY{}, while recorded actual spending rose {}. Only {}% of the added allocation is reflected in additional actuals; execution was {}% and {}% of adopted in the two years. The repeated gap merits a capacity and implementation test, not an assertion that the money was available or services failed.",dimensions[2],dimensions[1],display(budget_growth),dimensions[3],prior.0,latest.0,display(actual_growth),display(growth_execution),display(old_rate),display(new_rate)),&rows,&["Delayed hiring, program launch, procurement or reimbursement can postpone execution.","The organization, accounting scope, fiscal close or funding source may have changed between years.","Revised budgets may legitimately reduce or move authority; adopted funding does not establish spendable balances."],&format!("For {} FY{}–FY{} fund {}, obtain quarterly authorized/filled/vacant positions, program launch and contract milestones, revised appropriations and transfers, year-end commitments, and accepted outputs. Test whether staffing/procurement timing explains the gap and whether delivery later caught up.",dimensions[2],prior.0,latest.0,dimensions[1]));
                    lead.calculations = vec![
                        calculation(
                            "Increase in adopted allocation",
                            format!("{} − {}", display(b1), display(b0)),
                            budget_growth,
                            &dimensions[3],
                        ),
                        calculation(
                            "Increase in recorded actuals",
                            format!("{} − {}", display(a1), display(a0)),
                            actual_growth,
                            &dimensions[3],
                        ),
                        calculation(
                            "Added allocation reflected in additional actuals",
                            format!(
                                "({} − {}) / ({} − {}) × 100",
                                display(a1),
                                display(a0),
                                display(b1),
                                display(b0)
                            ),
                            growth_execution,
                            "%",
                        ),
                    ];
                    found.push(lead);
                }
            }
        }
        let [.., first, middle, last] = observations.as_slice() else {
            continue;
        };
        if middle.0 != first.0 + 1 || last.0 != middle.0 + 1 {
            continue;
        }
        let (Some(a0), Some(a1), Some(a2)) = (first.2, middle.2, last.2) else {
            continue;
        };
        let (Some(drop), Some(rebound), Some(remaining)) =
            (a0.checked_sub(a1), a2.checked_sub(a1), a0.checked_sub(a2))
        else {
            continue;
        };
        if a1 >= 0
            && rebound > 0
            && percent(drop, a0).is_some_and(|p| p >= 20 * SCALE)
            && percent(remaining, a0).is_some_and(|p| p >= 10 * SCALE)
        {
            let remaining_percent = percent(remaining, a0).unwrap();
            let rows = [first.3, middle.3, last.3]
                .into_iter()
                .flat_map(|r| r.iter().copied())
                .filter(|r| r.role == "actual_history")
                .collect::<Vec<_>>();
            let mut lead = finding("rebound_below_baseline","A spending rebound still leaves an earlier service-funding baseline unexplained",format!("{} fund {} actual spending went from {} {} in FY{} to {} in FY{} and {} in FY{}. The latest increase of {} still leaves recorded spending {} ({}%) below FY{}. A latest-year growth narrative therefore cannot establish restoration of the earlier scope of services; the program/funding boundary needs to be reconciled.",dimensions[2],dimensions[1],display(a0),dimensions[3],first.0,display(a1),middle.0,display(a2),last.0,display(rebound),display(remaining),display(remaining_percent),first.0),&rows,&["One-time grants, emergency programs or capital/startup costs may make the earlier year an inappropriate recurring baseline.","Services or accounts may have moved to another department, fund or administering agency.","A lower amount may reflect efficiency or changed service demand; spending alone does not measure services."],&format!("Obtain the FY{}–FY{} program/fund/department crosswalk for {} fund {}, expiring grant and one-time funding schedules, and comparable service capacities and outputs. Follow any transferred program to its receiving ledger before asserting a cut; separate one-time startup costs from recurring delivery.",first.0,last.0,dimensions[2],dimensions[1]));
            lead.calculations = vec![
                calculation(
                    "Amount below earlier baseline",
                    format!("{} − {}", display(a0), display(a2)),
                    remaining,
                    &dimensions[3],
                ),
                calculation(
                    "Latest rebound",
                    format!("{} − {}", display(a2), display(a1)),
                    rebound,
                    &dimensions[3],
                ),
                calculation(
                    "Below earlier baseline",
                    format!(
                        "({} − {}) / {} × 100",
                        display(a0),
                        display(a2),
                        display(a0)
                    ),
                    remaining_percent,
                    "%",
                ),
            ];
            found.push(lead);
        }
    }
    found
}

pub fn analyze(
    scope: FinancialScope,
    records: Vec<FinancialRecord>,
    evidence: Vec<FinancialCitation>,
    queries: BTreeMap<String, String>,
    mut limitations: Vec<String>,
    incomplete_roles: &BTreeSet<String>,
) -> FinancialReport {
    let mut findings = vec![];
    let mut amounts_by_finding = BTreeMap::new();
    let mut gaps = vec![];
    let crosswalks = records
        .iter()
        .filter(|r| r.role == "crosswalk")
        .collect::<Vec<_>>();
    let mut groups: BTreeMap<Vec<String>, Vec<&FinancialRecord>> = BTreeMap::new();
    let mut mapping_citations: BTreeMap<Vec<String>, BTreeSet<String>> = BTreeMap::new();
    let mut ambiguous_keys = BTreeSet::new();
    for record in records
        .iter()
        .filter(|r| ["budget", "actual", "encumbrance"].contains(&r.role.as_str()))
    {
        let (account, citations, ambiguous) = canonical_account(record, &crosswalks);
        let dimensions = vec![
            get(record, "fy").into(),
            get(record, "fund").into(),
            account,
            if get(record, "center").is_empty() {
                get(record, "department").into()
            } else {
                String::new()
            },
            get(record, "center").into(),
            get(record, "program").into(),
            get(record, "currency").into(),
            get(record, "agency").into(),
            funding_duration(record).into(),
        ];
        if ambiguous {
            ambiguous_keys.insert(dimensions.clone());
        }
        mapping_citations
            .entry(dimensions.clone())
            .or_default()
            .extend(citations);
        groups.entry(dimensions).or_default().push(record);
    }
    let mut reconciliation = vec![];
    for (dimensions, rows) in groups {
        let subset = |role: &str, basis: &str| {
            rows.iter()
                .copied()
                .filter(|r| {
                    r.role == role
                        && (basis.is_empty() || get(r, "basis").eq_ignore_ascii_case(basis))
                })
                .collect::<Vec<_>>()
        };
        let adopted_rows = subset("budget", "adopted");
        let revised_rows = subset("budget", "revised");
        let actual_rows = subset("actual", "");
        let encumbered_rows = subset("encumbrance", "");
        let mut warnings = Vec::new();
        let complete = |role: &str| !incomplete_roles.contains(role);
        let adopted = complete("budget")
            .then(|| sum(&adopted_rows, "amount"))
            .flatten();
        let revised = complete("budget")
            .then(|| sum(&revised_rows, "amount"))
            .flatten();
        let actual = complete("actual")
            .then(|| sum(&actual_rows, "amount"))
            .flatten();
        let encumbered = complete("encumbrance")
            .then(|| sum(&encumbered_rows, "amount"))
            .flatten();
        for (label, value) in [
            ("adopted appropriation", adopted),
            ("revised authority", revised),
            ("actual spending", actual),
            ("encumbrances", encumbered),
        ] {
            if value.is_none() {
                warnings.push(format!(
                    "Missing or incomplete {label}; absence is not zero."
                ));
            }
        }
        if rows.iter().any(|r| {
            r.role == "budget"
                && !["adopted", "revised"].contains(&get(r, "basis").to_lowercase().as_str())
        }) {
            warnings.push(
                "Unrecognized budget cycles were not treated as adopted or revised authority."
                    .into(),
            );
        }
        if ambiguous_keys.contains(&dimensions) {
            warnings.push("Conflicting account crosswalks prevent a defensible comparison.".into());
        }
        if dimensions[6].is_empty() {
            warnings.push("Currency has not been established.".into());
        }
        if rows.iter().any(|r| !get(r, "transfer").is_empty()) {
            warnings.push("Transfer classifications are present. Trace both sides before interpreting gross amounts as service costs; these stages are never added together.".into());
        }
        match dimensions[8].as_str() {
            "one_time" => warnings.push("The source identifies one-time funding. It does not establish support for recurring service obligations.".into()),
            "mixed" => warnings.push("The source combines one-time and recurring funding. Obtain the component split before assessing recurring commitments.".into()),
            "unknown" => warnings.push("Funding duration is unknown; an annual amount does not establish that funding recurs.".into()),
            _ => {}
        }
        let comparable = !ambiguous_keys.contains(&dimensions)
            && !dimensions[6].is_empty()
            && !dimensions[7].is_empty();
        let delta = comparable.then(|| actual?.checked_sub(adopted?)).flatten();
        let available = comparable
            .then(|| revised?.checked_sub(actual?)?.checked_sub(encumbered?))
            .flatten();
        let mut citations = ids(&rows);
        citations.extend(mapping_citations.remove(&dimensions).unwrap_or_default());
        if let Some(value) = delta.filter(|v| *v != 0) {
            let item = finding("budget_variance", "A spending difference needs the revised-authority bridge", format!("FY{} fund {} account {}: actual minus original adopted = {} {}. This is not a finding that revised legal authority was exceeded.",dimensions[0],dimensions[1],dimensions[2],display(value),dimensions[6]), &rows,
                &["Council revisions or transfers may change authority.","Timing, encumbrances, reimbursements, or account reclassifications may explain the difference."],"Obtain the adopted-to-revised appropriation bridge, dated transfers, close status, and matching encumbrance register.");
            amounts_by_finding.insert(item.id.clone(), value.unsigned_abs());
            findings.push(item);
        }
        reconciliation.push(FinancialReconciliation {
            agency: dimensions[7].clone(),
            funding_duration: dimensions[8].clone(),
            key: key(&dimensions.iter().map(String::as_str).collect::<Vec<_>>()),
            fiscal_year: dimensions[0].clone(),
            fund: dimensions[1].clone(),
            account: dimensions[2].clone(),
            account_label: rows
                .iter()
                .map(|r| get(r, "account_label"))
                .filter(|s| !s.is_empty())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
                .join(" | "),
            department: rows
                .iter()
                .map(|r| get(r, "department"))
                .filter(|s| !s.is_empty())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
                .join(" | "),
            center: dimensions[4].clone(),
            program: dimensions[5].clone(),
            currency: dimensions[6].clone(),
            adopted: formatted(adopted),
            revised: formatted(revised),
            actual: formatted(actual),
            encumbered: formatted(encumbered),
            actual_minus_adopted: formatted(delta),
            uncommitted_revised: formatted(available),
            citations,
            limitations: warnings,
        });
    }
    for lead in account_shift_findings(&reconciliation, &records)
        .into_iter()
        .chain(history_findings(&records, incomplete_roles))
    {
        gaps.push(FinancialGap { title: lead.title.clone(), records_needed: lead.next_test.clone(), why: "Resolve the accounting, funding-boundary or implementation explanation before interpreting spending as delivered services.".into(), citations: lead.citations.clone() });
        if let Some(amount) = lead.calculations.first().and_then(|c| decimal(&c.result)) {
            amounts_by_finding.insert(lead.id.clone(), amount.unsigned_abs());
        }
        findings.push(lead);
    }
    // Contract stages are separate quantities. An appropriation, signed ceiling and payment are
    // never added together. Only a stable contract identifier links the chain.
    let mut contract_groups: BTreeMap<(String, String, String), Vec<&FinancialRecord>> =
        BTreeMap::new();
    for record in &records {
        if !get(record, "contract_id").is_empty() {
            contract_groups
                .entry((
                    get(record, "contract_id").into(),
                    get(record, "currency").into(),
                    get(record, "agency").into(),
                ))
                .or_default()
                .push(record);
        }
    }
    let mut chains = vec![];
    for ((contract_id, currency, agency), rows) in contract_groups {
        let by_role = |role: &str| {
            rows.iter()
                .copied()
                .filter(|r| r.role == role)
                .collect::<Vec<_>>()
        };
        let contracts = by_role("contract");
        let amendments = by_role("amendment");
        let payments = by_role("payment");
        let mut warnings = vec![];
        let mut stages = BTreeMap::new();
        for role in ["budget", "contract", "amendment", "payment", "outcome"] {
            stages.insert(role.into(), ids(&by_role(role)));
        }
        let approved = |r: &&FinancialRecord| {
            ["executed", "approved", "signed"].contains(&get(r, "status").to_lowercase().as_str())
        };
        let base = if contracts.len() == 1
            && approved(&contracts[0])
            && !incomplete_roles.contains("contract")
        {
            amount(contracts[0], "amount")
        } else {
            None
        };
        if contracts.len() != 1 {
            warnings.push(
                "The base contract is missing or not unique; no authorization ceiling is inferred."
                    .into(),
            );
        }
        let amendment_ids = amendments
            .iter()
            .map(|r| get(r, "amendment_id"))
            .collect::<BTreeSet<_>>();
        let amendment_total = if !incomplete_roles.contains("amendment")
            && amendment_ids.len() == amendments.len()
            && !amendment_ids.contains("")
            && amendments.iter().all(approved)
        {
            if amendments.is_empty() {
                None
            } else {
                sum(&amendments, "amount")
            }
        } else {
            None
        };
        // A complete amendment register may explicitly report zero via a row. No records means
        // unknown amendment coverage, not that the base ceiling was never changed.
        let ceiling = base.and_then(|b| b.checked_add(amendment_total?));
        if amendment_total.is_none() {
            warnings.push("A complete, unique, executed amendment register (including an explicit zero/no-change row when applicable) is needed for current authority.".into());
        }
        let mut transactions: BTreeMap<&str, Vec<&FinancialRecord>> = BTreeMap::new();
        for row in &payments {
            transactions
                .entry(get(row, "transaction_id"))
                .or_default()
                .push(row);
        }
        let mut clean_payments = Vec::new();
        let mut payment_ambiguous = false;
        for (transaction, group) in transactions {
            let signatures = group.iter().map(|r| &r.values).collect::<Vec<_>>();
            if transaction.is_empty() || signatures.windows(2).any(|p| p[0] != p[1]) {
                payment_ambiguous = true;
                continue;
            }
            if group.len() > 1 {
                findings.push(finding("duplicate_transaction","Repeated transaction rows must not inflate spending",format!("Contract {contract_id} repeats transaction {transaction} {} times; identical rows count once in the displayed payment total.",group.len()),&group,&["The export may contain repeated copies.","A transaction identifier may describe several legitimate line items; verify its grain."],"Obtain transaction-line identifiers and posting/reversal history before treating repeated rows as duplicate disbursements."));
            }
            if ["paid", "posted", "settled"]
                .contains(&get(group[0], "status").to_lowercase().as_str())
            {
                clean_payments.push(group[0]);
            } else if !["void", "cancelled", "canceled"]
                .contains(&get(group[0], "status").to_lowercase().as_str())
            {
                payment_ambiguous = true;
            }
        }
        let paid = if payment_ambiguous || incomplete_roles.contains("payment") {
            None
        } else {
            sum(&clean_payments, "amount")
        };
        if payment_ambiguous {
            warnings.push("Payment identifiers/statuses are incomplete or conflicting; the payment total is withheld.".into());
        }
        let vendors = rows
            .iter()
            .map(|r| get(r, "vendor_id"))
            .filter(|s| !s.is_empty())
            .collect::<BTreeSet<_>>();
        let ceiling = if vendors.len() > 1 || currency.is_empty() || agency.is_empty() {
            warnings.push("Vendor identity or currency is unresolved; authorization comparisons are withheld.".into());
            None
        } else {
            ceiling
        };
        let remaining = ceiling.and_then(|c| c.checked_sub(paid?));
        if remaining.is_some_and(|v| v < 0) {
            findings.push(finding("ceiling_exceeded","Recorded payments exceed the retrieved executed ceiling",format!("Contract {contract_id}: paid {} {}; retrieved executed authority {} {}.",display(paid.unwrap()),currency,display(ceiling.unwrap()),currency),&rows,&["An omitted amendment may increase the ceiling.","Pass-through reimbursements, credits, or a reused contract ID may change the comparison."],"Reconcile all executed amendments, contract identifiers, line-level payments, and reimbursements with the agency's certified contract ledger."));
        }
        let mut invoices: BTreeMap<(String, String, String), Vec<&FinancialRecord>> =
            BTreeMap::new();
        for r in &clean_payments {
            if !get(r, "invoice_id").is_empty() && !get(r, "vendor_id").is_empty() {
                invoices
                    .entry((
                        get(r, "vendor_id").into(),
                        get(r, "invoice_id").to_uppercase(),
                        get(r, "amount").into(),
                    ))
                    .or_default()
                    .push(r);
            }
        }
        for ((vendor, invoice, amount), group) in invoices {
            if group.len() > 1 {
                findings.push(finding("duplicate_invoice","Same invoice and amount appear in distinct paid transactions",format!("Vendor {vendor}, contract {contract_id}, invoice {invoice}: {} distinct paid transactions each record {amount} {currency}.",group.len()),&group,&["Installments, separate invoice lines, reversals, or recurring invoice numbering can produce legitimate matches."],"Request invoice images, line numbers, approvals, check/EFT identifiers, voids, and credits for these exact transactions."));
            }
        }
        let missing_links = [
            ("budget", "Contract-level appropriation/funding authority"),
            ("contract", "Executed base contract"),
            ("amendment", "Complete executed amendment register"),
            ("payment", "Posted payment register"),
            ("outcome", "Accepted deliverables and outcomes"),
        ]
        .into_iter()
        .filter(|(role, _)| by_role(role).is_empty())
        .map(|(_, label)| label.into())
        .collect::<Vec<String>>();
        if by_role("outcome").is_empty() && !payments.is_empty() {
            findings.push(finding("deliverables_gap","Payments lack a linked delivery record in the retrieved data",format!("Contract {contract_id} has payment records but no outcome/deliverable row linked by its identifier. This is a coverage gap, not proof services were not delivered."),&payments,&["Acceptance records may exist in a separate system or use another identifier."],"Obtain the contract deliverable schedule, acceptance/sign-off records, performance reports and the contract-to-program identifier crosswalk."));
        }
        chains.push(FinancialChain {
            agency,
            contract_id,
            program: rows
                .iter()
                .map(|r| get(r, "program"))
                .find(|s| !s.is_empty())
                .unwrap_or("")
                .into(),
            vendor: vendors.into_iter().collect::<Vec<_>>().join(" | "),
            currency,
            base_authority: formatted(base),
            amendments: formatted(amendment_total),
            total_authority: formatted(ceiling),
            payments: formatted(paid),
            remaining_authority: formatted(remaining),
            stages,
            missing_links,
            limitations: warnings,
        });
    }
    let mut outcomes = vec![];
    for record in records.iter().filter(|r| r.role == "outcome") {
        let mut warnings = vec![];
        let cost = amount(record, "amount");
        let quantity = amount(record, "quantity");
        let denominator = amount(record, "denominator");
        let valid_period =
            chrono::NaiveDate::parse_from_str(get(record, "period_start"), "%Y-%m-%d")
                .ok()
                .zip(chrono::NaiveDate::parse_from_str(get(record, "period_end"), "%Y-%m-%d").ok())
                .is_some_and(|(a, b)| a <= b);
        let comparable = valid_period
            && [
                "population",
                "metric",
                "unit",
                "currency",
                "program",
                "agency",
            ]
            .iter()
            .all(|field| !get(record, field).is_empty())
            && get(record, "basis").eq_ignore_ascii_case("actual")
            && !incomplete_roles.contains("outcome");
        if !comparable {
            warnings.push("Comparison requires an actual-cost basis, explicit period, currency, population, unit and program; incomplete fields are not interchangeable.".into());
        }
        let unit_cost =
            if comparable && quantity.is_some_and(|q| q > 0) && cost.is_some_and(|v| v >= 0) {
                cost.and_then(|c| c.checked_mul(SCALE)?.checked_div(quantity?))
            } else {
                None
            };
        let rate = if comparable
            && !get(record, "denominator_unit").is_empty()
            && get(record, "denominator_unit") == get(record, "unit")
            && quantity.is_some_and(|q| q >= 0)
            && denominator.is_some_and(|d| d > 0)
            && quantity <= denominator
        {
            quantity.and_then(|q| q.checked_mul(100 * SCALE)?.checked_div(denominator?))
        } else {
            None
        };
        if denominator.is_some() && rate.is_none() {
            warnings.push("The outcome numerator/denominator is missing, nonpositive or internally inconsistent; no success rate is calculated.".into());
        }
        if quantity == Some(0) {
            warnings.push("Zero measured outputs make a unit cost undefined; zero is preserved as a measurement.".into());
        }
        let dimensions = [
            get(record, "period_start"),
            get(record, "period_end"),
            get(record, "population"),
            get(record, "unit"),
            get(record, "currency"),
            get(record, "basis"),
            get(record, "metric"),
            get(record, "denominator_unit"),
            get(record, "agency"),
        ];
        outcomes.push(FinancialOutcome {
            agency: get(record, "agency").into(),
            key: key(&[&record.citation_id]),
            program: get(record, "program").into(),
            contract_id: get(record, "contract_id").into(),
            period_start: dimensions[0].into(),
            period_end: dimensions[1].into(),
            population: dimensions[2].into(),
            metric: get(record, "metric").into(),
            denominator_unit: get(record, "denominator_unit").into(),
            unit: dimensions[3].into(),
            currency: dimensions[4].into(),
            cost: formatted(cost),
            quantity: formatted(quantity),
            denominator: formatted(denominator),
            cost_per_unit: formatted(unit_cost),
            success_rate_percent: formatted(rate),
            comparison_group: comparable.then(|| key(&dimensions)),
            citations: vec![record.citation_id.clone()],
            limitations: warnings,
        });
    }
    let mut comparable_groups: BTreeMap<String, Vec<&FinancialOutcome>> = BTreeMap::new();
    for outcome in &outcomes {
        if let Some(group) = &outcome.comparison_group {
            if outcome.cost_per_unit.is_some() {
                comparable_groups
                    .entry(group.clone())
                    .or_default()
                    .push(outcome);
            }
        }
    }
    for group in comparable_groups.values().filter(|g| g.len() > 1) {
        let low = group
            .iter()
            .filter_map(|o| o.cost_per_unit.as_deref().and_then(decimal))
            .min();
        let high = group
            .iter()
            .filter_map(|o| o.cost_per_unit.as_deref().and_then(decimal))
            .max();
        if low.zip(high).is_some_and(|(l, h)| l != h) {
            let rows = records
                .iter()
                .filter(|r| group.iter().any(|o| o.citations.contains(&r.citation_id)))
                .collect::<Vec<_>>();
            findings.push(finding("unit_cost_spread","Comparable reported units have different recorded costs",format!("For {} through {}, population '{}', unit '{}': observed costs range from {} to {} {} per unit. Period/unit labels match; quality and case mix still require review.",group[0].period_start,group[0].period_end,group[0].population,group[0].unit,display(low.unwrap()),display(high.unwrap()),group[0].currency),&rows,&["Service quality, acuity, facility costs, utilization, capital allocations, and accounting boundaries may differ."],"Obtain cost allocation methods, service specifications, case-mix measures and independent outcome verification before comparing provider efficiency."));
        }
    }
    for (role,title,needed,why) in [
        ("budget","Adopted and revised appropriations","Adopted/revised appropriation ledger with fund, fiscal year, account, department and transfer history.","An adopted-to-actual difference is not a legal-authority test."),
        ("encumbrance","Commitments already consuming authority","Open encumbrance register by fund/FY/account and close status.","Unspent cash cannot establish funds available for another program."),
        ("contract","Executed contract register","Executed contracts with stable IDs, vendor IDs, fund/FY, ceilings, status and program links.","Department spending alone cannot trace a specific purchased service."),
        ("amendment","Complete amendment history","Executed amendment register, delta amounts, effective dates and explicit no-amendment declarations.","A base award does not establish the current ceiling."),
        ("payment","Invoice-to-payment trace","Posted transaction-line ledger with contract/vendor/invoice IDs, amounts, reversals and statuses.","Invoices and transfers are not necessarily final service expenditures."),
        ("outcome","Comparable accepted service results","Program/contract output and outcome records with cost basis, date window, population, units, numerator and denominator.","A spending total cannot establish delivery or cost effectiveness."),
        ("restriction","Funding eligibility and restrictions","Fund restrictions, permitted programs and applicable fiscal year, with source authority.","Scenario funding remains blocked until restrictions are known.")
    ] { if !records.iter().any(|r|r.role==role) || incomplete_roles.contains(role) { gaps.push(FinancialGap{title:title.into(),records_needed:needed.into(),why:why.into(),citations:vec![]}); } }
    limitations.extend(["All amounts use exact fixed-point arithmetic to four decimal places. Displayed ratios truncate at four decimals; scenarios calculate from original amounts, not rounded unit costs.".into(),"Appropriations, contract ceilings, amendments, payments and outcomes are separate stages, never an additive spending total. Missing records establish evidence gaps, not nonperformance or wrongdoing.".into(),"Only explicitly mapped identifiers join stages. Equal names do not establish vendor or contract identity. Comparability of labels does not establish equal service quality.".into(),"Account-shift discovery compares at most the 16 largest movements per direction within the same agency/year/fund/center/program/currency/funding-duration/three-digit account family. It selects one pair per group with at least 70% offset and offset size at least 10% of combined adopted amounts. These are discovery filters, not statistical significance or evidence of a transfer.".into(),"Capacity leads require consecutive years, at least 20% adopted growth, both execution ratios below 75%, and less than half of the additional allocation reflected in additional actuals. Rebound leads require a three-year sequence with a drop of at least 20% and latest spending still at least 10% below the first year. Histories retain separate funding durations and jurisdictions; no inflation adjustment is inferred. A fiscal-year label does not certify a closed, final ledger.".into()]);
    let priority = |kind: &str| match kind {
        "ceiling_exceeded" => 0,
        "duplicate_invoice" => 1,
        "duplicate_transaction" => 2,
        "account_shift" | "execution_capacity_gap" | "rebound_below_baseline" => 3,
        "unit_cost_spread" => 4,
        "deliverables_gap" => 5,
        _ => 6,
    };
    findings.sort_by(|a, b| {
        priority(&a.kind)
            .cmp(&priority(&b.kind))
            .then(
                amounts_by_finding
                    .get(&b.id)
                    .unwrap_or(&0)
                    .cmp(amounts_by_finding.get(&a.id).unwrap_or(&0)),
            )
            .then(a.id.cmp(&b.id))
    });

    FinancialReport {
        id: uuid::Uuid::new_v4().to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
        scope,
        reconciliation,
        chains,
        findings,
        outcomes,
        gaps,
        evidence,
        queries,
        records,
        limitations,
    }
}

pub fn scenario(
    report: &FinancialReport,
    input: &FinancialScenarioInput,
) -> Result<FinancialScenario, AppError> {
    let funding = report
        .reconciliation
        .iter()
        .find(|r| r.key == input.funding_key)
        .ok_or_else(|| AppError::DataParsing("Choose a current reconciled funding line".into()))?;
    let outcome = report
        .outcomes
        .iter()
        .find(|r| r.key == input.outcome_key)
        .ok_or_else(|| AppError::DataParsing("Choose a sourced outcome/cost record".into()))?;
    let requested = decimal(&input.proposed_amount)
        .filter(|v| *v > 0)
        .ok_or_else(|| {
            AppError::DataParsing(
                "Scenario amount must be positive, with at most four decimal places".into(),
            )
        })?;
    let mut blockers = vec![];
    if report.scope.fiscal_year.map(|fy| fy.to_string()).as_deref()
        != Some(funding.fiscal_year.as_str())
    {
        blockers.push(
            "A scenario requires an explicit fiscal year matching its funding evidence.".into(),
        );
    }
    let balance = funding.uncommitted_revised.as_deref().and_then(decimal);
    if balance.is_none() {
        blockers.push("Revised authority, actual expenditures, and encumbrances must all be established on the same funding line.".into());
    }
    if balance.is_some_and(|v| requested > v) {
        blockers
            .push("The proposed amount exceeds the recorded uncommitted revised balance.".into());
    }
    if funding.agency.is_empty() || funding.agency != outcome.agency {
        blockers.push(
            "Funding and service records must explicitly identify the same agency/jurisdiction."
                .into(),
        );
    }
    if funding.currency.is_empty() || funding.currency != outcome.currency {
        blockers.push("Funding and unit-cost currencies must match explicitly.".into());
    }
    let restrictions = report
        .records
        .iter()
        .filter(|r| {
            r.role == "restriction"
                && get(r, "agency") == funding.agency
                && get(r, "fund") == funding.fund
                && get(r, "fy") == funding.fiscal_year
        })
        .collect::<Vec<_>>();
    if restrictions.is_empty() {
        blockers.push("No cited funding restriction/eligibility record was retrieved for this fund and fiscal year.".into());
    }
    let eligible = restrictions.iter().all(|r| {
        get(r, "restriction").eq_ignore_ascii_case("unrestricted")
            || (get(r, "restriction").eq_ignore_ascii_case("restricted")
                && get(r, "eligible_program") == input.target_program.trim())
    });
    if !eligible {
        blockers.push("A source restriction is unknown or does not expressly include the proposed target program.".into());
    }
    if input.target_program.trim().is_empty() {
        blockers.push("Name the specific target program.".into());
    }
    if outcome.comparison_group.is_none() || outcome.cost_per_unit.is_none() {
        blockers.push("The selected service record lacks a comparable positive quantity and actual-cost basis.".into());
    }
    let calculate = |o: &FinancialOutcome| -> Option<i128> {
        let cost = decimal(o.cost.as_deref()?)?;
        let quantity = decimal(o.quantity.as_deref()?)?;
        if cost <= 0 || quantity <= 0 {
            return None;
        }
        requested
            .checked_mul(quantity)?
            .checked_div(cost)?
            .checked_div(SCALE)
    };
    let units = calculate(outcome);
    if units.is_none() {
        blockers.push("Unit capacity is undefined for zero/missing cost or quantity, or the arithmetic exceeds the supported range.".into());
    }
    let comparables = report
        .outcomes
        .iter()
        .filter(|o| {
            outcome.comparison_group.is_some() && o.comparison_group == outcome.comparison_group
        })
        .collect::<Vec<_>>();
    let range = comparables
        .iter()
        .filter_map(|o| calculate(o))
        .collect::<Vec<_>>();
    let mut cited = funding
        .citations
        .iter()
        .chain(&outcome.citations)
        .cloned()
        .collect::<BTreeSet<_>>();
    cited.extend(ids(&restrictions));
    for o in comparables {
        cited.extend(o.citations.clone());
    }
    let allowed = blockers.is_empty();
    Ok(FinancialScenario{status:if allowed{"conditional_estimate"}else{"blocked"}.into(),funding_duration:funding.funding_duration.clone(),period_scope:format!("FY{} only; one fiscal period",funding.fiscal_year),target_program:input.target_program.trim().into(),available_revised_balance:funding.uncommitted_revised.clone(),proposed_amount:display(requested),currency:(!funding.currency.is_empty()).then(||funding.currency.clone()),unit:(!outcome.unit.is_empty()).then(||outcome.unit.clone()),estimated_whole_units:allowed.then(||units.unwrap().to_string()),lower_units:allowed.then(||range.iter().min().unwrap_or(&0).to_string()),upper_units:allowed.then(||range.iter().max().unwrap_or(&0).to_string()),formula:format!("floor(proposed amount × observed quantity ÷ observed actual cost). Baseline: {} × {} ÷ {}. Funding balance: revised − actual − encumbered.",display(requested),outcome.quantity.as_deref().unwrap_or("unknown"),outcome.cost.as_deref().unwrap_or("unknown")),assumptions:vec![format!("This estimate applies only to FY{}. Funding duration is {}. One-time, mixed or unknown funding does not establish recurring capacity; even recurring classifications do not guarantee a future appropriation. No continuation beyond this fiscal period is calculated.",funding.fiscal_year,funding.funding_duration),"This is a conditional service-capacity calculation, not a finding of legal transfer authority or a forecast of outcomes.".into(),"The cited period, population and unit must remain applicable. Unit costs are not assumed constant under expansion; staffing, procurement, capacity, service quality and timing require separate review.".into(),"The displayed range uses only retrieved records with exactly matching period, population, unit, currency and actual-cost basis; it is not a statistical confidence interval.".into(),"Ledger close status, pending invoices, restricted reserves and later commitments may reduce the apparent balance.".into()],blockers,queries:report.queries.clone(),citations:report.evidence.iter().filter(|e|cited.contains(&e.id)).cloned().collect()})
}

#[cfg(test)]
mod tests {
    use super::*;
    fn row(role: &str, id: &str, extra: &[(&str, &str)]) -> FinancialRecord {
        let mut values = [
            ("fy", "2025"),
            ("fund", "100000"),
            ("currency", "USD"),
            ("agency", "City of San Diego"),
            ("account", "5001"),
            ("center", "001"),
            ("department", "Police"),
            ("program", "Shelter"),
        ]
        .into_iter()
        .map(|(k, v)| (k.into(), Some(v.into())))
        .collect::<BTreeMap<_, _>>();
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
            &BTreeSet::new(),
        )
    }
    fn outcome(id: &str, cost: &str, quantity: &str) -> FinancialRecord {
        row(
            "outcome",
            id,
            &[
                ("amount", cost),
                ("quantity", quantity),
                ("denominator", "20"),
                ("denominator_unit", "people"),
                ("unit", "people"),
                ("metric", "permanent housing exits"),
                ("population", "adults entering the same program"),
                ("basis", "actual"),
                ("period_start", "2024-07-01"),
                ("period_end", "2025-06-30"),
            ],
        )
    }
    #[test]
    fn financial_reconciliation_uses_exact_amounts_and_stable_account_keys_with_no_stage_double_count(
    ) {
        let r = report(vec![
            row(
                "budget",
                "b1",
                &[
                    ("amount", "90071992547409.01"),
                    ("basis", "adopted"),
                    ("department", "Old Police name"),
                ],
            ),
            row("actual", "a1", &[("amount", "90071992547409.02")]),
        ]);
        assert_eq!(r.reconciliation.len(), 1);
        assert_eq!(
            r.reconciliation[0].actual_minus_adopted.as_deref(),
            Some("0.01")
        );
        assert!(r.reconciliation[0].uncommitted_revised.is_none());
        assert_eq!(decimal("-0.0012"), Some(-12));
        assert!(decimal("1.00001").is_none());
        assert!(decimal("1e10").is_none());
    }
    #[test]
    fn financial_crosswalks_are_period_and_fund_bound_and_conflicts_block_variance() {
        let b = row(
            "budget",
            "b",
            &[
                ("amount", "100"),
                ("basis", "adopted"),
                ("account", "500128"),
            ],
        );
        let a = row("actual", "a", &[("amount", "120"), ("account", "500124")]);
        let walk = row(
            "crosswalk",
            "w",
            &[("account", "500124"), ("canonical_account", "500128")],
        );
        let r = report(vec![b.clone(), a.clone(), walk.clone()]);
        assert_eq!(r.reconciliation.len(), 1);
        assert_eq!(
            r.reconciliation[0].actual_minus_adopted.as_deref(),
            Some("20.00")
        );
        assert!(r.reconciliation[0].citations.contains(&"w".into()));
        let conflict = row(
            "crosswalk",
            "bad",
            &[("account", "500124"), ("canonical_account", "500999")],
        );
        let r = report(vec![b, a, walk, conflict]);
        assert!(r
            .reconciliation
            .iter()
            .all(|r| r.actual_minus_adopted.is_none()));
    }
    #[test]
    fn financial_contract_audit_counts_identical_transactions_once_and_flags_distinct_invoice_payments(
    ) {
        let base = row(
            "contract",
            "c",
            &[
                ("contract_id", "C-1"),
                ("vendor_id", "V-1"),
                ("amount", "100"),
                ("status", "executed"),
            ],
        );
        let amendment = row(
            "amendment",
            "m",
            &[
                ("contract_id", "C-1"),
                ("amendment_id", "A-1"),
                ("amount", "20"),
                ("status", "executed"),
            ],
        );
        let p = row(
            "payment",
            "p1",
            &[
                ("contract_id", "C-1"),
                ("vendor_id", "V-1"),
                ("transaction_id", "T-1"),
                ("invoice_id", "I-1"),
                ("amount", "70"),
                ("status", "paid"),
            ],
        );
        let mut copy = p.clone();
        copy.citation_id = "copy".into();
        let mut second = p.clone();
        second
            .values
            .insert("transaction_id".into(), Some("T-2".into()));
        second.citation_id = "p2".into();
        let r = report(vec![base, amendment, p, copy, second]);
        assert_eq!(r.chains[0].total_authority.as_deref(), Some("120.00"));
        assert_eq!(r.chains[0].payments.as_deref(), Some("140.00"));
        assert_eq!(r.chains[0].remaining_authority.as_deref(), Some("-20.00"));
        for kind in [
            "duplicate_transaction",
            "duplicate_invoice",
            "ceiling_exceeded",
            "deliverables_gap",
        ] {
            assert!(r.findings.iter().any(|f| f.kind == kind), "{kind}");
        }
        assert!(r.findings.iter().all(|f| !f.alternatives.is_empty()
            && !f.next_test.is_empty()
            && !f.citations.is_empty()));
    }
    #[test]
    fn financial_partial_sources_missing_amendments_and_conflicting_postings_never_imply_zero() {
        let records = vec![
            row("budget", "b", &[("amount", "100"), ("basis", "adopted")]),
            row("actual", "a", &[("amount", "0")]),
        ];
        let r = analyze(
            FinancialScope::default(),
            records,
            vec![],
            BTreeMap::new(),
            vec![],
            &BTreeSet::from(["actual".into()]),
        );
        assert!(r.reconciliation[0].actual.is_none());
        let r = report(vec![
            row(
                "contract",
                "c",
                &[
                    ("contract_id", "C"),
                    ("amount", "100"),
                    ("status", "executed"),
                ],
            ),
            row(
                "payment",
                "p",
                &[
                    ("contract_id", "C"),
                    ("transaction_id", "T"),
                    ("amount", "110"),
                    ("status", "paid"),
                ],
            ),
            row(
                "payment",
                "p2",
                &[
                    ("contract_id", "C"),
                    ("transaction_id", "T"),
                    ("amount", "10"),
                    ("status", "paid"),
                ],
            ),
        ]);
        assert!(r.chains[0].amendments.is_none());
        assert!(r.chains[0].total_authority.is_none());
        assert!(r.chains[0].payments.is_none());
        assert!(!r.findings.iter().any(|f| f.kind == "ceiling_exceeded"));
    }
    #[test]
    fn financial_outcomes_require_matching_metric_population_period_and_denominator_units() {
        let first = outcome("o1", "100", "10");
        let second = outcome("o2", "200", "10");
        let mut different = outcome("o3", "50", "10");
        different
            .values
            .insert("metric".into(), Some("shelter admissions".into()));
        let mut wrong_denominator = outcome("o4", "100", "10");
        wrong_denominator
            .values
            .insert("denominator_unit".into(), Some("bed nights".into()));
        let zero = outcome("zero", "100", "0");
        let r = report(vec![first, second, different, wrong_denominator, zero]);
        assert_eq!(r.outcomes[0].cost_per_unit.as_deref(), Some("10.00"));
        assert_eq!(r.outcomes[0].success_rate_percent.as_deref(), Some("50.00"));
        assert_eq!(
            r.outcomes[0].comparison_group,
            r.outcomes[1].comparison_group
        );
        assert_ne!(
            r.outcomes[0].comparison_group,
            r.outcomes[2].comparison_group
        );
        assert!(r.outcomes[3].success_rate_percent.is_none());
        assert!(r.outcomes[4].cost_per_unit.is_none());
        assert_eq!(r.outcomes[4].quantity.as_deref(), Some("0.00"));
    }
    #[test]
    fn financial_policy_scenarios_use_uncommitted_revised_balance_and_fail_closed_on_restrictions()
    {
        let mut records = vec![
            row(
                "budget",
                "adopted",
                &[("amount", "900"), ("basis", "adopted")],
            ),
            row(
                "budget",
                "revised",
                &[("amount", "1200"), ("basis", "revised")],
            ),
            row("actual", "actual", &[("amount", "100")]),
            row("encumbrance", "commit", &[("amount", "100")]),
            outcome("o1", "100", "10"),
            outcome("o2", "200", "10"),
        ];
        let r = report(records.clone());
        let mut input = FinancialScenarioInput {
            scope: r.scope.clone(),
            funding_key: r.reconciliation[0].key.clone(),
            outcome_key: r.outcomes[0].key.clone(),
            target_program: "Shelter".into(),
            proposed_amount: "200".into(),
        };
        assert_eq!(scenario(&r, &input).unwrap().status, "blocked");
        records.push(row(
            "restriction",
            "restriction",
            &[
                ("restriction", "restricted"),
                ("eligible_program", "Shelter"),
            ],
        ));
        let r = report(records);
        let result = scenario(&r, &input).unwrap();
        assert_eq!(result.available_revised_balance.as_deref(), Some("1000.00"));
        assert_eq!(result.estimated_whole_units.as_deref(), Some("20"));
        assert_eq!(result.lower_units.as_deref(), Some("10"));
        assert_eq!(result.upper_units.as_deref(), Some("20"));
        input.target_program = "Police overtime".into();
        let blocked = scenario(&r, &input).unwrap();
        assert_eq!(blocked.status, "blocked");
        assert!(blocked.estimated_whole_units.is_none());
        input.target_program = "Shelter".into();
        input.proposed_amount = "1000.01".into();
        assert_eq!(scenario(&r, &input).unwrap().status, "blocked");
    }
    #[test]
    fn financial_replays_verified_san_diego_account_differences_without_inventing_missing_actuals()
    {
        let artifacts: Vec<ResearchEvidence> = serde_json::from_str(include_str!(
            "../../../docs/validation/finance-comparisons.json"
        ))
        .unwrap();
        let evidence = &artifacts[1];
        let result: serde_json::Value = serde_json::from_str(&evidence.text).unwrap();
        let source_rows = result["rows"].as_array().unwrap();
        let mut records = vec![];
        let mut source_links = vec![];
        for (i, source) in source_rows.iter().enumerate() {
            for (role, amount_index, department_index) in [("budget", 6, 3), ("actual", 7, 4)] {
                if source[amount_index].is_null() {
                    continue;
                }
                records.push(row(
                    role,
                    &format!("real-{i}-{role}"),
                    &[
                        ("fy", &source[0].to_string()),
                        ("account", source[2].as_str().unwrap()),
                        ("account_label", source[5].as_str().unwrap()),
                        ("center", source[1].as_str().unwrap()),
                        (
                            "department",
                            source[department_index].as_str().unwrap_or(""),
                        ),
                        ("program", ""),
                        ("amount", source[amount_index].as_str().unwrap()),
                        ("basis", "adopted"),
                    ],
                ));
                source_links.push(serde_json::json!({"citationId":format!("real-{i}-{role}"),"evidenceId":evidence.id,"rowIndex":i,"columns":result["columns"],"row":source}));
            }
        }
        let history: serde_json::Value = serde_json::from_str(&artifacts[2].text).unwrap();
        for (i, source) in history["rows"].as_array().unwrap().iter().enumerate() {
            for (role, column) in [("budget_history", 2), ("actual_history", 3)] {
                if let Some(amount) = source[column].as_str() {
                    let id = format!("history-{i}-{role}");
                    records.push(row(
                        role,
                        &id,
                        &[
                            ("fy", &source[0].to_string()),
                            ("department", source[1].as_str().unwrap()),
                            ("amount", amount),
                            ("program", ""),
                            ("center", ""),
                            ("account", ""),
                        ],
                    ));
                    source_links.push(serde_json::json!({"citationId":id,"evidenceId":artifacts[2].id,"rowIndex":i,"columns":history["columns"],"row":source}));
                }
            }
        }
        let replay = analyze(
            FinancialScope {
                fiscal_year: Some(2023),
                fund: Some("100000".into()),
                department: None,
            },
            records,
            vec![],
            BTreeMap::new(),
            vec![],
            &BTreeSet::new(),
        );
        for source in source_rows {
            let observed = replay
                .reconciliation
                .iter()
                .find(|r| {
                    r.account == source[2].as_str().unwrap()
                        && r.center == source[1].as_str().unwrap()
                })
                .unwrap();
            if source[13].is_null() {
                assert!(observed.actual_minus_adopted.is_none());
            } else {
                assert_eq!(
                    observed.actual_minus_adopted.as_deref().and_then(decimal),
                    decimal(source[13].as_str().unwrap())
                );
            }
            assert!(observed.revised.is_none());
            assert!(observed.encumbered.is_none());
            assert!(observed.uncommitted_revised.is_none());
        }
        let homelessness = replay
            .reconciliation
            .iter()
            .find(|r| r.center == "1716000011" && r.account == "512059")
            .unwrap();
        assert_eq!(homelessness.adopted.as_deref(), Some("11332872.00"));
        assert_eq!(homelessness.actual.as_deref(), Some("769568.00"));
        assert_eq!(
            homelessness.actual_minus_adopted.as_deref(),
            Some("-10563304.00")
        );
        let shift = replay
            .findings
            .iter()
            .find(|f| f.kind == "account_shift" && f.observation.contains("1716000011"))
            .unwrap();
        assert_eq!(shift.calculations[0].result, "8767029.00");
        assert_eq!(shift.calculations[1].result, "-1796275.00");
        assert!(shift.next_test.contains("512059 and 512066"));
        assert_eq!(shift.citations.len(), 4);
        let parks = replay
            .findings
            .iter()
            .find(|f| f.kind == "account_shift" && f.observation.contains("1714131301"))
            .unwrap();
        assert_eq!(parks.calculations[1].result, "-1137712.00");
        let rebound = replay
            .findings
            .iter()
            .find(|f| {
                f.kind == "rebound_below_baseline"
                    && f.observation.contains("Homelessness Strategies")
            })
            .unwrap();
        assert_eq!(rebound.calculations[0].result, "19858922.00");
        assert_eq!(rebound.calculations[1].result, "13104424.00");
        let capacity = replay
            .findings
            .iter()
            .find(|f| {
                f.kind == "execution_capacity_gap"
                    && f.observation.contains("Commission on Police Practices")
            })
            .unwrap();
        assert_eq!(capacity.calculations[0].result, "1223218.00");
        assert_eq!(capacity.calculations[1].result, "300402.00");
        for lead in [&shift, &parks, &rebound, &capacity] {
            assert!(lead.citations.iter().all(|id| source_links
                .iter()
                .any(|source| source["citationId"].as_str() == Some(id))));
        }
        if let Ok(path) = std::env::var("ONRECORD_FINANCIAL_REPLAY_OUTPUT") {
            std::fs::write(path,serde_json::to_vec_pretty(&serde_json::json!({"scope":replay.scope,"status":"Offline production-engine replay of preserved executed query rows; not a fresh download or a conclusion about service delivery.","findings":[shift,parks,rebound,capacity],"sourceRows":source_links,"executedEvidence":[artifacts[1],artifacts[2]]})).unwrap()).unwrap();
        }
    }
    #[test]
    fn financial_funding_duration_separates_grains_and_limits_scenarios_to_one_period() {
        let separated = report(vec![
            row(
                "budget",
                "one",
                &[
                    ("amount", "100"),
                    ("basis", "adopted"),
                    ("funding_duration", "one-time"),
                ],
            ),
            row(
                "actual",
                "recurring",
                &[("amount", "50"), ("funding_duration", "recurring")],
            ),
            row(
                "actual",
                "mixed",
                &[("amount", "25"), ("funding_duration", "mixed")],
            ),
            row(
                "actual",
                "unknown",
                &[("amount", "10"), ("funding_duration", "annual")],
            ),
        ]);
        assert_eq!(separated.reconciliation.len(), 4);
        assert!(separated
            .reconciliation
            .iter()
            .all(|line| line.actual_minus_adopted.is_none()));
        assert!(separated
            .reconciliation
            .iter()
            .find(|line| line.funding_duration == "unknown")
            .unwrap()
            .limitations
            .iter()
            .any(|limit| limit.contains("duration is unknown")));
        for duration in ["one_time", "recurring", "mixed", "unknown"] {
            let r = report(vec![
                row(
                    "budget",
                    "b",
                    &[
                        ("amount", "100"),
                        ("basis", "revised"),
                        ("funding_duration", duration),
                    ],
                ),
                row(
                    "actual",
                    "a",
                    &[("amount", "10"), ("funding_duration", duration)],
                ),
                row(
                    "encumbrance",
                    "e",
                    &[("amount", "10"), ("funding_duration", duration)],
                ),
                outcome("o", "100", "10"),
                row("restriction", "r", &[("restriction", "unrestricted")]),
            ]);
            let input = FinancialScenarioInput {
                scope: r.scope.clone(),
                funding_key: r.reconciliation[0].key.clone(),
                outcome_key: r.outcomes[0].key.clone(),
                target_program: "Shelter".into(),
                proposed_amount: "20".into(),
            };
            let estimate = scenario(&r, &input).unwrap();
            assert_eq!(estimate.status, "conditional_estimate");
            assert_eq!(estimate.funding_duration, duration);
            assert_eq!(estimate.period_scope, "FY2025 only; one fiscal period");
            assert_eq!(estimate.estimated_whole_units.as_deref(), Some("2"));
            assert!(estimate
                .assumptions
                .iter()
                .any(|a| a.contains("No continuation beyond this fiscal period is calculated")));
            let mut unscoped = r.clone();
            unscoped.scope.fiscal_year = None;
            assert_eq!(scenario(&unscoped, &input).unwrap().status, "blocked");
        }
    }
    #[test]
    fn financial_agency_isolation_prevents_shared_codes_becoming_false_comparisons() {
        let city = row("budget", "city", &[("amount", "100"), ("basis", "adopted")]);
        let county = row(
            "actual",
            "county",
            &[("amount", "120"), ("agency", "County of San Diego")],
        );
        let city_outcome = outcome("city-outcome", "100", "10");
        let mut county_outcome = outcome("county-outcome", "20", "10");
        county_outcome
            .values
            .insert("agency".into(), Some("County of San Diego".into()));
        let r = report(vec![city, county, city_outcome, county_outcome]);
        assert_eq!(r.reconciliation.len(), 2);
        assert!(r
            .reconciliation
            .iter()
            .all(|r| r.actual_minus_adopted.is_none()));
        assert_ne!(
            r.outcomes[0].comparison_group,
            r.outcomes[1].comparison_group
        );
        let input = FinancialScenarioInput {
            scope: r.scope.clone(),
            funding_key: r
                .reconciliation
                .iter()
                .find(|r| r.agency == "City of San Diego")
                .unwrap()
                .key
                .clone(),
            outcome_key: r.outcomes[1].key.clone(),
            target_program: "Shelter".into(),
            proposed_amount: "10".into(),
        };
        let blocked = scenario(&r, &input).unwrap();
        assert_eq!(blocked.status, "blocked");
        assert!(blocked
            .blockers
            .iter()
            .any(|b| b.contains("same agency/jurisdiction")));
    }
    #[test]
    fn financial_deep_patterns_reject_incompatible_accounts_periods_and_incomplete_history() {
        let records = vec![
            row(
                "budget",
                "b-down",
                &[
                    ("account", "512059"),
                    ("amount", "150"),
                    ("basis", "adopted"),
                ],
            ),
            row(
                "actual",
                "a-down",
                &[("account", "512059"), ("amount", "50")],
            ),
            row(
                "budget",
                "b-up",
                &[
                    ("account", "512066"),
                    ("amount", "100"),
                    ("basis", "adopted"),
                ],
            ),
            row(
                "actual",
                "a-up",
                &[("account", "512066"), ("amount", "190")],
            ),
        ];
        let normal = report(records.clone());
        assert_eq!(
            normal
                .findings
                .iter()
                .filter(|f| f.kind == "account_shift")
                .count(),
            1
        );
        for (field, other) in [
            ("agency", "Other agency"),
            ("fund", "other fund"),
            ("center", "other center"),
            ("currency", "CAD"),
            ("funding_duration", "recurring"),
            ("account", "513066"),
        ] {
            let mut changed = records.clone();
            for record in &mut changed[2..] {
                record.values.insert(field.into(), Some(other.into()));
            }
            assert!(
                !report(changed)
                    .findings
                    .iter()
                    .any(|f| f.kind == "account_shift"),
                "must not join on {field}"
            );
        }
        let partial = analyze(
            normal.scope,
            records,
            vec![],
            BTreeMap::new(),
            vec![],
            &BTreeSet::from(["actual".into()]),
        );
        assert!(!partial.findings.iter().any(|f| f.kind == "account_shift"));
        let history = vec![
            row(
                "actual_history",
                "2021",
                &[("fy", "2021"), ("amount", "200")],
            ),
            row(
                "actual_history",
                "2022",
                &[("fy", "2022"), ("amount", "40")],
            ),
            row(
                "actual_history",
                "2023",
                &[("fy", "2023"), ("amount", "100")],
            ),
        ];
        assert_eq!(history_findings(&history, &BTreeSet::new()).len(), 1);
        assert_eq!(
            history_findings(&history, &BTreeSet::from(["budget_history".into()])).len(),
            1
        );
        assert!(history_findings(&history, &BTreeSet::from(["actual_history".into()])).is_empty());
        for (field, value) in [
            ("fy", Some("2024")),
            ("currency", Some("CAD")),
            ("agency", Some("Other agency")),
            ("amount", None),
        ] {
            let mut changed = history.clone();
            changed[1]
                .values
                .insert(field.into(), value.map(str::to_string));
            assert!(
                history_findings(&changed, &BTreeSet::new()).is_empty(),
                "must not bridge missing/incompatible {field}"
            );
        }
        let mut unknown = history.clone();
        let mut null_row = history[1].clone();
        null_row.values.insert("amount".into(), None);
        unknown.push(null_row);
        assert!(history_findings(&unknown, &BTreeSet::new()).is_empty());
    }
}
