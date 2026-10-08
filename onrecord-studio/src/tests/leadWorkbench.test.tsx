// @vitest-environment jsdom
//
// The workbench keeps one dossier reader while the reader steps through the queue. A dossier read
// for lead A that resolves after lead B is on the stage would replace B's evidence chain with A's,
// or blank it: the stage must keep stating B.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import type { StoryLead } from '../types';

const bridge = vi.hoisted(() => ({
  leads: [] as ((leads: unknown) => void)[],
  dossiers: new Map<string, (dossier: unknown) => void>(),
  drafts: new Map<string, (availability: unknown) => void>(),
}));

vi.mock('../ipc', () => ({
  formatNativeError: (reason: unknown) => String(reason),
  invokeCommand: (command: string, args?: { findingId?: string }) => {
    if (command === 'list_story_leads') return new Promise((resolve) => bridge.leads.push(resolve));
    if (command === 'build_lead_dossier') return new Promise((resolve) => bridge.dossiers.set(args?.findingId ?? '', resolve));
    if (command === 'draft_availability_for_finding') return new Promise((resolve) => bridge.drafts.set(args?.findingId ?? '', resolve));
    return Promise.resolve([]);
  },
}));

import StoryDesk from '../components/dashboard/StoryDesk';
import { useAppStore } from '../store/useAppStore';

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
  useAppStore.setState({ workbenchSelection: {} });
  bridge.leads = [];
  bridge.dossiers.clear();
  bridge.drafts.clear();
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

  // With nothing selected the stage falls back to lead 1; stepping must start from what is shown,
  // or the first J re-selects the lead already on the stage.
  it('steps from the lead on the stage when nothing was selected yet', async () => {
    render(<StoryDesk />);
    await waitFor(() => expect(bridge.leads).toHaveLength(1));
    act(() => bridge.leads[0]([lead('a', 'First lead'), lead('b', 'Second lead')]));
    await waitFor(() => expect(bridge.dossiers.has('a')).toBe(true));
    expect(screen.getByRole('button', { name: /First lead/ }).getAttribute('aria-current')).toBe('true');

    fireEvent.keyDown(window, { key: 'j' });
    await waitFor(() => expect(bridge.dossiers.has('b')).toBe(true));
    expect(screen.getByRole('button', { name: /Second lead/ }).getAttribute('aria-current')).toBe('true');
    expect(screen.getByRole('button', { name: /First lead/ }).getAttribute('aria-current')).toBeNull();
  });

  // The stage's Draft action takes the reader to the drafts, and the drafts' buttons exist only
  // once the backend says which apply: focus waits for them instead of landing on nothing.
  it('lands the Draft action on the first draft control once the drafts that apply are known', async () => {
    render(<StoryDesk />);
    await waitFor(() => expect(bridge.leads).toHaveLength(1));
    act(() => bridge.leads[0]([lead('a', 'First lead')]));
    await waitFor(() => expect(bridge.dossiers.has('a')).toBe(true));
    act(() => bridge.dossiers.get('a')!(dossier('a', 'Dossier of the first lead')));
    const stage = await screen.findByRole('region', { name: 'Selected lead' });
    const draft = within(stage).getByRole('button', { name: 'Draft records request' });
    draft.focus();
    fireEvent.click(draft);
    await waitFor(() => expect(bridge.drafts.has('a')).toBe(true));
    await act(() => new Promise((resolve) => requestAnimationFrame(resolve)));
    expect(document.activeElement).toBe(draft);

    act(() => bridge.drafts.get('a')!({ followup: true, appeal: true }));
    const drafts = await screen.findByRole('region', { name: 'Drafted artifacts' });
    const first = within(drafts).getByRole('button', { name: 'Draft records request' });
    await waitFor(() => expect(document.activeElement).toBe(first));
    expect(first.hasAttribute('data-focus-landed')).toBe(true);
  });
});
