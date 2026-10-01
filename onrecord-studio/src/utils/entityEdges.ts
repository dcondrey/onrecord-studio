import type { CorpusEntityEdge, RelationshipBasis } from '../types';

export interface EdgeFilter {
  bases: ReadonlySet<RelationshipBasis>;
  /** `YYYY-MM-DD`, or null for no date filter. */
  asOf: string | null;
  /** An edge with no recorded dates is unknown at any date, not valid at every date. Shown only on request. */
  includeUndated: boolean;
}

const DAY_MS = 86_400_000;

function instant(value: string | null | undefined): number | null {
  if (!value) return null;
  const parsed = Date.parse(value);
  return Number.isFinite(parsed) ? parsed : null;
}

export function filterEdges(edges: CorpusEntityEdge[], filter: EdgeFilter): CorpusEntityEdge[] {
  const dayStart = filter.asOf ? Date.parse(`${filter.asOf}T00:00:00Z`) : null;
  const dayEnd = dayStart === null || Number.isNaN(dayStart) ? null : dayStart + DAY_MS - 1;
  return edges.filter((edge) => {
    if (!filter.bases.has(edge.basis)) return false;
    if (dayEnd === null || dayStart === null) return true;
    const from = instant(edge.validFrom);
    const until = instant(edge.validUntil);
    if (from === null && until === null) return filter.includeUndated;
    return (from === null || from <= dayEnd) && (until === null || until >= dayStart);
  });
}
