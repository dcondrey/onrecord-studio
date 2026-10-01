import type { CorpusEntityEdge, CorpusEntityNeighborhood, CorpusEntityNode, RelationshipBasis } from '../types';

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


const NAME_SUFFIXES = new Set(['inc', 'llc', 'corp', 'corporation', 'co', 'ltd', 'company', 'the']);

/** Order-insensitive, punctuation-free, suffix-free form of a name; two entities of one kind with the same form are worth a human look. */
export function nameKey(value: string): string {
  const tokens = value
    .toLowerCase()
    .replace(/[^\p{L}\p{N}\s]/gu, ' ')
    .split(/\s+/)
    .filter((token) => token && !NAME_SUFFIXES.has(token));
  return tokens.sort().join(' ');
}

/** Neighbours of `centre` that look like another spelling of it. A similar name is a lead, never proof of identity. */
export function lookalikeNeighbours(centre: CorpusEntityNode, edges: CorpusEntityEdge[]): CorpusEntityNode[] {
  const key = nameKey(centre.displayValue);
  if (!key) return [];
  return edges
    .map((edge) => edge.neighbor)
    .filter((node) => node.entityId !== centre.entityId && node.kind === centre.kind && nameKey(node.displayValue) === key);
}

/**
 * The centre's neighbourhood with the relationships of the entities the reader chose to treat as
 * the same added in. Edges between the merged entities themselves are dropped, and a neighbour
 * reached through several of them keeps the edge with the most shared documents rather than a sum,
 * because one document can mention both spellings.
 */
export function combineNeighbourhoods(
  base: CorpusEntityNeighborhood,
  merged: CorpusEntityNeighborhood[],
): CorpusEntityNeighborhood {
  const same = new Set([base.center.entityId, ...merged.map((neighbourhood) => neighbourhood.center.entityId)]);
  const best = new Map<string, CorpusEntityEdge>();
  for (const edge of [...base.edges, ...merged.flatMap((neighbourhood) => neighbourhood.edges)]) {
    if (same.has(edge.neighbor.entityId)) continue;
    const held = best.get(edge.neighbor.entityId);
    if (!held || edge.sharedDocumentCount > held.sharedDocumentCount) best.set(edge.neighbor.entityId, edge);
  }
  return {
    center: base.center,
    edges: [...best.values()].sort((a, b) => b.sharedDocumentCount - a.sharedDocumentCount),
    truncated: base.truncated || merged.some((neighbourhood) => neighbourhood.truncated),
  };
}
