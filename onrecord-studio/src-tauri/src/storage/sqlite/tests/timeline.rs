use super::*;

/// The reason the recording surface exists, stated as the verdict it changes.
///
/// The two writes below differ only in whether an extension was recorded. Without one, a
/// determination 19 days after the request exceeds the initial statutory period and the audit
/// reports `STATUTORY_LATENCY_VIOLATION`. With a notice given inside that period claiming a
/// period the statute grants, the same dates are compliant. Before this repository could
/// record an extension, only the first outcome was reachable, which is the difference between
/// a finding and a false accusation.
/// Findings stored under an older deadline rule are rewritten once, at the first launch that
/// carries a newer rule, and never again under the same rule.
#[tokio::test]
async fn a_deadline_rule_change_recomputes_stored_findings_once() {
    let repository = repository_with_document().await;
    let audit_count = || async {
        repository
            .conn
            .lock()
            .await
            .query_row(
                "SELECT COUNT(*) FROM case_audits WHERE audit_id = ?1",
                params![format!("auto-{PORTAL_SUBDOMAIN}-{REQUEST_ID}")],
                |row| row.get::<_, i64>(0),
            )
            .unwrap()
    };
    repository
        .conn
        .lock()
        .await
        .execute(
            "UPDATE requests SET created_at = ?1, updated_at = ?2, closed_at = ?2,
             status = 'closed' WHERE id = ?3",
            params!["2026-08-03T09:00:00Z", "2026-08-24T09:00:00Z", REQUEST_ID],
        )
        .unwrap();
    assert_eq!(audit_count().await, 0);

    assert_eq!(
        repository
            .recompute_cpra_findings_for_rule_change()
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        audit_count().await,
        1,
        "the overdue request must now carry its finding"
    );
    assert_eq!(
        repository
            .recompute_cpra_findings_for_rule_change()
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn a_recorded_extension_withdraws_the_statutory_latency_violation() {
    let repository = repository_with_document().await;
    {
        let connection = repository.conn.lock().await;
        connection
            .execute(
                "UPDATE requests
                 SET created_at = ?1, updated_at = ?2, closed_at = ?2, status = 'closed'
                 WHERE id = ?3 AND portal_subdomain = ?4",
                params![
                    "2026-08-01T09:00:00Z",
                    "2026-08-20T09:00:00Z",
                    REQUEST_ID,
                    PORTAL_SUBDOMAIN,
                ],
            )
            .expect("the overdue request fixture must update");
    }

    let without_extension = repository
        .refresh_request_timeline_audit(REQUEST_ID as u64, PORTAL_SUBDOMAIN)
        .await
        .expect("the audit must run without an extension");
    assert!(
        without_extension
            .iter()
            .any(|flag| flag == "STATUTORY_LATENCY_VIOLATION"),
        "a determination 19 days out with no recorded extension exceeds the initial period; \
         got {without_extension:?}"
    );

    repository
        .record_statutory_extension(
            REQUEST_ID as u64,
            PORTAL_SUBDOMAIN,
            &StatutoryExtensionRecord {
                notice_at: "2026-08-06T09:00:00Z".to_string(),
                expected_determination_at: "2026-08-22T09:00:00Z".to_string(),
                reason: StatutoryExtensionReason::VoluminousRecords,
                notice_source: "Agency extension letter dated 2026-08-06".to_string(),
            },
        )
        .await
        .expect("a valid extension must record");

    let with_extension = repository
        .refresh_request_timeline_audit(REQUEST_ID as u64, PORTAL_SUBDOMAIN)
        .await
        .expect("the audit must run with an extension");
    assert!(
        !with_extension
            .iter()
            .any(|flag| flag == "STATUTORY_LATENCY_VIOLATION"),
        "the same dates are within the extended period, so the violation must be withdrawn; \
         got {with_extension:?}"
    );
    assert!(
        !with_extension
            .iter()
            .any(|flag| flag == "STATUTORY_EXTENSION_DEFECTIVE"),
        "the notice is inside the initial period and claims a period the statute grants, so \
         it must not be reported as defective; got {with_extension:?}"
    );
}

/// The extension write is an upsert on `(request_id, portal_subdomain)`, so the read is what
/// stops an operator replacing recorded evidence they were never shown. This pins the three
/// statements the recording surface depends on: an unrecorded extension is `None` and not a
/// zero-valued record, a recorded one reads back field for field, and a second write replaces
/// the row rather than adding a second one the reader would then have to choose between.
#[tokio::test]
async fn a_recorded_statutory_extension_reads_back_and_a_second_write_replaces_it() {
    let repository = repository_with_document().await;
    assert_eq!(
        repository
            .load_statutory_extension(REQUEST_ID as u64, PORTAL_SUBDOMAIN)
            .await
            .expect("reading an absent extension must succeed"),
        None,
        "a request with no recorded extension must report absence, not an empty record"
    );

    let recorded = StatutoryExtensionRecord {
        notice_at: "2026-08-25T09:00:00Z".to_string(),
        expected_determination_at: "2026-09-08T09:00:00Z".to_string(),
        reason: StatutoryExtensionReason::VoluminousRecords,
        notice_source: "Agency letter dated 2026-08-25".to_string(),
    };
    repository
        .record_statutory_extension(REQUEST_ID as u64, PORTAL_SUBDOMAIN, &recorded)
        .await
        .expect("a well-formed extension must record");
    assert_eq!(
        repository
            .load_statutory_extension(REQUEST_ID as u64, PORTAL_SUBDOMAIN)
            .await
            .expect("reading a recorded extension must succeed"),
        Some(recorded),
        "the recorded extension must read back exactly, reason included"
    );

    let corrected = StatutoryExtensionRecord {
        notice_at: "2026-08-26T09:00:00Z".to_string(),
        expected_determination_at: "2026-09-09T09:00:00Z".to_string(),
        reason: StatutoryExtensionReason::InteragencyConsultation,
        notice_source: "Corrected agency letter dated 2026-08-26".to_string(),
    };
    repository
        .record_statutory_extension(REQUEST_ID as u64, PORTAL_SUBDOMAIN, &corrected)
        .await
        .expect("a corrected extension must record");
    assert_eq!(
        repository
            .load_statutory_extension(REQUEST_ID as u64, PORTAL_SUBDOMAIN)
            .await
            .expect("reading the corrected extension must succeed"),
        Some(corrected),
        "the second write must replace the first"
    );

    let connection = repository.conn.lock().await;
    let rows: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM statutory_extensions
             WHERE request_id = ?1 AND portal_subdomain = ?2",
            params![REQUEST_ID, PORTAL_SUBDOMAIN],
            |row| row.get(0),
        )
        .expect("the extension count must be readable");
    assert_eq!(rows, 1, "a request must never carry two extension rows");
}

#[tokio::test]
async fn request_timeline_synchronization_is_idempotent_and_removes_stale_flags() {
    let repository = repository_with_document().await;
    let mut request = PublicRequest {
        id: REQUEST_ID as u64,
        rpid: None,
        title: Some("Timeline synchronization".to_string()),
        description: None,
        status: "closed".to_string(),
        current_status_id: None,
        created_at: "2026-08-24T12:00:00Z".to_string(),
        updated_at: "2026-08-24T11:59:00Z".to_string(),
        closed_at: Some("2026-08-24T12:04:00Z".to_string()),
        due_date: None,
        determination_at: None,
        determination_source: None,
        neighborhood_block: None,
        statutory_extension: None,
        documents: Vec::new(),
        agency: None,
        category: None,
        timeline_anomalies: vec![
            "UPDATE_BEFORE_CREATION".to_string(),
            "UPDATE_BEFORE_CREATION".to_string(),
            "SUSPICIOUSLY_FAST_CLOSURE".to_string(),
            "UNRECOGNIZED_CALLER_VALUE".to_string(),
        ],
    };

    repository
        .insert_or_update_request(PORTAL_SUBDOMAIN, &request)
        .await
        .expect("first request synchronization must succeed");
    repository
        .insert_or_update_request(PORTAL_SUBDOMAIN, &request)
        .await
        .expect("repeated request synchronization must succeed");

    let audit_id = format!("auto-{}-{}", PORTAL_SUBDOMAIN, REQUEST_ID);
    {
        let connection = repository.conn.lock().await;
        let flags: Vec<String> = connection
            .prepare(
                "SELECT flag_type
                 FROM document_integrity_flags
                 WHERE audit_id = ?1
                 ORDER BY flag_type",
            )
            .expect("timeline flag query must prepare")
            .query_map(params![audit_id], |row| row.get(0))
            .expect("timeline flag query must execute")
            .collect::<duckdb::Result<Vec<_>>>()
            .expect("timeline flags must decode");
        assert_eq!(
            flags,
            vec![
                "SUSPICIOUSLY_FAST_CLOSURE".to_string(),
                "UPDATE_BEFORE_CREATION".to_string(),
            ]
        );
    }

    request.timeline_anomalies.clear();
    repository
        .insert_or_update_request(PORTAL_SUBDOMAIN, &request)
        .await
        .expect("cleared request synchronization must succeed");

    let connection = repository.conn.lock().await;
    let audit_state: (String, i64) = connection
        .query_row(
            "SELECT
                verification_status,
                (
                    SELECT COUNT(*)
                    FROM document_integrity_flags
                    WHERE document_integrity_flags.audit_id = case_audits.audit_id
                )
             FROM case_audits
             WHERE audit_id = ?1",
            params![audit_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("cleared timeline audit state must decode");
    assert_eq!(audit_state, ("Compliant".to_string(), 0));
}

/// A finding whose determination date and source were both recorded is the only one this path
/// may store at certainty. Nothing was inferred, so there is nothing to discount.
#[tokio::test]
async fn a_cpra_finding_built_from_recorded_dates_is_stored_at_certainty() {
    use crate::analysis::{DeterminationBasis, StatutoryExtensionStatus};

    // Receipt and a sourced determination event 46 calendar days later, well past the 10-day
    // determination deadline, with no extension claimed.
    let analysis = crate::analysis::evaluate_statutory_compliance_with_determination_at(
        REQUEST_ID as u64,
        "2026-01-05T09:00:00Z",
        Some("2026-02-20T09:00:00Z"),
        Some("2026-02-20T09:00:00Z"),
        Some("2026-02-20T09:00:00Z"),
        Some("portal determination event"),
        None,
        "2026-03-01T00:00:00Z",
    )
    .expect("the evaluation must succeed");
    assert!(analysis.is_violation);
    assert_eq!(
        analysis.determination_basis,
        DeterminationBasis::SourcedDeterminationEvent
    );
    assert_eq!(
        analysis.extension_status,
        StatutoryExtensionStatus::NotInvoked
    );

    let variance_days = analysis.calendar_days_elapsed - analysis.allowed_calendar_days;
    assert_eq!(
        cpra_calendar_confidence_basis_points(&analysis, variance_days),
        10_000
    );
}

/// The same overrun measured to the observation instant rather than to anything the agency
/// did is not the same claim, and neither is one whose extension notice was never recorded.
/// Both used to be stored at the identical 10000 basis points.
#[tokio::test]
async fn a_cpra_finding_resting_on_an_unobserved_date_is_stored_below_certainty() {
    use crate::analysis::{DeterminationBasis, StatutoryExtensionStatus};

    // Nothing was recorded at or after receipt, so the period was measured to "now" and is
    // still running.
    let inferred = crate::analysis::evaluate_statutory_compliance_at(
        REQUEST_ID as u64,
        "2026-01-05T09:00:00Z",
        None,
        None,
        None,
        "2026-02-20T09:00:00Z",
    )
    .expect("the evaluation must succeed");
    assert!(inferred.is_violation);
    assert_eq!(
        inferred.determination_basis,
        DeterminationBasis::ObservationWithoutRecordedActivity
    );

    let variance_days = inferred.calendar_days_elapsed - inferred.allowed_calendar_days;
    let inferred_confidence = cpra_calendar_confidence_basis_points(&inferred, variance_days);
    assert!(
        inferred_confidence < 10_000,
        "an inferred determination date must not be stored at certainty, got {inferred_confidence}"
    );
    // 10000 - 2500 + (2500 / 2 * 30 / 30): a 36-day overrun recovers the full half-share of
    // the deduction the formula allows, and still lands short of certainty.
    assert_eq!(inferred_confidence, 8_750);

    // An extension claimed with no recorded notice compounds the uncertainty of a generic
    // activity timestamp standing in for a sourced determination event.
    let mut unsourced_extension = inferred.clone();
    unsourced_extension.determination_basis = DeterminationBasis::EarliestRecordedActivityProxy;
    unsourced_extension.extension_status = StatutoryExtensionStatus::MissingNoticeSource;
    assert_eq!(
        cpra_calendar_confidence_basis_points(&unsourced_extension, variance_days),
        8_900
    );
}
