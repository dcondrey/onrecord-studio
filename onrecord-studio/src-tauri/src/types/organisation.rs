use serde::{Deserialize, Serialize};

/// One name-search match from ProPublica Nonprofit Explorer.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OrganisationMatch {
    pub ein: String,
    pub name: String,
    pub city: Option<String>,
    pub state: Option<String>,
}

/// A person or entity row of Form 990 Part VII Section A, as the return states it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Form990Person {
    pub name: Option<String>,
    pub title: Option<String>,
    /// The position boxes the return checked, in the form's order.
    pub positions: Vec<String>,
    /// The "Former" box of column (C).
    pub former: bool,
    pub reportable_compensation: Option<i64>,
    pub related_organisation_compensation: Option<i64>,
    pub other_compensation: Option<i64>,
}

/// Schedule J Part II columns (B)(i) to (F) for one payer. `None` is a column the return left out.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleJAmounts {
    pub base: Option<i64>,
    pub bonus: Option<i64>,
    pub other: Option<i64>,
    pub deferred: Option<i64>,
    pub nontaxable_benefits: Option<i64>,
    pub total: Option<i64>,
    pub reported_in_prior_return: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleJPerson {
    pub name: Option<String>,
    pub title: Option<String>,
    pub filing_organisation: ScheduleJAmounts,
    pub related_organisations: ScheduleJAmounts,
}

/// A supplemental explanation the filer wrote on a schedule, quoted in full.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FilingExplanation {
    pub line_reference: Option<String>,
    pub text: String,
}

/// The four parts of Schedule L, named by the form's own part titles.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ScheduleLPart {
    ExcessBenefitTransaction,
    Loan,
    GrantOrAssistance,
    BusinessTransaction,
}

/// A yes/no or text column of a schedule row that has no field of its own.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FilingLine {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleLTransaction {
    pub part: ScheduleLPart,
    pub name: Option<String>,
    pub relationship: Option<String>,
    pub description: Option<String>,
    /// Transaction amount, grant amount, or a loan's original principal.
    pub amount: Option<i64>,
    pub balance_due: Option<i64>,
    pub lines: Vec<FilingLine>,
}

/// What one e-filed Form 990 states. Every `Option` is `None` when the return carries no such
/// element; a missing answer is never read as "No" or as zero.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Form990Statement {
    pub return_version: Option<String>,
    pub ein: Option<String>,
    pub filer_name: Option<String>,
    pub tax_period_begin: Option<String>,
    pub tax_period_end: Option<String>,
    pub amended: Option<bool>,
    pub total_revenue: Option<i64>,
    pub total_expenses: Option<i64>,
    pub total_assets_end_of_year: Option<i64>,
    pub total_liabilities_end_of_year: Option<i64>,
    pub net_assets_end_of_year: Option<i64>,
    pub government_grants: Option<i64>,
    /// Part VI line 5: "Did the organization become aware during the year of a significant
    /// diversion of the organization's assets?"
    pub significant_diversion_reported: Option<bool>,
    /// Part VII Section A line 3: whether any former officer, director, trustee, key employee or
    /// highest compensated employee is listed.
    pub former_listed: Option<bool>,
    pub total_reportable_compensation: Option<i64>,
    pub people: Vec<Form990Person>,
    /// Whether the return carries each schedule at all, so an empty list is not read as "none
    /// reported" on a return that did not attach the schedule.
    pub schedule_j_attached: bool,
    /// Schedule J Part I line 4a: a severance or change-of-control payment to a listed person.
    pub severance_payment_reported: Option<bool>,
    pub schedule_j_people: Vec<ScheduleJPerson>,
    pub schedule_j_explanations: Vec<FilingExplanation>,
    pub schedule_l_attached: bool,
    pub schedule_l_transactions: Vec<ScheduleLTransaction>,
    pub schedule_l_explanations: Vec<FilingExplanation>,
}

/// One return the IRS index lists for the organisation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OrganisationReturn {
    pub object_id: String,
    pub index_year: i32,
    pub return_type: String,
    /// `YYYYMM`, as the index gives it.
    pub tax_period: String,
    pub taxpayer_name: String,
    pub archive_url: Option<String>,
    pub member_name: Option<String>,
    pub xml_sha256: Option<String>,
    pub retrieved_at: Option<String>,
    pub statement: Option<Form990Statement>,
    /// Why `statement` is absent. Set exactly when it is.
    pub unread_reason: Option<String>,
}

/// One year of ProPublica Nonprofit Explorer's extract of the IRS annual financial file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OrganisationSummaryYear {
    pub tax_year: i32,
    pub tax_period: Option<String>,
    pub form_type: Option<String>,
    pub total_revenue: Option<i64>,
    pub total_expenses: Option<i64>,
    pub total_assets_end_of_year: Option<i64>,
    pub total_liabilities_end_of_year: Option<i64>,
    pub officer_compensation: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OrganisationSummary {
    pub name: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub years: Vec<OrganisationSummaryYear>,
    pub source_url: String,
    pub retrieved_at: String,
}

/// One finding row of a single audit, as the Federal Audit Clearinghouse records it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SingleAuditFinding {
    pub reference_number: Option<String>,
    pub compliance_requirement: Option<String>,
    pub lines: Vec<FilingLine>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SingleAudit {
    pub report_id: String,
    pub audit_year: Option<String>,
    pub fiscal_year_end: Option<String>,
    pub accepted_date: Option<String>,
    pub auditor: Option<String>,
    pub federal_amount_expended: Option<i64>,
    /// The auditor's answers on the submission, label and recorded value.
    pub lines: Vec<FilingLine>,
    pub findings: Vec<SingleAuditFinding>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SingleAuditHistory {
    pub audits: Vec<SingleAudit>,
    pub source_url: String,
    pub retrieved_at: String,
}

/// Rows of one mapped contract, amendment or payment table whose vendor value contains the
/// organisation's name.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PublicMoneyLine {
    pub role: String,
    pub dataset_name: String,
    pub source_url: Option<String>,
    pub vendor_value: String,
    pub fiscal_year: Option<String>,
    pub rows: u64,
    /// `None` when any matched row's amount could not be read as a number.
    pub amount: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct IndexYearCheck {
    pub year: i32,
    pub checked_at: String,
    pub returns_listed: u32,
}

/// Everything held about one organisation's filings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OrganisationFilings {
    pub ein: String,
    pub name: Option<String>,
    pub checked_at: String,
    pub index_years: Vec<IndexYearCheck>,
    pub returns: Vec<OrganisationReturn>,
    pub summary: Option<OrganisationSummary>,
    /// `None` when no clearinghouse key was supplied, so no audit lookup ran.
    pub single_audits: Option<SingleAuditHistory>,
    /// Parts of the check that failed, each naming its source. The rest of the record stands.
    pub warnings: Vec<String>,
    /// Computed when read, never stored: the library's tables change between checks.
    #[serde(default)]
    pub public_money: Vec<PublicMoneyLine>,
    #[serde(default)]
    pub public_money_notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OrganisationCheckSummary {
    pub ein: String,
    pub name: Option<String>,
    pub checked_at: String,
}
