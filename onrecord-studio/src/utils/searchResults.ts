/** Row cap sent to `query_audit_records`. A result set of exactly this size may be truncated. */
export const SEARCH_RESULT_LIMIT = 500;

/** Count label that never states a capped result set as an exact total. */
export function formatMatchCount(count: number): string {
  return count >= SEARCH_RESULT_LIMIT ? `${SEARCH_RESULT_LIMIT.toLocaleString()}+` : count.toLocaleString();
}

export type MatchBasis = 'title' | 'description' | 'unshown';

/**
 * Where a free-text term is visible in a result. The list shows a truncated description, so a
 * term absent from the title and preview is `unshown`: it may match in the full description or in
 * a document's extracted text. It is never reported as absent.
 */
export function matchBasis(
  result: { title: string; descriptionPreview: string },
  term: string,
): MatchBasis | null {
  const words = term.toLowerCase().split(/\s+/).filter(Boolean);
  if (words.length === 0) return null;
  const containsAll = (text: string) => {
    const lowered = text.toLowerCase();
    return words.every((word) => lowered.includes(word));
  };
  if (containsAll(result.title)) return 'title';
  if (containsAll(result.descriptionPreview)) return 'description';
  return 'unshown';
}

export const MATCH_BASIS_LABELS: Record<MatchBasis, string> = {
  title: 'Term in title',
  description: 'Term in description',
  unshown: 'Match not shown here: full description or document text',
};
