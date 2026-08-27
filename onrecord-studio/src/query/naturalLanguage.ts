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

  text = text.replace(/\bafter\s+(\d{4}-\d{2}-\d{2})\b/gi, (_, value: string) => {
    filters.push({ field: 'after', operator: 'after', value, raw: `after ${value}` });
    return '';
  });
  text = text.replace(/\bbefore\s+(\d{4}-\d{2}-\d{2})\b/gi, (_, value: string) => {
    filters.push({ field: 'before', operator: 'before', value, raw: `before ${value}` });
    return '';
  });

  for (const status of ['closed', 'open', 'referred']) {
    const pattern = new RegExp(`\\b${status}\\b`, 'i');
    if (pattern.test(text)) {
      filters.push({ field: 'status', operator: 'equals', value: status, raw: status });
      text = text.replace(pattern, '');
      break;
    }
  }

  if (/\bwith\s+(?:no\s+)?response\b/i.test(text)) {
    const negated = /\bwith\s+no\s+response\b/i.test(text);
    filters.push({ field: 'has', operator: 'exists', value: 'response', negated, raw: negated ? 'no response' : 'with response' });
    text = text.replace(/\bwith\s+(?:no\s+)?response\b/i, '');
  }

  if (filters.length === 0) {
    issues.push({ severity: 'info', message: 'No structured filters detected; this will search full text.' });
  } else {
    issues.push({ severity: 'info', message: 'Review the interpreted filters before searching.' });
  }

  return { version: 1, source: 'natural-language', text: text.replace(/\s+/g, ' ').trim(), filters, issues };
}
