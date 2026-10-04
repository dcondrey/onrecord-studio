use super::*;
use crate::analysis::finding::{
    Finding, FindingCitation, FindingConfidence, FindingKind, FindingSeverity, FindingSubject,
};
use crate::integrity::sha256_hex;
use crate::spans::{
    CitationAnchorState, ProjectionComparison, SpanResolution, StaleReason, UnanchoredReason,
};

const FINDING_ID: &str = "finding-with-citations";
const TEXT: &str = "Opening. The fee was waived on \u{e9}\u{1f600} 3 May. Closing line.";

async fn set_text(repository: &DuckDbRepository, text: &str) {
    let connection = repository.conn.lock().await;
    connection
        .execute(
            "DELETE FROM documents_fts WHERE document_id = ?1 AND portal_subdomain = ?2",
            params![DOCUMENT_ID, PORTAL_SUBDOMAIN],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO documents_fts (document_id, portal_subdomain, file_name, extracted_text)
             VALUES (?1, ?2, 'release.pdf', ?3)",
            params![DOCUMENT_ID, PORTAL_SUBDOMAIN, text],
        )
        .unwrap();
    connection
        .execute(
            "UPDATE documents SET extracted_text_projection_sha256 = ?1
             WHERE id = ?2 AND portal_subdomain = ?3",
            params![sha256_hex(text.as_bytes()), DOCUMENT_ID, PORTAL_SUBDOMAIN],
        )
        .unwrap();
}

fn characters_of(text: &str, needle: &str) -> (i64, i64) {
    let byte = text.find(needle).expect("fixture text holds the needle");
    let start = text[..byte].chars().count() as i64;
    (start, start + needle.chars().count() as i64)
}

fn citation(range: Option<(i64, i64)>, quote: Option<&str>) -> FindingCitation {
    FindingCitation {
        portal_key: PORTAL_SUBDOMAIN.to_string(),
        document_id: DOCUMENT_ID,
        character_start: range.map(|(start, _)| start),
        character_end: range.map(|(_, end)| end),
        quote: quote.map(str::to_string),
        label: None,
    }
}

fn finding(citations: Vec<FindingCitation>) -> Finding {
    let now = chrono::Utc::now();
    Finding {
        id: FINDING_ID.to_string(),
        kind: FindingKind::HeldBackResponse,
        severity: FindingSeverity::Notable,
        confidence: FindingConfidence::measured(5000),
        subject: FindingSubject {
            portal_key: Some(PORTAL_SUBDOMAIN.to_string()),
            document_id: Some(DOCUMENT_ID),
            ..Default::default()
        },
        headline: "headline".to_string(),
        explanation: "explanation".to_string(),
        citations,
        discovered_at: now,
        last_seen_at: now,
        dismissed_at: None,
        source_detector: "held_back_response".to_string(),
    }
}

async fn resolutions(repository: &DuckDbRepository) -> Vec<CitationAnchorState> {
    repository
        .resolve_finding_citations(FINDING_ID.to_string())
        .await
        .unwrap()
        .expect("the finding is recorded")
        .into_iter()
        .map(|anchor| anchor.anchor)
        .collect()
}

fn resolution(state: &CitationAnchorState) -> SpanResolution {
    match state {
        CitationAnchorState::Anchored { resolution, .. } => *resolution,
        CitationAnchorState::Unanchored { reason } => panic!("expected an anchor, got {reason:?}"),
    }
}

async fn stored_spans(
    repository: &DuckDbRepository,
) -> Vec<(i64, i64, i64, String, Option<String>)> {
    let connection = repository.conn.lock().await;
    let mut statement = connection
        .prepare(
            "SELECT origin_ordinal, start_offset, end_offset, exact_text, text_projection_sha256
             FROM spans ORDER BY origin_ordinal",
        )
        .unwrap();
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    rows
}

