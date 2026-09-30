//! Agencies whose heavy-redaction documents show zero structural residual evidence.
//!
//! `unredactor.rs` finds positional leaks: filled rectangles overlapping selectable text,
//! invisible-rendering-mode text runs, unapplied `Redact` annotations, Form-XObject overlays. An
//! agency whose documents show heavy redaction density AND consistently zero of those structural
//! signals is either (a) fastidious about applying redactions correctly, or (b) engineering
//! redactions to leak precisely to the limit of what the detector sees. The second reading
//! becomes suggestive when the pattern holds across ≥ 10 heavy-redaction documents from one
//! agency AND its per-document integrity flags show no residual/leak/invisible entries at all.
//!
//! **Not a legal claim** (`is_legal_claim = false`). Adversarial signal at ≥ 6500 bp only. The
//! finding says the pattern; the interpretation is the reader's.
//!
//! **Signals we use** (all already recorded by the corpus/unredactor pipeline):
//! - `documents.suspicious_data_density = 1` — heavy redaction density.
//! - `document_integrity_flags.flag_type` — the audit trail from `unredactor.rs`. If any row for
//!   the document names a leak/residual/invisible/annotation signal, we count it as "detected".

use crate::{
    analysis::finding::{
        finding_id, Finding, FindingCitation, FindingConfidence, FindingKind, FindingSeverity,
        FindingSubject,
    },
    error::AppError,
    storage::DuckDbRepository,
};
use chrono::Utc;
use duckdb::params;
use std::collections::HashMap;

pub const SLUG: &str = "evasion_redaction_at_limit";

const MAX_DOCUMENTS_INSPECTED: i64 = 10_000;
/// Minimum heavy-redaction documents at one agency to fire.
const MIN_HEAVY_DOCUMENTS: usize = 10;
/// Maximum residual-signals ratio to fire. Zero is the honest signal but tolerate up to this
/// fraction as noise from documents with a genuine minor structural blip.
const MAX_RESIDUAL_RATIO: f64 = 0.05;
const MAX_FINDINGS: usize = 50;
const MIN_CONFIDENCE_TO_FIRE: u16 = 6500;

pub async fn run(repository: &DuckDbRepository) -> Result<Vec<Finding>, AppError> {
    let observations = load_observations(repository).await?;
    let mut findings = evaluate(&observations);
    findings.sort_by_key(|f| std::cmp::Reverse(f.confidence.rank_weight()));
    findings.truncate(MAX_FINDINGS);
    Ok(findings)
}

#[derive(Debug, Clone)]
struct DocumentObservation {
    portal_key: String,
    document_id: i64,
    agency_id: i64,
    file_name: Option<String>,
    heavy: bool,
    residual_signals: usize,
}

async fn load_observations(
    repository: &DuckDbRepository,
) -> Result<Vec<DocumentObservation>, AppError> {
    repository
        .query_connection("Redaction-at-limit observation query", move |conn| {
            let mut statement = conn
                .prepare(
                    "SELECT d.portal_subdomain, d.id, r.agency_id, d.file_name,
                            d.suspicious_data_density,
                            (SELECT COUNT(*)
                               FROM case_audits ca
                               JOIN document_integrity_flags dif
                                 ON dif.audit_id = ca.audit_id
                              WHERE ca.target_request_id = d.request_id
                                AND ca.target_subdomain = d.portal_subdomain
                                AND (LOWER(dif.flag_type) LIKE '%leak%'
                                     OR LOWER(dif.flag_type) LIKE '%residual%'
                                     OR LOWER(dif.flag_type) LIKE '%invisible%'
                                     OR LOWER(dif.flag_type) LIKE '%annotation%'
                                     OR LOWER(dif.flag_type) LIKE '%unapplied%'
                                     OR LOWER(dif.flag_type) LIKE '%overlay%')) AS residual_signals
                     FROM documents d
                     JOIN requests r
                        ON r.id = d.request_id
                       AND r.portal_subdomain = d.portal_subdomain
                     WHERE r.agency_id IS NOT NULL
                       AND LOWER(TRIM(d.visibility)) = 'public'
                     LIMIT ?1",
                )
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to prepare redaction-at-limit query: {error}"
                    ))
                })?;
            let rows = statement
                .query_map(params![MAX_DOCUMENTS_INSPECTED], |row| {
                    let density: i64 = row.get(4)?;
                    let residual: i64 = row.get(5)?;
                    Ok(DocumentObservation {
                        portal_key: row.get(0)?,
                        document_id: row.get(1)?,
                        agency_id: row.get(2)?,
                        file_name: row.get::<_, Option<String>>(3)?,
                        heavy: density != 0,
                        residual_signals: residual.max(0) as usize,
                    })
                })
                .map_err(|error| {
                    AppError::Storage(format!("Failed to run redaction-at-limit query: {error}"))
                })?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row.map_err(|error| {
                    AppError::Storage(format!("Failed to decode redaction-at-limit row: {error}"))
                })?);
            }
            Ok(out)
        })
        .await
}

