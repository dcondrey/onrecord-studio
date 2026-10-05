use crate::{
    analysis::{budget_lines, financial},
    error::AppError,
    integrity::sha256_hex,
    storage::sqlite::DuckDbRepository,
    types::*,
};
use duckdb::params;
use std::collections::{BTreeMap, BTreeSet};

const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS financial_mappings (role TEXT PRIMARY KEY, mapping_json TEXT NOT NULL, updated_at TEXT NOT NULL);";
pub const ROLES: &[&str] = &[
    "budget",
    "actual",
    "revenue",
    "encumbrance",
    "contract",
    "amendment",
    "payment",
    "outcome",
    "crosswalk",
    "restriction",
];
pub const FIELDS: &[&str] = &[
    "agency",
    "fy",
    "fund",
    "account",
    "account_label",
    "department",
    "center",
    "program",
    "currency",
    "amount",
    "basis",
    "contract_id",
    "vendor_id",
    "transaction_id",
    "invoice_id",
    "amendment_id",
    "status",
    "period_start",
    "period_end",
    "population",
    "metric",
    "denominator_unit",
    "unit",
    "quantity",
    "denominator",
    "canonical_account",
    "end_fy",
    "restriction",
    "eligible_program",
    "transfer",
    "funding_duration",
    "transaction_date",
    "approval",
];
fn error(e: impl std::fmt::Display) -> AppError {
    AppError::Storage(e.to_string())
}
fn quote(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}
fn literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}
fn requirements(role: &str) -> Vec<&'static str> {
    match role {
        "budget" => vec!["fy", "fund", "account", "amount", "basis"],
        "actual" | "encumbrance" | "revenue" => vec!["fy", "fund", "account", "amount"],
        "contract" => vec!["fy", "fund", "contract_id", "vendor_id", "amount", "status"],
        "amendment" => vec![
            "fy",
            "fund",
            "contract_id",
            "amendment_id",
            "amount",
            "status",
        ],
        "payment" => vec![
            "fy",
            "fund",
            "contract_id",
            "vendor_id",
            "transaction_id",
            "invoice_id",
            "amount",
            "status",
        ],
        "outcome" => vec![
            "fy",
            "fund",
            "program",
            "amount",
            "quantity",
            "basis",
            "period_start",
            "period_end",
            "population",
            "metric",
            "unit",
        ],
        "crosswalk" => vec!["fy", "fund", "account", "canonical_account"],
        "restriction" => vec!["fy", "fund", "restriction"],
        _ => vec![],
    }
}
fn validate_mapping(mapping: &FinancialMapping, schema: &AnalyticalSchema) -> Result<(), AppError> {
    if !ROLES.contains(&mapping.role.as_str()) {
        return Err(AppError::DataParsing(
            "Unknown financial record role".into(),
        ));
    }
    let dataset = schema
        .datasets
        .iter()
        .find(|d| d.snapshot_id == mapping.snapshot_id)
        .ok_or_else(|| {
            AppError::DataParsing("The selected analytical snapshot no longer exists".into())
        })?;
    for (field, column) in &mapping.columns {
        if !FIELDS.contains(&field.as_str())
            || !schema
                .columns
                .iter()
                .any(|c| c.table_name == dataset.table_name && c.column_name == *column)
        {
            return Err(AppError::DataParsing(format!(
                "The mapping for {field} does not identify an existing source column"
            )));
        }
    }
    if !mapping.columns.contains_key("agency")
        && !mapping
            .agency
            .as_ref()
            .is_some_and(|s| !s.trim().is_empty() && s.len() <= 150)
    {
        return Err(AppError::DataParsing("Map the source agency/jurisdiction column or explicitly declare the agency responsible for this dataset".into()));
    }
    for field in requirements(&mapping.role) {
        if !mapping.columns.contains_key(field) {
            return Err(AppError::DataParsing(format!(
                "{} requires the {field} source column",
                mapping.role
            )));
        }
    }
    if !["restriction", "crosswalk"].contains(&mapping.role.as_str())
        && !mapping.columns.contains_key("currency")
        && !mapping
            .currency
            .as_ref()
            .is_some_and(|v| v.len() == 3 && v.chars().all(|c| c.is_ascii_uppercase()))
    {
        return Err(AppError::DataParsing("Map the source currency column or explicitly declare a three-letter currency for this dataset".into()));
    }
    Ok(())
}
fn automatic_mappings(schema: &AnalyticalSchema) -> Vec<FinancialMapping> {
    [
        ("budget", "/operating_budget/budget_operating_datasd.csv"),
        ("actual", "/operating_actuals/actuals_operating_datasd.csv"),
    ]
    .into_iter()
    .filter_map(|(role, path)| {
        let dataset = schema
            .datasets
            .iter()
            .filter(|d| {
                d.source_uri
                    .as_deref()
                    .and_then(|u| reqwest::Url::parse(u).ok())
                    .is_some_and(|u| {
                        u.scheme() == "https"
                            && u.host_str() == Some("seshat.datasd.org")
                            && u.path() == path
                    })
            })
            .max_by_key(|d| &d.retrieved_at)?;
        let mut columns = [
            ("fy", "report_fy"),
            ("fund", "fund_number"),
            ("account", "account_number"),
            ("department", "dept_name"),
            ("center", "funds_center_number"),
            ("amount", "amount"),
        ]
        .into_iter()
        .map(|(a, b)| (a.into(), b.into()))
        .collect::<BTreeMap<_, _>>();
        if role == "budget" {
            columns.insert("basis".into(), "budget_cycle".into());
        }
        if schema
            .columns
            .iter()
            .any(|c| c.table_name == dataset.table_name && c.column_name == "account")
        {
            columns.insert("account_label".into(), "account".into());
        }
        let mapping = FinancialMapping {
            role: role.into(),
            snapshot_id: dataset.snapshot_id.clone(),
            columns,
            currency: Some("USD".into()),
            agency: Some("City of San Diego".into()),
        };
        validate_mapping(&mapping, schema).ok().map(|_| mapping)
    })
    .collect()
}
fn expression(mapping: &FinancialMapping, field: &str) -> String {
    if let Some(column) = mapping.columns.get(field) {
        let column = quote(column);
        if ["amount", "quantity", "denominator"].contains(&field) {
            format!("TRY_CAST({column} AS DECIMAL(28,4))")
        } else if ["fy", "end_fy"].contains(&field) {
            format!("CASE WHEN TRY_CAST({column} AS INTEGER) BETWEEN 0 AND 99 THEN 2000 + TRY_CAST({column} AS INTEGER) ELSE TRY_CAST({column} AS INTEGER) END")
        } else {
            format!("NULLIF(trim(CAST({column} AS VARCHAR)), '')")
        }
    } else if field == "agency" {
        mapping
            .agency
            .as_ref()
            .map(|v| literal(v))
            .unwrap_or_else(|| "NULL".into())
    } else if field == "currency" {
        mapping
            .currency
            .as_ref()
            .map(|v| literal(v))
            .unwrap_or_else(|| "NULL".into())
    } else {
        "NULL".into()
    }
}
/// Both known City exports contain revenue and expense rows. Expense investigations must not
/// silently net tax receipts against service expenditure.
fn official_city_export(mapping: &FinancialMapping) -> bool {
    mapping
        .columns
        .get("account")
        .is_some_and(|c| c == "account_number")
        && mapping.columns.get("fy").is_some_and(|c| c == "report_fy")
}

