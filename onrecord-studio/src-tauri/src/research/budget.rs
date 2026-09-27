//! Reproducible City finance comparisons seed the investigation before any model interpretation.
use super::evidence::query_evidence;
use crate::{storage::sqlite::DuckDbRepository, types::*};
use chrono::Datelike;

fn official_table<'a>(
    schema: &'a AnalyticalSchema,
    role: &str,
) -> Option<&'a AnalyticalDatasetSummary> {
    schema
        .datasets
        .iter()
        .filter(|dataset| {
            let official = dataset
                .source_uri
                .as_ref()
                .and_then(|s| reqwest::Url::parse(s).ok())
                .is_some_and(|url| {
                    url.host_str() == Some("seshat.datasd.org")
                        && url.path()
                            == match role {
                                "budget" => "/operating_budget/budget_operating_datasd.csv",
                                _ => "/operating_actuals/actuals_operating_datasd.csv",
                            }
                });
            let columns = schema
                .columns
                .iter()
                .filter(|c| c.table_name == dataset.table_name)
                .collect::<Vec<_>>();
            official
                && [
                    "report_fy",
                    "fund_number",
                    "funds_center_number",
                    "account_number",
                    "account",
                    "dept_name",
                    "amount",
                ]
                .iter()
                .all(|name| columns.iter().any(|c| c.column_name == *name))
                && (role != "budget" || columns.iter().any(|c| c.column_name == "budget_cycle"))
                && columns.iter().any(|c| {
                    c.column_name == "amount"
                        && matches!(c.logical_type.as_str(), "INTEGER" | "DECIMAL" | "NUMBER")
                })
        })
        .max_by_key(|d| &d.retrieved_at)
}
fn table(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}
fn normalized(table_name: &str, budget: bool) -> String {
    format!("SELECT CASE WHEN TRY_CAST(report_fy AS INTEGER) < 100 THEN 2000 + TRY_CAST(report_fy AS INTEGER) ELSE TRY_CAST(report_fy AS INTEGER) END AS fy, CAST(funds_center_number AS VARCHAR) AS center, CAST(account_number AS VARCHAR) AS account_number, dept_name, account, amount FROM {} WHERE CAST(fund_number AS VARCHAR) = '100000' AND CAST(account_number AS VARCHAR) LIKE '5%' {}",table(table_name), if budget {"AND lower(trim(budget_cycle)) = 'adopted'"} else {""})
}
fn scope(expression: &str, topic: &str) -> String {
    let words: &[&str] = match topic {
        "homelessness" => &["homeless"],
        "abatement" => &["homeless", "environmental services", "police"],
        "bathrooms" => &["homeless", "park", "librar"],
        "police" => &["police", "homeless", "park", "librar"],
        _ => return "TRUE".into(),
    };
    format!(
        "({})",
        words
            .iter()
            .map(|word| format!("contains(lower({expression}), '{word}')"))
            .collect::<Vec<_>>()
            .join(" OR ")
    )
}

