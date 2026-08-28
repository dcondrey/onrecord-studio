import type { LegacyQueryFilters, QueryAgencyOption, QueryAst, QueryFilter } from './types';

/**
 * Compatibility adapter for `query_audit_records`.
 *
 * NOTE(query): agency names, portal qualifiers, dates, document existence,
 * and boolean groups are intentionally returned as deferred filters until the
 * Rust query command gains typed support. They must never be silently dropped.
 */
export function compileToLegacyQuery(
  query: QueryAst,
  agencies: QueryAgencyOption[] = [],
): LegacyQueryFilters {
  const positiveFieldCounts = new Map<string, number>();
  for (const filter of query.filters) {
    if (!filter.negated) {
      positiveFieldCounts.set(filter.field, (positiveFieldCounts.get(filter.field) ?? 0) + 1);
    }
  }
  const portalFilter = query.filters.find(
    (filter) => filter.field === 'portal' && !filter.negated && positiveFieldCounts.get('portal') === 1,
  );
  let portalKey: string | null = portalFilter?.value ?? null;
  let agencyId: number | null = null;
  let status: string | null = null;
  let policyDeviation: string | null = null;
  const deferredFilters: QueryFilter[] = [];

  for (const filter of query.filters) {
    if (filter.negated || (positiveFieldCounts.get(filter.field) ?? 0) > 1) {
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
    if (filter.field === 'portal' && filter.operator === 'equals') {
      portalKey = filter.value;
      continue;
    }
    if (filter.field === 'agency' && filter.operator === 'equals') {
      const matches = agencies.filter(
        (agency) =>
          agency.agencyName.localeCompare(filter.value, undefined, { sensitivity: 'base' }) === 0
          && (!portalKey || agency.portalKey === portalKey),
      );
      if (matches.length === 1) {
        agencyId = matches[0].agencyId;
        if (!portalKey) portalKey = matches[0].portalKey;
        continue;
      }
    }
    deferredFilters.push(filter);
  }

  return {
    searchTerm: query.text,
    portalKey,
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