fn mapped_select(mapping: &FinancialMapping, fields: &[&str]) -> String {
    fields
        .iter()
        .map(|field| format!("{} AS {}", expression(mapping, field), quote(field)))
        .collect::<Vec<_>>()
        .join(",")
}

/// Totals by agency, currency, year, fund, department and basis, or by account as well for the
/// given accounts. Rows missing any grouping value form their own group, so an unbound row is
/// visible to the comparison instead of dropped; a group with an unparsable amount sums to NULL.
fn budget_line_query(
    mapping: &FinancialMapping,
    table: &str,
    accounts: Option<&BTreeSet<String>>,
) -> String {
    let selected = mapped_select(
        mapping,
        &[
            "agency",
            "currency",
            "fy",
            "fund",
            "department",
            "basis",
            "account",
            "amount",
        ],
    );
    let mut dimensions = vec!["agency", "currency", "fy", "fund", "department", "basis"];
    let mut filters = vec!["TRUE".to_string()];
    if official_city_export(mapping) {
        filters.push("account LIKE '5%'".into());
    }
    if mapping.role == "budget" {
        filters.push("(basis IS NULL OR lower(basis) = 'adopted')".into());
    }
    if let Some(accounts) = accounts {
        let list = accounts
            .iter()
            .map(|a| literal(a))
            .collect::<Vec<_>>()
            .join(",");
        filters.push(format!("account IN ({list})"));
        dimensions.push("account");
    }
    let dimensions = dimensions.join(",");
    format!(
        "WITH mapped AS (SELECT {selected} FROM {}) SELECT {dimensions}, CASE WHEN COUNT(*)=COUNT(amount) THEN SUM(amount) END AS amount, COUNT(*) AS _source_rows FROM mapped WHERE {} GROUP BY {dimensions} ORDER BY {dimensions}",
        quote(table),
        filters.join(" AND ")
    )
}

fn account_label_query(mapping: &FinancialMapping, table: &str) -> String {
    let selected = mapped_select(mapping, &["account", "account_label"]);
    let filter = if official_city_export(mapping) {
        "account LIKE '5%'"
    } else {
        "TRUE"
    };
    format!(
        "WITH mapped AS (SELECT {selected} FROM {}) SELECT DISTINCT account, account_label FROM mapped WHERE {filter} ORDER BY account, account_label",
        quote(table)
    )
}

const BUDGET_LINE_ROW_CAP: usize = 10_000;

fn query(mapping: &FinancialMapping, table: &str, scope: &FinancialScope) -> String {
    let selected = FIELDS
        .iter()
        .map(|field| format!("{} AS {}", expression(mapping, field), quote(field)))
        .collect::<Vec<_>>()
        .join(",");
    let mut filters = vec![];
    if let Some(fy) = scope.fiscal_year {
        if mapping.role == "crosswalk" {
            filters.push(format!("fy <= {fy} AND COALESCE(end_fy,fy) >= {fy}"));
        } else if ["contract", "amendment", "payment"].contains(&mapping.role.as_str()) {
            filters.push(format!("fy <= {fy}"));
        } else {
            filters.push(format!("fy = {fy}"));
        }
    }
    if let Some(fund) = &scope.fund {
        filters.push(format!("fund = {}", literal(fund)));
    }
    if let Some(department) = &scope.department {
        if mapping.columns.contains_key("department") {
            filters.push(format!("department = {}", literal(department)));
        }
    }
    if official_city_export(mapping)
        && ["budget", "actual", "encumbrance"].contains(&mapping.role.as_str())
    {
        filters.push("account LIKE '5%'".into());
    }
    // The same exports carry revenue under 4xxxx accounts; a revenue role mapped to one of them
    // must not net service expenditure into receipts.
    if official_city_export(mapping) && mapping.role == "revenue" {
        filters.push("account LIKE '4%'".into());
    }
    let filter = if filters.is_empty() {
        "TRUE".into()
    } else {
        filters.join(" AND ")
    };
    if ["budget", "actual", "revenue", "encumbrance"].contains(&mapping.role.as_str()) {
        let dimensions = FIELDS
            .iter()
            .filter(|f| !matches!(**f, "amount" | "quantity" | "denominator"))
            .map(|f| quote(f))
            .collect::<Vec<_>>();
        format!("WITH mapped AS (SELECT {selected} FROM {}) SELECT {}, CASE WHEN COUNT(*)=COUNT(amount) THEN SUM(amount) END AS amount, NULL AS quantity, NULL AS denominator, COUNT(*) AS _source_rows FROM mapped WHERE {filter} GROUP BY {} ORDER BY fy,fund,account,center,basis",quote(table),dimensions.join(","),dimensions.join(","))
    } else {
        format!("WITH mapped AS (SELECT {selected}, rowid AS _source_row FROM {}) SELECT *, 1 AS _source_rows FROM mapped WHERE {filter} ORDER BY _source_row",quote(table))
    }
}
fn value_string(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Null => None,
        serde_json::Value::String(s) => Some(s.clone()),
        _ => Some(value.to_string()),
    }
}

// Three annual department aggregates are queried independently of the bounded account detail.
// A top-N account result must never masquerade as a complete departmental history.
fn history_query(mapping: &FinancialMapping, table: &str, scope: &FinancialScope) -> String {
    let selected = [
        "agency",
        "fy",
        "fund",
        "department",
        "currency",
        "funding_duration",
        "account",
        "amount",
        "basis",
    ]
    .iter()
    .map(|field| format!("{} AS {}", expression(mapping, field), quote(field)))
    .collect::<Vec<_>>()
    .join(",");
    let year = scope.fiscal_year.unwrap();
    let mut filters = vec![
        format!("fy BETWEEN {} AND {year}", year - 2),
        format!("fund={}", literal(scope.fund.as_deref().unwrap())),
    ];
    if mapping.role == "budget" {
        filters.push("lower(basis)='adopted'".into());
    }
    if let Some(department) = &scope.department {
        if mapping.columns.contains_key("department") {
            filters.push(format!("department={}", literal(department)));
        }
    }
    if official_city_export(mapping) {
        filters.push("account LIKE '5%'".into());
    }
    let dimensions = "agency,fy,fund,department,currency,funding_duration";
    format!("WITH mapped AS (SELECT {selected} FROM {}) SELECT {dimensions},CASE WHEN COUNT(*)=COUNT(amount) THEN SUM(amount) END AS amount,COUNT(*) AS _source_rows FROM mapped WHERE {} GROUP BY {dimensions} ORDER BY agency,fund,department,fy,currency,funding_duration",quote(table),filters.join(" AND "))
}

