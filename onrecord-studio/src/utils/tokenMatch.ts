/**
 * Word-prefix matching for search-as-you-type: every word of the query must begin some word of the
 * text, so "lease" finds "Lease agreement" and never "release". Words come from the platform's
 * word segmenter, which knows where words break in every script; no pattern is involved.
 */
const segmenter = new Intl.Segmenter(undefined, { granularity: 'word' });

export function wordTokens(text: string): string[] {
  const tokens: string[] = [];
  for (const segment of segmenter.segment(text)) {
    if (segment.isWordLike) tokens.push(segment.segment.toLocaleLowerCase());
  }
  return tokens;
}

/** Whether every needle token begins some token of `text`. A needle with no words matches nothing. */
export function matchesTokens(needle: readonly string[], text: string): boolean {
  if (needle.length === 0) return false;
  const words = wordTokens(text);
  return needle.every((token) => words.some((word) => word.startsWith(token)));
}
