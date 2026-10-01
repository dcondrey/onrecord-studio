/** Row cap sent to `query_audit_records`. A result set of exactly this size may be truncated. */
export const SEARCH_RESULT_LIMIT = 500;

/** Count label that never states a capped result set as an exact total. */
export function formatMatchCount(count: number): string {
  return count >= SEARCH_RESULT_LIMIT ? `${SEARCH_RESULT_LIMIT.toLocaleString()}+` : count.toLocaleString();
}
