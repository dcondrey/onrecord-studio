import type { QueryAst, QueryFilter } from './types';

/** Serialize the canonical model into a stable, shareable qualifier string. */
export function serializeQualifierQuery(query: QueryAst): string {
  const text = query.text.trim();
  const filters = query.filters.map(serializeFilter).join(' ');
  return [text, filters].filter(Boolean).join(' ');
}

function serializeFilter(filter: QueryFilter) {
  const value = /\s|"/.test(filter.value) ? `"${filter.value.replace(/"/g, '\\"')}"` : filter.value;
  return `${filter.negated ? '-' : ''}${filter.field}:${value}`;
}
