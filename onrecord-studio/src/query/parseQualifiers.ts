import type { QueryAst, QueryField, QueryFilter, QueryIssue } from './types';

const supportedFields = new Set<QueryField>([
  'agency',
  'portal',
  'status',
  'after',
  'before',
  'has',
  'flag',
  'type',
]);

/**
 * Parse the intentionally small, human-readable qualifier grammar.
 *
 * Supported now: `field:value`, `field:"quoted value"`, and `-field:value`.
 * TODO(query): Add parenthesized groups and explicit AND/OR once the backend
 * can accept an expression tree instead of the current flat IPC payload.
 */
export function parseQualifierQuery(input: string): QueryAst {
  const filters: QueryFilter[] = [];
  const issues: QueryIssue[] = [];
  const textTokens: string[] = [];
  // Keep `agency:"City Attorney"` and standalone quoted phrases intact.
  // TODO(query): Support escaped quotes and a cursor-aware tokenizer for the editor.
  const tokenPattern = /(?:[A-Za-z][\w-]*:"[^"]*"|[^\s"]+|"[^"]*")+/g;
  const tokens = input.match(tokenPattern) ?? [];

  for (const token of tokens) {
    const match = token.match(/^(-)?([a-z][a-z-]*):(.*)$/i);
    if (!match) {
      textTokens.push(stripQuotes(token));
      continue;
    }

    const [, minus, rawField, rawValue] = match;
    const field = rawField.toLowerCase() as QueryField;
    const value = stripQuotes(rawValue);
    if (!value) {
      issues.push({ severity: 'error', message: `Add a value after ${rawField}:`, token });
      continue;
    }
    if (!supportedFields.has(field)) {
      issues.push({ severity: 'warning', message: `Unknown search field “${rawField}”.`, token });
      textTokens.push(stripQuotes(token));
      continue;
    }

    filters.push({
      field,
      operator: operatorFor(field),
      value,
      negated: Boolean(minus),
      raw: token,
    });
  }

  return {
    version: 1,
    source: 'qualifier',
    text: textTokens.join(' ').trim(),
    filters,
    issues,
  };
}

function operatorFor(field: QueryField) {
  if (field === 'after') return 'after' as const;
  if (field === 'before') return 'before' as const;
  if (field === 'has') return 'exists' as const;
  return 'equals' as const;
}

function stripQuotes(value: string) {
  return value.startsWith('"') && value.endsWith('"') ? value.slice(1, -1) : value;
}
