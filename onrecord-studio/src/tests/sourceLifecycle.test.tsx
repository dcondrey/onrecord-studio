// @vitest-environment jsdom
//
// Pins the argument names the lifecycle controls send. `scripts/check-ipc-contract.mjs` compares
// command names and telemetry variants across the bridge and deliberately does not compare the
// fields inside a payload, so a control that posts `portalKey` to a command whose Rust parameter
// is `source_key` type-checks, lints, and fails only at runtime in the real app. These three
// commands were renamed away from the portal vocabulary, which is exactly when that happens.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';

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
}));

vi.mock('@tauri-apps/api/core', () => ({
  isTauri: () => false,
  invoke: (command: string, args: Record<string, unknown>) => {
    bridge.calls.push({ command, args });
    if (command === 'remove_source') {
      return Promise.resolve({
        sourceKey: 'sandiego',
        discardedFrontierTasks: 12,
        retainedDocuments: 70,
      });
    }
    if (command === 'resume_source_sync') {
      // Declared inline rather than reusing `registration`: this factory runs while the module
      // imports, before the module-level const is initialized.
      return Promise.resolve({
        source: { portalKey: 'sandiego', displayName: 'San Diego' },
        syncRunId: 'run-resumed',
        detectedKind: 'NEXT_REQUEST',
        usedSavedGuidance: true,
      });
    }
    return Promise.resolve(true);
  },
}));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: () => Promise.resolve(null) }));

import Header from '../components/dashboard/Header';
import { useAppStore } from '../store/useAppStore';
import type { PortalRegistration, SourceLifecycleState } from '../types';

const registration: PortalRegistration = {
  portalKey: 'sandiego',
  displayName: 'San Diego',
  canonicalUrl: 'https://sandiego.nextrequest.com',
  originUrl: 'https://sandiego.nextrequest.com',
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

function mountWith(lifecycleState: SourceLifecycleState, syncing: boolean) {
  useAppStore.setState({
    portalUrlDraft: '',
    portals: [{
      registration,
      lifecycleState,
      healthStatus: 'verified',
      lastError: null,
    }],
    activePortalKey: registration.portalKey,
    syncingPortalKeys: syncing ? [registration.portalKey] : [],
    syncRunIdsByPortal: syncing ? { [registration.portalKey]: 'run-1' } : {},
    syncLogs: [],
  });
  render(<Header />);
}

/** The active source's actions live in one menu beside its name in the header. */
function chooseSourceAction(name: string) {
  fireEvent.click(screen.getByRole('button', { name: 'Source actions' }));
  fireEvent.click(screen.getByRole('menuitem', { name }));
}

describe('source lifecycle controls', () => {
  beforeEach(() => {
    bridge.calls.length = 0;
  });

  afterEach(cleanup);

  it('pauses the running source by source key', async () => {
    mountWith('ACTIVE', true);
    chooseSourceAction('Pause synchronization');
    await waitFor(() => expect(bridge.calls).toHaveLength(1));
    expect(bridge.calls[0]).toEqual({
      command: 'pause_source_sync',
      args: { sourceKey: 'sandiego' },
    });
    expect(useAppStore.getState().portals[0].lifecycleState).toBe('PAUSED');
    expect(useAppStore.getState().syncingPortalKeys).toEqual([]);
  });

  it('offers resume only once the source is paused, and adopts the run it returns', async () => {
    mountWith('ACTIVE', true);
    fireEvent.click(screen.getByRole('button', { name: 'Source actions' }));
    expect(screen.queryByRole('menuitem', { name: 'Resume synchronization' })).toBeNull();
    cleanup();

    mountWith('PAUSED', false);
    chooseSourceAction('Resume synchronization');
    await waitFor(() => expect(bridge.calls).toHaveLength(1));
    expect(bridge.calls[0]).toEqual({
      command: 'resume_source_sync',
      args: { sourceKey: 'sandiego' },
    });
    expect(useAppStore.getState().portals[0].lifecycleState).toBe('ACTIVE');
    expect(useAppStore.getState().syncRunIdsByPortal).toEqual({ sandiego: 'run-resumed' });
  });

  // A removal that discards the crawl but keeps the evidence has to say so before it happens.
  // Confirming first is the affordance; reporting the retained count afterwards is the claim.
  it('confirms a removal, states that documents are kept, and reports what was kept', async () => {
    mountWith('ACTIVE', false);
    chooseSourceAction('Remove source');
    expect(bridge.calls).toHaveLength(0);
    expect(screen.getByText(/Documents already ingested from this source are kept/)).toBeTruthy();

    fireEvent.click(screen.getByRole('button', { name: 'Remove and keep documents' }));
    await waitFor(() => expect(bridge.calls).toHaveLength(1));
    expect(bridge.calls[0]).toEqual({
      command: 'remove_source',
      args: { sourceKey: 'sandiego' },
    });
    expect(useAppStore.getState().portals).toEqual([]);
    expect(useAppStore.getState().syncLogs.join('\n')).toContain('70 ingested documents kept');
  });
});
