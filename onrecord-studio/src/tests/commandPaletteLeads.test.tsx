// @vitest-environment jsdom
//
// The palette's first rows (top leads, then suggested questions) arrive or sit above the
// navigation rows. Selection is held by row, so leads landing after open never move it, and Return
// on open still goes to the overview.
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { StoryLead } from '../types';

const bridge = vi.hoisted(() => ({ resolveLeads: null as ((leads: unknown) => void) | null }));

vi.mock('@tauri-apps/api/core', () => ({
  isTauri: () => false,
  invoke: (command: string) => {
    if (command === 'list_story_leads') return new Promise((resolve) => { bridge.resolveLeads = resolve; });
    return Promise.resolve([]);
  },
}));

import CommandPalette, { OPEN_COMMAND_PALETTE_EVENT } from '../components/dashboard/CommandPalette';
import { useAppStore } from '../store/useAppStore';

const lead = {
  findingId: 'f-1',
  kind: 'CONTRACT_CEILING_EXCEEDED',
  headline: 'Contract paid past its ceiling',
  severity: 'SERIOUS',
  confidence: { kind: 'MEASURED', basisPoints: 9000 },
  subject: { portalKey: null, agencyId: null, requestId: null, documentId: null, entityId: null },
  portalKey: null,
  publicInterest: { totalBasisPoints: 6700, factors: [{ name: 'money at stake', basisPoints: 2500, evidence: 'largest figure named: $1,486,200' }] },
  relatedFindingIds: [],
  firstSeenAt: '2026-10-07T00:00:00Z',
  lastSeenAt: '2026-10-07T00:00:00Z',
} as unknown as StoryLead;

afterEach(() => {
  cleanup();
  bridge.resolveLeads = null;
});

const selected = () => screen.getAllByRole('option').find((option) => option.getAttribute('aria-selected') === 'true')?.textContent ?? '';

describe('CommandPalette top leads', () => {
  it('goes to the overview on Return with the Leads and Questions groups above Navigate', async () => {
    Element.prototype.scrollIntoView = () => {};
    act(() => useAppStore.setState({ workspaceView: 'activity', sincePreset: 'all' }));
    render(<CommandPalette />);
    act(() => { window.dispatchEvent(new Event(OPEN_COMMAND_PALETTE_EVENT)); });
    await act(async () => { bridge.resolveLeads?.([lead]); });
    expect(screen.getByRole('group', { name: 'Top leads' })).toBeTruthy();
    expect(screen.getByRole('group', { name: 'Questions' })).toBeTruthy();
    expect(selected()).toContain('Go to Overview');
    fireEvent.keyDown(window, { key: 'Enter' });
    expect(useAppStore.getState().workspaceView).toBe('overview');
  });

  it('keeps the opening selection on its row when the leads arrive above it', async () => {
    Element.prototype.scrollIntoView = () => {};
    act(() => useAppStore.setState({ sincePreset: 'all' }));
    render(<CommandPalette />);
    act(() => { window.dispatchEvent(new Event(OPEN_COMMAND_PALETTE_EVENT)); });
    expect(selected()).toContain('Go to Overview');
    await act(async () => { bridge.resolveLeads?.([lead]); });
    expect(screen.getByRole('option', { name: /Contract paid past its ceiling/ })).toBeTruthy();
    expect(selected()).toContain('Go to Overview');
  });
});
