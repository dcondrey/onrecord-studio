// @vitest-environment jsdom
//
// A year missing from an exact, complete year facet is a measured zero; missing from a truncated
// or estimated facet it is unknown and drawn as 'none listed'. The two must never share a mark.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen } from '@testing-library/react';
import type { CountBasis, StandingQuerySummary } from '../types/standingSearch';

const bridge = vi.hoisted(() => ({ summaries: [] as unknown[] }));
vi.mock('../ipc', () => ({
  formatNativeError: (reason: unknown) => String(reason),
  invokeCommand: (command: string) => {
    if (command === 'list_standing_queries') return Promise.resolve(bridge.summaries);
    if (command === 'list_series_number_gaps') return Promise.resolve({ gaps: [], seriesCount: 0, truncated: false });
    return Promise.resolve([]);
  },
}));
import StandingSearchPanel from '../components/dashboard/StandingSearchPanel';

function summary(countBasis: CountBasis, truncated: boolean): StandingQuerySummary {
  return {
    query: { id: 'q', label: 'Query', astJson: '{}', filters: { searchTerm: 'x', portalKey: null, agencyId: null, status: null, policyDeviation: null }, createdAt: '2026-10-01T00:00:00Z' },
    lastRun: {
      runId: 'r', queryId: 'q', ranAt: '2026-10-07T00:00:00Z', corpusDigest: 'a'.repeat(64), corpusDocumentCount: 10, changedSincePrevious: null, previousMatchCount: null,
      result: { matchCount: 5, matchesDigest: 'b'.repeat(64), countBasis, clusters: [], facets: [{ dimension: 'year', truncated, values: [{ value: '2022', count: 3 }, { value: '2024', count: 2 }] }] },
    },
    currentCorpusDigest: 'a'.repeat(64),
    due: false,
  };
}

afterEach(cleanup);

describe('standing search year columns', () => {
  it('draws an unlisted year of an exact, complete facet as zero', async () => {
    bridge.summaries = [summary({ basis: 'EXACT' }, false)];
    render(<StandingSearchPanel />);
    const year = await screen.findByRole('img', { name: /^2023:/ });
    expect(year.getAttribute('aria-label')).toMatch(/^2023: 0 matching requests/);
    expect(year.querySelector('[data-absence]')).toBeNull();
  });

  it('draws an unlisted year of a truncated facet as none listed, never zero', async () => {
    bridge.summaries = [summary({ basis: 'EXACT' }, true)];
    render(<StandingSearchPanel />);
    const year = await screen.findByRole('img', { name: /^2023:/ });
    expect(year.getAttribute('aria-label')).toMatch(/^2023: none listed/);
    expect(year.querySelector('[data-absence="not-recorded"]')).toBeTruthy();
  });
});
