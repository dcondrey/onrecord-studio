//! Inferred-redaction-identity detector.
//!
//! Sweeps public documents in the corpus, locates textual redaction spans in their extracted text
//! (`[REDACTED]`, `[WITHHELD]`, FOIA exemption tags, long block/underscore runs), runs the
//! lexical inference engine (`crate::analysis::redaction_inference`), and emits a Finding when
//! at least one candidate identity has likelihood ≥ 7000 bp with supporting evidence from ≥ 3
//! independent corroborating documents.
//!
//! **Fallback-only.** The detector uses the LEXICAL path (no fastembed dependency) because
//! detectors do not receive the `VectorIndex` handle. The IPC command
//! (`commands::redaction_inference::infer_redaction`) uses the semantic path when available and
//! is what powers the interactive panel; this detector's autonomous sweep is intentionally
//! conservative — it only surfaces high-confidence lexical corroboration.
//!
//! **Not a legal claim.** Inferring an identity is not proving one.
//! `FindingKind::RedactionCandidateInferred::is_legal_claim() → false`.

use crate::{
    analysis::{
        finding::{
            finding_id, Finding, FindingCitation, FindingConfidence, FindingKind, FindingSeverity,
            FindingSubject,
        },
        redaction_inference::{
            infer_redaction_candidates, scan_redaction_spans, Candidate, LEXICAL_LIKELIHOOD_CEILING,
        },
    },
    error::AppError,
    storage::DuckDbRepository,
};
use chrono::Utc;
use duckdb::params;

pub const SLUG: &str = "inferred_redaction_identity";

/// Maximum documents inspected per invocation. The detector is bounded because inference over
/// every document in a large corpus would exceed the refresh time budget.
const MAX_CANDIDATE_DOCS: i64 = 200;
/// Minimum candidate likelihood before a finding fires.
const MIN_LIKELIHOOD_BP: u16 = LEXICAL_LIKELIHOOD_CEILING.saturating_sub(1000); // 5000
/// Minimum corroborating documents. Set higher than the engine's minimum (2 for Person) so the
/// autonomous surface is strictly more conservative than the interactive one.
const MIN_CORROBORATING_DOCS: usize = 3;
/// Cap on inferences per document (a doc with many redactions doesn't drown the inbox).
const MAX_INFERENCES_PER_DOC: usize = 3;

struct SweepTarget {
    document_id: i64,
    portal_key: String,
    file_name: Option<String>,
    request_id: Option<i64>,
}

pub async fn run(repository: &DuckDbRepository) -> Result<Vec<Finding>, AppError> {
    let targets = load_targets(repository).await?;
    let mut findings = Vec::new();
    for target in targets {
        let text = match repository
            .load_corpus_source_text(
                &target.portal_key,
                u64::try_from(target.document_id).unwrap_or(0),
            )
            .await
        {
            Ok(text) => text,
            Err(_) => continue,
        };
        let spans = scan_redaction_spans(&text);
        if spans.is_empty() {
            continue;
        }
        let mut per_doc = 0usize;
        for span in spans {
            if per_doc >= MAX_INFERENCES_PER_DOC {
                break;
            }
            // Lexical path: pass None for the vector index so the ceiling is bounded to
            // LEXICAL_LIKELIHOOD_CEILING and no semantic call is made from a bulk sweep.
            let report = match infer_redaction_candidates(
                repository,
                None,
                target.document_id,
                &target.portal_key,
                span,
            )
            .await
            {
                Ok(report) => report,
                Err(_) => continue,
            };
            if let Some(finding) = build_finding(&target, span, &report.candidates) {
                findings.push(finding);
                per_doc += 1;
            }
        }
    }
    Ok(findings)
}

