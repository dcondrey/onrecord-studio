use super::*;

/// A core written before fetch history existed has no `document_acquisitions`, and the startup
/// relation audit queries it, so opening that core must create the table before auditing.
#[tokio::test]
async fn a_core_from_before_fetch_history_still_opens() {
    let repository = repository_with_document().await;
    repository
        .conn
        .lock()
        .await
        .execute_batch(
            "DROP INDEX idx_document_acquisitions_document;
             DROP TABLE document_acquisitions;
             DROP SEQUENCE document_acquisition_seq;",
        )
        .unwrap();
    repository
        .run_migrations(MIGRATION)
        .await
        .expect("an older core must open");
    repository
        .record_document_acquisition(
            PORTAL_SUBDOMAIN,
            DOCUMENT_ID as u64,
            None,
            RAW_FILE_SHA256,
            1,
            &test_acquisition(),
        )
        .await
        .expect("the upgraded core must record fetches");
}

/// Startup runs the migration script on every launch, so a second run must be a no-op rather
/// than an error, and the provenance classification columns must survive it.
#[tokio::test]
async fn migrations_are_idempotent_and_persist_provenance_classification() {
    let directory = tempfile::tempdir().expect("temp dir must be created");
    let file_path = directory.path().join("release.txt");
    std::fs::write(&file_path, b"released bytes").expect("evidence file must be written");
    let actual_digest = crate::integrity::sha256_hex(b"released bytes");

    const CLASSIFICATION_COLUMNS: [&str; 3] = [
        "recorded_digest_source",
        "signature_verification",
        "provenance_confidence",
    ];

    let repository = repository_with_document().await;
    repository
        .run_migrations(MIGRATION)
        .await
        .expect("a second migration run must succeed");
    repository
        .run_migrations(MIGRATION)
        .await
        .expect("a third migration run must succeed");

    set_document_file(&repository, &file_path, Some(&actual_digest)).await;
    let before_upgrade = repository
        .verify_document_provenance(
            DOCUMENT_ID as u64,
            PORTAL_SUBDOMAIN,
            ProvenanceEvidenceFile::Readable(file_path.clone()),
        )
        .await
        .expect("verification must run after repeated migrations");

    // Reproduce a database written before classification existed, with a row already in the
    // table. Adding a NOT NULL DEFAULT column to a non-empty table is what the upgrade path
    // actually performs, and what would fail every existing installation at startup if DuckDB
    // rejected it. A fresh database never reaches that branch, because the base schema creates
    // the columns outright and `ensure_column` is then a no-op.
    {
        let connection = repository.conn.lock().await;
        // DuckDB's rule, measured rather than assumed: an index blocks DROP COLUMN on its
        // table but not ADD COLUMN. That is what makes `ensure_column` safe to run against an
        // indexed log, and it is why reproducing the pre-classification shape below has to
        // drop the index first.
        connection
            .execute_batch("ALTER TABLE provenance_verification_log ADD COLUMN probe_column TEXT;")
            .expect("ADD COLUMN must remain possible on an indexed table");
        connection
            .execute_batch("ALTER TABLE provenance_verification_log DROP COLUMN probe_column;")
            .expect_err("DROP COLUMN must be refused while an index depends on the table");
        connection
            .execute_batch("DROP INDEX idx_provenance_verification_document;")
            .expect("the log index must be droppable");
        connection
            .execute_batch("ALTER TABLE provenance_verification_log DROP COLUMN probe_column;")
            .expect("with the index gone the column must be removable");
        for column in CLASSIFICATION_COLUMNS {
            connection
                .execute_batch(&format!(
                    "ALTER TABLE provenance_verification_log DROP COLUMN {column};"
                ))
                .expect("the pre-classification shape must be reproducible");
        }
    }
    repository
        .run_migrations(MIGRATION)
        .await
        .expect("the upgrade path must restore the classification columns");

    let after_upgrade = repository
        .verify_document_provenance(
            DOCUMENT_ID as u64,
            PORTAL_SUBDOMAIN,
            ProvenanceEvidenceFile::Readable(file_path.clone()),
        )
        .await
        .expect("verification must run after the upgrade");
    assert_eq!(
        before_upgrade.provenance_confidence, after_upgrade.provenance_confidence,
        "the upgrade must not change how a document is classified"
    );

    let connection = repository.conn.lock().await;
    // A row that predates the columns cannot be reclassified after the fact, so it must carry
    // the sentinel rather than a plausible-looking value.
    let legacy_rows: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM provenance_verification_log
             WHERE provenance_confidence = 'UNCLASSIFIED_LEGACY_RECORD'
               AND signature_verification = 'UNCLASSIFIED_LEGACY_RECORD'
               AND recorded_digest_source = 'UNCLASSIFIED_LEGACY_RECORD'",
            [],
            |row| row.get(0),
        )
        .expect("the sentinel default must reach rows written before the upgrade");
    assert_eq!(legacy_rows, 1);

    let (confidence, digest_source, signature_state): (String, String, String) = connection
        .query_row(
            "SELECT provenance_confidence, recorded_digest_source, signature_verification
             FROM provenance_verification_log
             WHERE document_id = ?1 AND portal_subdomain = ?2
               AND provenance_confidence <> 'UNCLASSIFIED_LEGACY_RECORD'",
            params![DOCUMENT_ID, PORTAL_SUBDOMAIN],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("a verification written after the upgrade must be classified");
    assert_eq!(confidence, after_upgrade.provenance_confidence.as_str());
    assert_eq!(digest_source, after_upgrade.recorded_digest_source.as_str());
    assert_eq!(
        signature_state,
        after_upgrade.signature_verification.as_str()
    );
}

