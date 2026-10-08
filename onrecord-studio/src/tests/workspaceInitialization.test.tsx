// @vitest-environment jsdom
//
// An empty saved-source list is onboarding only after the source catalog loaded successfully.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';

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

  it('shows source onboarding on request after the source catalog reports a genuine empty list', async () => {
    useAppStore.setState({ workspaceInitializationStatus: 'ready' });
    render(<OverviewDashboard />);

    expect(screen.queryByRole('region', { name: 'San Diego research desk' })).toBeNull();
    expect(screen.queryByRole('region', { name: 'Investigation workbench' })).toBeNull();
    fireEvent.click(screen.getByRole('tab', { name: 'Tools' }));
    expect(screen.getByRole('tab', { name: 'Casework' })).toBeTruthy();
    const drawer = screen.getByText('Browse sources and add records').closest('details')!;
    drawer.open = true;
    fireEvent(drawer, new Event('toggle'));
    expect(await screen.findByRole('region', { name: 'San Diego research desk' })).toBeTruthy();
    expect(screen.queryByText('Your saved sources didn’t load')).toBeNull();
  });

  it('shows an unavailable state instead of first-run onboarding after a load failure', () => {
    useAppStore.setState({
      workspaceInitializationStatus: 'error',
      workspaceInitializationError: 'source catalog unavailable',
    });
    render(<OverviewDashboard />);

    expect(screen.getByText('Your saved sources didn’t load')).toBeTruthy();
    expect(screen.getByText('source catalog unavailable')).toBeTruthy();
    expect(screen.queryAllByText('Not yet in your library')).toHaveLength(0);
  });
});
