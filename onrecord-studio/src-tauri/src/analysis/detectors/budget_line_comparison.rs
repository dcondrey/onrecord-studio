//! Department year-over-year change, overtime against its adopted line, and fund spending against
//! its adopted amount, over the imported budget and actuals tables (see
//! `analysis::budget_lines`). [`sweep`] returns every comparison, including unreconciled ones;
//! only a comparison that reaches its threshold is recorded.

use crate::{
    analysis::{
        budget_lines::{compare, to_finding, Comparison, LineData},
        finding::Finding,
    },
    error::AppError,
    storage::DuckDbRepository,
};

pub const SLUG: &str = "budget_line_comparison";
const MAX_FINDINGS: usize = 100;

pub async fn sweep(repository: &DuckDbRepository) -> Result<(LineData, Vec<Comparison>), AppError> {
    let data = repository.budget_line_data().await?;
    let comparisons = compare(&data);
    Ok((data, comparisons))
}

pub async fn run(repository: &DuckDbRepository) -> Result<Vec<Finding>, AppError> {
    let (data, comparisons) = sweep(repository).await?;
    let mut ranked: Vec<(u128, Finding)> = comparisons
        .iter()
        .filter_map(|comparison| {
            let magnitude = comparison.change_basis_points()?.unsigned_abs();
            Some((magnitude, to_finding(&data, comparison, SLUG)?))
        })
        .collect();
    ranked.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.id.cmp(&b.1.id)));
    ranked.truncate(MAX_FINDINGS);
    Ok(ranked.into_iter().map(|(_, finding)| finding).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::{
        budget_lines::{Measure, Outcome},
        finding::FindingKind,
    };
    use duckdb::params;

    const BUDGET: &str = "/operating_budget/budget_operating_datasd.csv";
    const ACTUAL: &str = "/operating_actuals/actuals_operating_datasd.csv";

    /// The City export shapes, imported directly: two-digit `report_fy`, expense accounts starting
    /// with 5, and an `account` label column.
    async fn repository(
        budget_rows: &str,
        actual_rows: &str,
        actual_flags: (i64, i64),
    ) -> (tempfile::TempDir, DuckDbRepository) {
        let temp = tempfile::tempdir().unwrap();
        let repo = DuckDbRepository::new(temp.path().join("core.db")).unwrap();
        repo.run_migrations(include_str!("../../../migrations/0001_initial_schema.sql"))
            .await
            .unwrap();
        let conn = repo.analytics_connection().await.unwrap();
        conn.execute_batch(&format!(
            "CREATE TABLE analysis_fin_budget(report_fy INTEGER,fund_number VARCHAR,account_number VARCHAR,account VARCHAR,funds_center_number VARCHAR,dept_name VARCHAR,amount DECIMAL(28,4),budget_cycle VARCHAR);
             CREATE TABLE analysis_fin_actual(report_fy INTEGER,fund_number VARCHAR,account_number VARCHAR,account VARCHAR,funds_center_number VARCHAR,dept_name VARCHAR,amount DECIMAL(28,4));
             INSERT INTO analysis_fin_budget VALUES {budget_rows};
             INSERT INTO analysis_fin_actual VALUES {actual_rows};"
        ))
        .unwrap();
        for (role, table, path, (partial, rejected)) in [
            ("budget", "analysis_fin_budget", BUDGET, (0, 0)),
            ("actual", "analysis_fin_actual", ACTUAL, actual_flags),
        ] {
            conn.execute("INSERT INTO analytical_snapshots(snapshot_id,dataset_kind,display_name,table_name,source_uri,local_path,source_sha256,retrieved_at,validation_summary_json,imported_rows,partial_dataset,rejected_rows) VALUES (?1,'OPERATING_BUDGET',?1,?2,?3,'/fixture',repeat('a',64),'2026-09-26','{}',3,?4,?5)",params![role,table,format!("https://seshat.datasd.org{path}"),partial,rejected]).unwrap();
            conn.execute("INSERT INTO analytical_columns(snapshot_id,table_name,column_name,logical_type,ordinal) SELECT ?1, table_name, column_name, CASE WHEN data_type LIKE 'DECIMAL%' THEN 'DECIMAL' ELSE 'TEXT' END, ordinal_position FROM information_schema.columns WHERE table_name=?2",params![role,table]).unwrap();
        }
        drop(conn);
        (temp, repo)
    }

    fn outcome<'a>(comparisons: &'a [Comparison], measure: Measure, scope: &str) -> &'a Outcome {
        &comparisons
            .iter()
            .find(|c| c.measure == measure && c.scope.contains(scope))
            .unwrap_or_else(|| panic!("no {measure:?} comparison for {scope}: {comparisons:#?}"))
            .outcome
    }

    #[tokio::test]
    async fn each_threshold_fires_at_its_limit_and_not_one_basis_point_below() {
        // Fund: adopted 10000, actual 10499 / 10500 / 10501 (4.99%, 5%, 5.01%).
        // Department: 10000 -> 10999 / 11000 / 11001, and 10000 -> 9000 (a 10% decrease).
        // Overtime: three overtime accounts, adopted 10000 each, actual 10499 / 10500 / 10501,
        // inside a fund whose salaries line keeps the fund total below its own threshold.
        let budget = "(24,'200499','511000','Salaries','1','Police',10000,'adopted'),
            (24,'200500','511000','Salaries','1','Police',10000,'adopted'),
            (24,'200501','511000','Salaries','1','Police',10000,'adopted'),
            (24,'200501','511000','Salaries','1','Police',99999,'proposed'),
            (24,'400000','512013','Overtime','1','Police',10000,'adopted'),
            (24,'400000','512014','Overtime - Sworn','1','Police',10000,'adopted'),
            (24,'400000','512015','Overtime Civilian','1','Police',10000,'adopted'),
            (24,'400000','511000','Salaries','1','Police',100000,'adopted'),
            (24,'400000','412000','Tax revenue','1','Police',1,'adopted')";
        let actual = "(24,'200499','511000','Salaries','1','Police',10499),
            (24,'200500','511000','Salaries','1','Police',10500),
            (24,'200501','511000','Salaries','1','Police',10501),
            (23,'300000','511000','Salaries','1','D999',10000),(24,'300000','511000','Salaries','1','D999',10999),
            (23,'300000','511000','Salaries','1','D1000',10000),(24,'300000','511000','Salaries','1','D1000',11000),
            (23,'300000','511000','Salaries','1','D1001',10000),(24,'300000','511000','Salaries','1','D1001',11001),
            (23,'300000','511000','Salaries','1','DDOWN',10000),(24,'300000','511000','Salaries','1','DDOWN',9000),
            (24,'400000','512013','Overtime','1','Police',10499),
            (24,'400000','512014','Overtime - Sworn','1','Police',10500),
            (24,'400000','512015','Overtime Civilian','1','Police',10501),
            (24,'400000','511000','Salaries','1','Police',90000),
            (24,'400000','412000','Tax revenue','1','Police',9999999)";
        let (_temp, repo) = repository(budget, actual, (0, 0)).await;
        let (_, comparisons) = sweep(&repo).await.unwrap();
        let findings = run(&repo).await.unwrap();
        let fired = |kind: FindingKind, scope: &str| {
            findings
                .iter()
                .any(|f| f.kind == kind && f.headline.contains(scope))
        };
        use FindingKind::*;
        for (kind, below, at, above) in [
            (
                FundActualAboveAdopted,
                "fund 200499",
                "fund 200500",
                "fund 200501",
            ),
            (
                DepartmentSpendingYearOverYear,
                "department D999,",
                "department D1000,",
                "department D1001,",
            ),
            (
                OvertimeAboveAdoptedLine,
                "account 512013",
                "account 512014",
                "account 512015",
            ),
        ] {
            assert!(
                !fired(kind, below),
                "{kind:?} fired below its limit: {findings:#?}"
            );
            assert!(
                fired(kind, at),
                "{kind:?} did not fire at its limit: {findings:#?}"
            );
            assert!(fired(kind, above), "{kind:?} did not fire above its limit");
        }
        assert!(fired(DepartmentSpendingYearOverYear, "department DDOWN,"));
        // Fund 400000: adopted 130000, actual 121500. Revenue accounts are outside the expense
        // filter and never enter the totals.
        assert!(matches!(
            outcome(&comparisons, Measure::FundAgainstAdopted, "fund 400000"),
            Outcome::Compared {
                reference: 1_300_000_000,
                observed: 1_215_000_000,
                ..
            }
        ));
        assert!(!fired(FundActualAboveAdopted, "fund 400000"));
        // The fund 300000 has actuals but no adopted rows: unreconciled, not a zero budget.
        assert!(matches!(
            outcome(&comparisons, Measure::FundAgainstAdopted, "fund 300000"),
            Outcome::Unreconciled { detail } if detail.contains("no adopted")
        ));
        for finding in &findings {
            assert!(!finding.kind.is_legal_claim());
            assert!(
                finding.explanation.contains("query id"),
                "{}",
                finding.explanation
            );
        }
    }

    #[tokio::test]
    async fn unbound_rows_leave_their_totals_unreconciled_and_produce_no_finding() {
        // Every scope below is ten times over its reference, so only binding can hold it back.
        let budget = "(24,'500000','511000','Salaries','1','Police',100,NULL),
            (24,'500000','511000','Salaries','1','Police',100,'adopted'),
            (24,'700000','512013','Overtime','1','Police',100,'adopted'),
            (24,'700000','511000','Salaries','1','Police',100000,'adopted'),
            (24,'800000','511000','Salaries','1','Police',100,'adopted')";
        let actual = "(24,'500000','511000','Salaries','1','Police',1000),
            (23,'600000','511000','Salaries','1','Parks',100),(24,'600000','511000','Salaries','1','Parks',1000),
            (24,'600000','511000','Salaries','1',NULL,5),
            (23,'610000','511000','Salaries','1','Parks',100),(24,'610000','511000','Salaries','1','Homeless',1000),
            (24,'700000','512013','Overtime','1',NULL,1000),
            (24,'800000','511000','Salaries','1','Police',NULL)";
        let (_temp, repo) = repository(budget, actual, (0, 0)).await;
        let (_, comparisons) = sweep(&repo).await.unwrap();
        assert!(run(&repo).await.unwrap().is_empty());
        let unreconciled = |measure, scope: &str, needle: &str| {
            assert!(
                matches!(outcome(&comparisons, measure, scope), Outcome::Unreconciled { detail } if detail.contains(needle)),
                "{scope}: {:?}",
                outcome(&comparisons, measure, scope)
            );
        };
        unreconciled(
            Measure::FundAgainstAdopted,
            "fund 500000",
            "no budget basis",
        );
        unreconciled(
            Measure::DepartmentYearOverYear,
            "department Parks, fund 600000",
            "name no department",
        );
        unreconciled(
            Measure::DepartmentYearOverYear,
            "department Parks, fund 610000",
            "no actual rows for this department in FY 2024",
        );
        unreconciled(
            Measure::DepartmentYearOverYear,
            "department Homeless, fund 610000",
            "in FY 2023",
        );
        unreconciled(
            Measure::OvertimeAgainstAdopted,
            "fund 700000",
            "no department",
        );
        unreconciled(Measure::FundAgainstAdopted, "fund 800000", "does not parse");
    }

    #[tokio::test]
    async fn a_partial_or_rejected_actual_snapshot_yields_no_number() {
        let budget = "(24,'200000','512013','Overtime','1','Police',100,'adopted')";
        let actual = "(23,'200000','512013','Overtime','1','Police',100),(24,'200000','512013','Overtime','1','Police',1000)";
        for flags in [(1, 0), (0, 3)] {
            let (_temp, repo) = repository(budget, actual, flags).await;
            let (_, comparisons) = sweep(&repo).await.unwrap();
            assert!(!comparisons.is_empty());
            assert!(
                comparisons
                    .iter()
                    .all(|c| matches!(&c.outcome, Outcome::Unreconciled { detail } if detail.contains("partial or has"))),
                "{comparisons:#?}"
            );
            assert!(run(&repo).await.unwrap().is_empty());
        }
    }

    #[tokio::test]
    async fn an_account_labelled_overtime_in_one_table_only_is_not_an_overtime_line() {
        let budget = "(24,'200000','512013','Overtime','1','Police',100,'adopted')";
        let actual = "(24,'200000','512013','Premium pay','1','Police',1000)";
        let (_temp, repo) = repository(budget, actual, (0, 0)).await;
        let (_, comparisons) = sweep(&repo).await.unwrap();
        assert!(matches!(
            outcome(&comparisons, Measure::OvertimeAgainstAdopted, "account 512013"),
            Outcome::Unreconciled { detail } if detail.contains("something else") || detail.contains("only")
        ));
        assert!(!comparisons
            .iter()
            .any(|c| c.measure == Measure::OvertimeAgainstAdopted
                && matches!(c.outcome, Outcome::Compared { .. })));
    }
}