/// Reproduces a database written before the certificate-serial column existed, on both tables
/// it belongs to, and proves the upgrade adds it as a working, nullable column rather than
/// just making `run_migrations` stop erroring. A fresh database never exercises this path: the
/// base schema creates the column outright and `ensure_column` is a no-op.
///
/// Builds the legacy shape directly (per CLAUDE.md) rather than dropping the column from an
/// already-migrated database: `provenance_assertions` carries a foreign key to
/// `provenance_manifests`, which is the same family of constraint that refuses `DROP COLUMN`
/// on `documents`/`portals`/`legal_statutes`, so stripping a column back out is not a safe way
/// to reproduce this shape. The primary key is kept because that foreign key needs it; the
/// (unrelated) foreign key to `documents` is omitted since this test never touches it.
#[tokio::test]
async fn signing_certificate_serial_number_is_added_to_a_database_that_predates_it() {
    const LEGACY_PROVENANCE_TABLES: &str = "
        CREATE TABLE provenance_manifests (
            document_id BIGINT NOT NULL,
            portal_subdomain TEXT NOT NULL,
            file_hash_sha256 TEXT NOT NULL,
            is_cryptographically_signed BIGINT DEFAULT 0,
            signature_issuer TEXT,
            signing_timestamp TEXT,
            PRIMARY KEY (document_id, portal_subdomain)
        );
        CREATE TABLE provenance_verification_log (
            verification_id TEXT PRIMARY KEY,
            document_id BIGINT NOT NULL,
            portal_subdomain TEXT NOT NULL,
            verified_at TEXT NOT NULL,
            expected_sha256 TEXT,
            computed_sha256 TEXT NOT NULL,
            hash_matches BIGINT NOT NULL,
            manifest_present BIGINT NOT NULL,
            is_cryptographically_signed BIGINT NOT NULL,
            invalid_assertion_count BIGINT NOT NULL,
            chain_of_custody_confirmed BIGINT NOT NULL,
            verification_errors_json TEXT NOT NULL
        );
    ";

    let repository = DuckDbRepository::new(Path::new(":memory:")).expect("DuckDB core must open");
    {
        let connection = repository.conn.lock().await;
        connection
            .execute_batch(LEGACY_PROVENANCE_TABLES)
            .expect("the pre-upgrade provenance shape must be creatable");
        connection
            .execute(
                "INSERT INTO provenance_manifests (
                    document_id, portal_subdomain, file_hash_sha256,
                    is_cryptographically_signed, signature_issuer, signing_timestamp
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    DOCUMENT_ID,
                    PORTAL_SUBDOMAIN,
                    "deadbeef",
                    true,
                    "Legacy Issuer",
                    "2020-01-01T00:00:00Z",
                ],
            )
            .expect("a manifest recorded before the upgrade must insert");
    }

    repository
        .run_migrations(MIGRATION)
        .await
        .expect("the upgrade path must add the certificate-serial columns to a populated database");

    let connection = repository.conn.lock().await;
    let restored: Option<String> = connection
        .query_row(
            "SELECT signing_certificate_serial_number FROM provenance_manifests
             WHERE document_id = ?1 AND portal_subdomain = ?2",
            params![DOCUMENT_ID, PORTAL_SUBDOMAIN],
            |row| row.get(0),
        )
        .expect("the pre-upgrade manifest row must remain readable");
    assert_eq!(
        restored, None,
        "a manifest recorded before this column existed cannot be backfilled with a serial \
         number"
    );

    connection
        .execute(
            "UPDATE provenance_manifests
             SET signing_certificate_serial_number = ?1
             WHERE document_id = ?2 AND portal_subdomain = ?3",
            params!["01:23:45:67", DOCUMENT_ID, PORTAL_SUBDOMAIN],
        )
        .expect("the added column must accept a write");
    let recorded: Option<String> = connection
        .query_row(
            "SELECT signing_certificate_serial_number FROM provenance_manifests
             WHERE document_id = ?1 AND portal_subdomain = ?2",
            params![DOCUMENT_ID, PORTAL_SUBDOMAIN],
            |row| row.get(0),
        )
        .expect("the written value must be readable");
    assert_eq!(recorded, Some("01:23:45:67".to_string()));
}