impl DuckDbRepository {
    pub async fn financial_workspace(&self) -> Result<FinancialWorkspace, AppError> {
        let schema = self.analytical_schema().await?;
        let saved = self
            .query_connection("Read financial source mappings", |conn| {
                conn.execute_batch(SCHEMA).map_err(error)?;
                let mut statement = conn
                    .prepare("SELECT mapping_json FROM financial_mappings ORDER BY role")
                    .map_err(error)?;
                let values = statement
                    .query_map([], |row| row.get::<_, String>(0))
                    .map_err(error)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(error)?;
                values
                    .into_iter()
                    .map(|value| serde_json::from_str::<FinancialMapping>(&value).map_err(error))
                    .collect::<Result<Vec<_>, _>>()
            })
            .await?;
        let mut mappings = automatic_mappings(&schema);
        let mut warnings = vec![];
        for mapping in saved {
            mappings.retain(|m| m.role != mapping.role);
            if let Err(e) = validate_mapping(&mapping, &schema) {
                warnings.push(format!("{} mapping needs attention: {e}", mapping.role));
            } else {
                mappings.push(mapping);
            }
        }
        let mut fiscal_years = vec![];
        let mut funds = vec![];
        let mut departments = vec![];
        if let Some(mapping) = mappings
            .iter()
            .find(|m| m.role == "actual")
            .or_else(|| mappings.iter().find(|m| m.role == "budget"))
        {
            if let Some(dataset) = schema
                .datasets
                .iter()
                .find(|d| d.snapshot_id == mapping.snapshot_id)
            {
                for field in ["fy", "fund", "department"] {
                    let expr = expression(mapping, field);
                    let sql=format!("SELECT DISTINCT {expr} AS value FROM {} WHERE {expr} IS NOT NULL ORDER BY value DESC",quote(&dataset.table_name));
                    match self.execute_restricted_sql(&sql, 1000).await {
                        Ok(result) => {
                            let values = result
                                .rows
                                .iter()
                                .filter_map(|r| r.first())
                                .filter_map(value_string)
                                .collect::<Vec<_>>();
                            match field {
                                "fy" => {
                                    fiscal_years =
                                        values.iter().filter_map(|s| s.parse().ok()).collect()
                                }
                                "fund" => funds = values,
                                _ => departments = values,
                            }
                        }
                        Err(e) => warnings.push(format!(
                            "could not read distinct {field} values for default scope: {e}"
                        )),
                    }
                }
            }
        }
        let default_scope = FinancialScope {
            fiscal_year: fiscal_years.first().copied(),
            fund: if funds.iter().any(|f| f == "100000") {
                Some("100000".into())
            } else {
                funds.first().cloned()
            },
            department: departments
                .iter()
                .find(|d| d.to_lowercase().contains("homeless"))
                .cloned()
                .or_else(|| {
                    departments
                        .iter()
                        .any(|d| d == "Police")
                        .then(|| "Police".into())
                }),
        };
        Ok(FinancialWorkspace {
            datasets: schema
                .datasets
                .iter()
                .map(|d| FinancialDataset {
                    snapshot_id: d.snapshot_id.clone(),
                    name: d.display_name.clone(),
                    table_name: d.table_name.clone(),
                    source_url: d.source_uri.clone(),
                    source_sha256: d.source_sha256.clone(),
                    columns: schema
                        .columns
                        .iter()
                        .filter(|c| c.table_name == d.table_name)
                        .map(|c| c.column_name.clone())
                        .collect(),
                    partial: d.partial_dataset || d.rejected_rows > 0 || d.unconverted_cells() > 0,
                })
                .collect(),
            mappings,
            default_scope,
            roles: ROLES.iter().map(|s| (*s).into()).collect(),
            fields: FIELDS.iter().map(|s| (*s).into()).collect(),
            requirements: ROLES
                .iter()
                .map(|role| {
                    (
                        (*role).into(),
                        requirements(role).into_iter().map(String::from).collect(),
                    )
                })
                .collect(),
            warnings,
            fiscal_years,
            funds,
            departments,
        })
    }
    pub async fn save_financial_mapping(
        &self,
        mapping: FinancialMapping,
    ) -> Result<FinancialWorkspace, AppError> {
        let schema = self.analytical_schema().await?;
        validate_mapping(&mapping, &schema)?;
        self.query_connection("Save financial source mapping",move|conn| {
            conn.execute_batch(SCHEMA).map_err(error)?;
            conn.execute("INSERT INTO financial_mappings VALUES (?1,?2,?3) ON CONFLICT(role) DO UPDATE SET mapping_json=excluded.mapping_json,updated_at=excluded.updated_at",params![mapping.role,serde_json::to_string(&mapping).map_err(error)?,chrono::Utc::now().to_rfc3339()]).map_err(error)?; Ok(())
        }).await?;
        self.financial_workspace().await
    }
    /// Whether the mapped table holds any row for the fiscal year, under the same fiscal-year
    /// normalization the investigation filters on. `None` when no fiscal-year column is mapped.
    pub(crate) async fn financial_table_covers_year(
        &self,
        mapping: &FinancialMapping,
        table_name: &str,
        fiscal_year: i32,
    ) -> Result<Option<bool>, AppError> {
        if !mapping.columns.contains_key("fy") {
            return Ok(None);
        }
        let sql = format!(
            "SELECT 1 FROM {} WHERE {} = {fiscal_year}",
            quote(table_name),
            expression(mapping, "fy")
        );
        Ok(Some(
            !self.execute_restricted_sql(&sql, 1).await?.rows.is_empty(),
        ))
    }
    pub async fn run_financial_investigation(
        &self,
        scope: FinancialScope,
    ) -> Result<FinancialReport, AppError> {
        if scope
            .fiscal_year
            .is_none_or(|fy| !(1900..=2200).contains(&fy))
            || scope
                .fund
                .as_ref()
                .is_none_or(|s| s.trim().is_empty() || s.len() > 100)
            || scope.department.as_ref().is_some_and(|s| s.len() > 200)
        {
            return Err(AppError::DataParsing(
                "Choose an explicit fiscal year and fund; narrow the department if needed".into(),
            ));
        }
        let workspace = self.financial_workspace().await?;
        let schema = self.analytical_schema().await?;
        let mut records = vec![];
        let mut evidence = vec![];
        let mut queries = BTreeMap::new();
        let mut limitations = workspace.warnings;
        limitations.push("Account and outcome comparisons use the selected fiscal year; independently aggregated history covers that year and the two preceding fiscal years. Contract/amendment/payment chains include retrieved years through that fiscal year, within the selected fund; other funds, earlier omitted payments and later credits can change a contract-wide balance. The displayed remainder is not available contract authority.".into());
        let mut incomplete = BTreeSet::new();
        let mut identities = vec![];
        for mapping in workspace.mappings {
            validate_mapping(&mapping, &schema)?;
            let dataset = schema
                .datasets
                .iter()
                .find(|d| d.snapshot_id == mapping.snapshot_id)
                .unwrap();
            identities.push((
                dataset.snapshot_id.clone(),
                dataset.table_name.clone(),
                dataset.source_sha256.clone(),
            ));
            let mut plans = vec![(
                mapping.role.clone(),
                query(&mapping, &dataset.table_name, &scope),
                5000,
            )];
            if ["budget", "actual"].contains(&mapping.role.as_str()) {
                plans.push((
                    format!("{}_history", mapping.role),
                    history_query(&mapping, &dataset.table_name, &scope),
                    500,
                ));
            }
            if let Some(currency) = &mapping.currency {
                limitations.push(format!("{} currency is explicitly mapped as {currency}; verify the source documentation.",mapping.role));
            }
            // A ledger without a department column cannot honor the department scope; its totals
            // would be whole-fund amounts labelled as the selected department.
            let department_unscoped = scope.department.is_some()
                && ["budget", "actual", "revenue", "encumbrance"].contains(&mapping.role.as_str())
                && !mapping.columns.contains_key("department");
            if department_unscoped {
                limitations.push(format!("{}: the source has no mapped department column, so the department scope was not applied. Totals and comparisons requiring department coverage are withheld.",mapping.role));
            }
            for (role, sql, cap) in plans {
                if department_unscoped {
                    incomplete.insert(role.clone());
                }
                let query_id = sha256_hex(sql.as_bytes());
                queries.insert(query_id.clone(), sql.clone());
                let result = self.execute_restricted_sql(&sql, cap).await?;
                let unconverted =
                    dataset.unconverted_in(mapping.columns.values().map(String::as_str));
                if result.truncated
                    || dataset.partial_dataset
                    || dataset.rejected_rows > 0
                    || unconverted > 0
                {
                    incomplete.insert(role.clone());
                    limitations.push(format!("{}: query truncated={}, source partial={}, rejected rows={}, unconverted cells in mapped columns={}. Totals and comparisons requiring complete coverage are withheld; narrow the scope or obtain a complete export.",role,result.truncated,dataset.partial_dataset,dataset.rejected_rows,unconverted));
                }
                for row in result.rows {
                    let id = sha256_hex(
                        format!(
                            "{}:{}:{}",
                            role,
                            dataset.snapshot_id,
                            serde_json::to_string(&row).map_err(error)?
                        )
                        .as_bytes(),
                    );
                    let values = result
                        .columns
                        .iter()
                        .zip(&row)
                        .filter(|(column, _)| FIELDS.contains(&column.as_str()))
                        .map(|(column, value)| (column.clone(), value_string(value)))
                        .collect();
                    let source_rows = result
                        .columns
                        .iter()
                        .position(|c| c == "_source_rows")
                        .and_then(|i| value_string(&row[i]))
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(1);
                    records.push(FinancialRecord {
                        role: role.clone(),
                        values,
                        source_rows,
                        citation_id: id.clone(),
                    });
                    evidence.push(FinancialCitation {
                        id,
                        role: role.clone(),
                        snapshot_id: dataset.snapshot_id.clone(),
                        source_url: dataset.source_uri.clone(),
                        source_sha256: dataset.source_sha256.clone(),
                        query_id: query_id.clone(),
                        columns: result.columns.clone(),
                        row,
                    });
                }
            }
        }
        // Retrieval may span queries. A removed/replaced snapshot must never publish evidence
        // under its prior identity. Recheck before creating a report or policy calculation.
        self.query_connection("Validate financial snapshot identities", move |conn| {
            validate_financial_sources(conn, &identities)
        })
        .await?;
        Ok(financial::analyze(
            scope,
            records,
            evidence,
            queries,
            limitations,
            &incomplete,
        ))
    }
    pub async fn evaluate_financial_scenario(
        &self,
        input: FinancialScenarioInput,
    ) -> Result<FinancialScenario, AppError> {
        // Recompute from current source snapshots. A browser cannot supply its own purported
        // balance, cost, permissions or evidence when asking the native core for an estimate.
        let report = self
            .run_financial_investigation(input.scope.clone())
            .await?;
        financial::scenario(&report, &input)
    }

