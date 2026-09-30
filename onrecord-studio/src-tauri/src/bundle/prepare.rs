//! Loads the stored state a bundle is assembled from: the selected findings, the standing of the
//! documents they cite, and which portals have been removed.

use super::{
    assemble, document_keys, portal_keys, Assembly, BundleExclusion, ExclusionReason,
    MAXIMUM_SELECTED_FINDINGS,
};
use crate::error::AppError;
use crate::profile::DisclosureContext;
use crate::report::source::load_source_documents;
use crate::storage::sqlite::DuckDbRepository;
use std::collections::BTreeSet;

fn validate_ids(ids: Vec<String>) -> Result<Vec<String>, AppError> {
    if ids.is_empty() {
        return Err(AppError::DataParsing(
            "Select at least one finding to share".to_string(),
        ));
    }
    if ids.len() > MAXIMUM_SELECTED_FINDINGS {
        return Err(AppError::DataParsing(format!(
            "At most {MAXIMUM_SELECTED_FINDINGS} findings can be shared in one bundle"
        )));
    }
    let mut unique = BTreeSet::new();
    for id in ids {
        let id = id.trim();
        if id.is_empty()
            || id.len() > 64
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(AppError::DataParsing("Finding id is malformed".to_string()));
        }
        unique.insert(id.to_string());
    }
    Ok(unique.into_iter().collect())
}

/// Reads the selection from storage and decides, finding by finding, what may be shared.
pub async fn prepare(
    repository: &DuckDbRepository,
    disclosure: &DisclosureContext,
    finding_ids: Vec<String>,
) -> Result<Assembly, AppError> {
    let ids = validate_ids(finding_ids)?;
    let mut findings = Vec::with_capacity(ids.len());
    let mut missing = Vec::new();
    for id in ids {
        match repository.get_finding_by_id(id.clone()).await? {
            Some(finding) => findings.push(finding),
            None => missing.push(BundleExclusion {
                finding_id: id,
                headline: None,
                reason: ExclusionReason::NotFound,
                explanation: ExclusionReason::NotFound.explanation().to_string(),
            }),
        }
    }
    let documents = load_source_documents(repository, document_keys(&findings)).await?;
    let removed = repository
        .unavailable_portal_keys(portal_keys(&findings))
        .await?;
    let mut assembly = assemble(findings, &documents, &removed, disclosure)?;
    assembly.exclusions.extend(missing);
    Ok(assembly)
}
