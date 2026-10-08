// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';

// Node 26's flag-gated `localStorage` shadows jsdom's, so replace it before store import.
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
  invoked: [] as { command: string; args: unknown }[],
  listRows: [] as unknown[],
  refreshRejects: false,
  dismissRejects: false,
  /** Per-command override; when set, the mock returns whatever the function yields for that call. */
  override: {} as Record<string, ((args: unknown) => Promise<unknown>) | undefined>,
}));

vi.mock('@tauri-apps/api/core', () => ({
  isTauri: () => false,
  invoke: (command: string, args: unknown) => {
    bridge.invoked.push({ command, args });
    const override = bridge.override[command];
    if (override) return override(args);
    if (command === 'get_affiliation_disclosure') {
      return Promise.resolve({
        status: 'UNSET',
        sentence: null,
        affiliations: [],
        ownPortalKeys: [],
        ownAgencies: [],
        ownEntityIds: [],
      });
    }
    if (command === 'list_evidence_gaps') return Promise.resolve([]);
    if (command === 'get_profile') return Promise.resolve({ status: 'UNSET', profile: null });
    if (command === 'list_findings') return Promise.resolve(bridge.listRows);
    if (command === 'refresh_findings') {
      if (bridge.refreshRejects) return Promise.reject(new Error('refresh unavailable'));
      return Promise.resolve({
        detectorsRun: 1,
        findingsRecorded: 0,
        staleCleared: 0,
      });
    }
    if (command === 'dismiss_finding') {
      if (bridge.dismissRejects) return Promise.reject(new Error('dismiss unavailable'));
      return Promise.resolve(null);
    }
    return Promise.resolve(null);
  },
}));

import FindingsInbox from '../components/dashboard/FindingsInbox';
import { useAppStore } from '../store/useAppStore';
import { useProfileLensStore } from '../store/useProfileLensStore';
import type { Finding } from '../types';

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function sampleFinding(overrides: Partial<Finding> = {}): Finding {
  return {
    id: 'abc123',
    kind: 'HELD_BACK_RESPONSE',
    severity: 'SERIOUS',
    confidence: { kind: 'MEASURED', basisPoints: 7500 },
    subject: {
      portalKey: 'test-portal',
      agencyId: null,
      requestId: 42,
      documentId: 7,
      entityId: null,
    },
    headline: 'Response held 5 days after last edit',
    explanation: 'Doc modified 2026-01-01 but released 2026-01-06.',
    citations: [
      {
        portalKey: 'test-portal',
        documentId: 7,
        characterStart: null,
        characterEnd: null,
        quote: null,
        label: 'PDF ModDate on response.pdf',
      },
    ],
    discoveredAt: '2026-09-27T12:00:00Z',
    lastSeenAt: '2026-09-27T12:00:00Z',
    dismissedAt: null,
    sourceDetector: 'held_back_response',
    ...overrides,
  };
}

