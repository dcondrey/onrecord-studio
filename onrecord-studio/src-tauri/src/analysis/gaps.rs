//! Evidence gaps: an investigation route the corpus identified but cannot yet test.
//!
//! A detector that finds the start of a route (a policy stating a commitment, a budget with
//! spending records) and lacks the record that would confirm or refute it returns an
//! [`EvidenceGap`] instead of silently returning no finding. "Could not check" and "checked,
//! nothing found" are different statements about evidence and must not share a representation.
//!
//! Gaps drive acquisition without a model provider: each [`GapRole`] maps to public catalog
//! sources, which are queued in `research_sources`. The existing collection service picks them up.
//! A gap that outlives every mapped source is `Exhausted` and carries a records-request sentence;
//! it never becomes a finding and is never re-queued without bound (`research_source_due` holds
//! each URL to its backoff).

use crate::{
    analysis::financial::ROLE_GAPS, error::AppError, research::SOURCES, storage::DuckDbRepository,
    types::*,
};
use serde::Serialize;
use std::collections::HashSet;

const OPERATING_BUDGET: &str = "https://data.sandiego.gov/datasets/operating-budget/";
const OPERATING_ACTUALS: &str = "https://data.sandiego.gov/datasets/operating-actuals/";
const IBA_REPORTS: &str = "https://www.sandiego.gov/iba/reports";
const CONTRACTS: &str = "https://www.sandiego.gov/purchasing/bids-contracts";
const CONTRACT_REGISTER: &str = "https://www.sandiego.gov/purchasing/bids-contracts/contracts2027";
const HOMELESSNESS_DATA: &str =
    "https://www.sandiego.gov/homelessness-strategies-and-solutions/data-reports";
const RTFH_REPORTS: &str = "https://rtfhsd.org/reports-data";

/// The class of record that would resolve a gap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum GapRole {
    Appropriations,
    Encumbrances,
    Contracts,
    Amendments,
    Payments,
    Outcomes,
    FundRestrictions,
    /// Per-request determination dates from the issuing agency. No public catalog carries them.
    DeterminationDates,
    /// Records the agency released to other requesters. No public catalog carries them.
    AgencyRecords,
}