/// DuckDB's ADD COLUMN rule, measured against the linked library rather than carried over
/// from a CLI or a release note. The shape of every `ensure_column` definition depends on it,
/// and a future DuckDB bump that adds support should surface here rather than silently leave
/// the shims weaker than they need to be.
#[test]
fn duckdb_add_column_takes_a_default_but_no_constraint() {
    let connection = duckdb::Connection::open_in_memory().expect("an in-memory core must open");
    connection
        .execute_batch("CREATE TABLE probe (id INTEGER); INSERT INTO probe VALUES (1);")
        .expect("the probe table must be created");

    for definition in [
        "TEXT NOT NULL DEFAULT ''",
        "TEXT NOT NULL",
        "REAL DEFAULT 0.0 CHECK (added >= 0.0)",
        "INTEGER CHECK (added >= 0)",
    ] {
        connection
            .execute_batch(&format!("ALTER TABLE probe ADD COLUMN added {definition}"))
            .expect_err("a constrained ADD COLUMN must stay refused");
    }
    connection
        .execute_batch("ALTER TABLE probe ADD CONSTRAINT bound CHECK (id >= 0)")
        .expect_err("adding a constraint after the fact must stay unimplemented");

    connection
        .execute_batch("ALTER TABLE probe ADD COLUMN added TEXT DEFAULT 'backfilled'")
        .expect("a bare DEFAULT must remain accepted");
    let backfilled: String = connection
        .query_row("SELECT added FROM probe", [], |row| row.get(0))
        .expect("the default must reach a row written before the column existed");
    assert_eq!(
        backfilled, "backfilled",
        "the DEFAULT is what makes an upgraded database hold the same values as a fresh one"
    );

    // Not a hypothetical: every table these shims touch except `portals` is indexed, and
    // `portals` gaining an index later must not turn a working migration into a startup
    // failure. That is why no shim reaches for SET NOT NULL as a second step.
    connection
        .execute_batch("CREATE INDEX idx_probe ON probe(id);")
        .expect("the probe index must be created");
    connection
        .execute_batch("ALTER TABLE probe ALTER COLUMN added SET NOT NULL")
        .expect_err("SET NOT NULL must stay refused while an index depends on the table");
}

