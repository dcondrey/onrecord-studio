use crate::error::AppError;
use crate::storage::fts::core_query_tokens;
use crate::storage::sqlite::DuckDbRepository;

/// Refuses a query that reads the token projection while it is incomplete. Shared by every command
/// that answers from it, so a count can never be true of a partial index when a page would not be.
pub(crate) fn refuse_while_indexing(
    repository: &DuckDbRepository,
    search_term: &str,
) -> Result<(), AppError> {
    // A corpus whose token projection is still being built would answer with fewer matches than it
    // holds, and a short result list reads as "these are the records", not as "the index is not
    // ready". Refusing is the only answer that is true while the arrears drain. Only a query that
    // reads the projection is affected: with no searchable term the filters alone decide the
    // result set, and refusing those would hide every record for the whole drain window.
    let reads_token_projection = core_query_tokens(search_term)
        .map(|tokens| !tokens.is_empty())
        .unwrap_or(false);
    let pending = repository.pending_search_index_documents();
    if reads_token_projection && pending > 0 {
        return Err(AppError::Storage(if repository.search_index_stalled() {
            format!(
                "The search index stopped before it finished: {pending} documents are not \
                 indexed. Restart the application to resume building it."
            )
        } else {
            format!(
                "The search index is still being built: {pending} documents are not yet \
                 indexed. Searching now would report fewer matches than the corpus holds."
            )
        }));
    }
    Ok(())
}
