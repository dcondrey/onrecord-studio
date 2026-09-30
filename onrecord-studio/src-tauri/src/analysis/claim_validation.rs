//! Quantitative claims in prose, checked against the imported budget and actuals tables.
//!
//! A memo that says "total expenditures of $1.2 million in FY 2024" is a claim about rows the
//! analytical workspace may already hold. Extraction is cue-phrase based (word tokens, never
//! patterns over raw text); evaluation binds the claim's stated scope to the financial mapping
//! roles and runs through `run_financial_investigation`, so the basis of any verdict is the same
//! query id and rows the financial workbench cites. A claim whose scope, measure, or jurisdiction
//! cannot be bound to exactly one table is reported as unreconciled, never as a contradiction, and
//! a figure is only contradicted outside the rounding band its own wording carries.

use crate::types::{ClaimCheckOutcome, ClaimCheckScope, ClaimCheckView};
use crate::{
    analysis::{
        assertion_index::AssertionValue,
        finding::{
            finding_id, Finding, FindingCitation, FindingConfidence, FindingKind, FindingSeverity,
            FindingSubject,
        },
        phrase_match::{has_phrase, words, Phrases, Word},
    },
    error::AppError,
    ingestion::regional::source_jurisdiction,
    storage::DuckDbRepository,
    types::{FinancialReport, FinancialScope, FinancialWorkspace},
};
use chrono::Utc;
use std::collections::HashMap;

const UNCLASSIFIED: &str = "Unclassified source";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Basis {
    Adopted,
    Proposed,
    Revised,
}

impl Basis {
    fn stored(self) -> &'static str {
        match self {
            Self::Adopted => "adopted",
            Self::Proposed => "proposed",
            Self::Revised => "revised",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Measure {
    Budget(Basis),
    Actual,
    /// An amount in a budget sentence that names neither budget nor actual spending.
    Unstated,
}

impl Measure {
    fn role(self) -> Option<&'static str> {
        match self {
            Self::Budget(_) => Some("budget"),
            Self::Actual => Some("actual"),
            Self::Unstated => None,
        }
    }

    fn label(self) -> String {
        match self {
            Self::Budget(basis) => format!("{} budget", basis.stored()),
            Self::Actual => "actual expenditures".to_string(),
            Self::Unstated => "an unstated measure".to_string(),
        }
    }
}

/// How a stated figure bounds the truth. "About" widens the band; "over"/"under" make it one-sided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hedge {
    Exact,
    Approximate,
    AtLeast,
    AtMost,
}

