//! Validate every nested query, including CTE bodies and scalar subqueries.
use crate::error::AppError;
use sqlparser::{
    ast::{Expr, Query, SetExpr, Statement, TableFactor, Visit, Visitor},
    dialect::DuckDbDialect,
    parser::Parser,
};
use std::{collections::BTreeSet, ops::ControlFlow};

pub fn validate(sql: &str, tables: &BTreeSet<String>) -> Result<BTreeSet<String>, AppError> {
    let statements = Parser::parse_sql(&DuckDbDialect {}, sql)
        .map_err(|e| AppError::DataParsing(format!("SQL parse failed: {e}")))?;
    if statements.len() != 1 || !matches!(&statements[0], Statement::Query(_)) {
        return Err(AppError::DataParsing(
            "Submit exactly one read-only SELECT query".into(),
        ));
    }
    let mut checker = Checker {
        tables,
        aliases: BTreeSet::new(),
        referenced: BTreeSet::new(),
    };
    // Collect aliases first; DuckDB checks visibility. An alias may never shadow a protected
    // physical table, so an out-of-scope alias cannot accidentally authorize that table.
    if let ControlFlow::Break(error) = statements.visit(&mut Aliases(&mut checker)) {
        return Err(error);
    }
    if let ControlFlow::Break(error) = statements.visit(&mut checker) {
        return Err(error);
    }
    Ok(checker.referenced)
}

fn allowed(name: &str) -> bool {
    name.starts_with("analysis_")
        || matches!(
            name,
            "analytical_snapshots" | "analytical_columns" | "analytical_rows"
        )
}
struct Checker<'a> {
    tables: &'a BTreeSet<String>,
    aliases: BTreeSet<String>,
    referenced: BTreeSet<String>,
}
struct Aliases<'a, 'b>(&'a mut Checker<'b>);
impl Visitor for Aliases<'_, '_> {
    type Break = AppError;
    fn pre_visit_query(&mut self, query: &Query) -> ControlFlow<Self::Break> {
        if let Some(with) = &query.with {
            for cte in &with.cte_tables {
                let name = cte.alias.name.value.to_lowercase();
                if self.0.tables.contains(&name) && !allowed(&name) {
                    return ControlFlow::Break(AppError::DataParsing(format!(
                        "CTE '{name}' shadows a protected table"
                    )));
                }
                self.0.aliases.insert(name);
            }
        }
        ControlFlow::Continue(())
    }
}
impl Visitor for Checker<'_> {
    type Break = AppError;
    fn pre_visit_expr(&mut self, expr: &Expr) -> ControlFlow<Self::Break> {
        if let Expr::Function(function) = expr {
            let name = function.name.to_string().trim_matches('"').to_lowercase();
            if matches!(
                name.rsplit('.').next().unwrap_or(&name).trim_matches('"'),
                "nextval" | "setval" | "setseed" | "write_log"
            ) {
                return ControlFlow::Break(AppError::DataParsing(
                    "Functions with side effects are not allowed in analytical queries".into(),
                ));
            }
        }
        ControlFlow::Continue(())
    }
    fn pre_visit_query(&mut self, query: &Query) -> ControlFlow<Self::Break> {
        fn readonly(expr: &SetExpr) -> bool {
            match expr {
                SetExpr::Select(select) => select.into.is_none(),
                SetExpr::Query(_) => true,
                SetExpr::SetOperation { left, right, .. } => readonly(left) && readonly(right),
                _ => false,
            }
        }
        if !readonly(&query.body) || !query.locks.is_empty() {
            return ControlFlow::Break(AppError::DataParsing(
                "Only read-only SELECT queries are allowed".into(),
            ));
        }
        ControlFlow::Continue(())
    }
    fn pre_visit_table_factor(&mut self, factor: &TableFactor) -> ControlFlow<Self::Break> {
        match factor {
            TableFactor::Table { name, args: None, .. } if name.0.len() == 1 => {
                let name = name.to_string().trim_matches('"').replace("\"\"", "\"").to_lowercase();
                if self.tables.contains(&name) && allowed(&name) { self.referenced.insert(name.clone()); }
                if (self.tables.contains(&name) && allowed(&name)) || self.aliases.contains(&name) {
                    ControlFlow::Continue(())
                } else {
                    ControlFlow::Break(AppError::DataParsing(format!("Table '{name}' is not an approved analytical dataset")))
                }
            }
            TableFactor::Derived { .. } => ControlFlow::Continue(()),
            _ => ControlFlow::Break(AppError::DataParsing("Only analytical tables, CTEs, and subqueries are allowed; external table functions are disabled".into())),
        }
    }
}

