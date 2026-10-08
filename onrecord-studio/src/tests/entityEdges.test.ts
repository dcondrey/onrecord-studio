import { describe, expect, it } from 'vitest';
import type { CorpusEntityEdge, CorpusEntityNeighborhood, CorpusEntityNode } from '../types';
import { combineNeighbourhoods, lookalikeNeighbours, nameKey } from '../utils/entityEdges';

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
