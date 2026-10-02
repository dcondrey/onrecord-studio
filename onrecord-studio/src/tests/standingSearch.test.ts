import { describe, expect, it } from 'vitest';
import { describeCountBasis, describeGap, describeRun, formatFacetCount } from '../utils/standingSearch';
import type { StandingQueryRun } from '../types/standingSearch';

const run: StandingQueryRun = {
  runId: 'r', queryId: 'q', ranAt: '2026-10-01T10:00:00Z', corpusDigest: 'b'.repeat(64), corpusDocumentCount: 12,
  result: { matchCount: 20000, matchesDigest: 'c'.repeat(64), countBasis: { basis: 'ESTIMATED_FROM_SAMPLE', sampleSize: 10000 }, facets: [], clusters: [] },
  changedSincePrevious: true, previousMatchCount: 19990,
};

// A sampled breakdown is an estimate and must say so wherever its numbers appear; the total and
// the corpus state stay exact. A series gap is absence from the corpus, never a claim about the agency.
describe('standing search wording', () => {
  it('labels sampled counts as estimates and keeps the total and corpus state exact', () => {
    expect(describeCountBasis(run.result.countBasis)).toContain('estimates scaled from a sample of 10,000');
    expect(formatFacetCount(42, run.result.countBasis)).toBe('≈42');
    expect(formatFacetCount(42, { basis: 'EXACT' })).toBe('42');
    const line = describeRun(run);
    expect(line).toContain('20,000 matching requests (exact)');
    expect(line).toContain('sha256:bbbbbbbbbbbbbbbb');
    expect(line).toContain('changed from 19,990 to 20,000');
    expect(describeRun({ ...run, changedSincePrevious: null, previousMatchCount: null })).toContain('first run');
  });

  it('states a gap as absence from the corpus between its bounding members', () => {
    const text = describeGap({
      label: 'resolution', prefix: '2024-', fromNumber: 12, toNumber: 12, width: 3, seriesMemberCount: 3,
      before: { entityId: 'a', displayValue: 'Resolution No. 2024-011', documentCount: 2 },
      after: { entityId: 'b', displayValue: 'Resolution No. 2024-013', documentCount: 1 },
    });
    expect(text).toMatch(/^No document in the corpus carries resolution 2024-012, between Resolution No\. 2024-011/);
    expect(text).not.toMatch(/withheld|missing from the agency/);
  });
});
