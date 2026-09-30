// @vitest-environment jsdom
//
// A dossier assembled for finding A that resolves after the reader opened finding B would present
// A's evidence chain and gaps as statements about B.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, render, screen, waitFor } from '@testing-library/react';

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
  gates: new Map<string, Promise<void>>(),
}));

function dossier(findingId: string, headline: string) {
  return {
    finding: {
      id: findingId,
      kind: 'CLAIM_CONTRADICTS_DATA',
      severity: 'SERIOUS',
      confidence: { kind: 'MEASURED', basisPoints: 8000 },
      subject: { portalKey: 'sandiego', agencyId: null, requestId: 42, documentId: 9, entityId: null },
      headline,
      explanation: 'The stated total lies outside its rounding band.',
      citations: [{ portalKey: 'sandiego', documentId: 9, characterStart: 10, characterEnd: 40, quote: 'the adopted budget of $1.2 billion', label: null }],
      discoveredAt: '2026-09-30T00:00:00Z',
      lastSeenAt: '2026-09-30T00:00:00Z',
      dismissedAt: null,
      sourceDetector: 'claim_data_mismatch',
    },
    evidence: [{ portalKey: 'sandiego', documentId: 9, fileName: 'budget-memo.pdf', requestId: 42, agencyLabel: 'Finance', rawFileSha256: 'abc123def456abc', characterStart: 10, characterEnd: 40, quote: 'the adopted budget of $1.2 billion', label: null }],
    evidenceTruncated: false,
    related: [],
    relatedTruncated: false,
    falsification: { statement: 'The stated figure refers to a later revision of the data.', recordsToSeek: ['the report the statement drew its figure from'] },
    gaps: [{ gap: 'notComputed', portalKey: 'sandiego', documentId: 9, reason: 'the document text could not be read: missing' }],
    gapsTruncated: false,
    assembledAt: '2026-09-30T01:00:00Z',
  };
}

vi.mock('../ipc', () => ({
  invokeCommand: (command: string, args?: { findingId?: string }) => {
    if (command !== 'build_lead_dossier') return Promise.reject(new Error(`unexpected ${command}`));
    const id = args?.findingId ?? '';
    const answer = dossier(id, `Headline for ${id}`);
    const gate = bridge.gates.get(id);
    return gate ? gate.then(() => answer) : Promise.resolve(answer);
  },
  formatNativeError: (reason: unknown) => String(reason),
}));

import { useAppStore } from '../store/useAppStore';
import LeadDossier from '../components/dashboard/LeadDossier';

afterEach(() => {
  cleanup();
  bridge.gates.clear();
  act(() => useAppStore.setState({ dossierFindingId: null }));
});

describe('LeadDossier', () => {
  it('renders the evidence chain, falsification test and an uncomputed gap as such', async () => {
    act(() => useAppStore.getState().openLeadDossier('finding-a'));
    render(<LeadDossier />);
    await screen.findByText('Headline for finding-a');
    expect(screen.getByText(/budget-memo\.pdf/).textContent).toContain('Finance');
    expect(screen.getByText(/later revision of the data/)).toBeTruthy();
    expect(screen.getByText(/figures not examined/)).toBeTruthy();
    expect(screen.queryByText(/Every figure found/)).toBeNull();
  });

  it('discards a dossier that resolves after the reader opened another finding', async () => {
    let releaseA = () => {};
    bridge.gates.set('finding-a', new Promise<void>((resolve) => { releaseA = () => resolve(); }));
    act(() => useAppStore.getState().openLeadDossier('finding-a'));
    render(<LeadDossier />);
    act(() => useAppStore.getState().openLeadDossier('finding-b'));
    await screen.findByText('Headline for finding-b');
    await act(async () => {
      releaseA();
      await Promise.resolve();
      await Promise.resolve();
    });
    await waitFor(() => expect(screen.queryByText('Headline for finding-a')).toBeNull());
    expect(screen.getByText('Headline for finding-b')).toBeTruthy();
  });
});