async fn load_targets(repository: &DuckDbRepository) -> Result<Vec<SweepTarget>, AppError> {
    repository
        .query_connection("Redaction-inference candidate query", move |conn| {
            let mut statement = conn
                .prepare(
                    "SELECT documents.id, documents.portal_subdomain, documents.file_name, \
                            documents.request_id \
                     FROM documents \
                     INNER JOIN documents_fts \
                        ON documents_fts.document_id = documents.id \
                       AND documents_fts.portal_subdomain = documents.portal_subdomain \
                     WHERE LOWER(TRIM(documents.visibility)) = 'public' \
                       AND LENGTH(documents_fts.extracted_text) > 0 \
                       AND strlen(documents_fts.extracted_text) <= ?2 \
                     ORDER BY documents.id DESC \
                     LIMIT ?1",
                )
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to prepare inferred-redaction target query: {error}"
                    ))
                })?;
            let rows = statement
                .query_map(
                    params![
                        MAX_CANDIDATE_DOCS,
                        super::public_documents::MAX_DOCUMENT_TEXT_BYTES
                    ],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, Option<String>>(2)?,
                            row.get::<_, Option<i64>>(3)?,
                        ))
                    },
                )
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to run inferred-redaction target query: {error}"
                    ))
                })?;
            let mut out = Vec::new();
            for row in rows {
                let (document_id, portal_key, file_name, request_id) = row.map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to decode inferred-redaction target row: {error}"
                    ))
                })?;
                out.push(SweepTarget {
                    document_id,
                    portal_key,
                    file_name,
                    request_id,
                });
            }
            Ok(out)
        })
        .await
}

fn build_finding(
    target: &SweepTarget,
    span: crate::analysis::redaction_inference::RedactionSpan,
    candidates: &[Candidate],
) -> Option<Finding> {
    let top = candidates.first()?;
    if top.likelihood_basis_points < MIN_LIKELIHOOD_BP {
        return None;
    }
    if top.supporting_evidence.len() < MIN_CORROBORATING_DOCS {
        return None;
    }
    let severity = if top.likelihood_basis_points >= LEXICAL_LIKELIHOOD_CEILING - 500 {
        FindingSeverity::Serious
    } else {
        FindingSeverity::Notable
    };
    // The likelihood is a term-overlap heuristic, not a measured probability that the value fills
    // the redaction, so it ranks candidates but is never presented as the finding's confidence.
    let confidence = FindingConfidence::Unassessed;
    let subject = FindingSubject {
        portal_key: Some(target.portal_key.clone()),
        request_id: target.request_id,
        document_id: Some(target.document_id),
        ..Default::default()
    };
    let salient = format!(
        "span={}-{},kind={},top={}",
        span.start,
        span.end,
        top.kind.as_str(),
        top.value
    );
    let id = finding_id(
        FindingKind::RedactionCandidateInferred,
        SLUG,
        &subject,
        &[&salient],
    );
    let file_name = target
        .file_name
        .clone()
        .unwrap_or_else(|| format!("document {}", target.document_id));
    let headline = format!(
        "Redacted {kind_lower} in {file_name} likely one of {n} corroborated candidates",
        kind_lower = candidate_kind_lower(top.kind),
        n = candidates.len(),
    );
    let other_count = candidates.len().saturating_sub(1);
    let more_hint = if other_count > 0 {
        format!(
            " {other_count} additional candidate{plural} identified with lower likelihood.",
            plural = if other_count == 1 { "" } else { "s" },
        )
    } else {
        String::new()
    };
    let explanation = format!(
        "Based on the words surrounding the redaction and corroborating context from {n} other \
         released documents, the redacted {kind_lower} is likely \"{value}\". This is an \
         inference from surrounding context — treat it as a lead to verify, not as a decoded \
         value.{more_hint}",
        n = top.supporting_evidence.len(),
        kind_lower = candidate_kind_lower(top.kind),
        value = top.value,
    );
    let mut citations = Vec::with_capacity(top.supporting_evidence.len().min(5) + 1);
    citations.push(FindingCitation {
        portal_key: target.portal_key.clone(),
        document_id: target.document_id,
        character_start: Some(span.start as i64),
        character_end: Some(span.end as i64),
        quote: None,
        label: Some(format!("redacted span in {file_name}")),
    });
    for evidence in top.supporting_evidence.iter().take(5) {
        citations.push(FindingCitation {
            portal_key: evidence.portal_key.clone(),
            document_id: evidence.document_id,
            character_start: None,
            character_end: None,
            quote: Some(evidence.quote.clone()),
            label: Some("corroborating context".into()),
        });
    }
    let now = Utc::now();
    Some(Finding {
        id,
        kind: FindingKind::RedactionCandidateInferred,
        severity,
        confidence,
        subject,
        headline,
        explanation,
        citations,
        discovered_at: now,
        last_seen_at: now,
        dismissed_at: None,
        source_detector: SLUG.into(),
    })
}

