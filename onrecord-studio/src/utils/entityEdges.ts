import type { CorpusEntityEdge, CorpusEntityNeighborhood, CorpusEntityNode } from '../types';

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
