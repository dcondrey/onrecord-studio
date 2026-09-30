use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FinancialMapping {
    pub role: String,
    pub snapshot_id: String,
    pub columns: BTreeMap<String, String>,
    #[serde(default)]
    pub currency: Option<String>,
    #[serde(default)]
    pub agency: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FinancialScope {
    pub fiscal_year: Option<i32>,
    pub fund: Option<String>,
    pub department: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinancialDataset {
    pub snapshot_id: String,
    pub name: String,
    pub table_name: String,
    pub source_url: Option<String>,
    pub source_sha256: String,
    pub columns: Vec<String>,
    pub partial: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinancialWorkspace {
    pub datasets: Vec<FinancialDataset>,
    pub mappings: Vec<FinancialMapping>,
    pub default_scope: FinancialScope,
    pub roles: Vec<String>,
    pub fields: Vec<String>,
    pub requirements: BTreeMap<String, Vec<String>>,
    pub warnings: Vec<String>,
    pub fiscal_years: Vec<i32>,
    pub funds: Vec<String>,
    pub departments: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinancialCitation {
    pub id: String,
    pub role: String,
    pub snapshot_id: String,
    pub source_url: Option<String>,
    pub source_sha256: String,
    pub query_id: String,
    pub columns: Vec<String>,
    pub row: Vec<serde_json::Value>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinancialRecord {
    pub role: String,
    pub values: BTreeMap<String, Option<String>>,
    pub source_rows: u64,
    pub citation_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinancialReconciliation {
    pub agency: String,
    pub funding_duration: String,
    pub key: String,
    pub fiscal_year: String,
    pub fund: String,
    pub account: String,
    pub account_label: String,
    pub department: String,
    pub center: String,
    pub program: String,
    pub currency: String,
    pub adopted: Option<String>,
    pub revised: Option<String>,
    pub actual: Option<String>,
    pub encumbered: Option<String>,
    pub actual_minus_adopted: Option<String>,
    pub uncommitted_revised: Option<String>,
    pub citations: Vec<String>,
    pub limitations: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinancialChain {
    pub agency: String,
    pub contract_id: String,
    pub program: String,
    pub vendor: String,
    pub currency: String,
    pub base_authority: Option<String>,
    pub amendments: Option<String>,
    pub total_authority: Option<String>,
    pub payments: Option<String>,
    pub remaining_authority: Option<String>,
    pub stages: BTreeMap<String, Vec<String>>,
    pub missing_links: Vec<String>,
    pub limitations: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinancialAuditFinding {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub observation: String,
    pub citations: Vec<String>,
    pub alternatives: Vec<String>,
    pub next_test: String,
    pub calculations: Vec<FinancialCalculation>,
    /// `counted_comparison` when every input was a recorded row or a written limit, `pattern` for
    /// an inferential observation, `lead` for a hypothesis that needs more records.
    #[serde(default)]
    pub basis: String,
    /// Derived from the inputs the evaluation observed. `None` is not computed, not zero.
    #[serde(default)]
    pub confidence_basis_points: Option<u16>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinancialCalculation {
    pub label: String,
    pub formula: String,
    pub result: String,
    pub unit: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinancialOutcome {
    pub agency: String,
    pub key: String,
    pub program: String,
    pub contract_id: String,
    pub period_start: String,
    pub period_end: String,
    pub population: String,
    pub metric: String,
    pub denominator_unit: String,
    pub unit: String,
    pub currency: String,
    pub cost: Option<String>,
    pub quantity: Option<String>,
    pub denominator: Option<String>,
    pub cost_per_unit: Option<String>,
    pub success_rate_percent: Option<String>,
    pub comparison_group: Option<String>,
    pub citations: Vec<String>,
    pub limitations: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinancialGap {
    pub title: String,
    pub records_needed: String,
    pub why: String,
    pub citations: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinancialReport {
    pub id: String,
    pub created_at: String,
    pub scope: FinancialScope,
    pub reconciliation: Vec<FinancialReconciliation>,
    pub chains: Vec<FinancialChain>,
    pub findings: Vec<FinancialAuditFinding>,
    pub outcomes: Vec<FinancialOutcome>,
    pub gaps: Vec<FinancialGap>,
    pub evidence: Vec<FinancialCitation>,
    pub queries: BTreeMap<String, String>,
    pub records: Vec<FinancialRecord>,
    pub incomplete_roles: BTreeSet<String>,
    pub limitations: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinancialScenarioInput {
    pub scope: FinancialScope,
    pub funding_key: String,
    pub outcome_key: String,
    pub target_program: String,
    pub proposed_amount: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinancialScenario {
    pub status: String,
    pub funding_duration: String,
    pub period_scope: String,
    pub target_program: String,
    pub available_revised_balance: Option<String>,
    pub proposed_amount: String,
    pub currency: Option<String>,
    pub unit: Option<String>,
    pub estimated_whole_units: Option<String>,
    pub lower_units: Option<String>,
    pub upper_units: Option<String>,
    pub formula: String,
    pub assumptions: Vec<String>,
    pub blockers: Vec<String>,
    pub citations: Vec<FinancialCitation>,
    pub queries: BTreeMap<String, String>,
}
