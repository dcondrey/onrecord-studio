//! Prose figures checked against the imported budget tables.
//!
//! Sweeps public documents for quantitative claims (see `analysis::claim_validation`) and records a
//! finding only when a stated figure falls outside the rounding band its own wording carries. The
//! sweep's full outcome list, including consistent and unreconciled claims, is available through
//! [`sweep`] so a reader can see what was checked and what could not be bound.

use crate::{
    analysis::{
        claim_validation::{
            extract_claims, to_finding, ClaimContext, ClaimEvaluation, Evaluator, QuantitativeClaim,
        },
        finding::{Finding, FindingKind},
    },
    error::AppError,
    storage::DuckDbRepository,
};

use super::public_documents::load_public_documents;

pub const SLUG: &str = "claim_data_mismatch";

/// Memos and reports, not exports; the same order of magnitude as the sibling sweeps.
const MAX_DOCUMENTS: i64 = 2000;
/// Each distinct scope costs one financial investigation, so the claim count is bounded too.
const MAX_CLAIMS: usize = 300;
const MAX_FINDINGS: usize = 100;

pub async fn sweep(
    repository: &DuckDbRepository,
) -> Result<Vec<(ClaimContext, QuantitativeClaim, ClaimEvaluation)>, AppError> {
    let mut evaluator = Evaluator::load(repository).await?;
    if !evaluator.has_any_role(&["budget", "actual"]) {
        return Ok(Vec::new());
    }
    let mut outcomes = Vec::new();
    'documents: for document in load_public_documents(repository, MAX_DOCUMENTS).await? {
        for claim in extract_claims(&document.text) {
            if outcomes.len() >= MAX_CLAIMS {
                break 'documents;
            }
            let evaluation = evaluator
                .evaluate(repository, &document.portal_key, &claim)
                .await?;
            let context = ClaimContext {
                portal_key: document.portal_key.clone(),
                document_id: document.document_id,
                request_id: document.request_id,
                file_name: document.file_name.clone(),
            };
            outcomes.push((context, claim, evaluation));
        }
    }
    Ok(outcomes)
}

pub async fn run(repository: &DuckDbRepository) -> Result<Vec<Finding>, AppError> {
    let mut findings: Vec<Finding> = sweep(repository)
        .await?
        .iter()
        .filter_map(|(context, claim, evaluation)| {
            to_finding(
                context,
                claim,
                evaluation,
                FindingKind::ClaimContradictsData,
                SLUG,
            )
        })
        .collect();
    findings.sort_by(|a, b| {
        b.severity
            .rank_weight()
            .cmp(&a.severity.rank_weight())
            .then_with(|| b.confidence.rank_weight().cmp(&a.confidence.rank_weight()))
    });
    findings.truncate(MAX_FINDINGS);
    Ok(findings)
}