pub fn comparison_recipes(
    budget: &str,
    actuals: &str,
    topic: &str,
    last_closed_year: i32,
) -> Vec<(String, String)> {
    let common = format!(
        "b0 AS ({}), a0 AS ({})",
        normalized(budget, true),
        normalized(actuals, false)
    );
    let matched_scope = scope(
        "COALESCE(a.departments,'') || ' ' || COALESCE(b.departments,'')",
        topic,
    );
    let aggregate = |source: &str| {
        format!("SELECT fy, center, account_number, STRING_AGG(DISTINCT dept_name, ' | ' ORDER BY dept_name) AS departments, STRING_AGG(DISTINCT account, ' | ' ORDER BY account) AS account_labels, SUM(amount) AS amount, COUNT(*) AS source_rows, COUNT(*) - COUNT(amount) AS missing_amounts FROM {source} WHERE fy = (SELECT fy FROM period) GROUP BY 1,2,3")
    };
    let reconciliation = format!("WITH {common}, period AS (SELECT MAX(b0.fy) AS fy FROM (SELECT DISTINCT fy FROM b0) b0 JOIN (SELECT DISTINCT fy FROM a0) a0 ON b0.fy=a0.fy WHERE b0.fy <= {last_closed_year}), b AS ({}), a AS ({}) SELECT COALESCE(b.fy,a.fy) AS fiscal_year, COALESCE(b.center,a.center) AS funds_center_number, COALESCE(b.account_number,a.account_number) AS account_number, b.departments AS budget_departments, a.departments AS actual_departments, COALESCE(a.account_labels,b.account_labels) AS account, b.amount AS adopted_budget, a.amount AS actual_spending, b.source_rows AS budget_rows, a.source_rows AS actual_rows, b.missing_amounts AS budget_nulls, a.missing_amounts AS actual_nulls, CASE WHEN b.source_rows IS NULL THEN 'Missing budget baseline' WHEN a.source_rows IS NULL THEN 'Missing actuals' WHEN b.missing_amounts > 0 OR a.missing_amounts > 0 THEN 'Incomplete amounts' ELSE 'Matched' END AS comparison_status, CASE WHEN b.missing_amounts=0 AND a.missing_amounts=0 THEN a.amount-b.amount END AS actual_minus_adopted FROM b FULL OUTER JOIN a ON b.fy=a.fy AND b.center=a.center AND b.account_number=a.account_number WHERE {matched_scope} ORDER BY ABS(CASE WHEN b.source_rows IS NULL THEN a.amount WHEN a.source_rows IS NULL THEN b.amount ELSE a.amount-b.amount END) DESC NULLS LAST LIMIT 24",aggregate("b0"),aggregate("a0"));
    let department_scope = scope("dept_name", topic);
    let period = format!("period AS (SELECT MAX(b0.fy) AS fy FROM (SELECT DISTINCT fy FROM b0) b0 JOIN (SELECT DISTINCT fy FROM a0) a0 ON b0.fy=a0.fy WHERE b0.fy <= {last_closed_year})");
    let trend = format!("WITH {common}, {period}, b AS (SELECT fy,dept_name,CASE WHEN COUNT(*)=COUNT(amount) THEN SUM(amount) END AS adopted_budget FROM b0 WHERE {department_scope} AND fy BETWEEN (SELECT fy-2 FROM period) AND (SELECT fy FROM period) GROUP BY 1,2), a AS (SELECT fy,dept_name,CASE WHEN COUNT(*)=COUNT(amount) THEN SUM(amount) END AS actual_spending FROM a0 WHERE {department_scope} AND fy BETWEEN (SELECT fy-2 FROM period) AND (SELECT fy FROM period) GROUP BY 1,2) SELECT COALESCE(b.fy,a.fy) AS fiscal_year,COALESCE(b.dept_name,a.dept_name) AS department,b.adopted_budget,a.actual_spending,a.actual_spending-b.adopted_budget AS actual_minus_adopted FROM b FULL OUTER JOIN a ON b.fy=a.fy AND b.dept_name=a.dept_name ORDER BY department,fiscal_year DESC LIMIT 36");
    let allocation = format!("WITH b0 AS ({}), period AS (SELECT MAX(fy) AS fy FROM b0 WHERE fy <= {}), totals AS (SELECT fy,dept_name,CASE WHEN COUNT(*)=COUNT(amount) THEN SUM(amount) END AS adopted FROM b0 WHERE {department_scope} GROUP BY 1,2), current_year AS (SELECT * FROM totals WHERE fy=(SELECT fy FROM period)), prior_year AS (SELECT * FROM totals WHERE fy=(SELECT fy-1 FROM period)) SELECT (SELECT fy FROM period) AS fiscal_year,COALESCE(c.dept_name,p.dept_name) AS department,p.adopted AS prior_adopted,c.adopted AS current_adopted,c.adopted-p.adopted AS allocation_change FROM current_year c FULL OUTER JOIN prior_year p ON c.dept_name=p.dept_name ORDER BY ABS(allocation_change) DESC NULLS LAST LIMIT 24",normalized(budget,true),last_closed_year+1);
    let mut recipes = vec![("Largest account differences between adopted budget and actual spending, with unmatched counterparts".into(),reconciliation),("Three-year General Fund spending and allocation history for relevant departments".into(),trend),("Latest explicitly adopted General Fund allocation changes and service tradeoffs".into(),allocation)];
    recipes.insert(0,("Fiscal-year and budget-cycle coverage needed to establish comparable spending periods".into(), format!("WITH b AS (SELECT CASE WHEN TRY_CAST(report_fy AS INTEGER)<100 THEN 2000+TRY_CAST(report_fy AS INTEGER) ELSE TRY_CAST(report_fy AS INTEGER) END AS fiscal_year, COALESCE(NULLIF(lower(trim(budget_cycle)),''),'UNLABELED') AS cycle,amount FROM {} WHERE CAST(fund_number AS VARCHAR)='100000' AND CAST(account_number AS VARCHAR) LIKE '5%'), a AS (SELECT CASE WHEN TRY_CAST(report_fy AS INTEGER)<100 THEN 2000+TRY_CAST(report_fy AS INTEGER) ELSE TRY_CAST(report_fy AS INTEGER) END AS fiscal_year,amount FROM {} WHERE CAST(fund_number AS VARCHAR)='100000' AND CAST(account_number AS VARCHAR) LIKE '5%') SELECT 'budget' AS record_type,fiscal_year,cycle,COUNT(*) AS source_rows,COUNT(*)-COUNT(amount) AS null_amounts FROM b WHERE fiscal_year >= {} GROUP BY 1,2,3 UNION ALL SELECT 'actuals',fiscal_year,'actual',COUNT(*),COUNT(*)-COUNT(amount) FROM a WHERE fiscal_year >= {} GROUP BY 1,2,3 ORDER BY fiscal_year DESC,record_type,cycle",table(budget),table(actuals),last_closed_year-5,last_closed_year-5)));
    if matches!(topic, "police" | "abatement" | "waste") {
        recipes.push(("Police overtime: adopted allocation versus actual spending in ended fiscal years".into(),format!("WITH {common}, {period}, b AS (SELECT fy,CASE WHEN COUNT(*)=COUNT(amount) THEN SUM(amount) END AS adopted FROM b0 WHERE lower(trim(dept_name))='police' AND contains(lower(account),'overtime') GROUP BY fy), a AS (SELECT fy,CASE WHEN COUNT(*)=COUNT(amount) THEN SUM(amount) END AS actual FROM a0 WHERE lower(trim(dept_name))='police' AND contains(lower(account),'overtime') GROUP BY fy) SELECT COALESCE(b.fy,a.fy) AS fiscal_year,b.adopted AS adopted_overtime,a.actual AS actual_overtime,a.actual-b.adopted AS overtime_variance FROM b FULL OUTER JOIN a ON a.fy=b.fy WHERE COALESCE(b.fy,a.fy) BETWEEN (SELECT fy-2 FROM period) AND (SELECT fy FROM period) ORDER BY fiscal_year DESC")));
    }
    recipes
}