/// The write path anchors a span on the projection's own characters (a truncated display quote
/// is accepted only when the range contains it), and resolution reports re-extraction without
/// rewriting the stored anchor. Offsets past multibyte characters prove DuckDB's `substr` and
/// Rust's `chars` count the same units.
#[tokio::test]
async fn recorded_citations_anchor_on_the_projection_and_resolve_through_re_extraction() {
    let repository = repository_with_document().await;
    set_text(&repository, TEXT).await;
    let waived = characters_of(TEXT, "fee was waived on \u{e9}\u{1f600} 3 May");
    let closing = characters_of(TEXT, "Closing line");
    repository
        .record_findings(vec![finding(vec![
            citation(Some(waived), Some("fee was waived")),
            citation(Some(closing), None),
            citation(Some(waived), Some("a quote this range does not hold")),
            citation(None, Some("fee was waived")),
        ])])
        .await
        .unwrap();
    let digest = sha256_hex(TEXT.as_bytes());
    assert_eq!(
        stored_spans(&repository).await,
        vec![
            (
                0,
                waived.0,
                waived.1,
                "fee was waived on \u{e9}\u{1f600} 3 May".to_string(),
                Some(digest.clone())
            ),
            (
                1,
                closing.0,
                closing.1,
                "Closing line".to_string(),
                Some(digest)
            ),
        ]
    );
    let states = resolutions(&repository).await;
    assert_eq!(
        resolution(&states[0]),
        SpanResolution::Intact {
            start: waived.0 as u64,
            end: waived.1 as u64,
            projection: ProjectionComparison::Unchanged
        }
    );
    assert_eq!(
        states[2],
        CitationAnchorState::Unanchored {
            reason: UnanchoredReason::NotAnchored
        }
    );
    assert_eq!(
        states[3],
        CitationAnchorState::Unanchored {
            reason: UnanchoredReason::NoRange
        }
    );

    // Re-extraction moves one passage and drops the other.
    let re_extracted = format!("Header added.\n{}", TEXT.replace("Closing line.", "End."));
    set_text(&repository, &re_extracted).await;
    let moved = characters_of(&re_extracted, "fee was waived on \u{e9}\u{1f600} 3 May");
    let states = resolutions(&repository).await;
    assert_eq!(
        resolution(&states[0]),
        SpanResolution::Drifted {
            start: moved.0 as u64,
            end: moved.1 as u64
        }
    );
    assert_eq!(
        resolution(&states[1]),
        SpanResolution::Stale {
            reason: StaleReason::Missing
        }
    );
    assert_eq!(
        stored_spans(&repository).await[0].1,
        waived.0,
        "resolution must not rewrite the stored anchor"
    );

    // A detector re-run computes its citation against the new text; that is a new observation.
    repository
        .record_findings(vec![finding(vec![citation(Some(moved), None)])])
        .await
        .unwrap();
    let spans = stored_spans(&repository).await;
    assert_eq!(
        spans.len(),
        1,
        "citations the re-run no longer emits lose their spans"
    );
    assert_eq!((spans[0].1, spans[0].2), moved);
}

