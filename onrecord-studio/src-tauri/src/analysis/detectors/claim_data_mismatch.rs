//! Prose figures checked against the imported budget tables.
//!
//! Sweeps public documents for quantitative claims (see `analysis::claim_validation`) and records a
//! finding only when a stated figure falls outside the rounding band its own wording carries. The
//! sweep's full outcome list, including consistent and unreconciled claims, is available through
//! [`sweep`] so a reader can see what was checked and what could not be bound.

use crate::{
    analysis::{
        claim_validation::{
            extract_claims, to_finding, ClaimContext, ClaimEvaluation, Evaluator,
            QuantitativeClaim, UnreconciledReason,
        },
        document_class::{classify, DocumentClass},
        finding::{Finding, FindingKind},
        gaps::{EvidenceGap, GapRole},
    },
    error::AppError,
    ingestion::regional::source_jurisdiction,
    storage::DuckDbRepository,
};

use super::public_documents::load_public_documents;

pub const SLUG: &str = "claim_data_mismatch";

/// Memos and reports, not exports; the same order of magnitude as the sibling sweeps.
const MAX_DOCUMENTS: i64 = 2000;
/// Each distinct scope costs one financial investigation, so the claim count is bounded too.
const MAX_CLAIMS: usize = 300;
const MAX_FINDINGS: usize = 100;
const MAX_GAPS: usize = 20;

pub async fn sweep(
    repository: &DuckDbRepository,
) -> Result<Vec<(ClaimContext, QuantitativeClaim, ClaimEvaluation)>, AppError> {
    let mut evaluator = Evaluator::load(repository).await?;
    if !evaluator.has_any_role(&["budget", "actual"]) {
        return Ok(Vec::new());
    }
    let mut outcomes = Vec::new();
    'documents: for document in load_public_documents(repository, MAX_DOCUMENTS).await? {
        let source_is_statement = classify(document.file_name.as_deref(), &document.text)
            .is_some_and(DocumentClass::is_statement);
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
                source_is_statement,
            };
            outcomes.push((context, claim, evaluation));
        }
    }
    Ok(outcomes)
}

/// Routes this detector identified and could not test: a City document states a budget or
/// actuals figure for a fiscal year, the claim came back unreconciled for want of a dataset, and no
/// imported City table holds that role for that year. One gap per (role, year), however many
/// claims cite it. Only the City's own catalog can fill these, so another jurisdiction's claim or
/// an unclassified source yields no gap.
pub async fn gaps(repository: &DuckDbRepository) -> Result<Vec<EvidenceGap>, AppError> {
    let catalog_jurisdiction = source_jurisdiction(GapRole::Appropriations.catalog_urls()[0]);
    let mut evaluator = Evaluator::load(repository).await?;
    let mut gaps: Vec<EvidenceGap> = Vec::new();
    let mut checked = 0;
    'documents: for document in load_public_documents(repository, MAX_DOCUMENTS).await? {
        if evaluator.document_jurisdiction(&document.portal_key) != catalog_jurisdiction {
            continue;
        }
        for claim in extract_claims(&document.text) {
            if checked >= MAX_CLAIMS || gaps.len() >= MAX_GAPS {
                break 'documents;
            }
            checked += 1;
            let evaluation = evaluator
                .evaluate(repository, &document.portal_key, &claim)
                .await?;
            if !matches!(
                evaluation,
                ClaimEvaluation::Unreconciled {
                    reason: UnreconciledReason::NoMappedDataset
                        | UnreconciledReason::PartialOrRejectedRows,
                    ..
                }
            ) {
                continue;
            }
            for (role, year) in evaluator
                .uncovered_years(repository, &document.portal_key, &claim)
                .await?
            {
                let key = format!("{SLUG}:{role}:FY{year}");
                if gaps.iter().any(|gap| gap.key == key) {
                    continue;
                }
                let (gap_role, records) = if role == "budget" {
                    (GapRole::Appropriations, "adopted and revised budget")
                } else {
                    (GapRole::Payments, "actual expenditure")
                };
                let file_name = document
                    .file_name
                    .clone()
                    .unwrap_or_else(|| format!("document {}", document.document_id));
                gaps.push(EvidenceGap {
                    detector: SLUG,
                    key,
                    route: format!("'{file_name}' states a FY {year} {records} figure"),
                    blocked: format!(
                        "No imported {catalog_jurisdiction} dataset holds {records} rows for FY {year}, so the stated figure cannot be checked."
                    ),
                    role: gap_role,
                    request: format!(
                        "Provide the existing {records} records for FY {year} by fund and department, as exported from the financial system."
                    ),
                });
            }
        }
    }
    Ok(gaps)
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
