const SENTENCE_END = '.!?"”’)]';

/**
 * A cited passage as it is printed: never ending mid-word. A stored span can stop just after a
 * new sentence starts ("…were located. T"); that dangling fragment is dropped and marked as cut.
 * Past `limit` characters it is cut at the last word boundary inside the limit.
 */
export function clipPassage(quote: string, limit = Infinity): string {
  const trimmed = quote.trim();
  if (trimmed.length <= limit) {
    const lastSpace = trimmed.lastIndexOf(' ');
    const dangling = lastSpace > 0 && SENTENCE_END.includes(trimmed[lastSpace - 1]) && trimmed.length - lastSpace - 1 <= 2;
    return dangling ? `${trimmed.slice(0, lastSpace)} …` : trimmed;
  }
  const cut = trimmed.slice(0, limit);
  const boundary = cut.lastIndexOf(' ');
  return `${(boundary > 0 ? cut.slice(0, boundary) : cut).replace(/[\s.,;:]+$/, '')}…`;
}