describe('FindingsInbox', () => {
  beforeEach(() => {
    bridge.invoked = [];
    bridge.listRows = [];
    bridge.refreshRejects = false;
    bridge.dismissRejects = false;
    bridge.override = {};
  });

  afterEach(() => {
    cleanup();
  });

  it('does not report "no local profile" while the disclosure is still loading', async () => {
    const pending = deferred<unknown>();
    bridge.override.get_affiliation_disclosure = () => pending.promise;
    useProfileLensStore.setState({ lensEnabled: true });
    await act(async () => {
      render(<FindingsInbox />);
    });
    expect(screen.getByText(/reading the local profile/i)).toBeTruthy();
    expect(screen.queryByText(/no local profile is set/i)).toBeNull();
    await act(async () => {
      pending.resolve({
        status: 'UNSET',
        sentence: null,
        affiliations: [],
        ownPortalKeys: [],
        ownAgencies: [],
        ownEntityIds: [],
      });
    });
    expect(screen.getByText(/no local profile is set/i)).toBeTruthy();
    useProfileLensStore.setState({ lensEnabled: false });
  });

  it('renders the empty-state prompt when no findings are returned', async () => {
    await act(async () => {
      render(<FindingsInbox />);
    });
    await waitFor(() => {
      expect(screen.getByText(/no findings surfaced yet/i)).toBeTruthy();
    });
    // Refresh remains available even in the empty state.
    expect(screen.getByRole('button', { name: /^refresh$/i })).toBeTruthy();
  });

  it('only claims more findings exist when the backend returns more than the page size', async () => {
    const rows = (count: number) =>
      Array.from({ length: count }, (_, i) => sampleFinding({ id: `f${i}`, headline: `Finding ${i}` }));
    bridge.listRows = rows(50);
    await act(async () => {
      render(<FindingsInbox />);
    });
    await waitFor(() => expect(screen.getByText('Finding 0')).toBeTruthy());
    expect(screen.queryByText(/more exist/i)).toBeNull();
    cleanup();
    bridge.listRows = rows(51);
    await act(async () => {
      render(<FindingsInbox />);
    });
    await waitFor(() => expect(screen.getByText('Finding 49')).toBeTruthy());
    expect(screen.getByText(/more exist/i)).toBeTruthy();
    expect(screen.queryByText('Finding 50')).toBeNull();
  });

  it('withholds the agency matrix while findings remain unloaded, and loads all of them on request', async () => {
    const rows = (count: number) =>
      Array.from({ length: count }, (_, i) => sampleFinding({ id: `f${i}`, headline: `Finding ${i}`, subject: { ...sampleFinding().subject, agencyId: i % 2 } }));
    bridge.override.list_findings = (args) => Promise.resolve(rows(Math.min(53, (args as { limit: number }).limit)));
    bridge.override.count_findings = () => Promise.resolve(53);
    await act(async () => {
      render(<FindingsInbox />);
    });
    const loadAll = await screen.findByRole('button', { name: 'Load all 53 to compare by agency' });
    expect(screen.queryByRole('grid')).toBeNull();
    await act(async () => {
      fireEvent.click(loadAll);
    });
    await waitFor(() => expect(screen.getByRole('grid', { name: 'Findings by agency and severity' })).toBeTruthy());
    expect(bridge.invoked.filter((call) => call.command === 'list_findings').at(-1)?.args).toEqual({ filter: null, limit: 54 });
  });

  it('discards a finding count that resolves after a newer refresh', async () => {
    bridge.listRows = Array.from({ length: 51 }, (_, i) => sampleFinding({ id: `f${i}`, headline: `Finding ${i}` }));
    const stale = deferred<number>();
    let calls = 0;
    bridge.override.count_findings = () => (calls++ === 0 ? stale.promise : Promise.resolve(120));
    await act(async () => {
      render(<FindingsInbox />);
    });
    await waitFor(() => expect(calls).toBe(1));
    await act(async () => {
      useAppStore.getState().requestFindingsRefresh();
    });
    await screen.findByRole('button', { name: 'Load all 120 to compare by agency' });
    await act(async () => stale.resolve(999));
    expect(screen.queryByRole('button', { name: /Load all 999/ })).toBeNull();
    expect(screen.getByRole('button', { name: 'Load all 120 to compare by agency' })).toBeTruthy();
  });

  it('filters the list to the agency and severity of an activated matrix cell', async () => {
    useAppStore.setState({ agencyProfiles: [{ agencyId: 3, portalSubdomain: 'test-portal', agencyName: 'Environmental Services' } as never] });
    bridge.listRows = [
      sampleFinding({ id: 'a', severity: 'CRITICAL', headline: 'Environmental critical', subject: { ...sampleFinding().subject, agencyId: 3 } }),
      sampleFinding({ id: 'b', severity: 'SERIOUS', headline: 'Environmental serious', subject: { ...sampleFinding().subject, agencyId: 3 } }),
      sampleFinding({ id: 'c', severity: 'CRITICAL', headline: 'Unattributed critical' }),
    ];
    await act(async () => {
      render(<FindingsInbox />);
    });
    await screen.findByText('Environmental Services carries 2 of 3 findings, including 1 of 2 critical.');
    fireEvent.keyDown(screen.getByRole('img', { name: /^Environmental Services, critical: 1 finding/ }), { key: 'Enter' });
    expect(screen.getByText('Environmental critical')).toBeTruthy();
    expect(screen.queryByText('Environmental serious')).toBeNull();
    expect(screen.queryByText('Unattributed critical')).toBeNull();

    // A newer sweep with more findings than a page withholds the matrix; the agency filter it set
    // still narrows the list, so it stays visible and clearable.
    bridge.listRows = [...bridge.listRows, ...Array.from({ length: 50 }, (_, i) => sampleFinding({ id: `x${i}`, headline: `Extra ${i}` }))];
    await act(async () => {
      useAppStore.getState().requestFindingsRefresh();
    });
    await waitFor(() => expect(screen.queryByRole('grid')).toBeNull());
    fireEvent.click(screen.getByRole('button', { name: 'Clear' }));
    expect(screen.getByText('Unattributed critical')).toBeTruthy();
    useAppStore.setState({ agencyProfiles: [] });
  });

  it('renders ranked findings from list_findings and shows their severity, headline, and explanation', async () => {
    bridge.listRows = [
      sampleFinding({
        id: 'a',
        severity: 'CRITICAL',
        headline: 'Critical: 30 day holdback',
      }),
      sampleFinding({
        id: 'b',
        severity: 'NOTABLE',
        headline: 'Notable: 2 day holdback',
      }),
    ];
    await act(async () => {
      render(<FindingsInbox />);
    });
    await waitFor(() => {
      expect(screen.getByText('Critical: 30 day holdback')).toBeTruthy();
    });
    expect(screen.getByText('Notable: 2 day holdback')).toBeTruthy();
    // Severity + confidence pill visible.
    expect(screen.getAllByText(/75% measured/i).length).toBeGreaterThan(0);
  });

  it('optimistically removes a dismissed finding and calls dismiss_finding on the backend', async () => {
    bridge.listRows = [sampleFinding({ id: 'to-dismiss', headline: 'About to be dismissed' })];
    await act(async () => {
      render(<FindingsInbox />);
    });
    await waitFor(() => {
      expect(screen.getByText('About to be dismissed')).toBeTruthy();
    });
    const dismissButton = screen.getByRole('button', {
      name: /dismiss finding: about to be dismissed/i,
    });
    await act(async () => {
      fireEvent.click(dismissButton);
    });
    await waitFor(() => {
      expect(screen.queryByText('About to be dismissed')).toBeNull();
    });
    const dismissCall = bridge.invoked.find((call) => call.command === 'dismiss_finding');
    expect(dismissCall).toBeTruthy();
    expect((dismissCall?.args as { id: string }).id).toBe('to-dismiss');
  });

  it('rolls back the optimistic dismissal if the backend rejects', async () => {
    bridge.listRows = [sampleFinding({ id: 'keep', headline: 'Should reappear' })];
    bridge.dismissRejects = true;
    await act(async () => {
      render(<FindingsInbox />);
    });
    await waitFor(() => {
      expect(screen.getByText('Should reappear')).toBeTruthy();
    });
    await act(async () => {
      fireEvent.click(
        screen.getByRole('button', {
          name: /dismiss finding: should reappear/i,
        }),
      );
    });
    await waitFor(() => {
      expect(screen.getByText('Should reappear')).toBeTruthy();
      expect(screen.getByRole('alert')).toBeTruthy();
    });
  });

  it('invokes refresh_findings and re-loads the list on Refresh', async () => {
    bridge.listRows = [];
    await act(async () => {
      render(<FindingsInbox />);
    });
    await waitFor(() => {
      expect(screen.getByText(/no findings surfaced yet/i)).toBeTruthy();
    });
    // After the initial load, arrange a new list to be returned on next fetch.
    bridge.listRows = [sampleFinding({ id: 'fresh', headline: 'Fresh finding after refresh' })];
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: /^refresh$/i }));
    });
    await waitFor(() => {
      expect(bridge.invoked.some((call) => call.command === 'refresh_findings')).toBe(true);
      expect(screen.getByText('Fresh finding after refresh')).toBeTruthy();
    });
  });

  it('keeps a failed refresh_findings visible after the list re-loads', async () => {
    bridge.listRows = [sampleFinding({ id: 'stale', headline: 'Finding from the previous sweep' })];
    bridge.refreshRejects = true;
    await act(async () => {
      render(<FindingsInbox />);
    });
    await waitFor(() => {
      expect(screen.getByText('Finding from the previous sweep')).toBeTruthy();
    });
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: /^refresh$/i }));
    });
    // The reload keyed off the refresh counter must have settled before we judge the alert.
    await waitFor(() => {
      expect(bridge.invoked.filter((call) => call.command === 'list_findings').length).toBeGreaterThanOrEqual(2);
      expect(screen.getByRole('button', { name: /^refresh$/i })).toBeTruthy();
    });
    const alert = screen.getByRole('alert');
    expect(alert.textContent).toMatch(/refresh unavailable/);
    expect(alert.textContent).not.toMatch(/findings unavailable/i);
    // The previous sweep's rows are still shown; a failed sweep does not blank the list.
    expect(screen.getByText('Finding from the previous sweep')).toBeTruthy();
  });

  it('does not restore a successfully dismissed sibling when an earlier dismissal is rejected', async () => {
    bridge.listRows = [
      sampleFinding({ id: 'first', headline: 'First to dismiss' }),
      sampleFinding({ id: 'second', headline: 'Second to dismiss' }),
    ];
    const first = deferred<null>();
    bridge.override.dismiss_finding = (args) =>
      (args as { id: string }).id === 'first' ? first.promise : Promise.resolve(null);
    await act(async () => {
      render(<FindingsInbox />);
    });
    await waitFor(() => {
      expect(screen.getByText('First to dismiss')).toBeTruthy();
    });
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: /dismiss finding: first to dismiss/i }));
    });
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: /dismiss finding: second to dismiss/i }));
    });
    await waitFor(() => {
      expect(screen.queryByText('Second to dismiss')).toBeNull();
    });
    await act(async () => {
      first.reject(new Error('dismiss unavailable'));
    });
    await waitFor(() => {
      expect(screen.getByText('First to dismiss')).toBeTruthy();
    });
    // `second` was dismissed on the backend; the rollback of `first` must not resurrect it.
    expect(screen.queryByText('Second to dismiss')).toBeNull();
    expect(screen.getByRole('alert').textContent).toMatch(/dismiss unavailable/);
  });

  it('does not re-show a row whose dismissal committed while a list load was in flight', async () => {
    bridge.listRows = [sampleFinding({ id: 'gone', headline: 'Dismissed mid-load' })];
    await act(async () => {
      render(<FindingsInbox />);
    });
    await waitFor(() => {
      expect(screen.getByText('Dismissed mid-load')).toBeTruthy();
    });
    // A reload starts (an ingest finished) and is answered from the pre-dismissal state.
    const staleList = deferred<unknown[]>();
    bridge.override.list_findings = () => staleList.promise;
    await act(async () => {
      useAppStore.getState().requestFindingsRefresh();
    });
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: /dismiss finding: dismissed mid-load/i }));
    });
    await waitFor(() => {
      expect(screen.queryByText('Dismissed mid-load')).toBeNull();
    });
    await act(async () => {
      staleList.resolve([sampleFinding({ id: 'gone', headline: 'Dismissed mid-load' })]);
    });
    expect(screen.queryByText('Dismissed mid-load')).toBeNull();
  });

  it('does not overwrite a newer list with the pre-dismiss snapshot when a rejected dismissal rolls back', async () => {
    bridge.listRows = [
      sampleFinding({ id: 'kept', headline: 'Kept across sweeps' }),
      sampleFinding({ id: 'cleared', headline: 'Cleared by the next sweep' }),
    ];
    const pending = deferred<null>();
    bridge.override.dismiss_finding = () => pending.promise;
    await act(async () => {
      render(<FindingsInbox />);
    });
    await waitFor(() => {
      expect(screen.getByText('Kept across sweeps')).toBeTruthy();
    });
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: /dismiss finding: kept across sweeps/i }));
    });
    await waitFor(() => {
      expect(screen.queryByText('Kept across sweeps')).toBeNull();
    });
    // An ingest finishes while the dismissal is in flight: the next sweep dropped `cleared` and
    // surfaced `fresh`. The backend still holds `kept` because its dismissal has not committed.
    bridge.listRows = [
      sampleFinding({ id: 'kept', headline: 'Kept across sweeps' }),
      sampleFinding({ id: 'fresh', headline: 'Fresh from the new sweep' }),
    ];
    await act(async () => {
      useAppStore.getState().requestFindingsRefresh();
    });
    await waitFor(() => {
      expect(screen.getByText('Fresh from the new sweep')).toBeTruthy();
    });
    // The in-flight dismissal still hides `kept` even though the reload returned it.
    expect(screen.queryByText('Kept across sweeps')).toBeNull();
    await act(async () => {
      pending.reject(new Error('dismiss unavailable'));
    });
    await waitFor(() => {
      expect(screen.getByText('Kept across sweeps')).toBeTruthy();
    });
    expect(screen.getByText('Fresh from the new sweep')).toBeTruthy();
    expect(screen.queryByText('Cleared by the next sweep')).toBeNull();
  });

  it('renders a failed initial load as an error, not as an empty inbox', async () => {
    bridge.override.list_findings = () => Promise.reject(new Error('database locked'));
    await act(async () => {
      render(<FindingsInbox />);
    });
    await waitFor(() => {
      expect(screen.getByRole('alert').textContent).toMatch(/database locked/);
    });
    expect(screen.queryByText(/no findings surfaced yet/i)).toBeNull();
    expect(screen.queryByText(/loading findings/i)).toBeNull();
  });

  it('renders CLAIM_CONTRADICTS_DATA as a pattern signal with its full explanation', async () => {
    const explanation =
      'Band: stated $1,200,000; observed $840,000 in the audited fund ledger (scope: fund-general, FY2025); queries #12, #19.';
    bridge.listRows = [
      sampleFinding({
        id: 'claim',
        kind: 'CLAIM_CONTRADICTS_DATA',
        severity: 'NOTABLE',
        headline: 'Stated fund total exceeds the ledger',
        explanation,
      }),
    ];
    await act(async () => {
      render(<FindingsInbox />);
    });
    await waitFor(() => {
      expect(screen.getByText('Stated fund total exceeds the ledger')).toBeTruthy();
    });
    expect(screen.getAllByText(explanation).length).toBeGreaterThan(0);
    expect(screen.getByText(/pattern signal, not a legal claim/i)).toBeTruthy();
    expect(screen.queryByText(/legal claim: counted violation/i)).toBeNull();
  });
});