/// The rule rather than the symptom. A definition DuckDB cannot apply is refused here, where
/// the message names the column and the fix, instead of at startup on an installation old
/// enough to be missing the column -- which is never a developer's machine, which is why
/// fifteen of these went unnoticed.
#[test]
fn ensure_column_refuses_a_definition_duckdb_cannot_add() {
    let connection = duckdb::Connection::open_in_memory().expect("an in-memory core must open");
    connection
        .execute_batch("CREATE TABLE probe (id INTEGER);")
        .expect("the probe table must be created");

    let refusal = super::super::migrations::ensure_column(
        &connection,
        "probe",
        "added",
        "TEXT NOT NULL DEFAULT ''",
    )
    .expect_err("a constrained definition must be refused");
    assert!(
        format!("{refusal:?}").contains("probe.added"),
        "the refusal must name the offending column, got: {refusal:?}"
    );
    super::super::migrations::ensure_column(&connection, "probe", "added", "TEXT DEFAULT ''")
        .expect("a type and a DEFAULT must be accepted");
    // The match is on the parser's token, not the word: this definition is legal SQL and
    // refusing it would abort startup for the reason the guard exists to prevent.
    super::super::migrations::ensure_column(
        &connection,
        "probe",
        "state",
        "TEXT DEFAULT 'UNCHECKED'",
    )
    .expect("a DEFAULT literal containing the word must still be accepted");
}