pub async fn seed(
    repository: &DuckDbRepository,
    schema: &AnalyticalSchema,
    topic: &str,
) -> (Vec<ResearchEvidence>, Vec<String>) {
    let (Some(budget), Some(actuals)) = (
        official_table(schema, "budget"),
        official_table(schema, "actuals"),
    ) else {
        return (vec![],vec!["City finance comparisons await both official operating budget and actuals snapshots.".into()]);
    };
    let today = chrono::Local::now();
    let closed = if today.month() >= 7 {
        today.year()
    } else {
        today.year() - 1
    };
    let mut evidence = Vec::new();
    let mut log = Vec::new();
    for (question, sql) in
        comparison_recipes(&budget.table_name, &actuals.table_name, topic, closed)
    {
        match query_evidence(repository,&question,&sql,"City General Fund 100000; expense accounts beginning 5; adopted budget only. Fiscal-year codes under 100 normalized by adding 2000. Fiscal years have ended, but ledger close/finality is not established. Budget/actuals aggregate within each key before joining; missing counterparts and null amounts are not zero. Department names may change; inspect stable funds-center/account comparison. Department spending is not a program, abatement-event or restroom cost. Variance is against original adopted appropriation, not revised legal authority. Do not add City and SDHC transfers or infer outcomes/waste from these comparisons.").await {
            Ok(record) => { log.push(format!("Computed reproducible City finance comparison: {question}")); evidence.push(record); },
            Err(error) => log.push(format!("City comparison unavailable ({question}): {error}")),
        }
    }
    (evidence, log)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn joins_account_keys_before_aggregation_preserves_missing_and_respects_cycle_fund_and_period(
    ) {
        let root = tempfile::tempdir().unwrap();
        let db = DuckDbRepository::new(root.path().join("core.db")).unwrap();
        db.run_migrations(include_str!("../../migrations/0001_initial_schema.sql"))
            .await
            .unwrap();
        let conn = db.analytics_connection().await.unwrap();
        conn.execute_batch("CREATE TABLE analysis_budget(fund_number VARCHAR,report_fy INTEGER,funds_center_number VARCHAR,account_number VARCHAR,dept_name VARCHAR,account VARCHAR,amount DECIMAL(18,2),budget_cycle VARCHAR); CREATE TABLE analysis_actuals AS SELECT * EXCLUDE(budget_cycle) FROM analysis_budget;
            INSERT INTO analysis_budget VALUES ('100000',25,'001','5001','Police old name','Overtime',100.01,'adopted'),('100000',25,'001','5001','Police old name','Overtime',50,'adopted'),('100000',25,'002','5002','Library','Services',10,'adopted'),('100000',25,'001','5001','Police old name','Overtime',99999,'proposed'),('200000',25,'001','5001','Police','Overtime',99999,'adopted'),('100000',27,'001','5001','Police','Overtime',99999,'adopted');
            INSERT INTO analysis_actuals VALUES ('100000',25,'001','5001','Police renamed','Overtime',120.01),('100000',25,'001','5001','Police renamed','Overtime',50),('100000',25,'003','5003','Parks','Contract',23),('100000',27,'001','5001','Police','Overtime',1);").unwrap();
        let recipes = comparison_recipes("analysis_budget", "analysis_actuals", "police", 2026);
        for (_, sql) in &recipes {
            db.execute_restricted_sql(sql, 40).await.unwrap();
        }
        let result = db.execute_restricted_sql(&recipes[1].1, 40).await.unwrap();
        assert_eq!(result.rows.len(), 3);
        let matched = result.rows.iter().find(|row| row[1] == "001").unwrap();
        assert!(
            matched[13]
                .as_str()
                .and_then(|v| v.parse::<f64>().ok())
                .or_else(|| matched[13].as_f64())
                == Some(20.0),
            "{:?}",
            matched
        );
        assert_eq!(matched[8], 2);
        assert_eq!(matched[9], 2);
        let missing = result.rows.iter().find(|row| row[1] == "002").unwrap();
        assert!(missing[7].is_null());
        assert!(missing[13].is_null());
        assert_eq!(
            result.rows.iter().find(|row| row[1] == "003").unwrap()[12],
            "Missing budget baseline"
        );
    }
}
