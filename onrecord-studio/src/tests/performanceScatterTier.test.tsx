// @vitest-environment jsdom
//
// A department's tier on the scatter is a statistical claim about its closure time. A statutory
// count is a legal one: it gets its own mark and wording and never lifts a department into a σ
// tier it did not reach, and the headline counts with the same predicate the marks use.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen } from '@testing-library/react';
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

import { useAppStore } from '../store/useAppStore';
import PerformanceScatter from '../components/charts/PerformanceScatter';

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
    suspiciousFastClosureCount: 0,
    totalDocumentsReleased: 3,
    keywordFrequencyMap: {},
    ...overrides,
  };
}

afterEach(cleanup);

describe('PerformanceScatter tiers', () => {
  it('keeps a near-typical department with statutory findings out of the σ tiers and the headline count', () => {
    useAppStore.setState({
      activePortalKey: null,
      analyticsError: null,
      agencyProfiles: [
        profile({ agencyId: 1, agencyName: 'Parks', performanceZScore: 0.4, statutoryViolationCount: 5, statutoryViolationRate: 0.5 }),
        profile({ agencyId: 2, agencyName: 'Police', performanceZScore: 2.3, averageClosureDays: 40 }),
      ],
    });
    render(<PerformanceScatter />);
    const parks = screen.getByRole('button', { name: /^Parks,/ });
    expect(parks.getAttribute('aria-label')).toBe('Parks, typical, 0.4σ slower than peers, 12.0 average days, 5 requests past the statutory deadline');
    expect(screen.getByRole('button', { name: /^Police,/ }).getAttribute('aria-label')).toContain('outlier (≥2σ)');
    expect(screen.getByRole('heading', { level: 2 }).textContent).toBe('1 of 2 departments close requests ≥2σ slower than their peers');
  });
});
