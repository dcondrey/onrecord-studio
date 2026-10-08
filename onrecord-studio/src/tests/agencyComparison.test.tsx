// @vitest-environment jsdom
//
// A rank on this table is a measured latency or count. The tests pin the three ways it could
// lie: sorting an absent latency as a number, rendering that absence as 0, and marking an agency
// slower than peers on a z-score below the threshold.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import type { AgencyPerformanceProfile } from '../types/analysis';

// Node 26's flag-gated `localStorage` shadows jsdom's, so replace it before the store import.
vi.hoisted(() => {
  const entries = new Map<string, string>();
  Object.defineProperty(globalThis, 'localStorage', {
    configurable: true,
    value: {
      getItem: (key: string) => entries.get(key) ?? null,
      setItem: (key: string, value: string) => void entries.set(key, String(value)),
      removeItem: (key: string) => void entries.delete(key),
      clear: () => entries.clear(),
      key: (index: number) => [...entries.keys()][index] ?? null,
      get length() {
        return entries.size;
      },
    },
  });
});

import { useAppStore } from '../store/useAppStore';
import AgencyComparison, { NO_MEASURABLE_CLOSURE, PEER_LATENCY_OUTLIER_Z_SCORE, sortProfiles } from '../components/dashboard/AgencyComparison';

function profile(overrides: Partial<AgencyPerformanceProfile> & { agencyId: number; agencyName: string }): AgencyPerformanceProfile {
  return {
    portalSubdomain: 'sd-city',
    totalRequestsProcessed: 10,
    totalClosed: 8,
    validLatencySampleSize: 8,
    excludedInvalidLifetimeCount: 0,
    averageClosureDays: 12,
    globalMeanClosureDays: 12,
    globalStddevClosureDays: 4,
    performanceZScore: 0,
    complianceIndicator: 'Compliant',
    anomalyCount: 0,
    statutoryViolationCount: 0,
    statutoryViolationRate: 0,
    statutoryDeadlinePassedCount: 0,
    suspiciousFastClosureCount: 0,
    totalDocumentsReleased: 3,
    keywordFrequencyMap: {},
    ...overrides,
  };
}

function bodyRows() {
  const table = screen.getByRole('table', { name: 'Agency comparison' });
  return within(table).getAllByRole('row').slice(1);
}

afterEach(cleanup);

