import type { LegacyQueryFilters, QueryAst, QueryFilter } from './types';

/**
 * Compatibility adapter for `query_audit_records`.
 *
 * NOTE(query): agency names, portal qualifiers, dates, document existence,
 * and boolean groups are intentionally returned as deferred filters until the
 * Rust query command gains typed support. They must never be silently dropped.
 */
export function compileToLegacyQuery(query: QueryAst): LegacyQueryFilters {
  const agencyId: number | null = null;
  let status: string | null = null;
  let policyDeviation: string | null = null;
  const deferredFilters: QueryFilter[] = [];

  for (const filter of query.filters) {
    if (filter.negated) {
      deferredFilters.push(filter);
      continue;
    }
    if (filter.field === 'status' && filter.operator === 'equals') {
      status = filter.value;
      continue;
    }
    if (filter.field === 'flag' && filter.operator === 'equals') {
      policyDeviation = filter.value.toUpperCase();
      continue;
    }
    // TODO(query): Resolve agency names through the loaded agency index.
    deferredFilters.push(filter);
  }

  return {
    searchTerm: query.text,
    agencyId,
    status,
    policyDeviation,
    deferredFilters,
  };
}

/**
 * TODO(query): Replace this adapter with a typed IPC query payload once the
 * Rust command supports QueryAst. Keeping this function named and isolated
 * makes that migration a one-file boundary change.
 */
