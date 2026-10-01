import { describe, expect, it } from 'vitest';
import { filterEdges } from '../utils/entityEdges';
import type { CorpusEntityEdge, RelationshipBasis } from '../types';

const edge = (id: string, basis: RelationshipBasis, validFrom: string | null, validUntil: string | null) =>
  ({ neighbor: { entityId: id }, basis, validFrom, validUntil }) as unknown as CorpusEntityEdge;

const all = new Set<RelationshipBasis>(['OBSERVED', 'COMPUTED', 'ASSERTED']);
const ids = (edges: CorpusEntityEdge[]) => edges.map((e) => e.neighbor.entityId);

describe('filterEdges', () => {
  const edges = [
    edge('ended', 'OBSERVED', '2020-01-01T00:00:00Z', '2021-06-30T12:00:00Z'),
    edge('open', 'COMPUTED', '2022-01-01T00:00:00Z', null),
    edge('undated', 'OBSERVED', null, null),
  ];

  it('treats an undated edge as unknown at a date, and includes both boundary days', () => {
    expect(ids(filterEdges(edges, { bases: all, asOf: '2021-06-30', includeUndated: false }))).toEqual(['ended']);
    expect(ids(filterEdges(edges, { bases: all, asOf: '2021-07-01', includeUndated: false }))).toEqual([]);
    expect(ids(filterEdges(edges, { bases: all, asOf: '2022-01-01', includeUndated: false }))).toEqual(['open']);
    expect(ids(filterEdges(edges, { bases: all, asOf: '2021-07-01', includeUndated: true }))).toEqual(['undated']);
  });

  it('filters by how a relationship is known without needing a date', () => {
    expect(ids(filterEdges(edges, { bases: new Set(['OBSERVED']), asOf: null, includeUndated: true }))).toEqual(['ended', 'undated']);
  });
});
