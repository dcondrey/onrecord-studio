// @vitest-environment jsdom
//
// Covers the destructive half of the source controls. Withdrawal and purge are two operations with
// opposite intentions, and the failures worth pinning here are the ones that would misstate
// evidence: a control that destroys when the operator meant to withdraw, a confirmation that names
// a count it did not read, and a count belonging to a source other than the one named in the
// heading.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';

// See the note in App.test.tsx: Node 26's flag-gated `localStorage` shadows jsdom's, and
// `useAppStore` reads it at module scope.
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
  calls: [] as { command: string; args: Record<string, unknown> }[],
  // Preview resolution is held open per source key so a superseded request can be made to land
  // last, which is the only ordering in which the staleness guard is observable.
  heldPreviews: new Map<string, (value: unknown) => void>(),
  holdPreviewFor: null as string | null,
  // Same idea for the destructive call itself: held open so a second source's confirmation can be
  // opened while the first source's destruction is still in flight, which is the only ordering in
  // which "does the first source's completion clobber the second source's open dialog" is observable.
  heldPurges: new Map<string, (value: unknown) => void>(),
  holdPurgeFor: null as string | null,
}));

vi.mock('@tauri-apps/api/core', () => ({
  isTauri: () => false,
  invoke: (command: string, args: Record<string, unknown>) => {
    bridge.calls.push({ command, args });
    const sourceKey = String(args?.sourceKey ?? '');
    if (command === 'preview_source_purge') {
      const preview = sourceKey === 'oakland'
        ? { sourceKey, documentCount: 3, requestCount: 2, evidenceFileCount: 1 }
        : { sourceKey, documentCount: 412, requestCount: 57, evidenceFileCount: 400 };
      if (bridge.holdPreviewFor === sourceKey) {
        return new Promise((resolve) => bridge.heldPreviews.set(sourceKey, resolve));
      }
      return Promise.resolve(preview);
    }
    if (command === 'purge_source') {
      const purge = {
        sourceKey,
        documentsRemoved: 412,
        requestsRemoved: 57,
        localPaths: ['/var/data/sandiego/a.pdf', '/var/data/sandiego/b.pdf'],
      };
      if (bridge.holdPurgeFor === sourceKey) {
        return new Promise((resolve) => bridge.heldPurges.set(sourceKey, resolve));
      }
      return Promise.resolve(purge);
    }
    if (command === 'remove_source') {
      return Promise.resolve({ sourceKey, discardedFrontierTasks: 12, retainedDocuments: 70 });
    }
    return Promise.resolve(true);
  },
}));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: () => Promise.resolve(null) }));

import Header from '../components/dashboard/Header';
import { useAppStore } from '../store/useAppStore';
import type { PortalRegistration } from '../types';

function registrationFor(portalKey: string, displayName: string): PortalRegistration {
  return {
    portalKey,
    displayName,
    canonicalUrl: `https://${portalKey}.nextrequest.com`,
    originUrl: `https://${portalKey}.nextrequest.com`,
    apiLayout: {
      requestsEndpoint: '/client/requests',
      requestsPointer: '/requests',
      metadataPointer: null,
      pageParameter: 'page',
      pageSizeParameter: 'per_page',
    },
    verifiedAt: '2026-08-26T00:00:00Z',
    sourceKind: 'NEXT_REQUEST',
  };
}

const sanDiego = registrationFor('sandiego', 'San Diego');
const oakland = registrationFor('oakland', 'Oakland');

function mountWith(registrations: PortalRegistration[], activePortalKey: string) {
  useAppStore.setState({
    portalUrlDraft: '',
    portals: registrations.map((registration) => ({
      registration,
      lifecycleState: 'ACTIVE' as const,
      healthStatus: 'verified',
      lastError: null,
      recheckIntervalDays: null,
    })),
    activePortalKey,
    syncingPortalKeys: [],
    syncRunIdsByPortal: {},
    syncLogs: [],
  });
  render(<Header />);
}