    /// Reads the budget and actual roles for `analysis::budget_lines`. Any cut-short read, partial
    /// snapshot or rejected row marks the role incomplete rather than yielding a smaller total.
    pub async fn budget_line_data(&self) -> Result<budget_lines::LineData, AppError> {
        let workspace = self.financial_workspace().await?;
        let schema = self.analytical_schema().await?;
        let mut data = budget_lines::LineData::default();
        let mut sources = vec![];
        let mut identities = vec![];
        for mapping in workspace
            .mappings
            .iter()
            .filter(|m| ["budget", "actual"].contains(&m.role.as_str()))
        {
            let Some(dataset) = schema
                .datasets
                .iter()
                .find(|d| d.snapshot_id == mapping.snapshot_id)
            else {
                continue;
            };
            identities.push((
                dataset.snapshot_id.clone(),
                dataset.table_name.clone(),
                dataset.source_sha256.clone(),
            ));
            let mut role = budget_lines::RoleData {
                dataset: dataset.display_name.clone(),
                source_sha256: dataset.source_sha256.clone(),
                incomplete: {
                    let unconverted =
                        dataset.unconverted_in(mapping.columns.values().map(String::as_str));
                    (dataset.partial_dataset || dataset.rejected_rows > 0 || unconverted > 0).then(|| {
                        format!(
                            "the {} dataset '{}' is partial, has {} rejected rows, or has {} unconverted cells in mapped columns",
                            mapping.role, dataset.display_name, dataset.rejected_rows, unconverted
                        )
                    })
                },
                department_mapped: mapping.columns.contains_key("department"),
                labels: Err(format!(
                    "the {} dataset has no mapped account label, so no overtime line can be identified",
                    mapping.role
                )),
                totals: vec![],
                overtime: vec![],
                query_ids: vec![],
            };
            let sql = budget_line_query(mapping, &dataset.table_name, None);
            role.totals = self
                .budget_line_records(&mut role, &sql, &mapping.role, dataset)
                .await?;
            if mapping.columns.contains_key("account_label") {
                let result = self
                    .execute_restricted_sql(
                        &account_label_query(mapping, &dataset.table_name),
                        BUDGET_LINE_ROW_CAP,
                    )
                    .await?;
                role.labels = if result.truncated {
                    Err(format!(
                        "the {} dataset has more account labels than can be read",
                        mapping.role
                    ))
                } else {
                    Ok(result
                        .rows
                        .iter()
                        .map(|row| {
                            (
                                row.first().and_then(value_string),
                                row.get(1).and_then(value_string),
                            )
                        })
                        .collect())
                };
            }
            sources.push((mapping.clone(), dataset.clone()));
            data.roles.insert(mapping.role.clone(), role);
        }
        if let Ok((accounts, _)) = budget_lines::overtime_accounts(&data) {
            if !accounts.is_empty() {
                for (mapping, dataset) in &sources {
                    let sql = budget_line_query(mapping, &dataset.table_name, Some(&accounts));
                    let mut role = data.roles.remove(&mapping.role).expect("read above");
                    role.overtime = self
                        .budget_line_records(&mut role, &sql, &mapping.role, dataset)
                        .await?;
                    data.roles.insert(mapping.role.clone(), role);
                }
            }
        }
        self.query_connection("Validate budget line sources", move |conn| {
            validate_financial_sources(conn, &identities)
        })
        .await?;
        Ok(data)
    }