/// A stated currency amount with the precision its wording carries: "$1.2 billion" commits to a
/// hundred-million-dollar digit, "$1,234,567" to a dollar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Money {
    pub cents: i64,
    pub precision_cents: i64,
    pub hedge: Hedge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Percent {
    /// Signed basis points; a stated decrease is negative.
    pub basis_points: i64,
    pub precision_basis_points: i64,
    pub hedge: Hedge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BalanceState {
    Balanced,
    Surplus(Option<Money>),
    Deficit(Option<Money>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimKind {
    BudgetBalance(BalanceState),
    /// "Spending stayed within budget" (`true`) or "exceeded the budget" (`false`).
    WithinBudget(bool),
    Total {
        measure: Measure,
        amount: Money,
    },
    PercentChange {
        measure: Measure,
        change: Percent,
        /// An explicitly stated earlier year ("from FY 2023 to FY 2024").
        previous_fiscal_year: Option<i32>,
        /// "year over year", "from the prior year": the previous year is the stated year minus one.
        prior_year_cue: bool,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClaimScope {
    pub fiscal_year: Option<i32>,
    /// Whether the fiscal year was stated in the claim's own sentence rather than inherited from
    /// the document heading.
    pub fiscal_year_in_sentence: bool,
    pub fund: Option<String>,
    pub department: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuantitativeClaim {
    pub kind: ClaimKind,
    pub scope: ClaimScope,
    pub char_start: usize,
    pub char_end: usize,
    pub source_text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnreconciledReason {
    NoMappedDataset,
    AmbiguousScope,
    MissingFiscalYear,
    PartialOrRejectedRows,
    UnsupportedClaim,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimEvaluation {
    Consistent {
        query_ids: Vec<String>,
        observed: String,
        band: String,
        bound_scope: String,
    },
    Mismatch {
        query_ids: Vec<String>,
        claimed: String,
        observed: String,
        difference: String,
        band: String,
        bound_scope: String,
        /// Explicit fund is one of the observable facts confidence is derived from; see
        /// [`Evaluator::mismatch_confidence`].
        fund_explicit: bool,
        /// Difference relative to the observed value, in basis points.
        relative_basis_points: u64,
    },
    Unreconciled {
        reason: UnreconciledReason,
        detail: String,
    },
}

const BALANCED: Phrases = &[
    &["balanced", "budget"],
    &["budget", "was", "balanced"],
    &["budget", "is", "balanced"],
    &["budget", "remains", "balanced"],
    &["budget", "remained", "balanced"],
    &["no", "deficit"],
    &["without", "a", "deficit"],
];
const DEFICIT: Phrases = &[&["a", "deficit"], &["deficit", "of"], &["shortfall", "of"]];
const SURPLUS: Phrases = &[&["a", "surplus"], &["surplus", "of"]];
const WITHIN_BUDGET: Phrases = &[
    &["within", "budget"],
    &["under", "budget"],
    &["below", "budget"],
    &["within", "the", "adopted", "budget"],
    &["within", "the", "approved", "budget"],
    &["did", "not", "exceed", "the", "budget"],
    &["did", "not", "exceed", "its", "budget"],
];
const OVER_BUDGET: Phrases = &[
    &["over", "budget"],
    &["exceeded", "the", "budget"],
    &["exceeded", "its", "budget"],
    &["exceeded", "the", "adopted", "budget"],
];
const ACTUAL: Phrases = &[
    &["total", "expenditures"],
    &["total", "spending"],
    &["actual", "expenditures"],
    &["actual", "spending"],
    &["expenditures", "of"],
    &["expenditures", "totaled"],
    &["expenditures", "were"],
    &["spending", "totaled"],
    &["spent"],
    &["expended"],
    &["actuals"],
];
const ADOPTED: Phrases = &[
    &["adopted", "budget"],
    &["total", "budget"],
    &["budget", "of"],
    &["budgeted"],
    &["appropriated"],
    &["appropriations", "of"],
    &["total", "appropriations"],
];
const PROPOSED: Phrases = &[&["proposed", "budget"], &["proposed"]];
const REVISED: Phrases = &[&["revised", "budget"], &["amended", "budget"]];
const INCREASE: Phrases = &[
    &["increase", "of"],
    &["increased", "by"],
    &["rose", "by"],
    &["grew", "by"],
    &["up", "by"],
];
const DECREASE: Phrases = &[
    &["decrease", "of"],
    &["decreased", "by"],
    &["reduced", "by"],
    &["reduction", "of"],
    &["fell", "by"],
    &["declined", "by"],
    &["cut", "by"],
    &["down", "by"],
];
const PRIOR_YEAR: Phrases = &[
    &["year", "over", "year"],
    &["from", "the", "prior", "year"],
    &["from", "the", "previous", "year"],
    &["from", "last", "year"],
    &["over", "the", "prior", "year"],
    &["over", "the", "previous", "year"],
    &["over", "last", "year"],
    &["compared", "to", "last", "year"],
    &["compared", "with", "last", "year"],
    &["compared", "to", "the", "prior", "year"],
    &["compared", "with", "the", "prior", "year"],
    &["from", "the", "year", "before"],
];
const APPROXIMATE: Phrases = &[
    &["about"],
    &["approximately"],
    &["roughly"],
    &["around"],
    &["an", "estimated"],
    &["some"],
];
const AT_LEAST: Phrases = &[
    &["over"],
    &["more", "than"],
    &["at", "least"],
    &["exceeding"],
    &["in", "excess", "of"],
    &["upwards", "of"],
];
const AT_MOST: Phrases = &[
    &["under"],
    &["less", "than"],
    &["at", "most"],
    &["below"],
    &["nearly"],
    &["almost"],
    &["no", "more", "than"],
    &["up", "to"],
];
/// Words that make a bare four-digit year read as a fiscal year rather than a date or a case number.
const FISCAL_CONTEXT: Phrases = &[
    &["budget"],
    &["fiscal"],
    &["appropriations"],
    &["expenditures"],
    &["spending"],
];
const SCOPE_STOP_WORDS: &[&str] = &[
    "the", "a", "an", "this", "that", "each", "every", "of", "in", "from", "within", "to", "for",
    "its", "our", "city", "citys", "city's",
];

/// Extracts every quantitative claim in `text`, one per sentence at most, in document order.
pub fn extract_claims(text: &str) -> Vec<QuantitativeClaim> {
    let heading_year = sentences(text)
        .into_iter()
        .take_while(|(start, _)| *start < 600)
        .find_map(|(start, end)| fiscal_years(&words(&text[start..end])).last().copied());
    sentences(text)
        .into_iter()
        .filter_map(|(start, end)| {
            let sentence = &text[start..end];
            let tokens = words(sentence);
            let kind = classify(sentence, &tokens)?;
            let years = fiscal_years(&tokens);
            let sentence_year = years.last().copied();
            let scope = ClaimScope {
                fiscal_year: sentence_year.or(heading_year),
                fiscal_year_in_sentence: sentence_year.is_some(),
                fund: fund_name(&tokens),
                department: department_name(&tokens),
            };
            Some(QuantitativeClaim {
                kind,
                scope,
                char_start: text[..start].chars().count(),
                char_end: text[..end].chars().count(),
                source_text: sentence.trim().to_string(),
            })
        })
        .collect()
}

fn hedge(tokens: &[Word]) -> Hedge {
    if has_phrase(tokens, AT_LEAST) {
        Hedge::AtLeast
    } else if has_phrase(tokens, AT_MOST) {
        Hedge::AtMost
    } else if has_phrase(tokens, APPROXIMATE) {
        Hedge::Approximate
    } else {
        Hedge::Exact
    }
}

fn classify(sentence: &str, tokens: &[Word]) -> Option<ClaimKind> {
    let hedge = hedge(tokens);
    let money = scan_money(sentence, hedge);
    if has_phrase(tokens, BALANCED) {
        return Some(ClaimKind::BudgetBalance(BalanceState::Balanced));
    }
    if has_phrase(tokens, DEFICIT) {
        return Some(ClaimKind::BudgetBalance(BalanceState::Deficit(money)));
    }
    if has_phrase(tokens, SURPLUS) {
        return Some(ClaimKind::BudgetBalance(BalanceState::Surplus(money)));
    }
    if has_phrase(tokens, WITHIN_BUDGET) {
        return Some(ClaimKind::WithinBudget(true));
    }
    if has_phrase(tokens, OVER_BUDGET) {
        return Some(ClaimKind::WithinBudget(false));
    }
    let measure = if has_phrase(tokens, PROPOSED) {
        Measure::Budget(Basis::Proposed)
    } else if has_phrase(tokens, REVISED) {
        Measure::Budget(Basis::Revised)
    } else if has_phrase(tokens, ACTUAL) {
        Measure::Actual
    } else if has_phrase(tokens, ADOPTED) {
        Measure::Budget(Basis::Adopted)
    } else if has_phrase(tokens, FISCAL_CONTEXT) {
        Measure::Unstated
    } else {
        return None;
    };
    if let Some(change) = scan_percent(sentence, hedge) {
        let signed = if has_phrase(tokens, DECREASE) {
            Some(-change.basis_points)
        } else if has_phrase(tokens, INCREASE) {
            Some(change.basis_points)
        } else {
            None
        };
        if let Some(basis_points) = signed {
            let years = fiscal_years(tokens);
            return Some(ClaimKind::PercentChange {
                measure,
                change: Percent {
                    basis_points,
                    ..change
                },
                previous_fiscal_year: (years.len() >= 2).then(|| years[0]),
                prior_year_cue: has_phrase(tokens, PRIOR_YEAR),
            });
        }
    }
    money.map(|amount| ClaimKind::Total { measure, amount })
}

/// Byte ranges of sentences. A period ends a sentence only before whitespace or end of text, so
/// "$1.2 million" stays whole.
fn sentences(text: &str) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut start = 0usize;
    let bytes = text.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        let ends = match bytes[index] {
            b'\n' | b';' | b'!' | b'?' => true,
            b'.' => bytes
                .get(index + 1)
                .is_none_or(|next| next.is_ascii_whitespace()),
            _ => false,
        };
        index += 1;
        if ends {
            if text[start..index].trim().len() >= 12 {
                ranges.push((start, index));
            }
            start = index;
        }
    }
    if text[start..].trim().len() >= 12 {
        ranges.push((start, bytes.len()));
    }
    ranges
}

/// Whether `text` holds a `$` immediately followed by a digit.
fn has_dollar_amount(text: &str) -> bool {
    text.match_indices('$')
        .any(|(index, _)| text[index + 1..].starts_with(|c: char| c.is_ascii_digit()))
}

/// Parses the one `$` amount in `sentence`: digits with thousands separators, an optional
/// fraction, then an optional magnitude word. A sentence naming two amounts ("grew from $1.0
/// million to $1.2 million") states a comparison, not one total, so it yields no amount: picking
/// either figure would bind it to the sentence's fiscal year and contradict a correct document.
fn scan_money(sentence: &str, hedge: Hedge) -> Option<Money> {
    let dollar = sentence.find('$')?;
    let rest = &sentence[dollar + 1..];
    let (whole, fraction, consumed) = scan_number(rest)?;
    if has_dollar_amount(&rest[consumed..]) {
        return None;
    }
    let after = rest[consumed..].trim_start();
    let magnitude_word = after
        .split(|c: char| !c.is_alphanumeric())
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    let magnitude: i64 = match magnitude_word.as_str() {
        "thousand" | "k" => 1_000,
        "million" | "m" | "mm" => 1_000_000,
        "billion" | "b" | "bn" => 1_000_000_000,
        _ => 1,
    };
    let decimals = fraction.len() as u32;
    let fraction_value = fraction.parse::<i64>().unwrap_or(0);
    let divisor = 10i64.checked_pow(decimals)?;
    let cents = whole
        .checked_mul(magnitude)?
        .checked_mul(100)?
        .checked_add(fraction_value.checked_mul(magnitude)?.checked_mul(100)? / divisor)?;
    let precision_cents = if magnitude == 1 {
        match decimals {
            0 => 100,
            1 => 10,
            _ => 1,
        }
    } else {
        (magnitude * 100) / divisor
    };
    Some(Money {
        cents,
        precision_cents: precision_cents.max(1),
        hedge,
    })
}

/// Whether the text after a number reads it as a percentage: `%`, `percent`, `per cent`. A
/// "percentage point" is a change in a share, not a percent change of the measure.
fn percent_marker(after: &str) -> bool {
    if after.starts_with('%') {
        return true;
    }
    let mut tokens = after
        .split(|c: char| !c.is_alphabetic())
        .filter(|t| !t.is_empty())
        .map(str::to_ascii_lowercase);
    match tokens.next().as_deref() {
        Some("percent") => true,
        Some("per") => tokens.next().as_deref() == Some("cent"),
        _ => false,
    }
}

/// Parses the one percentage in `sentence` as basis points with the precision its digits carry.
/// Two percentages ("rose 5% in FY 2023 and 12% in FY 2024") state a comparison, not one change.
fn scan_percent(sentence: &str, hedge: Hedge) -> Option<Percent> {
    let mut found: Option<Percent> = None;
    let mut search = 0usize;
    while search < sentence.len() {
        let slice = &sentence[search..];
        let Some(digit_at) = slice.find(|c: char| c.is_ascii_digit()) else {
            break;
        };
        let (whole, fraction, consumed) = scan_number(&slice[digit_at..])?;
        let after = slice[digit_at + consumed..].trim_start();
        if percent_marker(after) {
            if found.is_some() {
                return None;
            }
            let decimals = fraction.len() as u32;
            let divisor = 10i64.checked_pow(decimals)?;
            let fraction_value = fraction.parse::<i64>().unwrap_or(0);
            found = Some(Percent {
                basis_points: whole
                    .checked_mul(100)?
                    .checked_add(fraction_value.checked_mul(100)? / divisor)?,
                precision_basis_points: (100 / divisor).max(1),
                hedge,
            });
        }
        search += digit_at + consumed.max(1);
    }
    found
}

/// `(whole, fraction digits, bytes consumed)` for a leading number with `,` separators.
fn scan_number(text: &str) -> Option<(i64, &str, usize)> {
    let bytes = text.as_bytes();
    let mut index = 0usize;
    let mut whole: i64 = 0;
    let mut saw_digit = false;
    while index < bytes.len() {
        match bytes[index] {
            digit @ b'0'..=b'9' => {
                whole = whole.checked_mul(10)?.checked_add((digit - b'0') as i64)?;
                saw_digit = true;
            }
            b',' if saw_digit
                && bytes
                    .get(index + 1)
                    .is_some_and(|next| next.is_ascii_digit()) => {}
            _ => break,
        }
        index += 1;
    }
    if !saw_digit {
        return None;
    }
    let mut fraction = "";
    if bytes.get(index) == Some(&b'.') {
        let fraction_start = index + 1;
        let mut end = fraction_start;
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            end += 1;
        }
        if end > fraction_start {
            fraction = &text[fraction_start..end];
            index = end;
        }
    }
    Some((whole, fraction, index))
}

fn parse_year(word: &str) -> Option<i32> {
    match word.len() {
        4 => word
            .parse::<i32>()
            .ok()
            .filter(|year| (1990..=2100).contains(year)),
        2 => word.parse::<i32>().ok().map(|year| 2000 + year),
        _ => None,
    }
}

/// Every fiscal year the sentence states, ascending: "FY 2024", "FY2024", "FY 24", "fiscal year
/// 2023-24" (the ending year), or bare years when the sentence is about a budget.
fn fiscal_years(tokens: &[Word]) -> Vec<i32> {
    let mut years = Vec::new();
    // "2023-24" and "2023/2024" name the fiscal year ending in the second year, whether the
    // range follows "FY ", "fiscal year ", or is glued to the prefix as "FY2023-24".
    let range_end = |index: usize, year: i32| -> i32 {
        let ranged = tokens
            .get(index)
            .is_some_and(|w| matches!(w.followed_by, Some('-' | '/' | '–')));
        ranged
            .then(|| tokens.get(index + 1).and_then(|w| parse_year(&w.text)))
            .flatten()
            .unwrap_or(year)
    };
    let year_at = |index: usize| -> Option<i32> {
        let year = parse_year(&tokens.get(index)?.text)?;
        Some(range_end(index, year))
    };
    for (index, word) in tokens.iter().enumerate() {
        if word.text == "fy" {
            years.extend(year_at(index + 1));
        } else if let Some(year) = word.text.strip_prefix("fy").and_then(parse_year) {
            years.push(range_end(index, year));
        } else if word.text == "fiscal" && tokens.get(index + 1).is_some_and(|w| w.text == "year") {
            years.extend(year_at(index + 2));
        }
    }
    if years.is_empty() && has_phrase(tokens, FISCAL_CONTEXT) {
        years.extend(
            tokens
                .iter()
                .filter(|word| word.text.len() == 4)
                .filter_map(|word| parse_year(&word.text)),
        );
    }
    years.sort_unstable();
    years.dedup();
    years
}

fn preceding_name(tokens: &[Word], index: usize) -> Option<String> {
    let mut parts = Vec::new();
    for word in tokens[..index].iter().rev().take(2) {
        if word.text.chars().all(|c| c.is_alphabetic())
            && !SCOPE_STOP_WORDS.contains(&word.text.as_str())
        {
            parts.push(word.text.clone());
        } else {
            break;
        }
    }
    if parts.is_empty() {
        return None;
    }
    parts.reverse();
    Some(parts.join(" "))
}

/// "General Fund", "fund 100000".
fn fund_name(tokens: &[Word]) -> Option<String> {
    tokens.iter().enumerate().find_map(|(index, word)| {
        if word.text != "fund" {
            return None;
        }
        if let Some(next) = tokens.get(index + 1) {
            if next.text.chars().all(|c| c.is_ascii_digit()) {
                return Some(next.text.clone());
            }
        }
        preceding_name(tokens, index).map(|name| format!("{name} fund"))
    })
}

/// "Police Department", "Department of Parks".
fn department_name(tokens: &[Word]) -> Option<String> {
    tokens.iter().enumerate().find_map(|(index, word)| {
        if word.text != "department" {
            return None;
        }
        if tokens.get(index + 1).is_some_and(|w| w.text == "of") {
            let name: Vec<&str> = tokens[index + 2..]
                .iter()
                .take(2)
                .filter(|w| w.text.chars().all(|c| c.is_alphabetic()))
                .map(|w| w.text.as_str())
                .collect();
            if !name.is_empty() {
                return Some(name.join(" "));
            }
        }
        preceding_name(tokens, index)
    })
}

fn normalized_tokens(value: &str) -> Vec<String> {
    words(value).into_iter().map(|word| word.text).collect()
}

/// `stated` names `candidate` only exactly: a stated fund number must appear among the
/// candidate's tokens, and a stated name must equal the candidate's non-numeric tokens. A subset
/// test would let "general fund" bind to a lone "general fund reserve", which is a different fund.
fn tokens_match(stated: &[String], candidate: &str) -> bool {
    let is_number = |token: &String| token.chars().all(|c| c.is_ascii_digit());
    let candidate = normalized_tokens(candidate);
    if candidate.is_empty() {
        return false;
    }
    if let [number] = stated {
        if is_number(number) {
            return candidate.contains(number);
        }
    }
    let candidate_name: Vec<&String> = candidate.iter().filter(|t| !is_number(t)).collect();
    let stated_name: Vec<&String> = stated.iter().filter(|t| !is_number(t)).collect();
    !candidate_name.is_empty() && candidate_name == stated_name
}

/// Holds the financial workspace, portal jurisdictions, and every investigation already run this
/// sweep, so claims that share a scope share one set of queries.
pub struct Evaluator {
    workspace: FinancialWorkspace,
    portal_jurisdictions: HashMap<String, &'static str>,
    reports: HashMap<(i32, String, Option<String>), FinancialReport>,
    /// Distinct values of a mapped field per role, keyed `(role, field)`.
    /// Distinct values per (role, field), with whether the 5000-row read was cut short.
    values: HashMap<(String, String), (Vec<String>, bool)>,
}

impl Evaluator {
    pub async fn load(repository: &DuckDbRepository) -> Result<Self, AppError> {
        let portal_jurisdictions = repository
            .list_portal_registrations()
            .await?
            .into_iter()
            .map(|source| {
                (
                    source.registration.portal_key,
                    source_jurisdiction(&source.registration.canonical_url),
                )
            })
            .collect();
        Ok(Self {
            workspace: repository.financial_workspace().await?,
            portal_jurisdictions,
            reports: HashMap::new(),
            values: HashMap::new(),
        })
    }

    /// The distinct values a role's own dataset holds for a mapped field. The workspace's picker
    /// lists come from one preferred dataset, so a fund that only the budget table carries would
    /// otherwise read as unknown when a budget claim names it.
    async fn distinct_values(
        &mut self,
        repository: &DuckDbRepository,
        role: &str,
        field: &str,
    ) -> Result<(Vec<String>, bool), AppError> {
        let key = (role.to_string(), field.to_string());
        if let Some(values) = self.values.get(&key) {
            return Ok(values.clone());
        }
        let Some(mapping) = self.workspace.mappings.iter().find(|m| m.role == role) else {
            return Ok((Vec::new(), false));
        };
        let Some(column) = mapping.columns.get(field) else {
            self.values.insert(key, (Vec::new(), false));
            return Ok((Vec::new(), false));
        };
        let Some(dataset) = self
            .workspace
            .datasets
            .iter()
            .find(|d| d.snapshot_id == mapping.snapshot_id)
        else {
            return Ok((Vec::new(), false));
        };
        let quote = |identifier: &str| format!("\"{}\"", identifier.replace('"', "\"\""));
        let sql = format!(
            "SELECT DISTINCT NULLIF(trim(CAST({column} AS VARCHAR)), '') AS value FROM {table} \
             WHERE {column} IS NOT NULL ORDER BY value LIMIT 5000",
            column = quote(column),
            table = quote(&dataset.table_name),
        );
        let result = repository.execute_restricted_sql(&sql, 5000).await?;
        let truncated = result.truncated;
        let values = result
            .rows
            .into_iter()
            .filter_map(|row| row.first().and_then(|v| v.as_str().map(str::to_string)))
            .collect::<Vec<_>>();
        self.values.insert(key, (values.clone(), truncated));
        Ok((values, truncated))
    }

    pub fn has_any_role(&self, roles: &[&str]) -> bool {
        self.workspace
            .mappings
            .iter()
            .any(|mapping| roles.contains(&mapping.role.as_str()))
    }

    /// Confidence is derived from what the sentence itself pinned down: 6000 bp for a mismatch
    /// whose scope had to be completed from context, +1500 when the fund was stated, +1500 when
    /// the fiscal year was stated in the same sentence.
    fn mismatch_confidence(fund_explicit: bool, year_in_sentence: bool) -> u16 {
        6000 + if fund_explicit { 1500 } else { 0 } + if year_in_sentence { 1500 } else { 0 }
    }

    /// The role's dataset must come from the same jurisdiction as the document making the claim;
    /// a City memo is never checked against a County table, and an unclassified source on either
    /// side is not a match.
    fn check_jurisdiction(&self, portal_key: &str, role: &str) -> Result<(), String> {
        let document = self
            .portal_jurisdictions
            .get(portal_key)
            .copied()
            .unwrap_or(UNCLASSIFIED);
        let mapping = self
            .workspace
            .mappings
            .iter()
            .find(|mapping| mapping.role == role)
            .ok_or_else(|| format!("no dataset is mapped to the {role} role"))?;
        let dataset = self
            .workspace
            .datasets
            .iter()
            .find(|dataset| dataset.snapshot_id == mapping.snapshot_id)
            .ok_or_else(|| format!("the {role} mapping points at a removed dataset"))?;
        let data = dataset
            .source_url
            .as_deref()
            .map(source_jurisdiction)
            .unwrap_or(UNCLASSIFIED);
        if document == UNCLASSIFIED {
            return Err(format!(
                "the document's source '{portal_key}' has no recognised jurisdiction"
            ));
        }
        if data == UNCLASSIFIED {
            return Err(format!(
                "the {role} dataset '{}' has no source URL with a recognised jurisdiction",
                dataset.name
            ));
        }
        if document != data {
            return Err(format!(
                "the document is from {document} but the {role} dataset is from {data}"
            ));
        }
        Ok(())
    }

    fn bind_fund(funds: &[String], stated: Option<&str>) -> Result<(String, bool), String> {
        let Some(stated) = stated else {
            return match funds {
                [only] => Ok((only.clone(), false)),
                [] => Err("the mapped dataset carries no fund values".to_string()),
                _ => Err(format!(
                    "the sentence names no fund and the data has {} distinct funds",
                    funds.len()
                )),
            };
        };
        let stated_tokens = normalized_tokens(stated);
        let matches: Vec<&String> = funds
            .iter()
            .filter(|fund| tokens_match(&stated_tokens, fund))
            .collect();
        match matches.as_slice() {
            [only] => Ok(((*only).clone(), true)),
            [] => Err(format!("no fund in the data matches '{stated}'")),
            _ => Err(format!(
                "{} funds in the data match '{stated}'",
                matches.len()
            )),
        }
    }

    fn bind_department(
        departments: &[String],
        stated: Option<&str>,
    ) -> Result<Option<String>, String> {
        let Some(stated) = stated else {
            return Ok(None);
        };
        let stated_tokens = normalized_tokens(stated);
        let matches: Vec<&String> = departments
            .iter()
            .filter(|department| tokens_match(&stated_tokens, department))
            .collect();
        match matches.as_slice() {
            [only] => Ok(Some((*only).clone())),
            [] => Err(format!("no department in the data matches '{stated}'")),
            _ => Err(format!(
                "{} departments in the data match '{stated}'",
                matches.len()
            )),
        }
    }

    async fn report(
        &mut self,
        repository: &DuckDbRepository,
        scope: FinancialScope,
    ) -> Result<&FinancialReport, AppError> {
        let key = (
            scope.fiscal_year.unwrap_or_default(),
            scope.fund.clone().unwrap_or_default(),
            scope.department.clone(),
        );
        if !self.reports.contains_key(&key) {
            let report = repository.run_financial_investigation(scope).await?;
            self.reports.insert(key.clone(), report);
        }
        Ok(&self.reports[&key])
    }

    pub async fn evaluate(
        &mut self,
        repository: &DuckDbRepository,
        portal_key: &str,
        claim: &QuantitativeClaim,
    ) -> Result<ClaimEvaluation, AppError> {
        let unreconciled =
            |reason, detail: String| Ok(ClaimEvaluation::Unreconciled { reason, detail });
        let (measures, previous_year) = match claim.kind {
            // Revenue is not a mapped financial role and no mapping declares a revenue-versus-
            // expenditure distinction, so a balance claim has nothing to compare against.
            ClaimKind::BudgetBalance(_) => {
                return unreconciled(
                    UnreconciledReason::UnsupportedClaim,
                    "no mapped role distinguishes revenue from expenditure, so a balance, surplus or deficit cannot be computed from expenditure tables alone".to_string(),
                )
            }
            ClaimKind::WithinBudget(_) => (vec![Measure::Budget(Basis::Adopted), Measure::Actual], None),
            ClaimKind::Total { measure, .. } => (vec![measure], None),
            ClaimKind::PercentChange {
                measure,
                previous_fiscal_year,
                prior_year_cue,
                ..
            } => {
                let Some(fiscal_year) = claim.scope.fiscal_year else {
                    return unreconciled(UnreconciledReason::MissingFiscalYear, "the claim states no fiscal year".to_string());
                };
                let previous = match previous_fiscal_year {
                    Some(year) => year,
                    None if prior_year_cue => fiscal_year - 1,
                    None => {
                        return unreconciled(
                            UnreconciledReason::MissingFiscalYear,
                            "a change needs both years: none is stated for the comparison year".to_string(),
                        )
                    }
                };
                (vec![measure], Some(previous))
            }
        };
        let mut roles = Vec::new();
        for measure in &measures {
            match measure.role() {
                Some(role) => roles.push(role),
                None => {
                    return unreconciled(
                        UnreconciledReason::AmbiguousScope,
                        "budget or actual not stated".to_string(),
                    )
                }
            }
        }
        for role in &roles {
            if !self.has_any_role(&[role]) {
                return unreconciled(
                    UnreconciledReason::NoMappedDataset,
                    format!("no imported dataset is mapped to the {role} role"),
                );
            }
            if let Err(detail) = self.check_jurisdiction(portal_key, role) {
                return unreconciled(UnreconciledReason::NoMappedDataset, detail);
            }
        }
        let Some(fiscal_year) = claim.scope.fiscal_year else {
            return unreconciled(
                UnreconciledReason::MissingFiscalYear,
                "the claim states no fiscal year".to_string(),
            );
        };
        // Scope binds against the first role's own distinct values; every other role the claim
        // needs must carry the same bound value, or the comparison would span different funds.
        // A cut-short value list could hide the one fund or department that would have matched,
        // or a second one that would have made the match ambiguous; neither can be bound.
        let (funds, funds_truncated) = self.distinct_values(repository, roles[0], "fund").await?;
        if funds_truncated {
            return unreconciled(
                UnreconciledReason::AmbiguousScope,
                format!(
                    "the {} dataset has more distinct funds than can be read for binding",
                    roles[0]
                ),
            );
        }
        let (fund, fund_explicit) = match Self::bind_fund(&funds, claim.scope.fund.as_deref()) {
            Ok(bound) => bound,
            Err(detail) => return unreconciled(UnreconciledReason::AmbiguousScope, detail),
        };
        let (departments, departments_truncated) = self
            .distinct_values(repository, roles[0], "department")
            .await?;
        if departments_truncated {
            return unreconciled(
                UnreconciledReason::AmbiguousScope,
                format!(
                    "the {} dataset has more distinct departments than can be read for binding",
                    roles[0]
                ),
            );
        }
        let department =
            match Self::bind_department(&departments, claim.scope.department.as_deref()) {
                Ok(bound) => bound,
                Err(detail) => return unreconciled(UnreconciledReason::AmbiguousScope, detail),
            };
        for role in roles.iter().skip(1) {
            let (other_funds, other_truncated) =
                self.distinct_values(repository, role, "fund").await?;
            if other_truncated || !other_funds.contains(&fund) {
                return unreconciled(
                    UnreconciledReason::AmbiguousScope,
                    format!("fund {fund} has no rows in the {role} dataset"),
                );
            }
        }
        let scope = FinancialScope {
            fiscal_year: Some(fiscal_year),
            fund: Some(fund.clone()),
            department: department.clone(),
        };
        let bound_scope = [
            Some(format!("FY {fiscal_year}")),
            Some(format!("fund {fund}")),
            department.as_ref().map(|d| format!("department {d}")),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(", ");

        let mut query_ids = Vec::new();
        let mut totals: HashMap<&'static str, i64> = HashMap::new();
        for measure in &measures {
            let role = measure.role().unwrap_or_default();
            let report = self.report(repository, scope.clone()).await?;
            let Some(total) = role_total(report, *measure) else {
                return unreconciled(
                    UnreconciledReason::PartialOrRejectedRows,
                    format!(
                        "the {role} rows for {bound_scope} are partial, truncated or unparsable"
                    ),
                );
            };
            totals.insert(role, total);
            query_ids.extend(role_query_ids(report, role));
        }
        let previous_total = match previous_year {
            None => None,
            Some(previous_year) => {
                let measure = measures[0];
                let previous_scope = FinancialScope {
                    fiscal_year: Some(previous_year),
                    ..scope.clone()
                };
                let report = self.report(repository, previous_scope).await?;
                let Some(total) = role_total(report, measure) else {
                    return unreconciled(
                        UnreconciledReason::PartialOrRejectedRows,
                        format!("the {} rows for FY {previous_year} are partial, truncated or unparsable", measure.role().unwrap_or_default()),
                    );
                };
                query_ids.extend(role_query_ids(report, measure.role().unwrap_or_default()));
                Some(total)
            }
        };
        query_ids.sort();
        query_ids.dedup();

        let verdict =
            |claimed: String, observed: String, band: String, difference: i64, reference: i64| {
                if difference == 0 {
                    return ClaimEvaluation::Consistent {
                        query_ids: query_ids.clone(),
                        observed,
                        band,
                        bound_scope: bound_scope.clone(),
                    };
                }
                let relative_basis_points = if reference == 0 {
                    10_000
                } else {
                    (difference.unsigned_abs() as u128 * 10_000 / reference.unsigned_abs() as u128)
                        .min(u64::MAX as u128) as u64
                };
                ClaimEvaluation::Mismatch {
                    query_ids: query_ids.clone(),
                    claimed,
                    observed,
                    difference: AssertionValue::Cents(difference).display(),
                    band,
                    bound_scope: bound_scope.clone(),
                    fund_explicit,
                    relative_basis_points,
                }
            };
        Ok(match claim.kind {
            ClaimKind::Total { measure, amount } => {
                let observed = totals[measure.role().unwrap_or_default()];
                let (lower, upper) = money_band(amount);
                let outside = observed < lower || observed > upper;
                verdict(
                    format!(
                        "{} of {}",
                        measure.label(),
                        AssertionValue::Cents(amount.cents).display()
                    ),
                    format!(
                        "{} of {}",
                        measure.label(),
                        AssertionValue::Cents(observed).display()
                    ),
                    describe_band(amount.cents, lower, upper),
                    if outside {
                        observed.saturating_sub(amount.cents)
                    } else {
                        0
                    },
                    observed,
                )
            }
            ClaimKind::WithinBudget(within) => {
                let budget = totals["budget"];
                let actual = totals["actual"];
                // "Within budget" is read at dollar precision: an overrun of less than a dollar is
                // bookkeeping noise, not a contradiction.
                let overspent = actual > budget.saturating_add(100);
                verdict(
                    if within {
                        "spending within the adopted budget".to_string()
                    } else {
                        "spending over the adopted budget".to_string()
                    },
                    format!(
                        "adopted budget {} against actual expenditures {}",
                        AssertionValue::Cents(budget).display(),
                        AssertionValue::Cents(actual).display()
                    ),
                    "±$1.00 of the adopted total".to_string(),
                    if overspent == within {
                        actual.saturating_sub(budget)
                    } else {
                        0
                    },
                    budget,
                )
            }
            ClaimKind::PercentChange {
                measure, change, ..
            } => {
                let current = totals[measure.role().unwrap_or_default()];
                let previous = previous_total.unwrap_or_default();
                if previous == 0 {
                    return unreconciled(
                        UnreconciledReason::PartialOrRejectedRows,
                        "the comparison year total is zero, so no percentage change is defined"
                            .to_string(),
                    );
                }
                let observed_bp = ((current as i128 - previous as i128) * 10_000 / previous as i128)
                    .clamp(i64::MIN as i128, i64::MAX as i128)
                    as i64;
                let (lower, upper) = percent_band(change);
                let outside = observed_bp < lower || observed_bp > upper;
                verdict(
                    format!(
                        "{} changed by {}",
                        measure.label(),
                        display_basis_points(change.basis_points)
                    ),
                    format!(
                        "{} changed by {} ({} to {})",
                        measure.label(),
                        display_basis_points(observed_bp),
                        AssertionValue::Cents(previous).display(),
                        AssertionValue::Cents(current).display()
                    ),
                    format!(
                        "{} to {}",
                        display_basis_points(lower),
                        display_basis_points(upper)
                    ),
                    if outside {
                        current.saturating_sub(previous)
                    } else {
                        0
                    },
                    previous,
                )
            }
            ClaimKind::BudgetBalance(_) => unreachable!("returned above"),
        })
    }
}

/// The inclusive range of totals a stated amount is consistent with. Half a unit of the stated
/// precision either side; "about" widens that to a full unit; "over"/"under" leave one side open.
fn money_band(amount: Money) -> (i64, i64) {
    band(amount.cents, amount.precision_cents, amount.hedge)
}

fn percent_band(change: Percent) -> (i64, i64) {
    band(
        change.basis_points,
        change.precision_basis_points,
        change.hedge,
    )
}

/// Saturating: a stated figure near `i64::MAX` (which `scan_number` admits) must widen to the
/// type's edge, not wrap into a band that excludes everything.
fn band(stated: i64, precision: i64, hedge: Hedge) -> (i64, i64) {
    let half = (precision / 2).max(1);
    match hedge {
        Hedge::Exact => (stated.saturating_sub(half), stated.saturating_add(half)),
        Hedge::Approximate => (
            stated.saturating_sub(half * 2),
            stated.saturating_add(half * 2),
        ),
        Hedge::AtLeast => (stated.saturating_sub(half), i64::MAX),
        Hedge::AtMost => (i64::MIN, stated.saturating_add(half)),
    }
}

fn describe_band(claimed: i64, lower: i64, upper: i64) -> String {
    match (lower, upper) {
        (i64::MIN, upper) => format!("at most {}", AssertionValue::Cents(upper).display()),
        (lower, i64::MAX) => format!("at least {}", AssertionValue::Cents(lower).display()),
        (lower, _) => format!("±{}", AssertionValue::Cents(claimed - lower).display()),
    }
}

fn display_basis_points(basis_points: i64) -> String {
    let sign = if basis_points < 0 { "-" } else { "+" };
    let magnitude = basis_points.unsigned_abs();
    format!("{sign}{}.{:02}%", magnitude / 100, magnitude % 100)
}

/// Sum of a role's aggregated rows, one basis only for budgets. `None` when the report marks the
/// role incomplete or any row's amount failed to parse: a partial total is not a total.
fn role_total(report: &FinancialReport, measure: Measure) -> Option<i64> {
    let role = measure.role()?;
    if report.incomplete_roles.iter().any(|r| r == role) {
        return None;
    }
    let mut total = 0i64;
    let mut rows = 0usize;
    for record in report.records.iter().filter(|r| r.role == role) {
        if let Measure::Budget(basis) = measure {
            if !record
                .values
                .get("basis")
                .and_then(|b| b.as_deref())
                .is_some_and(|b| b.eq_ignore_ascii_case(basis.stored()))
            {
                continue;
            }
        }
        let amount = record.values.get("amount")?.as_deref()?;
        total = total.checked_add(decimal_to_cents(amount)?)?;
        rows += 1;
    }
    (rows > 0).then_some(total)
}

fn role_query_ids(report: &FinancialReport, role: &str) -> Vec<String> {
    report
        .evidence
        .iter()
        .filter(|citation| citation.role == role)
        .map(|citation| citation.query_id.clone())
        .collect()
}

fn decimal_to_cents(text: &str) -> Option<i64> {
    let text = text.trim();
    let (negative, digits) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let (whole, fraction, consumed) = scan_number(digits)?;
    if consumed != digits.len() {
        return None;
    }
    let fraction_cents = match fraction.len() {
        0 => 0,
        1 => fraction.parse::<i64>().ok()? * 10,
        _ => fraction[..2].parse::<i64>().ok()?,
    };
    let cents = whole.checked_mul(100)?.checked_add(fraction_cents)?;
    Some(if negative { -cents } else { cents })
}

/// Where a claim was read from.
pub struct ClaimContext {
    pub portal_key: String,
    pub document_id: i64,
    pub request_id: i64,
    pub file_name: Option<String>,
}

/// Only a mismatch is a finding. Consistent and unreconciled outcomes are returned by the sweep
/// for the record but produce no row: an absent finding is not a statement that a claim held.
pub fn to_finding(
    context: &ClaimContext,
    claim: &QuantitativeClaim,
    evaluation: &ClaimEvaluation,
    kind: FindingKind,
    source_detector: &str,
) -> Option<Finding> {
    let ClaimEvaluation::Mismatch {
        query_ids,
        claimed,
        observed,
        difference,
        band,
        bound_scope,
        fund_explicit,
        relative_basis_points,
    } = evaluation
    else {
        return None;
    };
    let subject = FindingSubject {
        portal_key: Some(context.portal_key.clone()),
        request_id: Some(context.request_id),
        document_id: Some(context.document_id),
        ..Default::default()
    };
    let queries = query_ids.join(", ");
    let id = finding_id(
        kind,
        source_detector,
        &subject,
        &[claimed.as_str(), observed.as_str(), queries.as_str()],
    );
    let severity = if *relative_basis_points >= 2_500 {
        FindingSeverity::Serious
    } else {
        FindingSeverity::Notable
    };
    let file_name = context
        .file_name
        .clone()
        .unwrap_or_else(|| format!("document {}", context.document_id));
    let now = Utc::now();
    Some(Finding {
        id,
        kind,
        severity,
        confidence: FindingConfidence::measured(Evaluator::mismatch_confidence(
            *fund_explicit,
            claim.scope.fiscal_year_in_sentence,
        )),
        subject,
        headline: format!("'{file_name}' states {claimed}; the imported data shows {observed}"),
        explanation: format!(
            "The document states {claimed}, read as consistent with any total within {band}. The \
             imported data bound to {bound_scope}, aggregated by the same queries the financial \
             workbench cites, shows {observed}: a difference of {difference}, outside that band. \
             Re-run query id(s) {queries} in the analysis workbench to see the rows. This compares a \
             written statement with a released dataset; it does not decide which is correct, and a \
             revised or superseded export can account for the gap."
        ),
        citations: vec![FindingCitation {
            portal_key: context.portal_key.clone(),
            document_id: context.document_id,
            character_start: Some(claim.char_start as i64),
            character_end: Some(claim.char_end as i64),
            quote: Some(claim.source_text.chars().take(400).collect()),
            label: Some("The stated figure".to_string()),
        }],
        discovered_at: now,
        last_seen_at: now,
        dismissed_at: None,
        source_detector: source_detector.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::PortalRegistration;
    use duckdb::params;

    #[test]
    fn extracts_each_claim_kind_and_ignores_years_that_are_not_fiscal() {
        let exact = |cents, precision_cents| Money {
            cents,
            precision_cents,
            hedge: Hedge::Exact,
        };
        let cases: &[(&str, Option<ClaimKind>, Option<i32>, Option<&str>)] = &[
            (
                "The FY 2024 budget was balanced with no deficit in the General Fund.",
                Some(ClaimKind::BudgetBalance(BalanceState::Balanced)),
                Some(2024),
                Some("general fund"),
            ),
            (
                "Total expenditures of $1.2 million in fiscal year 2023-24 came from fund 100000.",
                Some(ClaimKind::Total {
                    measure: Measure::Actual,
                    amount: exact(120_000_000, 10_000_000),
                }),
                Some(2024),
                Some("100000"),
            ),
            (
                "The adopted budget of $4,250,000.50 for FY2023 funds the Police Department.",
                Some(ClaimKind::Total {
                    measure: Measure::Budget(Basis::Adopted),
                    amount: exact(425_000_050, 1),
                }),
                Some(2023),
                None,
            ),
            (
                "The proposed budget of over $1.2 billion for FY 2025 is the largest ever.",
                Some(ClaimKind::Total {
                    measure: Measure::Budget(Basis::Proposed),
                    amount: Money {
                        cents: 120_000_000_000,
                        precision_cents: 10_000_000_000,
                        hedge: Hedge::AtLeast,
                    },
                }),
                Some(2025),
                None,
            ),
            (
                "Total spending in fiscal year 2024 decreased by 12.5% from the prior year.",
                Some(ClaimKind::PercentChange {
                    measure: Measure::Actual,
                    change: Percent {
                        basis_points: -1_250,
                        precision_basis_points: 10,
                        hedge: Hedge::Exact,
                    },
                    previous_fiscal_year: None,
                    prior_year_cue: true,
                }),
                Some(2024),
                None,
            ),
            (
                "Actual expenditures remained within budget in FY 24.",
                Some(ClaimKind::WithinBudget(true)),
                Some(2024),
                None,
            ),
            (
                "The budget for 2023 included $500 for the program.",
                Some(ClaimKind::Total {
                    measure: Measure::Unstated,
                    amount: exact(50_000, 100),
                }),
                Some(2023),
                None,
            ),
            (
                "Case 2023-0412 was closed after the requester withdrew the appeal.",
                None,
                None,
                None,
            ),
        ];
        for (text, kind, year, fund) in cases {
            let claims = extract_claims(text);
            assert_eq!(claims.first().map(|c| c.kind), *kind, "{text}");
            if let Some(claim) = claims.first() {
                assert_eq!(claim.scope.fiscal_year, *year, "{text}");
                assert_eq!(claim.scope.fund.as_deref(), *fund, "{text}");
                assert_eq!(claim.source_text, text.trim());
            }
        }
        let department = extract_claims("The Police Department spent $10 in FY 2023.");
        assert_eq!(department[0].scope.department.as_deref(), Some("police"));
    }

    #[test]
    fn a_sentence_naming_two_figures_is_a_comparison_not_a_claim() {
        // Picking the first amount would bind $1.0M to FY 2024 and contradict a correct memo.
        for text in [
            "Total expenditures grew from $1.0 million in FY 2023 to $1.2 million in FY 2024.",
            "Total spending rose by 5% in FY 2023 and by 12% in FY 2024 from the prior year.",
        ] {
            assert!(extract_claims(text).is_empty(), "{text}");
        }
        // A share change in percentage points is not a percent change of the measure.
        let points = extract_claims(
            "Total spending as a share of revenue increased by 2 percentage points in FY 2024 from the prior year.",
        );
        assert!(points.is_empty(), "{points:?}");
        let percent = extract_claims(
            "Total spending increased by 2 per cent in FY 2024 from the prior year.",
        );
        assert!(
            matches!(percent[0].kind, ClaimKind::PercentChange { change, .. } if change.basis_points == 200),
            "{percent:?}"
        );
    }

    #[test]
    fn compact_and_spaced_fiscal_year_ranges_name_the_same_ending_year() {
        for text in [
            "The adopted budget of $50 million for FY2023-24 funds the program.",
            "The adopted budget of $50 million for FY 2023-24 funds the program.",
            "The adopted budget of $50 million for fiscal year 2023/2024 funds the program.",
            "The adopted budget of $50 million for FY2023–24 funds the program.",
        ] {
            let claims = extract_claims(text);
            assert_eq!(claims[0].scope.fiscal_year, Some(2024), "{text}");
        }
    }

    #[test]
    fn figures_at_the_edge_of_i64_widen_to_the_edge_instead_of_wrapping() {
        let claims = extract_claims("Total expenditures of $92,233,720,368,547,758.07 in FY 2024.");
        let ClaimKind::Total { amount, .. } = claims[0].kind else {
            panic!("{claims:?}");
        };
        assert_eq!(amount.cents, i64::MAX);
        assert_eq!(money_band(amount), (i64::MAX - 1, i64::MAX));
        assert!(extract_claims(
            "Total spending increased by 92233720368547758.08% in FY 2024 from the prior year."
        )
        .is_empty());
        assert_eq!(
            percent_band(Percent {
                basis_points: i64::MAX - 10,
                precision_basis_points: 100,
                hedge: Hedge::Approximate,
            }),
            (i64::MAX - 110, i64::MAX)
        );
    }

    async fn repository_with_city_exports() -> (tempfile::TempDir, DuckDbRepository) {
        let temp = tempfile::tempdir().unwrap();
        let repo = DuckDbRepository::new(temp.path().join("core.db")).unwrap();
        repo.run_migrations(include_str!("../../migrations/0001_initial_schema.sql"))
            .await
            .unwrap();
        let portal: PortalRegistration = serde_json::from_value(serde_json::json!({"portalKey":"sandiego", "displayName":"City of San Diego", "canonicalUrl":"https://sandiego.nextrequest.com", "originUrl":"https://sandiego.nextrequest.com", "verifiedAt":"2026-09-13T00:00:00Z", "sourceKind":"NEXT_REQUEST", "apiLayout":{"requestsEndpoint":"https://sandiego.nextrequest.com/api", "requestsPointer":"/resources", "pageParameter":"page", "pageSizeParameter":"limit"}})).unwrap();
        repo.insert_or_update_portal_registration(&portal)
            .await
            .unwrap();
        let conn = repo.analytics_connection().await.unwrap();
        conn.execute_batch("CREATE TABLE analysis_fin_budget(report_fy INTEGER,fund_number VARCHAR,account_number VARCHAR,funds_center_number VARCHAR,dept_name VARCHAR,amount DECIMAL(28,4),budget_cycle VARCHAR);
          CREATE TABLE analysis_fin_actual(report_fy INTEGER,fund_number VARCHAR,account_number VARCHAR,funds_center_number VARCHAR,dept_name VARCHAR,amount DECIMAL(28,4));
          INSERT INTO analysis_fin_budget VALUES (23,'100000','5001','001','Police',100.01,'adopted'),(23,'100000','5001','001','Police',150.01,'revised'),(23,'100000','4001','001','Police',9999,'adopted'),(23,'200000','5001','001','Police',9000,'adopted'),(23,'300000','5001','001','Police',1187654321,'adopted'),(23,'400000','5001','001','Police',1320000000,'adopted');
          INSERT INTO analysis_fin_actual VALUES (23,'100000','5001','001','Police',120.02),(23,'100000','5001','001','Police',0.01),(23,'100000','4001','001','Police',9999);").unwrap();
        for (role, table, path) in [
            (
                "budget",
                "analysis_fin_budget",
                "/operating_budget/budget_operating_datasd.csv",
            ),
            (
                "actual",
                "analysis_fin_actual",
                "/operating_actuals/actuals_operating_datasd.csv",
            ),
        ] {
            conn.execute("INSERT INTO analytical_snapshots(snapshot_id,dataset_kind,display_name,table_name,source_uri,local_path,source_sha256,retrieved_at,validation_summary_json,imported_rows) VALUES (?1,'OPERATING_BUDGET',?1,?2,?3,'/fixture',repeat('a',64),'2026-09-26','{}',3)",params![role,table,format!("https://seshat.datasd.org{path}")]).unwrap();
            let mut statement = conn.prepare("SELECT column_name,data_type,ordinal_position FROM information_schema.columns WHERE table_name=?1 ORDER BY ordinal_position").unwrap();
            let columns = statement
                .query_map(params![table], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, i64>(2)?,
                    ))
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            for (name, kind, ordinal) in columns {
                conn.execute("INSERT INTO analytical_columns(snapshot_id,table_name,column_name,logical_type,ordinal) VALUES (?1,?2,?3,?4,?5)",params![role,table,name,if kind.starts_with("DECIMAL"){"DECIMAL"}else{"TEXT"},ordinal]).unwrap();
            }
        }
        drop(conn);
        (temp, repo)
    }

    #[tokio::test]
    async fn claims_are_consistent_mismatched_or_unreconciled_against_the_imported_tables() {
        let (_temp, repo) = repository_with_city_exports().await;
        let mut evaluator = Evaluator::load(&repo).await.unwrap();
        let first = |text: &str| extract_claims(text).remove(0);

        let consistent = first(
            "The adopted budget of $100.01 for fiscal year 2023 was allocated in fund 100000.",
        );
        assert!(matches!(
            evaluator.evaluate(&repo, "sandiego", &consistent).await.unwrap(),
            ClaimEvaluation::Consistent { ref query_ids, ref bound_scope, .. }
                if !query_ids.is_empty() && bound_scope == "FY 2023, fund 100000"
        ));

        let mismatched =
            first("Total expenditures of $500 in fiscal year 2023 were charged to fund 100000.");
        let evaluation = evaluator
            .evaluate(&repo, "sandiego", &mismatched)
            .await
            .unwrap();
        let ClaimEvaluation::Mismatch {
            observed,
            fund_explicit,
            query_ids,
            band,
            ..
        } = &evaluation
        else {
            panic!("expected a mismatch, got {evaluation:?}");
        };
        assert!(observed.contains("$120.03"), "{observed}");
        assert_eq!(band, "±$0.50");
        assert!(*fund_explicit);
        let context = ClaimContext {
            portal_key: "sandiego".into(),
            document_id: 7,
            request_id: 3,
            file_name: Some("memo.pdf".into()),
        };
        let finding = to_finding(
            &context,
            &mismatched,
            &evaluation,
            FindingKind::ClaimContradictsData,
            "claim_data_mismatch",
        )
        .expect("a mismatch is a finding");
        assert_eq!(
            finding.confidence,
            FindingConfidence::measured(9_000),
            "explicit fund and in-sentence year are the observed basis"
        );
        assert_eq!(finding.severity, FindingSeverity::Serious);
        for query_id in query_ids {
            assert!(finding.explanation.contains(query_id));
        }
        assert!(finding.explanation.contains("FY 2023, fund 100000"));
        let unchanged = evaluator
            .evaluate(&repo, "sandiego", &consistent)
            .await
            .unwrap();
        assert!(to_finding(
            &context,
            &consistent,
            &unchanged,
            FindingKind::ClaimContradictsData,
            "claim_data_mismatch"
        )
        .is_none());

        // "$1.2 billion" commits to the hundred-million digit: ±$50M either side.
        let rounded =
            first("The adopted budget of $1.2 billion for FY 2023 was approved for fund 300000.");
        let rounded_evaluation = evaluator
            .evaluate(&repo, "sandiego", &rounded)
            .await
            .unwrap();
        assert!(
            matches!(rounded_evaluation, ClaimEvaluation::Consistent { .. }),
            "{rounded_evaluation:?}"
        );
        let beyond =
            first("The adopted budget of $1.2 billion for FY 2023 was approved for fund 400000.");
        let beyond_evaluation = evaluator
            .evaluate(&repo, "sandiego", &beyond)
            .await
            .unwrap();
        assert!(
            matches!(beyond_evaluation, ClaimEvaluation::Mismatch { ref band, .. } if band == "±$50000000.00"),
            "{beyond_evaluation:?}"
        );

        // No fund is named and the budget role carries four distinct funds, so the claim cannot
        // be bound. (The actual role holds a single fund, so an unstated fund binds to it there.)
        let ambiguous = first("The adopted budget of $100 for FY 2023 was approved.");
        let ambiguous_evaluation = evaluator
            .evaluate(&repo, "sandiego", &ambiguous)
            .await
            .unwrap();
        assert!(matches!(
            ambiguous_evaluation,
            ClaimEvaluation::Unreconciled {
                reason: UnreconciledReason::AmbiguousScope,
                ..
            }
        ));
        let balance = first("The FY 2023 budget was balanced with no deficit in fund 100000.");
        assert!(matches!(
            evaluator.evaluate(&repo, "sandiego", &balance).await.unwrap(),
            ClaimEvaluation::Unreconciled { reason: UnreconciledReason::UnsupportedClaim, ref detail }
                if detail.contains("revenue")
        ));
        assert!(matches!(
            evaluator
                .evaluate(&repo, "unknown-portal", &consistent)
                .await
                .unwrap(),
            ClaimEvaluation::Unreconciled {
                reason: UnreconciledReason::NoMappedDataset,
                ..
            }
        ));
    }
}

#[cfg(test)]
mod fund_binding_tests {
    use super::{normalized_tokens, tokens_match};

    #[test]
    fn a_stated_fund_binds_only_to_the_same_fund() {
        let general = normalized_tokens("general fund");
        assert!(tokens_match(&general, "GENERAL FUND"));
        assert!(tokens_match(&general, "100000 GENERAL FUND"));
        assert!(!tokens_match(&general, "GENERAL FUND RESERVE"));
        assert!(!tokens_match(&general, "FUND"));
        let number = normalized_tokens("300000");
        assert!(tokens_match(&number, "300000"));
        assert!(!tokens_match(&number, "3000001"));
    }
}

impl ClaimKind {
    pub fn label(&self) -> String {
        match self {
            Self::BudgetBalance(BalanceState::Balanced) => "balanced budget".to_string(),
            Self::BudgetBalance(BalanceState::Surplus(_)) => "surplus".to_string(),
            Self::BudgetBalance(BalanceState::Deficit(_)) => "deficit".to_string(),
            Self::WithinBudget(true) => "spending within budget".to_string(),
            Self::WithinBudget(false) => "spending over budget".to_string(),
            Self::Total { measure, .. } => format!("stated total, {}", measure.label()),
            Self::PercentChange { measure, .. } => format!("percent change, {}", measure.label()),
        }
    }

    fn basis_label(&self) -> Option<String> {
        match self {
            Self::BudgetBalance(_) => None,
            Self::WithinBudget(_) => Some("adopted budget and actual expenditures".to_string()),
            Self::Total { measure, .. } | Self::PercentChange { measure, .. } => {
                Some(measure.label())
            }
        }
    }
}

impl UnreconciledReason {
    pub fn label(self) -> &'static str {
        match self {
            Self::NoMappedDataset => "no mapped dataset",
            Self::AmbiguousScope => "ambiguous scope",
            Self::MissingFiscalYear => "missing fiscal year",
            Self::PartialOrRejectedRows => "partial or rejected rows",
            Self::UnsupportedClaim => "unsupported claim",
        }
    }
}

/// The reader-facing form of one checked claim. Every outcome is carried, not only mismatches,
/// so a reader can see what was checked and why a claim could not be bound.
pub fn claim_check_view(claim: &QuantitativeClaim, evaluation: &ClaimEvaluation) -> ClaimCheckView {
    let result = match evaluation {
        ClaimEvaluation::Consistent {
            query_ids,
            observed,
            band,
            ..
        } => ClaimCheckOutcome::Consistent {
            observed: observed.clone(),
            band: band.clone(),
            query_ids: query_ids.clone(),
        },
        ClaimEvaluation::Mismatch {
            query_ids,
            claimed,
            observed,
            difference,
            band,
            ..
        } => ClaimCheckOutcome::Mismatch {
            claimed: claimed.clone(),
            observed: observed.clone(),
            difference: difference.clone(),
            band: band.clone(),
            query_ids: query_ids.clone(),
        },
        ClaimEvaluation::Unreconciled { reason, detail } => ClaimCheckOutcome::Unreconciled {
            reason: reason.label().to_string(),
            detail: detail.clone(),
        },
    };
    ClaimCheckView {
        claim_text: claim.source_text.chars().take(400).collect(),
        kind_label: claim.kind.label(),
        character_start: claim.char_start,
        character_end: claim.char_end,
        scope: ClaimCheckScope {
            fiscal_year: claim.scope.fiscal_year,
            fund: claim.scope.fund.clone(),
            department: claim.scope.department.clone(),
            basis: claim.kind.basis_label(),
        },
        result,
    }
}

#[cfg(test)]
mod claim_check_view_tests {
    use super::*;

    #[test]
    fn every_outcome_is_carried_with_its_reason_or_figures() {
        let claim = extract_claims(
            "The adopted budget of $1.2 billion for FY 2023 was approved for fund 300000.",
        )
        .pop()
        .expect("total claim");
        let mismatch = claim_check_view(
            &claim,
            &ClaimEvaluation::Mismatch {
                query_ids: vec!["q1".into()],
                claimed: "$1.2B".into(),
                observed: "$1.32B".into(),
                difference: "$120M".into(),
                band: "±$50M".into(),
                bound_scope: "fund 300000".into(),
                fund_explicit: true,
                relative_basis_points: 909,
            },
        );
        assert_eq!(mismatch.kind_label, "stated total, adopted budget");
        assert_eq!(mismatch.scope.fund.as_deref(), Some("300000"));
        assert_eq!(mismatch.scope.fiscal_year, Some(2023));
        assert!(matches!(
            mismatch.result,
            ClaimCheckOutcome::Mismatch { ref band, ref query_ids, .. } if band == "±$50M" && query_ids == &["q1"]
        ));
        let unreconciled = claim_check_view(
            &claim,
            &ClaimEvaluation::Unreconciled {
                reason: UnreconciledReason::AmbiguousScope,
                detail: "the sentence names no fund".into(),
            },
        );
        assert!(matches!(
            unreconciled.result,
            ClaimCheckOutcome::Unreconciled { ref reason, ref detail }
                if reason == "ambiguous scope" && detail == "the sentence names no fund"
        ));
    }
}
