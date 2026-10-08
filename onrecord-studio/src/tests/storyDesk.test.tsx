// @vitest-environment jsdom
//
// A lead list requested before a findings refresh must not render after the refreshed list: the
// desk would rank stale findings under a "refreshed" heading.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import type { StoryLead } from '../types';

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

const bridge = vi.hoisted(() => ({
  pending: [] as ((leads: unknown) => void)[],
  availabilityRequests: [] as string[],
  findings: null as ((rows: unknown) => void) | null,
  discovery: { controller: null, state: null } as { controller: null; state: unknown },
}));

vi.mock('../components/dashboard/AutomaticDiscoveryProvider', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../components/dashboard/AutomaticDiscoveryProvider')>()),
  useAutomaticDiscovery: () => bridge.discovery,
}));

vi.mock('../ipc', () => ({
  formatNativeError: (reason: unknown) => String(reason),
  invokeCommand: (command: string, args?: { findingId?: string }) => {
    if (command === 'build_lead_dossier') return new Promise(() => {});
    if (command === 'draft_availability_for_finding') {
      bridge.availabilityRequests.push(args?.findingId ?? '');
      return Promise.resolve({ followup: true, appeal: false });
    }
    if (command === 'list_findings') return new Promise((resolve) => { bridge.findings = resolve; });
    if (command !== 'list_story_leads') return Promise.resolve([]);
    return new Promise((resolve) => bridge.pending.push(resolve));
  },
}));

import { useAppStore } from '../store/useAppStore';
import { useFirstRunDataStore } from '../store/useFirstRunDataStore';
import StoryDesk from '../components/dashboard/StoryDesk';

function lead(id: string, headline: string): StoryLead {
  return {
    findingId: id,
    kind: 'CLAIM_CONTRADICTS_DATA',
    headline,
    severity: 'SERIOUS',
    confidence: { kind: 'MEASURED', basisPoints: 7500 },
    subject: { portalKey: 'sd-city', agencyId: null, requestId: 7, documentId: 3, entityId: null },
    portalKey: 'sd-city',
    publicInterest: {
      totalBasisPoints: 5200,
      factors: [
        { name: 'money at stake', basisPoints: 2500, evidence: 'largest figure named: $1200000.00' },
        { name: 'record contradicts statement', basisPoints: 2000, evidence: 'finding kind CLAIM_CONTRADICTS_DATA' },
      ],
    },
    relatedFindingIds: ['other'],
    firstSeenAt: '2026-09-30T00:00:00Z',
    lastSeenAt: '2026-09-30T00:00:00Z',
  };
}

afterEach(() => {
  cleanup();
  bridge.pending = [];
  bridge.availabilityRequests = [];
  bridge.findings = null;
  bridge.discovery = { controller: null, state: null };
});

const CORE_DATA = 'Get San Diego’s core public data';
const NO_SOURCE_COPY = /Leads come from findings on the records you collect/;

/** These cases read the factor table: the bar-table figure, its rows and their in-place details. */
function showFactorTable() {
  fireEvent.click(screen.getByRole('tab', { name: 'Factor table' }));
}

async function renderWithNoLeads(schema: { datasets: unknown[] } | null) {
  bridge.discovery = { controller: null, state: { schema } };
  render(<StoryDesk />);
  await waitFor(() => expect(bridge.pending).toHaveLength(1));
  act(() => bridge.pending[0]([]));
  await screen.findByText('No leads yet');
}