/// A core from before spans has citations in `findings.citations_json` and no `spans` table. It
/// must open, keep every finding, and anchor exactly the citations whose range still holds the
/// recorded quote, with no projection digest claimed for them.
#[tokio::test]
async fn a_core_from_before_spans_backfills_only_citations_its_text_still_confirms() {
    let repository = repository_with_document().await;
    set_text(&repository, TEXT).await;
    let waived = characters_of(TEXT, "fee was waived");
    let closing = characters_of(TEXT, "Closing line");
    let legacy_citations = serde_json::json!([
        {"portalKey": PORTAL_SUBDOMAIN, "documentId": DOCUMENT_ID,
         "characterStart": waived.0, "characterEnd": waived.1,
         "quote": "fee was waived", "label": null},
        {"portalKey": PORTAL_SUBDOMAIN, "documentId": DOCUMENT_ID,
         "characterStart": closing.0, "characterEnd": closing.1,
         "quote": null, "label": null},
        {"portalKey": PORTAL_SUBDOMAIN, "documentId": DOCUMENT_ID,
         "characterStart": closing.0, "characterEnd": closing.1,
         "quote": "words that moved away", "label": null}
    ])
    .to_string();
    {
        let connection = repository.conn.lock().await;
        connection
            .execute_batch(
                "DROP INDEX idx_spans_document;
                 DROP TABLE spans;
                 DELETE FROM analysis_rule_versions WHERE rule_key = 'finding_citation_spans';",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO findings (id, kind, severity, confidence_basis_points,
                    subject_portal_key, subject_document_id, headline, explanation,
                    citations_json, discovered_at, last_seen_at, source_detector)
                 VALUES (?1, 'HELD_BACK_RESPONSE', 'NOTABLE', 5000, ?2, ?3, 'h', 'e', ?4,
                    '2026-09-01T00:00:00+00:00', '2026-09-01T00:00:00+00:00',
                    'held_back_response')",
                params![FINDING_ID, PORTAL_SUBDOMAIN, DOCUMENT_ID, legacy_citations],
            )
            .unwrap();
    }

    repository
        .run_migrations(MIGRATION)
        .await
        .expect("a core from before spans must open");
    repository
        .run_migrations(MIGRATION)
        .await
        .expect("the backfill must be safe to meet again");

    assert_eq!(
        stored_spans(&repository).await,
        vec![(0, waived.0, waived.1, "fee was waived".to_string(), None)]
    );
    let states = resolutions(&repository).await;
    assert_eq!(
        resolution(&states[0]),
        SpanResolution::Intact {
            start: waived.0 as u64,
            end: waived.1 as u64,
            projection: ProjectionComparison::NotCompared
        }
    );
    for unanchored in &states[1..] {
        assert_eq!(
            *unanchored,
            CitationAnchorState::Unanchored {
                reason: UnanchoredReason::NotAnchored
            }
        );
    }
    let listed = repository
        .list_findings(Default::default(), 10)
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].citations.len(), 3);
}

/// Spans are document-owned and finding-owned. Proven by deleting the `spans` arm from
/// `purge_document_rows`: the span of the finding whose subject is elsewhere then survives the
/// purge and the emptiness assertion fails.
#[tokio::test]
async fn purging_a_document_or_clearing_its_finding_removes_its_spans() {
    let repository = repository_with_document().await;
    set_text(&repository, TEXT).await;
    let waived = characters_of(TEXT, "fee was waived");
    // Its subject is not the document, so purging the document leaves this finding standing and
    // only the purge's own `spans` arm can remove the span on the document.
    let mut subject_elsewhere = finding(vec![citation(Some(waived), None)]);
    subject_elsewhere.id = "subject-elsewhere".to_string();
    subject_elsewhere.subject.document_id = None;
    let mut cleared = finding(vec![citation(Some(waived), None)]);
    cleared.last_seen_at = chrono::Utc::now() - chrono::Duration::hours(48);
    repository
        .record_findings(vec![cleared, subject_elsewhere])
        .await
        .unwrap();
    assert_eq!(stored_spans(&repository).await.len(), 2);

    repository
        .clear_stale_findings(
            "held_back_response".to_string(),
            chrono::Utc::now() - chrono::Duration::hours(24),
        )
        .await
        .unwrap();
    assert_eq!(
        stored_spans(&repository).await.len(),
        1,
        "a cleared finding's citation spans go with it"
    );

    repository
        .purge_document(PORTAL_SUBDOMAIN, DOCUMENT_ID as u64)
        .await
        .unwrap();
    assert!(stored_spans(&repository).await.is_empty());
    let connection = repository.conn.lock().await;
    verify_logical_relations(&connection).expect("a purged document leaves no span behind");
}

#[tokio::test]
async fn purging_a_source_removes_its_spans() {
    let repository = repository_with_document().await;
    set_text(&repository, TEXT).await;
    let waived = characters_of(TEXT, "fee was waived");
    repository
        .record_findings(vec![finding(vec![citation(Some(waived), None)])])
        .await
        .unwrap();
    repository.purge_source(PORTAL_SUBDOMAIN).await.unwrap();
    assert!(stored_spans(&repository).await.is_empty());
    let connection = repository.conn.lock().await;
    verify_logical_relations(&connection).expect("a purged source leaves no span behind");
}