    async fn budget_line_records(
        &self,
        role: &mut budget_lines::RoleData,
        sql: &str,
        role_name: &str,
        dataset: &AnalyticalDatasetSummary,
    ) -> Result<Vec<FinancialRecord>, AppError> {
        let query_id = sha256_hex(sql.as_bytes());
        let result = self
            .execute_restricted_sql(sql, BUDGET_LINE_ROW_CAP)
            .await?;
        if result.truncated && role.incomplete.is_none() {
            role.incomplete = Some(format!(
                "the dataset '{}' has more grouped rows than one read returns",
                dataset.display_name
            ));
        }
        let records = result
            .rows
            .iter()
            .map(|row| FinancialRecord {
                role: role_name.to_string(),
                values: result
                    .columns
                    .iter()
                    .zip(row)
                    .filter(|(column, _)| FIELDS.contains(&column.as_str()))
                    .map(|(column, value)| (column.clone(), value_string(value)))
                    .collect(),
                source_rows: result
                    .columns
                    .iter()
                    .position(|c| c == "_source_rows")
                    .and_then(|i| value_string(&row[i]))
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(1),
                citation_id: sha256_hex(
                    format!("{query_id}:{}", serde_json::Value::Array(row.clone())).as_bytes(),
                ),
            })
            .collect();
        role.query_ids.push(query_id);
        Ok(records)
    }
}

fn validate_financial_sources(
    conn: &duckdb::Connection,
    identities: &[(String, String, String)],
) -> Result<(), AppError> {
    for (snapshot, table, hash) in identities {
        let exists:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM analytical_snapshots WHERE snapshot_id=?1 AND table_name=?2 AND source_sha256=?3)",params![snapshot,table,hash],|row|row.get(0)).map_err(error)?;
        if !exists {
            return Err(AppError::DataParsing("A financial source changed or was removed during analysis. Run the investigation again.".into()));
        }
    }
    Ok(())
}