describe('AgencyComparison', () => {
  it('ranks two portals on average closure days, descending, with an absent latency last', () => {
    useAppStore.setState({
      agencyProfiles: [
        profile({ agencyId: 1, agencyName: 'Parks', averageClosureDays: 5, portalSubdomain: 'sd-city' }),
        profile({ agencyId: 2, agencyName: 'Sheriff', averageClosureDays: 40, portalSubdomain: 'sd-county' }),
        profile({ agencyId: 3, agencyName: 'Water', averageClosureDays: null, validLatencySampleSize: 0, performanceZScore: null }),
      ],
    });
    render(<AgencyComparison />);
    const names = bodyRows().map((row) => within(within(row).getAllByRole('cell')[1]).getAllByRole('button')[0].textContent);
    expect(names).toEqual(['Sheriff', 'Parks', 'Water']);
    expect(screen.getByRole('columnheader', { name: /Avg days/ }).getAttribute('aria-sort')).toBe('descending');
  });

  it('keeps an absent latency last when the sort direction flips', () => {
    const rows = [
      profile({ agencyId: 1, agencyName: 'Parks', averageClosureDays: 5 }),
      profile({ agencyId: 3, agencyName: 'Water', averageClosureDays: null }),
      profile({ agencyId: 2, agencyName: 'Sheriff', averageClosureDays: 40 }),
    ];
    expect(sortProfiles(rows, 'averageClosureDays', 'asc').map((p) => p.agencyName)).toEqual(['Parks', 'Sheriff', 'Water']);
    expect(sortProfiles(rows, 'averageClosureDays', 'desc').map((p) => p.agencyName)).toEqual(['Sheriff', 'Parks', 'Water']);
  });

  it('renders a null average closure as the absence text, never 0', () => {
    useAppStore.setState({
      agencyProfiles: [profile({ agencyId: 3, agencyName: 'Water', averageClosureDays: null, validLatencySampleSize: 0, performanceZScore: null })],
    });
    render(<AgencyComparison />);
    const cells = within(bodyRows()[0]).getAllByRole('cell').map((cell) => cell.textContent);
    expect(cells[2]).toBe(NO_MEASURABLE_CLOSURE);
    expect(cells).not.toContain('0.0');
    expect(within(bodyRows()[0]).getByText('Small sample')).toBeTruthy();
  });

  it('marks slower than peers only at or above the threshold, and never as a violation', () => {
    useAppStore.setState({
      agencyProfiles: [
        profile({ agencyId: 1, agencyName: 'Below', performanceZScore: PEER_LATENCY_OUTLIER_Z_SCORE - 0.01 }),
        profile({ agencyId: 2, agencyName: 'At', performanceZScore: PEER_LATENCY_OUTLIER_Z_SCORE }),
        profile({ agencyId: 3, agencyName: 'Above', performanceZScore: PEER_LATENCY_OUTLIER_Z_SCORE + 0.01 }),
        profile({ agencyId: 4, agencyName: 'Fast', performanceZScore: -(PEER_LATENCY_OUTLIER_Z_SCORE + 1) }),
      ],
    });
    render(<AgencyComparison />);
    const marked = bodyRows()
      .filter((row) => within(row).queryByText('Slower than peers (statistical)') !== null)
      .map((row) => within(within(row).getAllByRole('cell')[1]).getAllByRole('button')[0].textContent);
    expect(marked.sort()).toEqual(['Above', 'At']);
    expect(screen.queryByText(/violat/i, { selector: 'td span' })).toBeNull();
  });

  it('filters to one portal', () => {
    useAppStore.setState({
      agencyProfiles: [
        profile({ agencyId: 1, agencyName: 'Parks', portalSubdomain: 'sd-city' }),
        profile({ agencyId: 2, agencyName: 'Sheriff', portalSubdomain: 'sd-county' }),
      ],
    });
    render(<AgencyComparison />);
    fireEvent.change(screen.getByRole('combobox', { name: 'Portal filter' }), { target: { value: 'sd-county' } });
    expect(bodyRows().map((row) => within(within(row).getAllByRole('cell')[1]).getAllByRole('button')[0].textContent)).toEqual(['Sheriff']);
  });

  it('offers to add a source when there is nothing to profile', () => {
    useAppStore.setState({ agencyProfiles: [], analyticsStatus: 'ready', portals: [] });
    render(<AgencyComparison />);
    expect(screen.getByRole('heading', { name: 'No agencies to compare yet' })).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Add source' })).toBeTruthy();
    expect(screen.queryByRole('button', { name: 'Refresh analytics' })).toBeNull();
  });

  it('does not call profiles uncomputed while the first analytics response is still pending', () => {
    useAppStore.setState({
      agencyProfiles: [],
      analyticsStatus: 'pending',
      analyticsError: null,
      portals: [{ registration: { portalKey: 'p', displayName: 'P', canonicalUrl: 'https://p.example', originUrl: 'https://p.example', apiLayout: { requestsEndpoint: '', requestsPointer: '', metadataPointer: null, pageParameter: '', pageSizeParameter: '' }, verifiedAt: '2026-01-01T00:00:00Z', sourceKind: 'NEXT_REQUEST' }, lifecycleState: 'ACTIVE', healthStatus: 'verified', lastError: null }],
    });
    render(<AgencyComparison />);
    expect(screen.queryByRole('heading', { name: 'Agency profiles have not been computed' })).toBeNull();
    expect(screen.queryByRole('button', { name: 'Refresh analytics' })).toBeNull();
    expect(screen.getByText('Loading agency profiles')).toBeTruthy();
  });

  it('renders the empty state with a refresh action when analytics has produced no profiles', () => {
    useAppStore.setState({
      agencyProfiles: [],
      analyticsStatus: 'ready',
      analyticsRefreshRequests: 0,
      portals: [{ registration: { portalKey: 'p', displayName: 'P', canonicalUrl: 'https://p.example', originUrl: 'https://p.example', apiLayout: { requestsEndpoint: '', requestsPointer: '', metadataPointer: null, pageParameter: '', pageSizeParameter: '' }, verifiedAt: '2026-01-01T00:00:00Z', sourceKind: 'NEXT_REQUEST' }, lifecycleState: 'ACTIVE', healthStatus: 'verified', lastError: null }],
    });
    render(<AgencyComparison />);
    expect(screen.getByRole('heading', { name: 'Agency profiles have not been computed' })).toBeTruthy();
    expect(screen.queryByRole('table')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Refresh analytics' }));
    expect(useAppStore.getState().analyticsRefreshRequests).toBe(1);
  });

  it('states a failed read instead of claiming the profiles were never computed', () => {
    useAppStore.setState({ agencyProfiles: [], analyticsError: 'DATABASE_UNAVAILABLE: busy', analyticsRefreshRequests: 0 });
    render(<AgencyComparison />);
    expect(screen.getByRole('alert').textContent).toContain('Agency profiles could not be read.');
    expect(screen.queryByRole('heading', { name: 'Agency profiles have not been computed' })).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Try again' }));
    expect(useAppStore.getState().analyticsRefreshRequests).toBe(1);
    useAppStore.setState({ analyticsError: null });
  });
});
