// @vitest-environment jsdom
//
// The workbench keeps one dossier reader while the reader steps through the queue. A dossier read
// for lead A that resolves after lead B is on the stage would replace B's evidence chain with A's,
// or blank it: the stage must keep stating B.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import type { StoryLead } from '../types';

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
  leads: [] as ((leads: unknown) => void)[],
  dossiers: new Map<string, (dossier: unknown) => void>(),
}));

vi.mock('../ipc', () => ({
  formatNativeError: (reason: unknown) => String(reason),
  invokeCommand: (command: string, args?: { findingId?: string }) => {
    if (command === 'list_story_leads') return new Promise((resolve) => bridge.leads.push(resolve));
    if (command === 'build_lead_dossier') return new Promise((resolve) => bridge.dossiers.set(args?.findingId ?? '', resolve));
    return Promise.resolve([]);
  },
}));

import StoryDesk from '../components/dashboard/StoryDesk';

// jsdom does not lay out, so it has no scrollIntoView; the queue keeps its selection in view with it.
Element.prototype.scrollIntoView = vi.fn();

function lead(id: string, headline: string): StoryLead {
  return {
    findingId: id,
    kind: 'CLAIM_CONTRADICTS_DATA',
    headline,
    severity: 'SERIOUS',
    confidence: { kind: 'MEASURED', basisPoints: 7500 },
    subject: { portalKey: 'sd-city', agencyId: null, requestId: 7, documentId: 3, entityId: null },
    portalKey: 'sd-city',
    publicInterest: { totalBasisPoints: 2000, factors: [{ name: 'record contradicts statement', basisPoints: 2000, evidence: 'finding kind CLAIM_CONTRADICTS_DATA' }] },
    relatedFindingIds: [],
    firstSeenAt: '2026-09-30T00:00:00Z',
    lastSeenAt: '2026-09-30T00:00:00Z',
  };
}

function dossier(id: string, headline: string) {
  return {
    finding: {
      id,
      kind: 'CLAIM_CONTRADICTS_DATA',
      severity: 'SERIOUS',
      confidence: { kind: 'MEASURED', basisPoints: 7500 },
      subject: { portalKey: 'sd-city', agencyId: null, requestId: 7, documentId: 3, entityId: null },
      headline,
      explanation: `${headline}, as the records state it.`,
      citations: [],
      discoveredAt: '2026-09-30T00:00:00Z',
      lastSeenAt: '2026-09-30T00:00:00Z',
      dismissedAt: null,
      sourceDetector: 'claims',
    },
    evidence: [],
    evidenceTruncated: false,
    related: [],
    relatedTruncated: false,
    falsification: { statement: 'Wrong if the table was revised.', recordsToSeek: [] },
    gaps: [],
    gapsTruncated: false,
    assembledAt: '2026-09-30T00:00:00Z',
  };
}

afterEach(() => {
  cleanup();
  bridge.leads = [];
  bridge.dossiers.clear();
});

describe('lead workbench', () => {
  it('keeps the selected lead on the stage when the previous lead’s dossier resolves late', async () => {
    render(<StoryDesk />);
    await waitFor(() => expect(bridge.leads).toHaveLength(1));
    act(() => bridge.leads[0]([lead('a', 'First lead'), lead('b', 'Second lead')]));
    await waitFor(() => expect(bridge.dossiers.has('a')).toBe(true));

    fireEvent.click(screen.getByRole('button', { name: /Second lead/ }));
    await waitFor(() => expect(bridge.dossiers.has('b')).toBe(true));
    act(() => bridge.dossiers.get('b')!(dossier('b', 'Dossier of the second lead')));
    expect(await screen.findByRole('heading', { name: 'Dossier of the second lead' })).toBeTruthy();

    await act(async () => {
      bridge.dossiers.get('a')!(dossier('a', 'Dossier of the first lead'));
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(screen.queryByRole('heading', { name: 'Dossier of the first lead' })).toBeNull();
    expect(screen.getByRole('heading', { name: 'Dossier of the second lead' })).toBeTruthy();
  });
});