/// Stable schema/mapping semantics join the autonomous scheduler's meaningful revision.
/// This also works before the financial panel has ever been opened on an upgraded database.
pub fn financial_mapping_revision(conn: &duckdb::Connection) -> Result<String, AppError> {
    conn.execute_batch(SCHEMA).map_err(error)?;
    conn.query_row("SELECT COALESCE(md5(string_agg(mapping_json,'|' ORDER BY role)),'') FROM financial_mappings",[],|row|row.get(0)).map_err(error)
}
impl DuckDbRepository {
    pub async fn financial_research_context(
        &self,
        topic: &str,
    ) -> Result<serde_json::Value, AppError> {
        let workspace = self.financial_workspace().await?;
        if workspace.mappings.is_empty()
            || workspace.default_scope.fiscal_year.is_none()
            || workspace.default_scope.fund.is_none()
        {
            return Ok(
                serde_json::json!({"available":false,"gap":"Ingest and map financial records before testing contract/payment/outcome hypotheses."}),
            );
        }
        let mut scope = workspace.default_scope;
        let terms: &[&str] = match topic {
            "homelessness" => &["homeless"],
            "bathrooms" => &["homeless", "environmental", "park"],
            "police" => &["police"],
            _ => &[],
        };
        scope.department = terms.iter().find_map(|term| {
            workspace
                .departments
                .iter()
                .find(|d| d.to_lowercase().contains(term))
                .cloned()
        });
        let report = self.run_financial_investigation(scope).await?;
        let mut selected = vec![];
        let mut rows = vec![];
        let mut queries = BTreeMap::new();
        let mut cited = BTreeSet::new();
        let mut used = 0;
        for finding in report.findings.iter().take(8) {
            let mut additional = vec![];
            let mut additional_queries = BTreeMap::new();
            for source in report
                .evidence
                .iter()
                .filter(|e| finding.citations.contains(&e.id))
                .take(6)
            {
                if !cited.contains(&source.id) {
                    additional.push(source.clone());
                }
                if !queries.contains_key(&source.query_id) {
                    if let Some(sql) = report.queries.get(&source.query_id) {
                        additional_queries.insert(source.query_id.clone(), sql.clone());
                    }
                }
            }
            // A lead with more source rows than fit here remains in the full financial report;
            // do not present a deliberately incomplete evidence list as its complete support.
            if finding.citations.len() > 6 {
                continue;
            }
            let bytes = serde_json::to_vec(&(finding, &additional, &additional_queries))
                .map_err(error)?
                .len();
            if used + bytes > 16_000 {
                continue;
            }
            used += bytes;
            for row in &additional {
                cited.insert(row.id.clone());
            }
            rows.extend(additional);
            queries.extend(additional_queries);
            selected.push(finding.clone());
        }
        let omitted = serde_json::json!({"leads":report.findings.len()-selected.len(),"acquisitionGaps":report.gaps.len().saturating_sub(7),"limitations":report.limitations.len().saturating_sub(8)});
        let context = serde_json::json!({"available":true,"scope":report.scope,"status":"Deterministic investigative leads, not independently supported conclusions. Source rows retain the executed result structure with personal identifiers masked. Execute the supplied SQL in the research tools to create citable ResearchEvidence; recompute any derived comparison before publishing it. Restrictions and missing records remain binding. Omission counts disclose material retained only in the full financial report.","omitted":omitted,"leads":selected,"sourceRows":rows,"queries":queries,"acquisitionGaps":report.gaps.into_iter().take(7).collect::<Vec<_>>(),"limitations":report.limitations.into_iter().take(8).collect::<Vec<_>>() });
        let mut context = crate::storage::pii::mask_json_pii(&context)?;
        // Bound the final serialized, masked payload, including its query dictionary. Never
        // clip a source row or leave a lead whose supporting rows/queries were removed.
        while serde_json::to_vec(&context).map_err(error)?.len() >= 20_000 {
            let field = if !context["acquisitionGaps"].as_array().unwrap().is_empty() {
                "acquisitionGaps"
            } else if !context["limitations"].as_array().unwrap().is_empty() {
                "limitations"
            } else if !context["leads"].as_array().unwrap().is_empty() {
                "leads"
            } else {
                return Err(error(
                    "Financial context metadata exceeds its bounded payload",
                ));
            };
            context[field].as_array_mut().unwrap().pop();
            context["omitted"][field] =
                serde_json::json!(context["omitted"][field].as_u64().unwrap_or(0) + 1);
            if field == "leads" {
                let cited = context["leads"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .flat_map(|lead| lead["citations"].as_array().unwrap())
                    .filter_map(|id| id.as_str().map(str::to_string))
                    .collect::<BTreeSet<_>>();
                context["sourceRows"]
                    .as_array_mut()
                    .unwrap()
                    .retain(|row| row["id"].as_str().is_some_and(|id| cited.contains(id)));
                let used = context["sourceRows"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(|row| row["queryId"].as_str().map(str::to_string))
                    .collect::<BTreeSet<_>>();
                context["queries"]
                    .as_object_mut()
                    .unwrap()
                    .retain(|id, _| used.contains(id));
            }
        }
        Ok(context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    async fn repository() -> (tempfile::TempDir, DuckDbRepository) {
        let temp = tempfile::tempdir().unwrap();
        let repo = DuckDbRepository::new(temp.path().join("core.db")).unwrap();
        repo.run_migrations(include_str!("../../migrations/0001_initial_schema.sql"))
            .await
            .unwrap();
        (temp, repo)
    }
    fn install_city(conn: &duckdb::Connection) {
        conn.execute_batch("CREATE TABLE analysis_fin_budget(report_fy INTEGER,fund_number VARCHAR,account_number VARCHAR,funds_center_number VARCHAR,dept_name VARCHAR,amount DECIMAL(28,4),budget_cycle VARCHAR);
          CREATE TABLE analysis_fin_actual(report_fy INTEGER,fund_number VARCHAR,account_number VARCHAR,funds_center_number VARCHAR,dept_name VARCHAR,amount DECIMAL(28,4));
          INSERT INTO analysis_fin_budget VALUES (23,'100000','5001','001','Police',100.01,'adopted'),(23,'100000','5001','001','Police',150.01,'revised'),(23,'100000','4001','001','Police',9999,'adopted'),(23,'200000','5001','001','Police',9000,'adopted'),(23,'100000','5001','001','Police',9999,'proposed');
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
            let mut statement=conn.prepare("SELECT column_name,data_type,ordinal_position FROM information_schema.columns WHERE table_name=?1 ORDER BY ordinal_position").unwrap();
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
    }
    #[tokio::test]
    async fn financial_existing_city_snapshots_produce_exact_cited_rows_and_stable_autonomous_context(
    ) {
        let (_temp, repo) = repository().await;
        let conn = repo.analytics_connection().await.unwrap();
        install_city(&conn);
        drop(conn);
        let workspace = repo.financial_workspace().await.unwrap();
        assert_eq!(workspace.mappings.len(), 2);
        assert_eq!(workspace.default_scope.fiscal_year, Some(2023));
        assert_eq!(
            workspace.default_scope.department.as_deref(),
            Some("Police")
        );
        let report = repo
            .run_financial_investigation(workspace.default_scope)
            .await
            .unwrap();
        assert_eq!(report.reconciliation.len(), 1);
        let line = &report.reconciliation[0];
        assert_eq!(line.adopted.as_deref(), Some("100.01"));
        assert_eq!(line.revised.as_deref(), Some("150.01"));
        assert_eq!(line.actual.as_deref(), Some("120.03"));
        assert_eq!(line.actual_minus_adopted.as_deref(), Some("20.02"));
        assert!(line.encumbered.is_none());
        assert!(line.uncommitted_revised.is_none());
        for citation in &report.evidence {
            let result = repo
                .execute_restricted_sql(&report.queries[&citation.query_id], 5000)
                .await
                .unwrap();
            assert_eq!(result.columns, citation.columns);
            assert!(result.rows.contains(&citation.row));
        }
        let first = repo.financial_research_context("police").await.unwrap();
        let second = repo.financial_research_context("police").await.unwrap();
        assert_eq!(first, second);
        assert!(serde_json::to_vec(&first).unwrap().len() < 20_000);
        assert!(!first["leads"].as_array().unwrap().is_empty());
        assert!(first["status"].as_str().unwrap().contains("recompute"));
    }
    #[tokio::test]
    async fn financial_mapping_rejects_invented_columns_and_partial_sources_withhold_totals() {
        let (_temp, repo) = repository().await;
        let conn = repo.analytics_connection().await.unwrap();
        install_city(&conn);
        drop(conn);
        let workspace = repo.financial_workspace().await.unwrap();
        let mut mapping = workspace.mappings[0].clone();
        mapping
            .columns
            .insert("amount".into(), "amount); DROP TABLE documents; --".into());
        assert!(repo.save_financial_mapping(mapping).await.is_err());
        let conn = repo.analytics_connection().await.unwrap();
        let before = financial_mapping_revision(&conn).unwrap();
        drop(conn);
        repo.save_financial_mapping(workspace.mappings[0].clone())
            .await
            .unwrap();
        let conn = repo.analytics_connection().await.unwrap();
        assert_ne!(before, financial_mapping_revision(&conn).unwrap());
        conn.execute(
            "UPDATE analytical_snapshots SET partial_dataset=1 WHERE snapshot_id='actual'",
            [],
        )
        .unwrap();
        drop(conn);
        let report = repo
            .run_financial_investigation(workspace.default_scope)
            .await
            .unwrap();
        assert!(report.reconciliation[0].actual.is_none());
        assert!(report.reconciliation[0].actual_minus_adopted.is_none());
    }
    #[tokio::test]
    async fn financial_department_scope_is_withheld_for_ledgers_that_cannot_apply_it() {
        let (_temp, repo) = repository().await;
        let conn = repo.analytics_connection().await.unwrap();
        install_city(&conn);
        drop(conn);
        let workspace = repo.financial_workspace().await.unwrap();
        let mut actual = workspace
            .mappings
            .iter()
            .find(|m| m.role == "actual")
            .unwrap()
            .clone();
        actual.columns.remove("department");
        repo.save_financial_mapping(actual).await.unwrap();
        let report = repo
            .run_financial_investigation(workspace.default_scope)
            .await
            .unwrap();
        assert!(report
            .limitations
            .iter()
            .any(|l| l.starts_with("actual: the source has no mapped department column")));
        assert!(report.reconciliation.iter().all(|r| r.actual.is_none()));
    }
    #[tokio::test]
    async fn unconverted_cells_in_a_mapped_column_withhold_its_totals() {
        let (_temp, repo) = repository().await;
        let conn = repo.analytics_connection().await.unwrap();
        install_city(&conn);
        let workspace = repo.financial_workspace().await.unwrap();
        let actual = workspace
            .mappings
            .iter()
            .find(|m| m.role == "actual")
            .unwrap();
        let snapshot = actual.snapshot_id.clone();
        let amount = actual.columns["amount"].clone();
        conn.execute(
            "UPDATE analytical_snapshots SET validation_summary_json = ?2 WHERE snapshot_id = ?1",
            params![
                snapshot,
                serde_json::json!({"unconvertedByColumn": {amount: 4}}).to_string()
            ],
        )
        .unwrap();
        drop(conn);
        let report = repo
            .run_financial_investigation(workspace.default_scope)
            .await
            .unwrap();
        assert!(report.limitations.iter().any(
            |l| l.starts_with("actual:") && l.contains("unconverted cells in mapped columns=4")
        ));
        assert!(report.reconciliation.iter().all(|r| r.actual.is_none()));
    }
    #[test]
    fn financial_snapshot_identity_guard_rejects_replacement_and_removal() {
        let conn = duckdb::Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE analytical_snapshots(snapshot_id VARCHAR,table_name VARCHAR,source_sha256 VARCHAR);INSERT INTO analytical_snapshots VALUES ('snapshot','analysis_payments','old-hash');").unwrap();
        let ids = vec![(
            "snapshot".into(),
            "analysis_payments".into(),
            "old-hash".into(),
        )];
        assert!(validate_financial_sources(&conn, &ids).is_ok());
        conn.execute(
            "UPDATE analytical_snapshots SET source_sha256='replaced'",
            [],
        )
        .unwrap();
        assert!(validate_financial_sources(&conn, &ids).is_err());
        conn.execute("DELETE FROM analytical_snapshots", [])
            .unwrap();
        assert!(validate_financial_sources(&conn, &ids).is_err());
    }
    #[tokio::test]
    async fn financial_upgrade_from_pre_feature_analytical_schema_preserves_records_and_adds_mappings(
    ) {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("legacy.db");
        let conn = duckdb::Connection::open(&path).unwrap();
        // Directly create the old analytical shape; do not strip a current migrated database.
        conn.execute_batch("CREATE TABLE analytical_snapshots(snapshot_id TEXT,dataset_kind TEXT,display_name TEXT,table_name TEXT,source_uri TEXT,source_sha256 TEXT,retrieved_at TEXT,observation_period_start TEXT,observation_period_end TEXT,partial_dataset BIGINT,imported_rows BIGINT,rejected_rows BIGINT,validation_summary_json TEXT);
          CREATE TABLE analytical_columns(snapshot_id TEXT,table_name TEXT,column_name TEXT,logical_type TEXT,original_name TEXT,nullable BIGINT,dictionary TEXT,ordinal BIGINT);
          INSERT INTO analytical_snapshots VALUES ('historic','REGIONAL_DATA','Existing records','analysis_historic',NULL,repeat('b',64),'2025-01-01',NULL,NULL,0,17,0,'{}');").unwrap();
        drop(conn);
        let repo = DuckDbRepository::new(path).unwrap();
        let workspace = repo.financial_workspace().await.unwrap();
        assert_eq!(workspace.datasets.len(), 1);
        assert_eq!(workspace.datasets[0].name, "Existing records");
        let conn = repo.analytics_connection().await.unwrap();
        assert_eq!(
            conn.query_row("SELECT imported_rows FROM analytical_snapshots", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap(),
            17
        );
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM financial_mappings", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    #[tokio::test]
    async fn financial_ingested_contract_payment_outcome_tables_drive_chain_and_restricted_scenario(
    ) {
        let (_temp, repo) = repository().await;
        let conn = repo.analytics_connection().await.unwrap();
        install_city(&conn);
        let fields = [
            "fy",
            "fund",
            "currency",
            "account",
            "department",
            "center",
            "program",
            "amount",
            "contract_id",
            "vendor_id",
            "amendment_id",
            "transaction_id",
            "invoice_id",
            "status",
            "basis",
            "period_start",
            "period_end",
            "population",
            "metric",
            "unit",
            "quantity",
            "denominator",
            "denominator_unit",
            "restriction",
            "eligible_program",
            "funding_duration",
        ];
        let mut mappings = vec![];
        for role in [
            "contract",
            "amendment",
            "payment",
            "encumbrance",
            "outcome",
            "restriction",
        ] {
            let table = format!("analysis_fin_{role}");
            conn.execute_batch(&format!(
                "CREATE TABLE {table} ({})",
                fields
                    .iter()
                    .map(|f| format!("{} VARCHAR", quote(f)))
                    .collect::<Vec<_>>()
                    .join(",")
            ))
            .unwrap();
            let mut values: BTreeMap<&str, &str> = [
                ("fy", "2023"),
                ("fund", "100000"),
                ("currency", "USD"),
                ("account", "5001"),
                ("department", "Police"),
                ("center", "001"),
                ("program", "Shelter"),
                ("contract_id", "C-1"),
                ("vendor_id", "V-1"),
                ("amount", "100"),
                ("status", "executed"),
            ]
            .into_iter()
            .collect();
            match role {
                "amendment" => {
                    values.extend([("amendment_id", "A-1"), ("amount", "20")]);
                }
                "payment" => {
                    values.extend([
                        ("transaction_id", "T-1"),
                        ("invoice_id", "I-1"),
                        ("amount", "70"),
                        ("status", "paid"),
                    ]);
                }
                "encumbrance" => {
                    values.extend([("amount", "10"), ("contract_id", "")]);
                }
                "outcome" => {
                    values.extend([
                        ("basis", "actual"),
                        ("quantity", "10"),
                        ("denominator", "20"),
                        ("unit", "people"),
                        ("denominator_unit", "people"),
                        ("metric", "Permanent housing exits"),
                        ("population", "Adults"),
                        ("period_start", "2022-07-01"),
                        ("period_end", "2023-06-30"),
                    ]);
                }
                "restriction" => {
                    values.extend([
                        ("restriction", "restricted"),
                        ("eligible_program", "Shelter"),
                    ]);
                }
                _ => {}
            }
            let insert = |values: &BTreeMap<&str, &str>| {
                conn.execute_batch(&format!(
                    "INSERT INTO {table} VALUES ({})",
                    fields
                        .iter()
                        .map(|f| values
                            .get(f)
                            .map(|s| literal(s))
                            .unwrap_or_else(|| "NULL".into()))
                        .collect::<Vec<_>>()
                        .join(",")
                ))
                .unwrap();
            };
            insert(&values);
            if role == "payment" {
                values.insert("transaction_id", "T-2");
                insert(&values);
            }
            conn.execute("INSERT INTO analytical_snapshots(snapshot_id,dataset_kind,display_name,table_name,source_uri,local_path,source_sha256,retrieved_at,validation_summary_json) VALUES (?1,'REGIONAL_DATA',?1,?2,?3,'/fixture',repeat('c',64),'2026-09-26','{}')",params![role,table,format!("https://www.sandiego.gov/fixture-{role}.csv")]).unwrap();
            for (ordinal, field) in fields.iter().enumerate() {
                conn.execute("INSERT INTO analytical_columns(snapshot_id,table_name,column_name,logical_type,ordinal) VALUES (?1,?2,?3,'TEXT',?4)",params![role,table,field,ordinal]).unwrap();
            }
            mappings.push(FinancialMapping {
                role: role.into(),
                snapshot_id: role.into(),
                columns: fields.iter().map(|f| ((*f).into(), (*f).into())).collect(),
                currency: None,
                agency: Some("City of San Diego".into()),
            });
        }
        drop(conn);
        for mapping in mappings {
            repo.save_financial_mapping(mapping).await.unwrap();
        }
        let scope = FinancialScope {
            fiscal_year: Some(2023),
            fund: Some("100000".into()),
            department: Some("Police".into()),
        };
        let report = repo
            .run_financial_investigation(scope.clone())
            .await
            .unwrap();
        assert_eq!(report.chains[0].payments.as_deref(), Some("140.00"));
        assert_eq!(report.chains[0].total_authority.as_deref(), Some("120.00"));
        assert!(report
            .findings
            .iter()
            .any(|f| f.kind == "duplicate_invoice"));
        let funding = report
            .reconciliation
            .iter()
            .find(|r| r.adopted.is_some())
            .unwrap();
        // Program is not available in the official budget, so it cannot silently match an
        // encumbrance tagged to a program. Correct the mapping to the same observed grain.
        assert!(funding.uncommitted_revised.is_none());
        let mut encumbrance = repo
            .financial_workspace()
            .await
            .unwrap()
            .mappings
            .into_iter()
            .find(|m| m.role == "encumbrance")
            .unwrap();
        encumbrance.columns.remove("program");
        repo.save_financial_mapping(encumbrance).await.unwrap();
        let report = repo
            .run_financial_investigation(scope.clone())
            .await
            .unwrap();
        let funding = report
            .reconciliation
            .iter()
            .find(|r| r.adopted.is_some())
            .unwrap();
        assert_eq!(funding.uncommitted_revised.as_deref(), Some("19.98"));
        let scenario = repo
            .evaluate_financial_scenario(FinancialScenarioInput {
                scope: scope.clone(),
                funding_key: funding.key.clone(),
                outcome_key: report.outcomes[0].key.clone(),
                target_program: "Shelter".into(),
                proposed_amount: "10".into(),
            })
            .await
            .unwrap();
        assert_eq!(scenario.status, "conditional_estimate");
        assert_eq!(scenario.estimated_whole_units.as_deref(), Some("1"));
        assert!(!scenario.citations.is_empty());
        assert_eq!(scenario.funding_duration, "unknown");
        assert_eq!(scenario.period_scope, "FY2023 only; one fiscal period");
        let conn = repo.analytics_connection().await.unwrap();
        conn.execute(
            "UPDATE analysis_fin_encumbrance SET funding_duration='one-time'",
            [],
        )
        .unwrap();
        drop(conn);
        let duration_report = repo.run_financial_investigation(scope).await.unwrap();
        let one_time = duration_report
            .reconciliation
            .iter()
            .find(|r| r.funding_duration == "one_time")
            .unwrap();
        assert_eq!(one_time.encumbered.as_deref(), Some("10.00"));
        assert!(one_time.revised.is_none());
        assert!(one_time.uncommitted_revised.is_none());
        let cited = duration_report
            .evidence
            .iter()
            .find(|e| one_time.citations.contains(&e.id))
            .unwrap();
        let duration_column = cited
            .columns
            .iter()
            .position(|c| c == "funding_duration")
            .unwrap();
        assert_eq!(cited.row[duration_column], serde_json::json!("one-time"));
    }
    #[tokio::test]
    async fn financial_independent_history_queries_feed_actionable_autonomous_patterns() {
        let (_temp, repo) = repository().await;
        let conn = repo.analytics_connection().await.unwrap();
        install_city(&conn);
        conn.execute_batch("DELETE FROM analysis_fin_budget; DELETE FROM analysis_fin_actual;
          INSERT INTO analysis_fin_budget VALUES (21,'100000','512059','H','Homelessness',200,'adopted'),(22,'100000','512059','H','Homelessness',100,'adopted'),(23,'100000','512059','H','Homelessness',150,'adopted'),(23,'100000','512066','H','Homelessness',100,'adopted'),(23,'100000','512059','H','Homelessness',900000,'proposed');
          INSERT INTO analysis_fin_actual VALUES (21,'100000','512059','H','Homelessness',500),(22,'100000','512059','H','Homelessness',40),(23,'100000','512059','H','Homelessness',50),(23,'100000','512066','H','Homelessness',190);").unwrap();
        drop(conn);
        let scope = FinancialScope {
            fiscal_year: Some(2023),
            fund: Some("100000".into()),
            department: Some("Homelessness".into()),
        };
        let report = repo
            .run_financial_investigation(scope.clone())
            .await
            .unwrap();
        assert_eq!(report.reconciliation.len(), 2);
        assert!(report
            .reconciliation
            .iter()
            .all(|r| r.fiscal_year == "2023"));
        assert_eq!(
            report
                .records
                .iter()
                .filter(|r| r.role == "actual_history")
                .count(),
            3
        );
        let rebound = report
            .findings
            .iter()
            .find(|f| f.kind == "rebound_below_baseline")
            .unwrap();
        assert_eq!(rebound.calculations[0].result, "260.00");
        assert_eq!(rebound.citations.len(), 3);
        let context = repo
            .financial_research_context("homelessness")
            .await
            .unwrap();
        assert!(context["leads"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["kind"] == "account_shift" || f["kind"] == "rebound_below_baseline"));
        assert_eq!(
            context,
            repo.financial_research_context("homelessness")
                .await
                .unwrap()
        );
        assert!(serde_json::to_vec(&context).unwrap().len() < 20_000);
        for lead in context["leads"].as_array().unwrap() {
            for id in lead["citations"].as_array().unwrap() {
                let source = context["sourceRows"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|row| row["id"] == *id)
                    .expect("each retained lead keeps every cited source row");
                assert!(context["queries"]
                    .get(source["queryId"].as_str().unwrap())
                    .is_some());
            }
        }
        for citation in report
            .evidence
            .iter()
            .filter(|e| e.role.ends_with("_history"))
        {
            let result = repo
                .execute_restricted_sql(&report.queries[&citation.query_id], 500)
                .await
                .unwrap();
            assert_eq!(result.columns, citation.columns);
            assert!(result.rows.contains(&citation.row));
        }
        let conn = repo.analytics_connection().await.unwrap();
        conn.execute(
            "UPDATE analytical_snapshots SET partial_dataset=1 WHERE snapshot_id='actual'",
            [],
        )
        .unwrap();
        drop(conn);
        let partial = repo.run_financial_investigation(scope).await.unwrap();
        assert!(!partial.findings.iter().any(|f| [
            "account_shift",
            "execution_capacity_gap",
            "rebound_below_baseline"
        ]
        .contains(&f.kind.as_str())));
    }
}
