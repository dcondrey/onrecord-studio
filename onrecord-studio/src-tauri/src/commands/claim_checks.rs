use crate::analysis::claim_validation::{claim_check_view, extract_claims, Evaluator};
use crate::commands::validation::portal_key as validate_portal_key;
use crate::error::AppError;
use crate::storage::sqlite::DuckDbRepository;
use crate::types::{
    ClaimCheckOutcome, ClaimCheckView, ClaimQueryPack, DocumentClaimChecks, QueryPackDataset,
    QueryPackQuery,
};
use tauri::State;

/// The repository's own chunk ceiling; a longer document is reported as truncated rather than
/// read further.
const MAX_CHARACTERS: usize = 250_000;
/// Each distinct scope costs one financial investigation, the same bound the sweep detector uses.
const MAX_CLAIMS: usize = 300;

/// Rows carried per query in a pack. The SQL is the whole answer; the rows are what it returned
/// here, cut so a pack stays a file a reader can open.
const MAX_PACK_ROWS: usize = 2_000;

struct DocumentChecks {
    checks: Vec<ClaimCheckView>,
    truncated: bool,
    evaluator: Evaluator,
}

async fn check_document(
    repository: &DuckDbRepository,
    document_id: u64,
    portal_key: &str,
) -> Result<DocumentChecks, AppError> {
    let chunk = repository
        .load_document_text_chunk(document_id, portal_key, 0, MAX_CHARACTERS)
        .await?;
    let claims = extract_claims(&chunk.masked_text);
    let truncated = claims.len() > MAX_CLAIMS || chunk.next_character_offset.is_some();
    let mut evaluator = Evaluator::load(repository).await?;
    let mut checks = Vec::with_capacity(claims.len().min(MAX_CLAIMS));
    for claim in claims.iter().take(MAX_CLAIMS) {
        let evaluation = evaluator.evaluate(repository, portal_key, claim).await?;
        checks.push(claim_check_view(claim, &evaluation));
    }
    Ok(DocumentChecks {
        checks,
        truncated,
        evaluator,
    })
}

#[tauri::command]
pub async fn list_document_claim_checks(
    document_id: u64,
    portal_key: String,
    repository: State<'_, DuckDbRepository>,
) -> Result<DocumentClaimChecks, AppError> {
    let portal_key = validate_portal_key(&portal_key)?.to_string();
    let checked = check_document(repository.inner(), document_id, &portal_key).await?;
    Ok(DocumentClaimChecks {
        document_id,
        portal_key,
        checks: checked.checks,
        truncated: checked.truncated,
    })
}

/// The checks plus the SQL and rows behind every query id they cite, and the identity (source
/// URI, hash, retrieval time) of each imported dataset, so the figures can be re-run outside the
/// app against the same exports.
#[tauri::command]
pub async fn export_claim_query_pack(
    document_id: u64,
    portal_key: String,
    repository: State<'_, DuckDbRepository>,
) -> Result<ClaimQueryPack, AppError> {
    let portal_key = validate_portal_key(&portal_key)?.to_string();
    let repository = repository.inner();
    let checked = check_document(repository, document_id, &portal_key).await?;
    let mut cited: Vec<String> = checked
        .checks
        .iter()
        .flat_map(|check| match &check.result {
            ClaimCheckOutcome::Consistent { query_ids, .. }
            | ClaimCheckOutcome::Mismatch { query_ids, .. } => query_ids.clone(),
            ClaimCheckOutcome::Unreconciled { .. } => Vec::new(),
        })
        .collect();
    cited.sort();
    cited.dedup();
    let mut queries = Vec::with_capacity(cited.len());
    for query_id in cited {
        let Some(sql) = checked.evaluator.query_sql(&query_id) else {
            return Err(AppError::Internal(format!(
                "A check cited query {query_id} that no evaluated report holds"
            )));
        };
        let result = repository
            .execute_restricted_sql(sql, MAX_PACK_ROWS)
            .await?;
        queries.push(QueryPackQuery {
            query_id,
            sql: sql.to_string(),
            columns: result.columns,
            rows: result.rows,
            row_count: result.row_count,
            truncated: result.truncated,
        });
    }
    let datasets = repository
        .analytical_schema()
        .await?
        .datasets
        .into_iter()
        .map(|dataset| QueryPackDataset {
            snapshot_id: dataset.snapshot_id,
            table_name: dataset.table_name,
            source_uri: dataset.source_uri,
            source_sha256: dataset.source_sha256,
            retrieved_at: dataset.retrieved_at,
        })
        .collect();
    Ok(ClaimQueryPack {
        document_id,
        portal_key,
        generated_at: chrono::Utc::now().to_rfc3339(),
        checks: checked.checks,
        queries,
        datasets,
        truncated: checked.truncated,
    })
}