describe('StoryDesk', () => {
  it('renders leads with their score and every factor and its evidence', async () => {
    render(<StoryDesk />);
    showFactorTable();
    expect(screen.getByRole('status').textContent).toContain('Ranking leads');
    await waitFor(() => expect(bridge.pending).toHaveLength(1));
    act(() => bridge.pending[0]([lead('a', 'Stated total does not match the table')]));
    await screen.findByText('Stated total does not match the table');
    const row = screen.getByRole('row', { name: /^#1, Stated total does not match the table, score 52:/ });
    expect(row.querySelectorAll('td')[0].textContent).toBe('52');
    expect(screen.getByText(/Every factor but a counted violation is a pattern signal/)).toBeTruthy();
    fireEvent.click(within(row).getByRole('button', { name: 'Details: Stated total does not match the table' }));
    expect(screen.getByText(/largest figure named/)).toBeTruthy();
    expect(screen.getByText(/1 related finding/)).toBeTruthy();
  });

  it('draws an unassessed evidence strength as a not-assessed slot, never a zero bar', async () => {
    const unassessed = lead('a', 'Unassessed lead');
    unassessed.confidence = { kind: 'UNASSESSED' };
    unassessed.publicInterest.factors.push({ name: 'evidence strength', basisPoints: 0, evidence: 'confidence not assessed' });
    const measuredZero = lead('b', 'Measured lead');
    measuredZero.confidence = { kind: 'MEASURED', basisPoints: 3 };
    measuredZero.publicInterest.factors.push({ name: 'evidence strength', basisPoints: 0, evidence: 'confidence 0.03%' });
    measuredZero.publicInterest.factors.push({ name: 'counted statutory or policy violation', basisPoints: 1800, evidence: 'counted' });
    render(<StoryDesk />);
    showFactorTable();
    await waitFor(() => expect(bridge.pending).toHaveLength(1));
    act(() => bridge.pending[0]([unassessed, measuredZero]));
    await screen.findByText('Unassessed lead');
    const column = (title: string) => screen.getAllByRole('columnheader').findIndex((header) => header.getAttribute('title') === title) - 1;
    const evidence = (name: RegExp) => screen.getByRole('row', { name }).querySelectorAll('td')[column('evidence strength')];
    const notAssessed = evidence(/^#1, Unassessed lead/);
    expect(notAssessed.textContent).toMatch(/\)?not assessed$/);
    expect(notAssessed.querySelector('path')).toBeNull();
    expect(notAssessed.querySelector('[data-absence="not-recorded"]')).toBeTruthy();
    const zero = evidence(/^#2, Measured lead/);
    expect(zero.textContent).toBe('+0');
    expect(zero.querySelector('line')).toBeTruthy();
    expect(zero.querySelector('[data-absence]')).toBeNull();
    // A factor the record lacks prints nothing; a screen reader hears why the cell is empty.
    const lacking = screen.getByRole('row', { name: /^#1, Unassessed lead/ }).querySelectorAll('td')[column('counted statutory or policy violation')];
    expect(lacking.querySelector('.sr-only')?.textContent).toBe('not in this finding’s record');
    expect(lacking.querySelector('svg')).toBeNull();
    expect(screen.getByText('Counted violation (legal claim)')).toBeTruthy();
  });

  it('claims no type for a factor outside the ranker’s closed set', async () => {
    const unknown = lead('a', 'Lead with a new factor');
    unknown.publicInterest.factors = [{ name: 'novel factor', basisPoints: 300, evidence: 'new' }];
    render(<StoryDesk />);
    showFactorTable();
    await waitFor(() => expect(bridge.pending).toHaveLength(1));
    act(() => bridge.pending[0]([unknown]));
    await screen.findByText('Lead with a new factor');
    const header = screen.getAllByRole('columnheader').find((cell) => cell.getAttribute('title')?.startsWith('novel factor'));
    expect(header?.textContent).toBe('Novel factor');
  });

  it('opens the dossier for the lead whose button was pressed', async () => {
    render(<StoryDesk />);
    showFactorTable();
    await waitFor(() => expect(bridge.pending).toHaveLength(1));
    act(() => bridge.pending[0]([lead('a', 'First lead'), lead('b', 'Second lead')]));
    await screen.findByText('Second lead');
    fireEvent.click(within(screen.getByRole('row', { name: /^#2, Second lead/ })).getByRole('button', { name: 'Details: Second lead' }));
    act(() => {
      screen.getByRole('button', { name: 'Open evidence chain: Second lead' }).click();
    });
    expect(useAppStore.getState().inspectorStack).toEqual([{ kind: 'finding', id: 'b' }]);
    act(() => useAppStore.getState().closeInspector());
  });

  it('asks which drafts apply to a lead only once its draft toggle is opened', async () => {
    render(<StoryDesk />);
    showFactorTable();
    await waitFor(() => expect(bridge.pending).toHaveLength(1));
    act(() => bridge.pending[0]([lead('a', 'First lead'), lead('b', 'Second lead')]));
    await screen.findByText('Second lead');
    await act(async () => {
      await Promise.resolve();
    });
    expect(bridge.availabilityRequests).toEqual([]);
    expect(screen.queryByRole('region', { name: 'Drafted artifacts' })).toBeNull();
    expect(screen.getAllByRole('button', { name: /^Draft a request or appeal: /, hidden: true })).toHaveLength(2);
    fireEvent.click(within(screen.getByRole('row', { name: /^#2, Second lead/ })).getByRole('button', { name: 'Details: Second lead' }));
    act(() => {
      screen.getByRole('button', { name: 'Draft a request or appeal: Second lead' }).click();
    });
    await screen.findByRole('region', { name: 'Drafted artifacts' });
    expect(bridge.availabilityRequests).toEqual(['b']);
    expect(screen.getAllByRole('region', { name: 'Drafted artifacts' })).toHaveLength(1);
  });

  it('offers the core public data when no source is saved and the library has no tables', async () => {
    const run = vi.fn(async () => {});
    const original = useFirstRunDataStore.getState().run;
    useFirstRunDataStore.setState({ run });
    act(() => useAppStore.getState().setWorkspaceView('activity'));
    try {
      await renderWithNoLeads({ datasets: [] });
      expect(screen.getByRole('button', { name: 'Add source' })).toBeTruthy();
      act(() => screen.getByRole('button', { name: CORE_DATA }).click());
      expect(run).toHaveBeenCalledTimes(1);
      expect(useAppStore.getState().workspaceView).toBe('overview');
    } finally {
      useFirstRunDataStore.setState({ run: original });
    }
  });

  it('disables the core public data button while a first-run fetch is running', async () => {
    useFirstRunDataStore.setState({ running: true });
    try {
      await renderWithNoLeads({ datasets: [] });
      expect((screen.getByRole('button', { name: CORE_DATA }) as HTMLButtonElement).disabled).toBe(true);
    } finally {
      useFirstRunDataStore.setState({ running: false });
    }
  });

  it('does not treat a schema that has not loaded as an empty library', async () => {
    await renderWithNoLeads(null);
    expect(screen.queryByRole('button', { name: CORE_DATA })).toBeNull();
    expect(screen.getByRole('button', { name: 'Add source' })).toBeTruthy();
    expect(screen.getByText(NO_SOURCE_COPY)).toBeTruthy();
  });

  it('keeps the add-a-source copy when the library holds a table that produced no leads', async () => {
    await renderWithNoLeads({ datasets: [{ snapshotId: 's1' }] });
    expect(screen.queryByRole('button', { name: CORE_DATA })).toBeNull();
    expect(screen.getByText(NO_SOURCE_COPY)).toBeTruthy();
  });

  it('discards a lead list that resolves after a findings refresh superseded it', async () => {
    render(<StoryDesk />);
    showFactorTable();
    await waitFor(() => expect(bridge.pending).toHaveLength(1));
    act(() => useAppStore.getState().requestFindingsRefresh());
    await waitFor(() => expect(bridge.pending).toHaveLength(2));
    act(() => bridge.pending[1]([lead('fresh', 'Fresh lead')]));
    await screen.findByText('Fresh lead');
    await act(async () => {
      bridge.pending[0]([lead('stale', 'Stale lead')]);
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(screen.queryByText('Stale lead')).toBeNull();
    expect(screen.getByText('Fresh lead')).toBeTruthy();
  });

  it('finishes a briefing add that was in flight when the leads were refreshed', async () => {
    render(<StoryDesk />);
    showFactorTable();
    await waitFor(() => expect(bridge.pending).toHaveLength(1));
    act(() => bridge.pending[0]([lead('kept', 'Kept lead')]));
    fireEvent.click(await screen.findByRole('button', { name: 'Details: Kept lead' }));
    fireEvent.click(screen.getByRole('button', { name: 'Add to briefing' }));
    await waitFor(() => expect(bridge.findings).not.toBeNull());
    fireEvent.click(screen.getByRole('button', { name: 'Refresh' }));
    await waitFor(() => expect(bridge.pending).toHaveLength(2));
    act(() => bridge.pending[1]([lead('kept', 'Kept lead')]));
    await act(async () => bridge.findings!([{ id: 'kept', headline: 'Kept lead', explanation: 'x', citations: [], subject: { portalKey: 'sd-city', requestId: 7, documentId: 3 } }]));
    expect(await screen.findByText('Added to the briefing: Kept lead')).toBeTruthy();
  });
});