/// The compatibility shims only ever ran against databases that already had the column, so a
/// definition DuckDB cannot apply was a no-op everywhere except the upgrade the shim exists to
/// perform. This builds the two pre-upgrade shapes directly rather than stripping columns from
/// a current database -- DuckDB refuses DROP COLUMN on a table anything depends on, and both
/// of these have dependents -- so what the migration runs against is the column set an older
/// build actually wrote. It must complete, and the rows already there must come back carrying
/// the declared defaults rather than NULL.
#[tokio::test]
async fn migrations_restore_columns_missing_from_an_older_database() {
    let repository = DuckDbRepository::new(Path::new(":memory:")).expect("DuckDB core must open");
    {
        let connection = repository.conn.lock().await;
        // `CREATE TABLE IF NOT EXISTS` in the base schema leaves both of these alone, so the
        // shims are the only thing that can complete them.
        connection
            .execute_batch(
                "CREATE TABLE portals (
                    subdomain TEXT PRIMARY KEY,
                    display_name TEXT NOT NULL,
                    base_url TEXT NOT NULL
                 );
                 CREATE TABLE legal_statutes (
                    statute_id TEXT PRIMARY KEY,
                    jurisdiction TEXT NOT NULL,
                    statute_citation TEXT NOT NULL,
                    title TEXT NOT NULL,
                    full_text TEXT NOT NULL,
                    mandated_deadline_days BIGINT
                 );
                 INSERT INTO legal_statutes VALUES (
                    'legacy-statute', 'CA', 'CPRA 7922.535', 'Legacy Authority',
                    'The agency shall respond within ten days.', 10
                 );
                 CREATE TABLE requests (
                    id BIGINT NOT NULL,
                    portal_subdomain TEXT NOT NULL,
                    rpid TEXT,
                    title TEXT,
                    description TEXT,
                    status TEXT NOT NULL,
                    current_status_id BIGINT,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    closed_at TEXT,
                    due_date TEXT,
                    determination_at TEXT,
                    determination_source TEXT,
                    neighborhood_block TEXT,
                    agency_id BIGINT,
                    category_id BIGINT,
                    PRIMARY KEY (id, portal_subdomain)
                 );
                 CREATE TABLE documents (
                    id BIGINT NOT NULL,
                    request_id BIGINT NOT NULL,
                    portal_subdomain TEXT NOT NULL,
                    visibility TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    download_status TEXT DEFAULT 'pending',
                    PRIMARY KEY (id, portal_subdomain)
                 );
                 CREATE TABLE portal_sync_runs (
                    run_id TEXT PRIMARY KEY,
                    portal_subdomain TEXT NOT NULL,
                    started_at TEXT NOT NULL,
                    completed_at TEXT,
                    status TEXT NOT NULL,
                    pages_processed BIGINT NOT NULL DEFAULT 0,
                    requests_processed BIGINT NOT NULL DEFAULT 0,
                    documents_processed BIGINT NOT NULL DEFAULT 0,
                    failures BIGINT NOT NULL DEFAULT 0
                 );
                 INSERT INTO portals
                 VALUES ('legacy-portal', 'Legacy Portal', 'https://legacy.test');
                 INSERT INTO requests (
                    id, portal_subdomain, status, created_at, updated_at
                 ) VALUES (
                    41, 'legacy-portal', 'open',
                    '2026-08-24T12:00:00Z', '2026-08-24T12:00:00Z'
                 );
                 INSERT INTO portal_sync_runs (
                    run_id, portal_subdomain, started_at, status
                 ) VALUES (
                    'legacy-run', 'legacy-portal', '2026-08-24T12:00:00Z', 'CANCELLED'
                 );",
            )
            .expect("the pre-upgrade tables must be creatable");
        connection
            .execute(
                "INSERT INTO documents (
                    id, request_id, portal_subdomain, visibility, created_at, updated_at
                 ) VALUES (?1, ?2, 'legacy-portal', 'public', ?3, ?3)",
                params![DOCUMENT_ID, REQUEST_ID, "2026-08-24T12:00:00Z"],
            )
            .expect("the pre-upgrade document row must insert");
    }

    repository
        .run_migrations(MIGRATION)
        .await
        .expect("the upgrade path must complete against tables written before it");

    let connection = repository.conn.lock().await;
    let (origin, pointer, health, kind): (String, String, String, String) = connection
        .query_row(
            "SELECT origin_url, requests_pointer, health_status, source_kind
             FROM portals WHERE subdomain = 'legacy-portal'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("the portal row must survive the upgrade carrying its defaults");
    assert_eq!(origin, "");
    assert_eq!(pointer, "/requests");
    assert_eq!(health, "verified");
    assert_eq!(kind, "NEXT_REQUEST");
    let scheduler_counts: (i64, i64) = connection
        .query_row(
            "SELECT crawl_pages_started, resources_started
             FROM portal_sync_runs WHERE run_id = 'legacy-run'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("legacy scheduler counters must be backfilled explicitly");
    assert_eq!(scheduler_counts, (0, 0));

    // `prepare_legacy_legal_schema` runs before the base schema and is gated on the table
    // already existing, so its five shims are unreachable on a fresh database and were never
    // covered until this stub put a pre-upgrade `legal_statutes` in front of them.
    let (authority_kind, digest): (String, String) = connection
        .query_row(
            "SELECT authority_kind, content_sha256 FROM legal_statutes
             WHERE statute_id = 'legacy-statute'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("the legacy statute must survive the repair path carrying its default");
    assert_eq!(authority_kind, "OTHER");
    assert_eq!(
        digest.len(),
        64,
        "the repair path must replace the placeholder digest with one over the stored text"
    );

    let (entropy, sample_bytes, density): (Option<f64>, i64, i64) = connection
        .query_row(
            "SELECT shannon_entropy, entropy_sample_bytes, suspicious_data_density
             FROM documents WHERE id = ?1 AND portal_subdomain = 'legacy-portal'",
            params![DOCUMENT_ID],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("the document row must survive the upgrade carrying its defaults");
    // `shannon_entropy` alone takes no DEFAULT: this row was never assessed, and 0.0 would
    // claim every byte of it was identical. The other two defaults are honest claims.
    assert_eq!(
        entropy, None,
        "an upgraded row that was never assessed must read as absent, not as measured 0.0"
    );
    assert_eq!(sample_bytes, 0);
    assert_eq!(density, 0);
}

/// Measures the rule `reject_narrow_identifier_core` rests on, against the linked library.
///
/// If a future DuckDB lifts this, the narrow-core refusal can become a real migration and this
/// test is what will say so.
#[test]
fn duckdb_refuses_to_widen_a_key_column() {
    let connection = duckdb::Connection::open_in_memory().expect("DuckDB must open");
    connection
        .execute_batch(
            "CREATE TABLE parent (id INTEGER NOT NULL, portal TEXT NOT NULL,
                 PRIMARY KEY (id, portal));
             CREATE TABLE child (document_id INTEGER NOT NULL, portal TEXT NOT NULL,
                 FOREIGN KEY (document_id, portal) REFERENCES parent(id, portal));",
        )
        .expect("the narrow fixture must apply");

    let child = connection.execute_batch("ALTER TABLE child ALTER COLUMN document_id TYPE BIGINT");
    let parent = connection.execute_batch("ALTER TABLE parent ALTER COLUMN id TYPE BIGINT");
    assert!(
        child.is_err() && parent.is_err(),
        "DuckDB is expected to refuse widening a key column: child {child:?}, parent {parent:?}"
    );
}

/// A core written before identifiers were widened must say so, not fail as a binder error.
///
/// Without this the widened schema aborts on the first foreign key whose `BIGINT` child will
/// not bind to an `INTEGER` parent, and that error is raised from the setup hook, which panics.
/// The operator sees a crash instead of a sentence naming what happened.
#[tokio::test]
async fn a_core_with_thirty_two_bit_identifiers_is_refused_with_its_reason() {
    let repository = DuckDbRepository::new(Path::new(":memory:")).expect("DuckDB must open");
    {
        let connection = repository.conn.lock().await;
        connection
            .execute_batch(
                "CREATE TABLE requests (id INTEGER NOT NULL, portal_subdomain TEXT NOT NULL,
                     status TEXT NOT NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
                     PRIMARY KEY (id, portal_subdomain));",
            )
            .expect("the narrow core fixture must apply");
    }

    let error = repository
        .run_migrations(MIGRATION)
        .await
        .expect_err("a narrow core must be refused");
    let message = error.to_string();
    assert!(
        message.contains("2147483647") && message.contains("cannot be upgraded automatically"),
        "the refusal must name the limit and say it cannot be repaired: {message}"
    );
}

#[tokio::test]
async fn logical_relation_audit_names_orphans_without_modifying_them() {
    let repository = repository_with_document().await;
    let connection = repository.conn.lock().await;
    connection
        .execute(
            "INSERT INTO documents (
                id, request_id, portal_subdomain, visibility, created_at, updated_at
             ) VALUES (999, 998, ?1, 'public', '2026-08-27T00:00:00Z', '2026-08-27T00:00:00Z')",
            params![PORTAL_SUBDOMAIN],
        )
        .expect("the intentionally unconstrained orphan fixture must insert");

    let error = verify_logical_relations(&connection)
        .expect_err("the startup consistency audit must refuse an orphan");
    assert!(
        error
            .to_string()
            .contains("documents -> requests: 1 orphan row"),
        "the report must identify the broken relation and count: {error}"
    );
    let retained: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM documents WHERE id = 999 AND portal_subdomain = ?1",
            params![PORTAL_SUBDOMAIN],
            |row| row.get(0),
        )
        .expect("the orphan count must decode");
    assert_eq!(
        retained, 1,
        "the audit must never delete canonical evidence"
    );
}

