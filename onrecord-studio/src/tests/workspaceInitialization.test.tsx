// @vitest-environment jsdom
//
// An empty saved-source list is onboarding only after the source catalog loaded successfully.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen } from '@testing-library/react';

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

vi.mock('../components/charts/AnomalyRadar', () => ({ default: () => <div /> }));
vi.mock('../components/charts/LifecycleFunnel', () => ({ default: () => <div /> }));
vi.mock('../components/charts/PerformanceScatter', () => ({ default: () => <div /> }));
vi.mock('../components/charts/TemporalMap', () => ({ default: () => <div /> }));
vi.mock('../components/charts/AnalyticsUnavailable', () => ({ default: () => <div /> }));
vi.mock('../components/dashboard/MetricsGrid', () => ({ default: () => <div /> }));

import OverviewDashboard from '../components/dashboard/OverviewDashboard';
import { useAppStore } from '../store/useAppStore';

describe('workspace initialization states', () => {
  beforeEach(() => {
    useAppStore.setState({
      portals: [],
      activePortalKey: null,
      agencyProfiles: [],
      crossAgencyCorrelations: [],
      analyticsUpdatedAt: null,
      analyticsError: null,
      workspaceInitializationStatus: 'loading',
      workspaceInitializationError: null,
    });
  });

  afterEach(cleanup);

  it('shows onboarding only after the source catalog reports a genuine empty list', () => {
    useAppStore.setState({ workspaceInitializationStatus: 'ready' });
    render(<OverviewDashboard />);

    expect(screen.getByText('Add your first source')).toBeTruthy();
    expect(screen.queryByText('Saved sources unavailable')).toBeNull();
  });

  it('shows an unavailable state instead of first-run onboarding after a load failure', () => {
    useAppStore.setState({
      workspaceInitializationStatus: 'error',
      workspaceInitializationError: 'source catalog unavailable',
    });
    render(<OverviewDashboard />);

    expect(screen.getByText('Saved sources unavailable')).toBeTruthy();
    expect(screen.getByText('source catalog unavailable')).toBeTruthy();
    expect(screen.queryByText('Add your first source')).toBeNull();
  });
});
