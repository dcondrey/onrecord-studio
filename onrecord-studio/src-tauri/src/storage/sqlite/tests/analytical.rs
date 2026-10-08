use super::*;

/// A symlink at the export target would redirect the write to a location the caller never
/// actually chose. Proves both halves: the export call itself is refused, and the file the
/// symlink actually points at is untouched, not just overwritten with different content.
#[cfg(unix)]
#[tokio::test]
async fn export_restricted_sql_csv_refuses_to_write_through_a_symlinked_target() {
    let repository = DuckDbRepository::new(Path::new(":memory:")).expect("DuckDB core must open");
    repository
        .run_migrations(MIGRATION)
        .await
        .expect("test migration must succeed");
    let directory = tempfile::tempdir().expect("temp dir must be created");
    let real_target = directory.path().join("real-secret.txt");
    std::fs::write(&real_target, b"do not overwrite me").expect("seed file must be written");
    let link_path = directory.path().join("export.csv");
    std::os::unix::fs::symlink(&real_target, &link_path).expect("symlink must be created");

    let error = repository
        .export_restricted_sql_csv("SELECT 1 AS n", link_path.to_str().unwrap(), 10)
        .await
        .expect_err("a symlinked export target must be refused");
    assert!(
        error.to_string().contains("symbolic link"),
        "unexpected error: {error}"
    );
    assert_eq!(
        std::fs::read(&real_target).expect("real file must still exist"),
        b"do not overwrite me",
        "the file behind the symlink must be untouched"
    );
}

/// The formula-injection guard, through an actual export: every formula trigger is prefixed with
/// an apostrophe so spreadsheets read it as text, without corrupting a number that reaches the
/// guard as a string. DECIMAL is rendered as text before the CSV writer sees it, so a negative
/// amount arrives looking like a `-`-led string.
#[tokio::test]
async fn export_restricted_sql_csv_neutralizes_formula_cells_and_keeps_negative_decimals_numeric() {
    let repository = DuckDbRepository::new(Path::new(":memory:")).expect("DuckDB core must open");
    repository
        .run_migrations(MIGRATION)
        .await
        .expect("test migration must succeed");
    let directory = tempfile::tempdir().expect("temp dir must be created");
    let output = directory.path().join("export.csv");

    repository
        .export_restricted_sql_csv(
            "SELECT CAST(-1234.50 AS DECIMAL(10, 2)) AS amount, \
             chr(9) || '=HYPERLINK(1)' AS tabbed, '-1-1' AS expression, \
             '=cmd|/c calc!A1' AS payload",
            output.to_str().unwrap(),
            10,
        )
        .await
        .expect("export must succeed");

    let written = std::fs::read_to_string(&output).expect("export file must be readable");
    let row = written
        .lines()
        .nth(1)
        .expect("export must carry a data row");
    assert_eq!(row, "-1234.50,'\t=HYPERLINK(1),'-1-1,'=cmd|/c calc!A1");
}
