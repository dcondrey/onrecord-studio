// @vitest-environment jsdom
import { act, cleanup, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import CaseTimeline from '../components/dashboard/CaseTimeline';
import { useAppStore } from '../store/useAppStore';
import type { Finding } from '../types';

// Node 26's flag-gated `localStorage` shadows jsdom's, so replace it before the store is imported.
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

const invoke = vi.hoisted(() => vi.fn());
vi.mock('../ipc', () => ({ invokeCommand: invoke, formatNativeError: String }));
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => true }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn() }));
afterEach(() => {
  cleanup();
  invoke.mockReset();
});

describe('CaseTimeline', () => {
  it('names a source that failed to load and still shows the others', async () => {
    invoke.mockImplementation((command: string) => {
      if (command === 'list_findings') return Promise.reject(new Error('index unavailable'));
      if (command === 'list_watch_changes') {
        return Promise.resolve([
          { id: 'w1', watchId: 'x', dimension: 'DOCUMENTS', changeKind: 'NEW_DOCUMENT', subject: 'Budget memo', oldValue: null, newValue: 'added', oldAsOf: null, newAsOf: '2026-09-01T00:00:00Z', basis: 'OBSERVED', detectedAt: '2026-09-02T00:00:00Z', portalKey: null },
        ]);
      }
      if (command === 'get_casework_state') return Promise.resolve({ cases: [], events: [], leads: [], availableEvidence: [] });
      return Promise.resolve([]);
    });
    render(<CaseTimeline />);
    await waitFor(() => expect(screen.getByText(/new document: Budget memo/i)).toBeTruthy());
    expect(screen.getByRole('alert').textContent).toMatch(/Findings could not be read.*missing from this timeline.*index unavailable/);
    expect(screen.getByText('Noticed by this app')).toBeTruthy();
  });

  it('keeps the inspectable findings from the latest load when an earlier load resolves last', async () => {
    const finding = (id: string): Finding => ({
      id, kind: 'HELD_BACK_RESPONSE', severity: 'NOTABLE', confidence: { kind: 'MEASURED', basisPoints: 5000 },
      subject: { portalKey: 'p', agencyId: null, requestId: 1, documentId: null, entityId: null },
      headline: `headline ${id}`, explanation: `explanation ${id}`, citations: [],
      discoveredAt: '2026-09-01T00:00:00Z', lastSeenAt: '2026-09-01T00:00:00Z', dismissedAt: null, sourceDetector: 'test',
    });
    let releaseFirst: (rows: Finding[]) => void = () => {};
    let findingCalls = 0;
    invoke.mockImplementation((command: string) => {
      if (command === 'list_findings') {
        findingCalls += 1;
        return findingCalls === 1 ? new Promise((resolve) => { releaseFirst = resolve; }) : Promise.resolve([finding('new')]);
      }
      if (command === 'get_casework_state') return Promise.resolve({ cases: [], events: [], leads: [], availableEvidence: [] });
      return Promise.resolve([]);
    });
    render(<CaseTimeline />);
    await waitFor(() => expect(findingCalls).toBe(1));
    act(() => useAppStore.getState().requestFindingsRefresh());
    await screen.findByText('headline new');
    await act(async () => releaseFirst([finding('old')]));
    expect(screen.queryByText('headline old')).toBeNull();
    expect(screen.getByRole('button', { name: 'Open finding' })).toBeTruthy();
  });
});