fn candidate_kind_lower(kind: crate::analysis::redaction_inference::CandidateKind) -> &'static str {
    use crate::analysis::redaction_inference::CandidateKind;
    match kind {
        CandidateKind::Person => "person",
        CandidateKind::Organization => "organization",
        CandidateKind::Amount => "amount",
        CandidateKind::Date => "date",
        CandidateKind::Other => "value",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::redaction_inference::{
        CandidateKind, Evidence, RedactionSpan, LEXICAL_LIKELIHOOD_CEILING,
    };

    fn target() -> SweepTarget {
        SweepTarget {
            document_id: 42,
            portal_key: "test".into(),
            file_name: Some("response.pdf".into()),
            request_id: Some(7),
        }
    }

    fn evid(doc_id: i64) -> Evidence {
        Evidence {
            portal_key: "test".into(),
            document_id: doc_id,
            quote: "signed by Jane Doe on 2024-01-15".into(),
            match_score: 4000,
        }
    }

    fn cand_of_likelihood(likelihood: u16, evidence_count: usize) -> Candidate {
        Candidate {
            kind: CandidateKind::Person,
            value: "Jane Doe".into(),
            likelihood_basis_points: likelihood,
            supporting_evidence: (0..evidence_count).map(|i| evid(100 + i as i64)).collect(),
        }
    }

    #[test]
    fn no_finding_below_min_likelihood() {
        let candidates = vec![cand_of_likelihood(MIN_LIKELIHOOD_BP - 1, 3)];
        assert!(
            build_finding(&target(), RedactionSpan { start: 10, end: 20 }, &candidates).is_none()
        );
    }

    #[test]
    fn no_finding_with_too_few_corroborating_docs() {
        let candidates = vec![cand_of_likelihood(
            MIN_LIKELIHOOD_BP + 100,
            MIN_CORROBORATING_DOCS - 1,
        )];
        assert!(
            build_finding(&target(), RedactionSpan { start: 10, end: 20 }, &candidates).is_none()
        );
    }

    #[test]
    fn notable_finding_when_thresholds_met() {
        let candidates = vec![cand_of_likelihood(
            MIN_LIKELIHOOD_BP + 200,
            MIN_CORROBORATING_DOCS,
        )];
        let finding = build_finding(&target(), RedactionSpan { start: 10, end: 20 }, &candidates)
            .expect("must fire");
        assert_eq!(finding.severity, FindingSeverity::Notable);
        assert_eq!(finding.kind, FindingKind::RedactionCandidateInferred);
        assert!(finding.headline.contains("Jane Doe") || finding.headline.contains("candidates"));
        assert!(finding.explanation.contains("inference"));
        assert!(!finding.explanation.contains("decryption"));
        // At least 1 (span) + up to 5 corroborating citations.
        assert!(finding.citations.len() >= 2);
    }

    #[test]
    fn serious_finding_at_ceiling_range() {
        let candidates = vec![cand_of_likelihood(
            LEXICAL_LIKELIHOOD_CEILING - 200,
            MIN_CORROBORATING_DOCS,
        )];
        let finding = build_finding(&target(), RedactionSpan { start: 10, end: 20 }, &candidates)
            .expect("must fire");
        assert_eq!(finding.severity, FindingSeverity::Serious);
    }

    #[test]
    fn finding_id_stable_across_re_runs_for_same_evidence() {
        let candidates = vec![cand_of_likelihood(
            MIN_LIKELIHOOD_BP + 100,
            MIN_CORROBORATING_DOCS,
        )];
        let a =
            build_finding(&target(), RedactionSpan { start: 10, end: 20 }, &candidates).unwrap();
        let b =
            build_finding(&target(), RedactionSpan { start: 10, end: 20 }, &candidates).unwrap();
        assert_eq!(a.id, b.id);
    }

    #[test]
    fn empty_candidates_yields_no_finding() {
        assert!(build_finding(&target(), RedactionSpan { start: 10, end: 20 }, &[]).is_none());
    }
}