impl GapRole {
    /// Public catalog URLs that can hold this role. Each must appear in [`SOURCES`].
    pub fn catalog_urls(self) -> &'static [&'static str] {
        match self {
            Self::Appropriations => &[OPERATING_BUDGET, IBA_REPORTS],
            Self::Contracts | Self::Amendments => &[CONTRACT_REGISTER, CONTRACTS],
            Self::Payments => &[OPERATING_ACTUALS],
            Self::Outcomes => &[HOMELESSNESS_DATA, RTFH_REPORTS],
            Self::Encumbrances
            | Self::FundRestrictions
            | Self::DeterminationDates
            | Self::AgencyRecords => &[],
        }
    }

    pub fn from_financial_role(role: &str) -> Option<Self> {
        Some(match role {
            "budget" => Self::Appropriations,
            "encumbrance" => Self::Encumbrances,
            "contract" => Self::Contracts,
            "amendment" => Self::Amendments,
            "payment" => Self::Payments,
            "outcome" => Self::Outcomes,
            "restriction" => Self::FundRestrictions,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceGap {
    pub detector: &'static str,
    /// Stable across refreshes for the same missing record.
    pub key: String,
    /// The route the corpus identified, in the reader's terms.
    pub route: String,
    /// What cannot be tested without the record.
    pub blocked: String,
    pub role: GapRole,
    /// A records-request sentence naming the custodian's existing records.
    pub request: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum GapStatus {
    /// At least one mapped public source is queued or was collected too recently to judge.
    Acquiring,
    /// Every mapped source was collected and the record is still absent.
    Exhausted,
    /// No public catalog holds this record class; only a records request can obtain it.
    NoAutomaticSource,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceGapView {
    pub detector: String,
    pub key: String,
    pub route: String,
    pub blocked: String,
    pub role: GapRole,
    pub request: String,
    pub status: GapStatus,
    pub sources: Vec<String>,
}

/// Each resolve queues at most this many sources; the rest wait for the next refresh.
const MAX_QUEUED_PER_RESOLVE: usize = 8;

fn label_for(url: &str) -> Option<&'static str> {
    SOURCES.iter().find(|(_, u)| *u == url).map(|(l, _)| *l)
}

fn same_url(a: &str, b: &str) -> bool {
    a.trim_end_matches('/') == b.trim_end_matches('/')
}

/// Gathers every detector's gaps. A detector that fails contributes none and its error is
/// returned alongside, so one failure neither hides another route's insufficiency nor vanishes.
pub async fn collect(repository: &DuckDbRepository) -> (Vec<EvidenceGap>, Vec<String>) {
    let mut gaps = Vec::new();
    let mut errors = Vec::new();
    match crate::analysis::detectors::policy_violation::gaps(repository).await {
        Ok(found) => gaps.extend(found),
        Err(error) => errors.push(format!("policy_violation gap sweep failed: {error}")),
    }
    match crate::analysis::detectors::practice_departure::gaps(repository).await {
        Ok(found) => gaps.extend(found),
        Err(error) => errors.push(format!("practice_departure gap sweep failed: {error}")),
    }
    match crate::analysis::detectors::no_records_contradicted::gaps(repository).await {
        Ok(found) => gaps.extend(found),
        Err(error) => errors.push(format!("no_records_contradicted gap sweep failed: {error}")),
    }
    match crate::analysis::detectors::spending_cases::gaps(repository).await {
        Ok(found) => gaps.extend(found),
        Err(error) => errors.push(format!("spending_cases gap sweep failed: {error}")),
    }
    match financial_gaps(repository).await {
        Ok(found) => gaps.extend(found),
        Err(error) => errors.push(format!("financial gap sweep failed: {error}")),
    }
    match crate::analysis::detectors::claim_data_mismatch::gaps(repository).await {
        Ok(found) => gaps.extend(found),
        Err(error) => errors.push(format!("claim_data_mismatch gap sweep failed: {error}")),
    }
    let mut seen = HashSet::new();
    gaps.retain(|gap| seen.insert(gap.key.clone()));
    (gaps, errors)
}

/// A financial route exists only when saved mappings produced reconciled rows or findings; a
/// corpus with no financial data has no route to be insufficient.
async fn financial_gaps(repository: &DuckDbRepository) -> Result<Vec<EvidenceGap>, AppError> {
    let workspace = repository.financial_workspace().await?;
    if workspace.mappings.is_empty() {
        return Ok(Vec::new());
    }
    let report = repository
        .run_financial_investigation(workspace.default_scope)
        .await?;
    if report.reconciliation.is_empty() && report.findings.is_empty() {
        return Ok(Vec::new());
    }
    let scope = format!(
        "FY{} {}",
        report
            .scope
            .fiscal_year
            .map_or_else(|| "all years".to_string(), |year| year.to_string()),
        report
            .scope
            .department
            .as_deref()
            .unwrap_or("all departments"),
    );
    let mut gaps = Vec::new();
    for gap in &report.gaps {
        if !gap.citations.is_empty() {
            continue;
        }
        let Some(role) = ROLE_GAPS
            .iter()
            .find(|(_, title, _, _)| *title == gap.title)
            .and_then(|(role, ..)| GapRole::from_financial_role(role))
        else {
            continue;
        };
        gaps.push(EvidenceGap {
            detector: "financial",
            key: format!("financial:{}:{scope}", gap.title),
            route: format!("Spending in {scope} was reconciled from the retrieved ledgers"),
            blocked: gap.why.clone(),
            role,
            request: format!("Provide existing records: {}", gap.records_needed),
        });
    }
    Ok(gaps)
}

/// Queues each gap's catalog sources and reports where every gap stands. Queueing honors the
/// same dedup as every other acquisition: a saved source or a URL inside its backoff is skipped.
pub async fn resolve(
    repository: &DuckDbRepository,
    gaps: &[EvidenceGap],
    queue: bool,
) -> Result<Vec<EvidenceGapView>, AppError> {
    let saved = repository.list_portal_registrations().await?;
    let states = repository.research_source_states().await?;
    let is_saved = |url: &str| {
        saved
            .iter()
            .any(|s| same_url(&s.registration.canonical_url, url))
    };
    let mut views = Vec::new();
    let mut queued: HashSet<&str> = HashSet::new();
    for gap in gaps {
        let urls = gap.role.catalog_urls();
        let mut acquiring = false;
        for url in urls {
            if is_saved(url) {
                continue;
            }
            match states.get(*url) {
                Some((state, elapsed)) if state == "REGISTERED" => {
                    acquiring |= !elapsed;
                }
                // Already waiting for `collect_one`; rewriting it would only reset its place.
                Some((state, _)) if state == "QUEUED" || state == "STARTING" => {
                    acquiring = true;
                }
                _ => {
                    acquiring = true;
                    if queue
                        && !queued.contains(url)
                        && queued.len() < MAX_QUEUED_PER_RESOLVE
                        && repository.research_source_due(url).await?
                    {
                        queued.insert(url);
                        repository
                            .record_research_source(
                                ResearchSourceStatus {
                                    url: (*url).to_string(),
                                    label: label_for(url).unwrap_or(url).to_string(),
                                    state: "QUEUED".into(),
                                    detail: format!(
                                        "Evidence gap from {}: {}",
                                        gap.detector, gap.route
                                    ),
                                },
                                0,
                            )
                            .await?;
                    }
                }
            }
        }
        let status = if urls.is_empty() {
            GapStatus::NoAutomaticSource
        } else if acquiring {
            GapStatus::Acquiring
        } else {
            GapStatus::Exhausted
        };
        views.push(EvidenceGapView {
            detector: gap.detector.to_string(),
            key: gap.key.clone(),
            route: gap.route.clone(),
            blocked: gap.blocked.clone(),
            role: gap.role,
            request: gap.request.clone(),
            status,
            sources: urls.iter().map(|u| (*u).to_string()).collect(),
        });
    }
    Ok(views)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_catalog_url_is_a_collectable_source() {
        for role in [
            GapRole::Appropriations,
            GapRole::Contracts,
            GapRole::Amendments,
            GapRole::Payments,
            GapRole::Outcomes,
        ] {
            for url in role.catalog_urls() {
                assert!(label_for(url).is_some(), "{url} is not in SOURCES");
            }
        }
    }
}
