// @vitest-environment jsdom
//
// The ledger states one legal claim (counted deadline misses) beside two statistical ones. These
// tests pin the ways it could lie: drawing an absent closure time as a dot, styling a statistical
// outlier as a counted finding, counting outliers in the headline with a different predicate than
// the marks, and drawing a period window that the reader has already left.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import type { AgencyPerformanceProfile } from '../types/analysis';

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

const bridge = vi.hoisted(() => ({ pending: [] as ((value: unknown) => void)[] }));

vi.mock('@tauri-apps/api/core', () => ({
  isTauri: () => false,
  invoke: (command: string) =>
    command === 'get_agency_performance_profiles'
      ? new Promise((resolve) => bridge.pending.push(resolve))
      : Promise.resolve(null),
}));

import DepartmentLedger, { ledgerOrder, NO_MEASURABLE_CLOSURE, PEER_LATENCY_OUTLIER_Z_SCORE, peerHeadline } from '../components/charts/figures/DepartmentLedger';
import { PeriodComparison } from '../components/dashboard/OverviewDashboard';

function profile(overrides: Partial<AgencyPerformanceProfile> & { agencyId: number; agencyName: string }): AgencyPerformanceProfile {
  return {
    portalSubdomain: 'sd-city',
    totalRequestsProcessed: 20,
    totalClosed: 18,
    validLatencySampleSize: 18,
    excludedInvalidLifetimeCount: 0,
    averageClosureDays: 12,
    globalMeanClosureDays: 12,
    globalStddevClosureDays: 4,
    performanceZScore: 0,
    complianceIndicator: 'Compliant',
    anomalyCount: 0,
    statutoryViolationCount: 0,
    statutoryViolationRate: 0,
    suspiciousFastClosureCount: 0,
    totalDocumentsReleased: 0,
    keywordFrequencyMap: {},
    ...overrides,
  };
}

function renderLedger(profiles: AgencyPerformanceProfile[], onInspect = vi.fn()) {
  render(
    <DepartmentLedger variant="overview" profiles={profiles} status="ready" portalName={(key) => key} onInspect={onInspect} />,
  );
  return onInspect;
}

const row = (name: string) => screen.getByRole('img', { name: new RegExp(`^${name}:`) });
const painted = (element: Element, token: string) => [...element.querySelectorAll('*')].filter((node) => (node.getAttribute('style') ?? '').includes(token));

afterEach(() => {
  cleanup();
  bridge.pending = [];
});

