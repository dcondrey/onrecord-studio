/** Rows requested per page. A page size is a unit of transfer; the total comes from the count. */
export const SEARCH_PAGE_SIZE = 200;

/** An exact match count with thousands separators. */
export function formatMatchCount(count: number): string {
  return count.toLocaleString('en-US');
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

export interface SearchRecordInput {
  /** Stable name for this exact query; see `searchQueryId`. Null when hashing was unavailable. */
  queryId?: string | null;
  /** Digest and size of the document corpus when the record was made; null when it could not be read. */
  corpus?: { digest: string; documentCount: number } | null;
  ranAt: string;
  query: string;
  filters: { label: string; value: string }[];
  sourceCount: number;
  /** Exact number of matches, from the count that accompanied the first page. */
  resultCount: number;
}

/** A citable account of one search. It records what was asked and answered, not that the answer is complete or permanent. */
export function searchRecordText(input: SearchRecordInput): string {
  const filters = input.filters.length === 0 ? 'none' : input.filters.map((f) => `${f.label}: ${f.value}`).join('; ');
  return [
    `OnRecord Studio search run ${input.ranAt}`,
    ...(input.queryId ? [`Query ID: ${input.queryId}`] : []),
    `Text: ${input.query.trim() || '(none)'}`,
    `Filters: ${filters}`,
    `Sources searched: ${input.sourceCount}`,
    `Matches: ${formatMatchCount(input.resultCount)} (exact count when the search ran)`,
    input.corpus
      ? `Corpus state when copied: sha256:${input.corpus.digest.slice(0, 16)} (${input.corpus.documentCount.toLocaleString('en-US')} documents). It covers extracted document text only, not request status or titles, and may differ from the state the search ran against.`
      : 'Corpus state when copied: could not be read.',
    'The query ID names the query, not its answer. Re-running it against the same corpus state returns the same document-text matches; request metadata can still change as sources synchronize.',
  ].join('\n');
}

/** A short, stable name for one exact query: the same text and filters always give the same ID. */
export async function searchQueryId(query: string, filters: { label: string; value: string }[]): Promise<string | null> {
  const canonical = JSON.stringify({
    text: query.trim().replace(/\s+/g, ' '),
    filters: [...filters].sort((a, b) => a.label.localeCompare(b.label)).map((f) => [f.label, f.value]),
  });
  try {
    const digest = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(canonical));
    return Array.from(new Uint8Array(digest).slice(0, 6), (byte) => byte.toString(16).padStart(2, '0')).join('');
  } catch {
    return null;
  }
}
