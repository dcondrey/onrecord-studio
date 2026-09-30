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
}));

vi.mock('@tauri-apps/api/core', () => ({
  isTauri: () => false,
  invoke: (command: string, args: unknown) => {
    bridge.invoked.push({ command, args });
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
import type { Finding } from '../types';

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
  });

  afterEach(() => {
    cleanup();
  });

  it('renders the empty-state prompt when no findings are returned', async () => {
    await act(async () => {
      render(<FindingsInbox />);
    });
    await waitFor(() => {
      expect(screen.getByText(/no findings surfaced yet/i)).toBeTruthy();
    });
    // Refresh remains available even in the empty state.
    expect(screen.getByRole('button', { name: /refresh findings/i })).toBeTruthy();
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
      fireEvent.click(screen.getByRole('button', { name: /refresh findings/i }));
    });
    await waitFor(() => {
      expect(bridge.invoked.some((call) => call.command === 'refresh_findings')).toBe(true);
      expect(screen.getByText('Fresh finding after refresh')).toBeTruthy();
    });
  });
});