/// Interrupt expensive analytical scans without a watchdog ever leaking into the next query.
/// Dropping this guard joins its watchdog while the caller still holds the connection lock.
pub(crate) struct QueryDeadline {
    cancel: std::sync::mpsc::Sender<()>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl QueryDeadline {
    pub(crate) fn start(
        conn: &duckdb::Connection,
        duration: std::time::Duration,
    ) -> Result<Self, AppError> {
        let interrupt = conn.interrupt_handle();
        let (cancel, receiver) = std::sync::mpsc::channel();
        let worker = std::thread::Builder::new()
            .name("analytical-deadline".into())
            .spawn(move || {
                if matches!(
                    receiver.recv_timeout(duration),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout)
                ) {
                    interrupt.interrupt();
                }
            })
            .map_err(|error| {
                AppError::Internal(format!("Could not start query deadline: {error}"))
            })?;
        Ok(Self {
            cancel,
            worker: Some(worker),
        })
    }
}
impl Drop for QueryDeadline {
    fn drop(&mut self) {
        let _ = self.cancel.send(());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deadline_interrupts_work_and_cannot_interrupt_the_next_query() {
        let conn = duckdb::Connection::open_in_memory().unwrap();
        {
            let _deadline =
                QueryDeadline::start(&conn, std::time::Duration::from_millis(25)).unwrap();
            assert!(conn
                .query_row("SELECT SUM(i) FROM range(1000000000000) t(i)", [], |row| {
                    row.get::<_, i64>(0)
                })
                .is_err());
        }
        assert_eq!(
            conn.query_row("SELECT 42", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            42
        );
        {
            let _deadline =
                QueryDeadline::start(&conn, std::time::Duration::from_millis(25)).unwrap();
            assert_eq!(
                conn.query_row("SELECT 7", [], |row| row.get::<_, i64>(0))
                    .unwrap(),
                7
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(40));
        assert_eq!(
            conn.query_row("SELECT 9", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            9
        );
    }
    #[test]
    fn validates_ctes_and_every_nested_read() {
        let tables = ["analysis_fixture", "documents", "analytical_rows"]
            .map(str::to_string)
            .into_iter()
            .collect();
        assert!(validate(
            "WITH budget AS (SELECT * FROM analysis_fixture) SELECT * FROM budget",
            &tables
        )
        .is_ok());
        for sql in [
            "WITH analysis_safe AS (SELECT * FROM documents) SELECT * FROM analysis_safe",
            "SELECT (SELECT COUNT(*) FROM documents) FROM analysis_fixture",
            "SELECT * FROM analysis_fixture WHERE EXISTS (SELECT 1 FROM documents)",
            "WITH documents AS (SELECT * FROM documents) SELECT * FROM documents",
            "SELECT * FROM analysis_fake('secret.csv')",
            "SELECT * FROM read_csv('/tmp/a.csv')",
            "SELECT * FROM main.documents",
            "SELECT * INTO analysis_new FROM analysis_fixture",
            "DROP TABLE analysis_fixture",
            "SELECT nextval('internal_sequence') FROM analysis_fixture",
            "SELECT main.\"nextval\"('internal_sequence') FROM analysis_fixture",
        ] {
            assert!(validate(sql, &tables).is_err(), "accepted {sql}");
        }
        assert_eq!(
            validate(
                "WITH x AS (SELECT * FROM analysis_fixture) SELECT * FROM x",
                &tables
            )
            .unwrap(),
            BTreeSet::from(["analysis_fixture".into()])
        );
    }
}