/// A core from before statutory jurisdictions: `portals` lacks `statutory_jurisdiction`, with a
/// child row referencing it (DuckDB constrains writes to a referenced parent), and
/// `statutory_violations` lacks the clock columns. Opening it adds the columns with no value,
/// fills a jurisdiction only for a host recognized as a California public agency, and leaves a
/// stored finding's basis unrecorded rather than inventing one.
#[tokio::test]
async fn a_core_from_before_statutory_jurisdictions_opens_without_assuming_one() {
    let repository = DuckDbRepository::new(Path::new(":memory:")).expect("DuckDB core must open");
    {
        let connection = repository.conn.lock().await;
        connection
            .execute_batch(
                "CREATE TABLE portals (
                    subdomain TEXT PRIMARY KEY,
                    display_name TEXT NOT NULL,
                    base_url TEXT NOT NULL
                 );
                 CREATE TABLE legacy_portal_children (
                    portal_subdomain TEXT NOT NULL REFERENCES portals(subdomain)
                 );
                 CREATE TABLE statutory_violations (
                    violation_id TEXT PRIMARY KEY,
                    portal_subdomain TEXT NOT NULL,
                    request_id BIGINT NOT NULL,
                    matched_statute_id TEXT NOT NULL,
                    matched_version_id TEXT NOT NULL,
                    mandate_id TEXT,
                    classification TEXT NOT NULL,
                    calculated_variance_metric TEXT,
                    variance_days BIGINT,
                    confidence_basis_points BIGINT NOT NULL,
                    evidence_summary TEXT NOT NULL,
                    supporting_document_ids_json TEXT NOT NULL,
                    missing_evidence_json TEXT NOT NULL,
                    classifier_version TEXT NOT NULL,
                    review_status TEXT NOT NULL DEFAULT 'OPEN',
                    reviewed_at TEXT,
                    reviewer_notes TEXT,
                    first_detected_at TEXT NOT NULL,
                    evaluated_at TEXT NOT NULL
                 );
                 INSERT INTO portals VALUES
                    ('sandiego', 'City of San Diego', 'https://sandiego.nextrequest.com/'),
                    ('seattle', 'Seattle', 'https://seattle.nextrequest.com/');
                 INSERT INTO legacy_portal_children VALUES ('sandiego'), ('seattle');
                 INSERT INTO statutory_violations (
                    violation_id, portal_subdomain, request_id, matched_statute_id,
                    matched_version_id, classification, confidence_basis_points,
                    evidence_summary, supporting_document_ids_json, missing_evidence_json,
                    classifier_version, first_detected_at, evaluated_at
                 ) VALUES (
                    'legacy-finding', 'sandiego', 1, 'CPRA_7922.535',
                    'builtin-cpra-7922-535-2026', 'STATUTORY_LATENCY_VIOLATION', 9000,
                    'legacy', '[]', '[]', 'cpra-calendar-v1',
                    '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'
                 );",
            )
            .expect("the pre-upgrade shape must be creatable");
    }

    repository
        .run_migrations(MIGRATION)
        .await
        .expect("an older core must open");

    assert_eq!(
        repository
            .portal_statutory_jurisdiction("sandiego")
            .await
            .unwrap(),
        Some(Some("CA_CPRA".to_string()))
    );
    assert_eq!(
        repository
            .portal_statutory_jurisdiction("seattle")
            .await
            .unwrap(),
        Some(None),
        "a host on the same platform is not California by implication"
    );
    let connection = repository.conn.lock().await;
    let stored: (Option<String>, Option<String>) = connection
        .query_row(
            "SELECT clock_rule_id, determination_basis FROM statutory_violations
             WHERE violation_id = 'legacy-finding'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("the pre-upgrade finding must remain readable");
    assert_eq!(stored, (None, None));
    let foia_authority: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM legal_statute_versions WHERE statute_id = 'FOIA_552_A_6'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(foia_authority, 1, "a FOIA finding needs its authority row");
}
