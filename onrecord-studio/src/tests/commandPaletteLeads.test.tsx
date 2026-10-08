// @vitest-environment jsdom
import { act, cleanup, render, screen } from '@testing-library/react';
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

const lead = {
  findingId: 'f-1',
  kind: 'CONTRACT_CEILING_EXCEEDED',
  headline: 'Contract paid past its ceiling',
  severity: 'SERIOUS',
  confidence: { kind: 'ASSESSED', basisPoints: 9000 },
  subject: { portalKey: null, agencyId: null },
  portalKey: null,
  publicInterest: { totalBasisPoints: 6700, factors: [{ name: 'money at stake', basisPoints: 2500, evidence: 'largest figure named: $1,486,200' }] },
  relatedFindingIds: [],
  firstSeenAt: '2026-10-07T00:00:00Z',
  lastSeenAt: '2026-10-07T00:00:00Z',
} as unknown as StoryLead;

afterEach(cleanup);

describe('CommandPalette top leads', () => {
  it('keeps the opening selection on its row when the leads arrive above it', async () => {
    Element.prototype.scrollIntoView = () => {};
    render(<CommandPalette />);
    act(() => { window.dispatchEvent(new Event(OPEN_COMMAND_PALETTE_EVENT)); });
    const selected = () => screen.getAllByRole('option').find((option) => option.getAttribute('aria-selected') === 'true')?.textContent ?? '';
    expect(selected()).toContain('Go to Overview');
    await act(async () => { bridge.resolveLeads?.([lead]); });
    expect(screen.getByRole('option', { name: /Contract paid past its ceiling/ })).toBeTruthy();
    expect(selected()).toContain('Go to Overview');
  });
});
