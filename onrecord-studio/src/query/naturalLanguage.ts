import type { QueryAst, QueryFilter, QueryIssue } from './types';

/**
 * Conservative local interpreter for obvious phrases.
 *
 * NOTE(query): This is a deterministic scaffold, not an LLM. It intentionally
 * leaves ambiguous language in `text` and reports a note for the UI. A future
 * model-backed interpreter must return the same QueryAst and show confidence
 * plus an editable preview before execution.
 */
export function interpretNaturalLanguage(input: string): QueryAst {
  const filters: QueryFilter[] = [];
  const issues: QueryIssue[] = [];
  let text = input.trim();

  text = text.replace(/\bstatus\s+(?:is\s+)?(closed|open|referred)\b/gi, (raw: string, value: string) => {
    filters.push({ field: 'status', operator: 'equals', value: value.toLocaleLowerCase(), raw });
    return '';
  });
  text = extractNamedFilter(text, /\bagency\s+(?:is\s+)?(?:"([^"]+)"|'([^']+)'|([^,;]+?))(?=\s+(?:portal|status|flag)\b|[,;]|$)/i, 'agency', filters);
  text = extractNamedFilter(text, /\bportal\s+(?:is\s+)?(?:"([^"]+)"|'([^']+)'|([a-z0-9._-]+))/i, 'portal', filters);
  text = extractNamedFilter(text, /\bflag\s+(?:is\s+)?(?:"([^"]+)"|'([^']+)'|([a-z0-9_-]+))/i, 'flag', filters);

  if (/\b(?:after|before)\s+\d{4}-\d{2}-\d{2}\b/i.test(text)) {
    issues.push({ severity: 'warning', message: 'Date phrases remain full-text terms because the native query contract has no date filter.' });
  }
  if (/\bwith\s+(?:no\s+)?response\b/i.test(text)) {
    issues.push({ severity: 'warning', message: 'Response-existence phrases remain full-text terms because the native query contract has no existence filter.' });
  }

  if (filters.length === 0) {
    issues.push({ severity: 'info', message: 'No structured filters detected; this will search full text.' });
  } else {
    issues.push({ severity: 'info', message: 'Review the interpreted filters before searching.' });
  }

  return { version: 1, source: 'natural-language', text: text.replace(/\s+/g, ' ').trim(), filters, issues };
}

function extractNamedFilter(
  text: string,
  pattern: RegExp,
  field: 'agency' | 'portal' | 'flag',
  filters: QueryFilter[],
) {
  return text.replace(pattern, (raw: string, ...groups: Array<string | undefined>) => {
    const value = groups.slice(0, 3).find((candidate) => candidate?.trim())?.trim();
    if (!value) return raw;
    filters.push({ field, operator: 'equals', value, raw });
    return '';
  });
}
