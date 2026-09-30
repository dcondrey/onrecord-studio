use crate::analysis::claim_validation::{claim_check_view, extract_claims, Evaluator};
use crate::commands::validation::portal_key as validate_portal_key;
use crate::error::AppError;
use crate::storage::sqlite::DuckDbRepository;
use crate::types::DocumentClaimChecks;
use tauri::State;

/// The repository's own chunk ceiling; a longer document is reported as truncated rather than
/// read further.
const MAX_CHARACTERS: usize = 250_000;
/// Each distinct scope costs one financial investigation, the same bound the sweep detector uses.
const MAX_CLAIMS: usize = 300;

#[tauri::command]
pub async fn list_document_claim_checks(
    document_id: u64,
    portal_key: String,
    repository: State<'_, DuckDbRepository>,
) -> Result<DocumentClaimChecks, AppError> {
    let portal_key = validate_portal_key(&portal_key)?.to_string();
    let repository = repository.inner();
    let chunk = repository
        .load_document_text_chunk(document_id, &portal_key, 0, MAX_CHARACTERS)
        .await?;
    let claims = extract_claims(&chunk.masked_text);
    let truncated = claims.len() > MAX_CLAIMS || chunk.next_character_offset.is_some();
    let mut evaluator = Evaluator::load(repository).await?;
    let mut checks = Vec::with_capacity(claims.len().min(MAX_CLAIMS));
    for claim in claims.iter().take(MAX_CLAIMS) {
        let evaluation = evaluator.evaluate(repository, &portal_key, claim).await?;
        checks.push(claim_check_view(claim, &evaluation));
    }
    Ok(DocumentClaimChecks {
        document_id,
        portal_key,
        checks,
        truncated,
    })
}
