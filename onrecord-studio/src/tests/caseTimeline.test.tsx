// @vitest-environment jsdom
import { cleanup, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import CaseTimeline from '../components/dashboard/CaseTimeline';

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
});
