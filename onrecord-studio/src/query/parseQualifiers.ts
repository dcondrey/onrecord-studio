import type { QueryAst, QueryField, QueryFilter, QueryIssue } from './types';

const supportedFields = new Set<QueryField>(['agency', 'portal', 'status', 'flag']);

/**
 * Parse the intentionally small, human-readable qualifier grammar.
 *
 * Supported fields are deliberately limited to the four fields the native query executes.
 * Negation is retained only so the compiler can label it as not applied.
 */
export function parseQualifierQuery(input: string): QueryAst {
  const filters: QueryFilter[] = [];
  const issues: QueryIssue[] = [];
  const textTokens: string[] = [];
  // Keep `agency:"City Attorney"` and standalone quoted phrases intact. A quote inside a value
  // ends that value; the resulting extra token remains visible as searchable text.
  const tokenPattern = /(?:[A-Za-z][\w-]*:"[^"]*"|[^\s"]+|"[^"]*")+/g;
  const tokens = input.match(tokenPattern) ?? [];

  for (const token of tokens) {
    const match = token.match(/^(-)?([a-z][a-z-]*):(.*)$/i);
    if (!match) {
      // Quotes stay in the text: the native search reads a quoted span as a phrase.
      textTokens.push(token);
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
      continue;
    }

    filters.push({
      field,
      operator: 'equals',
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

function stripQuotes(value: string) {
  return value.startsWith('"') && value.endsWith('"') ? value.slice(1, -1) : value;
}
