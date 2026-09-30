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
import type { Finding } from '../types';

function practiceFinding(overrides: Partial<Finding>): Finding {
  return {
    id: 'practice',
    kind: 'PRACTICE_DEPARTURE_COUNTED',
    severity: 'SERIOUS',
    confidence: { kind: 'MEASURED', basisPoints: 9000 },
    subject: { portalKey: 'sd', agencyId: 3, requestId: 42, documentId: 7, entityId: null },
    headline: 'Police Department: a document states 14 of 40 cases did not follow the written rule',
    explanation: 'Compares what two documents state. It does not establish that the conduct was unlawful.',
    citations: [
      {
        portalKey: 'sd',
        documentId: 5,
        characterStart: 120,
        characterEnd: 210,
        quote: 'Officers shall activate their body-worn cameras before arriving at a call.',
        label: 'Agency policy requirement',
      },
      {
        portalKey: 'sd',
        documentId: 7,
        characterStart: 400,
        characterEnd: 470,
        quote: 'Body-worn cameras were not activated in 14 of 40 incidents.',
        label: 'Documented departure',
      },
    ],
    discoveredAt: '2026-09-27T12:00:00Z',
    lastSeenAt: '2026-09-27T12:00:00Z',
    dismissedAt: null,
    sourceDetector: 'practice_departure',
    ...overrides,
  };
}

describe('practice departure findings in the inbox', () => {
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

  it('shows both citations, and labels only the counted kind as a counted violation', async () => {
    bridge.listRows = [
      practiceFinding({}),
      practiceFinding({
        id: 'stated',
        kind: 'PRACTICE_DEPARTURE_STATED',
        severity: 'NOTABLE',
        confidence: { kind: 'MEASURED', basisPoints: 5000 },
        headline: 'Police Department: a document states the written rule was not followed',
      }),
    ];
    await act(async () => {
      render(<FindingsInbox />);
    });
    await waitFor(() => {
      expect(screen.getByText(/did not follow the written rule/)).toBeTruthy();
    });
    expect(screen.getAllByText(/legal claim: counted violation/i)).toHaveLength(1);
    expect(screen.getAllByText(/pattern signal, not a legal claim/i)).toHaveLength(1);

    const trigger = screen.getByRole('button', {
      name: /why we believe this: police department: a document states 14 of 40/i,
    });
    fireEvent.click(trigger);
    expect(screen.getByText('Agency policy requirement')).toBeTruthy();
    expect(screen.getByText('Documented departure')).toBeTruthy();
    expect(screen.getByText(/shall activate their body-worn cameras/)).toBeTruthy();
    expect(screen.getByText(/not activated in 14 of 40 incidents/)).toBeTruthy();
  });
});
