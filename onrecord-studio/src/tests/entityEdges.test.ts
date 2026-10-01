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

import { combineNeighbourhoods, lookalikeNeighbours, nameKey } from '../utils/entityEdges';
import type { CorpusEntityNeighborhood, CorpusEntityNode } from '../types';

const node = (entityId: string, displayValue: string, kind = 'ORGANIZATION') => ({ entityId, displayValue, kind }) as unknown as CorpusEntityNode;
const link = (neighbor: CorpusEntityNode, sharedDocumentCount: number) => ({ neighbor, sharedDocumentCount, basis: 'COMPUTED' }) as unknown as CorpusEntityEdge;

describe('name lookalikes and combining', () => {
  it('treats spelling variants as leads and never across kinds', () => {
    expect(nameKey('ACME, Inc.')).toBe(nameKey('Acme'));
    expect(nameKey('Smith John')).toBe(nameKey('John Smith'));
    const centre = node('c', 'Acme Inc');
    const edges = [link(node('a', 'ACME'), 3), link(node('p', 'Acme', 'PERSON'), 1), link(node('x', 'Other'), 2)];
    expect(lookalikeNeighbours(centre, edges).map((n) => n.entityId)).toEqual(['a']);
  });

  it('keeps the larger shared-document count instead of summing, and drops links among the merged entities', () => {
    const centre = node('c', 'Acme Inc');
    const alias = node('a', 'ACME');
    const vendor = node('v', 'Vendor');
    const base: CorpusEntityNeighborhood = { center: centre, edges: [link(alias, 5), link(vendor, 2)], truncated: false };
    const other: CorpusEntityNeighborhood = { center: alias, edges: [link(centre, 5), link(vendor, 4), link(node('n', 'New'), 1)], truncated: true };
    const combined = combineNeighbourhoods(base, [other]);
    expect(combined.edges.map((e) => [e.neighbor.entityId, e.sharedDocumentCount])).toEqual([['v', 4], ['n', 1]]);
    expect(combined.truncated).toBe(true);
    expect(combined.center.entityId).toBe('c');
  });
});
