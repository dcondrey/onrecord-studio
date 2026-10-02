use super::*;

#[test]
fn serializes_telemetry_variants_with_camel_case_payload_fields() {
    let page = serde_json::to_value(IngestionProgress::PageCompleted {
        portal_key: "records".to_string(),
        page: 3,
        records_found: 50,
    })
    .unwrap();
    assert_eq!(
        page,
        serde_json::json!({
            "type": "pageCompleted",
            "payload": {
                "portalKey": "records",
                "page": 3,
                "recordsFound": 50
            }
        })
    );

    let stopped = serde_json::to_value(IngestionProgress::SyncStopped {
        portal_key: "records".to_string(),
        sync_run_id: "run-7".to_string(),
    })
    .unwrap();
    assert_eq!(
        stopped,
        serde_json::json!({
            "type": "syncStopped",
            "payload": {
                "portalKey": "records",
                "syncRunId": "run-7"
            }
        })
    );

    let envelope = serde_json::to_value(ingestion_telemetry_envelope(
        IngestionProgress::CorpusReconciliationStarted {},
    ))
    .unwrap();
    assert_eq!(envelope["protocolVersion"], 1);
    assert!(envelope["sequence"]
        .as_str()
        .is_some_and(|value| value.parse::<u64>().is_ok()));
    assert!(envelope["emittedAt"].as_str().is_some());
    assert_eq!(envelope["type"], "corpusReconciliationStarted");
}