fn evaluate(observations: &[DocumentObservation]) -> Vec<Finding> {
    let mut per_agency: HashMap<(String, i64), Vec<&DocumentObservation>> = HashMap::new();
    for observation in observations {
        per_agency
            .entry((observation.portal_key.clone(), observation.agency_id))
            .or_default()
            .push(observation);
    }
    let mut findings = Vec::new();
    for ((portal_key, agency_id), group) in per_agency {
        let heavy: Vec<&&DocumentObservation> = group.iter().filter(|o| o.heavy).collect();
        if heavy.len() < MIN_HEAVY_DOCUMENTS {
            continue;
        }
        let heavy_with_signal = heavy.iter().filter(|o| o.residual_signals > 0).count();
        let ratio = heavy_with_signal as f64 / heavy.len() as f64;
        if ratio > MAX_RESIDUAL_RATIO {
            continue;
        }
        // Confidence scales with heavy-count (bigger sample = stronger signal) and inverse ratio.
        let size_bp =
            ((heavy.len() as f64 - MIN_HEAVY_DOCUMENTS as f64) * 200.0).clamp(0.0, 2400.0) as u16;
        let clean_bp = ((MAX_RESIDUAL_RATIO - ratio) * 30000.0).clamp(0.0, 1500.0) as u16;
        let raw_bp = 5000_u16.saturating_add(size_bp).saturating_add(clean_bp);
        let bp = raw_bp.min(10_000);
        if bp < MIN_CONFIDENCE_TO_FIRE {
            continue;
        }
        let subject = FindingSubject {
            portal_key: Some(portal_key.clone()),
            agency_id: Some(agency_id),
            ..Default::default()
        };
        let salient = format!(
            "agency={},heavy={},with_signal={}",
            agency_id,
            heavy.len(),
            heavy_with_signal
        );
        let id = finding_id(
            FindingKind::EvasionRedactionAtLimit,
            SLUG,
            &subject,
            &[&salient],
        );
        let headline = format!(
            "Agency {}: {} heavy-redaction documents, {} structural residual signals",
            agency_id,
            heavy.len(),
            heavy_with_signal
        );
        let explanation = format!(
            "{n} public documents from agency {agency} carry heavy redaction density, of which \
             {with_signal} ({percent:.1}%) also carry any structural residual signal (leak, \
             invisible-mode text, unapplied Redact annotation, or overlay). A large heavy-redaction \
             population with a near-zero structural-signal rate is either careful practice or a \
             redaction workflow tuned to what the residual-evidence audit detects. Verify by \
             hand before drawing the second inference.",
            n = heavy.len(),
            agency = agency_id,
            with_signal = heavy_with_signal,
            percent = ratio * 100.0,
        );
        let citations: Vec<FindingCitation> = heavy
            .iter()
            .take(5)
            .map(|o| FindingCitation {
                portal_key: o.portal_key.clone(),
                document_id: o.document_id,
                character_start: None,
                character_end: None,
                quote: None,
                label: o
                    .file_name
                    .clone()
                    .or_else(|| Some(format!("document {}", o.document_id))),
            })
            .collect();
        let now = Utc::now();
        findings.push(Finding {
            id,
            kind: FindingKind::EvasionRedactionAtLimit,
            severity: FindingSeverity::Serious,
            confidence: FindingConfidence::measured(bp),
            subject,
            headline,
            explanation,
            citations,
            discovered_at: now,
            last_seen_at: now,
            dismissed_at: None,
            source_detector: SLUG.into(),
        });
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obs(id: i64, agency: i64, heavy: bool, residual: usize) -> DocumentObservation {
        DocumentObservation {
            portal_key: "test".into(),
            document_id: id,
            agency_id: agency,
            file_name: Some(format!("doc-{id}.pdf")),
            heavy,
            residual_signals: residual,
        }
    }

    #[test]
    fn fires_when_many_heavy_docs_and_no_structural_signals() {
        let observations: Vec<DocumentObservation> = (0..12).map(|i| obs(i, 7, true, 0)).collect();
        let findings = evaluate(&observations);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].headline.contains("Agency 7"));
    }

    #[test]
    fn suppressed_when_signals_present_above_ratio() {
        // 10 heavy docs, 3 carry residual signals (30% > 5% threshold): suppress.
        let observations: Vec<DocumentObservation> = (0..7)
            .map(|i| obs(i, 7, true, 0))
            .chain((7..10).map(|i| obs(i, 7, true, 1)))
            .collect();
        assert!(evaluate(&observations).is_empty());
    }

    #[test]
    fn suppressed_when_heavy_count_below_minimum() {
        // Only 9 heavy docs; MIN_HEAVY_DOCUMENTS is 10.
        let observations: Vec<DocumentObservation> = (0..9).map(|i| obs(i, 7, true, 0)).collect();
        assert!(evaluate(&observations).is_empty());
    }

    #[test]
    fn non_heavy_docs_do_not_contribute_to_the_ratio() {
        // 10 heavy with clean signals + 100 non-heavy: fires (non-heavy are irrelevant).
        let observations: Vec<DocumentObservation> = (0..10)
            .map(|i| obs(i, 7, true, 0))
            .chain((10..110).map(|i| obs(i, 7, false, 5)))
            .collect();
        let findings = evaluate(&observations);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn id_stable_for_same_agency_and_bucketed_counts() {
        let observations: Vec<DocumentObservation> = (0..12).map(|i| obs(i, 7, true, 0)).collect();
        let a = evaluate(&observations);
        let b = evaluate(&observations);
        assert_eq!(a[0].id, b[0].id);
    }
}