describe('source purge', () => {
  beforeEach(() => {
    bridge.calls.length = 0;
    bridge.heldPreviews.clear();
    bridge.holdPreviewFor = null;
    bridge.heldPurges.clear();
    bridge.holdPurgeFor = null;
  });

  afterEach(cleanup);

  // The whole reason the second command exists is that it is not the first one. If the withdrawal
  // control could reach `purge_source`, evidence the operator meant to keep would be gone.
  it('keeps withdrawal and deletion on separate controls and separate confirmations', async () => {
    mountWith([sanDiego], 'sandiego');

    const refreshCountsBefore = {
      evidence: useAppStore.getState().evidenceRefreshRequests,
      findings: useAppStore.getState().findingsRefreshRequests,
      analytics: useAppStore.getState().analyticsRefreshRequests,
    };
    fireEvent.click(screen.getByRole('button', { name: 'Remove source' }));
    expect(screen.getByText(/Documents already ingested from this source are kept/)).toBeTruthy();
    expect(screen.queryByRole('button', { name: 'Delete source and documents' })).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Remove and keep documents' }));
    await waitFor(() => expect(bridge.calls).toHaveLength(1));
    expect(bridge.calls[0].command).toBe('remove_source');
    expect(bridge.calls.some((call) => call.command === 'purge_source')).toBe(false);
    // A removed source no longer belongs in evidence, findings, or analytics: stale charts and
    // lists must be told to refetch.
    expect(useAppStore.getState().evidenceRefreshRequests).toBe(refreshCountsBefore.evidence + 1);
    expect(useAppStore.getState().findingsRefreshRequests).toBe(refreshCountsBefore.findings + 1);
    expect(useAppStore.getState().analyticsRefreshRequests).toBe(refreshCountsBefore.analytics + 1);
  });

  it('names the counts before destroying anything, and reports what was destroyed', async () => {
    mountWith([sanDiego], 'sandiego');

    fireEvent.click(screen.getByRole('button', { name: 'Delete source and its documents' }));
    // Opening the confirmation reads counts and destroys nothing.
    await waitFor(() => expect(bridge.calls).toHaveLength(1));
    expect(bridge.calls[0]).toEqual({
      command: 'preview_source_purge',
      args: { sourceKey: 'sandiego' },
    });

    const confirm = await screen.findByRole('button', { name: 'Delete source and documents' });
    await waitFor(() => expect(screen.getByText(/412/)).toBeTruthy());
    expect(screen.getByText(/57/)).toBeTruthy();
    expect(screen.getByText(/400/)).toBeTruthy();

    const refreshCountsBefore = {
      evidence: useAppStore.getState().evidenceRefreshRequests,
      findings: useAppStore.getState().findingsRefreshRequests,
      analytics: useAppStore.getState().analyticsRefreshRequests,
    };
    fireEvent.click(confirm);
    await waitFor(() => expect(bridge.calls).toHaveLength(2));
    expect(bridge.calls[1]).toEqual({
      command: 'purge_source',
      args: { sourceKey: 'sandiego' },
    });
    expect(useAppStore.getState().portals).toEqual([]);
    const log = useAppStore.getState().syncLogs.join('\n');
    expect(log).toContain('412 documents and 57 requests destroyed');
    expect(log).toContain('2 evidence files deleted');
    expect(log).toContain('can be added again');
    // A purged source no longer belongs in evidence, findings, or analytics: stale charts and
    // lists must be told to refetch.
    expect(useAppStore.getState().evidenceRefreshRequests).toBe(refreshCountsBefore.evidence + 1);
    expect(useAppStore.getState().findingsRefreshRequests).toBe(refreshCountsBefore.findings + 1);
    expect(useAppStore.getState().analyticsRefreshRequests).toBe(refreshCountsBefore.analytics + 1);
  });

  // A confirmation whose counts arrived for a different source is a false statement about the
  // evidence the operator is approving the destruction of. Proven by deleting the
  // `purgePreviewGate.isCurrent(request)` check in the preview effect in Header.tsx: San Diego's
  // 412 then overwrites Oakland's 3 and the final assertion fails.
  it('discards a count that arrives for a source other than the one being confirmed', async () => {
    bridge.holdPreviewFor = 'sandiego';
    mountWith([sanDiego, oakland], 'sandiego');

    fireEvent.click(screen.getByRole('button', { name: 'Delete source and its documents' }));
    await waitFor(() => expect(bridge.heldPreviews.has('sandiego')).toBe(true));
    // The dialog is still counting; nothing can be confirmed yet.
    expect(screen.getByRole('button', { name: 'Delete source and documents' })).toHaveProperty('disabled', true);

    fireEvent.click(screen.getByRole('button', { name: 'Close deletion confirmation' }));
    act(() => useAppStore.setState({ activePortalKey: 'oakland' }));
    fireEvent.click(screen.getByRole('button', { name: 'Delete source and its documents' }));
    await screen.findByText(/Delete Oakland and everything from it\?/);
    await waitFor(() => expect(screen.getByText('3', { exact: true })).toBeTruthy());

    // San Diego's superseded count now lands, after Oakland's dialog is on screen.
    act(() => bridge.heldPreviews.get('sandiego')?.({
      sourceKey: 'sandiego', documentCount: 412, requestCount: 57, evidenceFileCount: 400,
    }));
    await waitFor(() => expect(screen.getByText(/Delete Oakland and everything from it\?/)).toBeTruthy());
    expect(screen.queryByText(/412/)).toBeNull();
    expect(bridge.calls.some((call) => call.command === 'purge_source')).toBe(false);
  });

  // A confirmation dialog the operator is actively looking at is a statement about the source
  // named in its heading. If an earlier, already-dismissed purge for a different source resolves
  // later and blindly clears `pendingPurge`, it silently closes the dialog the operator is looking
  // at right now, for a source that was never touched. Proven by reverting `purgeSource`'s
  // `setPendingPurge` call in useSourceConnector.ts to the unconditional `setPendingPurge(null)`:
  // Oakland's confirmation then disappears out from under the operator when San Diego's purge lands.
  it('does not let a dismissed purge from one source clobber another source\'s open confirmation', async () => {
    bridge.holdPurgeFor = 'sandiego';
    mountWith([sanDiego, oakland], 'sandiego');

    fireEvent.click(screen.getByRole('button', { name: 'Delete source and its documents' }));
    const confirmSanDiego = await screen.findByRole('button', { name: 'Delete source and documents' });
    fireEvent.click(confirmSanDiego);
    await waitFor(() => expect(bridge.heldPurges.has('sandiego')).toBe(true));

    // The operator gives up watching San Diego's in-flight deletion and checks Oakland instead.
    fireEvent.click(screen.getByRole('button', { name: 'Close deletion confirmation' }));
    act(() => useAppStore.setState({ activePortalKey: 'oakland' }));
    fireEvent.click(screen.getByRole('button', { name: 'Delete source and its documents' }));
    await screen.findByText(/Delete Oakland and everything from it\?/);
    await waitFor(() => expect(screen.getByText('3', { exact: true })).toBeTruthy());

    // San Diego's purge, opened and confirmed before Oakland's dialog even existed, lands now.
    act(() => bridge.heldPurges.get('sandiego')?.({
      sourceKey: 'sandiego', documentsRemoved: 412, requestsRemoved: 57,
      localPaths: ['/var/data/sandiego/a.pdf', '/var/data/sandiego/b.pdf'],
    }));

    // The real destruction still applies: San Diego is gone and the refresh fired.
    await waitFor(() => expect(useAppStore.getState().portals.map((p) => p.registration.portalKey)).toEqual(['oakland']));
    // But Oakland's still-open, still-unconfirmed dialog must survive San Diego's unrelated landing.
    expect(screen.getByText(/Delete Oakland and everything from it\?/)).toBeTruthy();
    expect(bridge.calls.some((call) => call.command === 'purge_source' && call.args.sourceKey === 'oakland')).toBe(false);
  });
});
