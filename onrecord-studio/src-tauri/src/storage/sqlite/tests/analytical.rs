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

/// Proves the CSV formula-injection guard by round-tripping an actual export: a string cell
/// beginning with `=` must land in the written file prefixed with an apostrophe, the standard
/// mitigation every mainstream spreadsheet application honors to force text interpretation.
#[tokio::test]
async fn export_restricted_sql_csv_neutralizes_a_formula_leading_cell() {
    let repository = DuckDbRepository::new(Path::new(":memory:")).expect("DuckDB core must open");
    repository
        .run_migrations(MIGRATION)
        .await
        .expect("test migration must succeed");
    let directory = tempfile::tempdir().expect("temp dir must be created");
    let output = directory.path().join("export.csv");

    repository
        .export_restricted_sql_csv(
            "SELECT '=cmd|/c calc!A1' AS payload",
            output.to_str().unwrap(),
            10,
        )
        .await
        .expect("export must succeed");

    let written = std::fs::read_to_string(&output).expect("export file must be readable");
    assert!(
        written.contains("'=cmd|/c calc!A1"),
        "a formula-leading cell must be prefixed with an apostrophe so it reads as text, got: {written}"
    );
}