describe('DepartmentLedger', () => {
  it('draws no dot for a department with no measurable closure and says so', () => {
    renderLedger([profile({ agencyId: 1, agencyName: 'Water', averageClosureDays: null, validLatencySampleSize: 0, performanceZScore: null })]);
    const water = row('Water');
    expect(water.getAttribute('aria-label')).toContain(NO_MEASURABLE_CLOSURE);
    expect(water.textContent).toContain(NO_MEASURABLE_CLOSURE);
    expect(water.querySelectorAll('circle')).toHaveLength(0);
  });

  it('rings a department at the outlier threshold in ink with statistical wording, never in the counted colour', () => {
    renderLedger([
      profile({ agencyId: 1, agencyName: 'At', performanceZScore: PEER_LATENCY_OUTLIER_Z_SCORE, averageClosureDays: 20 }),
      profile({ agencyId: 2, agencyName: 'Below', performanceZScore: PEER_LATENCY_OUTLIER_Z_SCORE - 0.01, averageClosureDays: 19.9 }),
      profile({ agencyId: 3, agencyName: 'Counted', statutoryViolationCount: 4, statutoryViolationRate: 0.2 }),
    ]);
    const at = row('At');
    expect(at.textContent).toContain('slower than peers (statistical)');
    // A measured zero of the counted column is a zero-length tick; nothing else in the row is crimson.
    expect(painted(at, '--viz-counted').map((node) => node.tagName)).toEqual(['line']);
    expect(at.querySelectorAll('circle').length).toBeGreaterThan(row('Below').querySelectorAll('circle').length);
    expect(row('Below').textContent).not.toContain('slower than peers');
    expect(painted(row('Counted'), '--viz-counted').map((node) => node.tagName)).toEqual(['path']);
  });

  it('counts outliers in the headline with the same predicate the ring uses', () => {
    const rows = [
      profile({ agencyId: 1, agencyName: 'At', performanceZScore: PEER_LATENCY_OUTLIER_Z_SCORE }),
      profile({ agencyId: 2, agencyName: 'Below', performanceZScore: PEER_LATENCY_OUTLIER_Z_SCORE - 0.01 }),
      profile({ agencyId: 3, agencyName: 'Fast', performanceZScore: -3 }),
    ];
    expect(peerHeadline(rows)).toContain('1 of 3 agencies is slower than peers');
  });

  it('orders by counted misses, then requests, never by rate', () => {
    const order = ledgerOrder([
      profile({ agencyId: 1, agencyName: 'Parks', statutoryViolationCount: 1, statutoryViolationRate: 1, totalRequestsProcessed: 1 }),
      profile({ agencyId: 2, agencyName: 'Police', statutoryViolationCount: 22, statutoryViolationRate: 0.08, totalRequestsProcessed: 268 }),
      profile({ agencyId: 3, agencyName: 'Library', statutoryViolationCount: 1, statutoryViolationRate: 0.01, totalRequestsProcessed: 90 }),
    ]);
    expect(order.map((p) => p.agencyName)).toEqual(['Police', 'Library', 'Parks']);
  });

  it('opens a department with Enter', () => {
    const onInspect = renderLedger([profile({ agencyId: 7, agencyName: 'Police' })]);
    fireEvent.keyDown(row('Police'), { key: 'Enter' });
    expect(onInspect).toHaveBeenCalledWith(expect.objectContaining({ agencyId: 7 }));
  });

  it('draws nothing under a case scope and says why', () => {
    render(<DepartmentLedger variant="overview" profiles={[profile({ agencyId: 1, agencyName: 'Police' })]} status="scoped-out" scopedOutText="Hidden while a case scope is active." portalName={(key) => key} onInspect={vi.fn()} />);
    expect(screen.getByText('Hidden while a case scope is active.')).toBeTruthy();
    expect(screen.queryByRole('img', { name: /^Police:/ })).toBeNull();
  });
});

describe('PeriodComparison', () => {
  it('discards a window read the reader has already left', async () => {
    render(<PeriodComparison activePortalKey={null} />);
    expect(bridge.pending).toHaveLength(2);
    fireEvent.change(screen.getByRole('combobox', { name: 'Interval' }), { target: { value: '7' } });
    expect(bridge.pending).toHaveLength(4);
    const window = (requests: number) => [profile({ agencyId: 1, agencyName: 'Police', totalRequestsProcessed: requests, statutoryViolationCount: 1 })];
    await act(async () => {
      bridge.pending[2](window(10));
      bridge.pending[3](window(10));
    });
    await act(async () => {
      bridge.pending[0](window(99));
      bridge.pending[1](window(99));
    });
    const headline = screen.getByText(/Counted deadline-miss rate/);
    expect(headline.textContent).toContain('10 requests created in the last 7 days');
    expect(headline.textContent).not.toContain('99');
  });

  it('reads a window with no requests as absent, never as a 0% rate', async () => {
    render(<PeriodComparison activePortalKey={null} />);
    await act(async () => {
      bridge.pending[0]([profile({ agencyId: 1, agencyName: 'Police', totalRequestsProcessed: 12, statutoryViolationCount: 3 })]);
      bridge.pending[1]([]);
    });
    expect(screen.getByText(/Counted deadline-miss rate/).textContent).toContain('no comparable prior window');
  });
});
