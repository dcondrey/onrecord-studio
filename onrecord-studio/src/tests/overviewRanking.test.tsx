// @vitest-environment jsdom
//
// The overview queues the Story desk's ranking: its order and "#N of M" are list_story_leads'
// order after the Since filter, never a second ordering (severity first) that would print "#1" on
// a lead the Story desk ranks third.
import { act, cleanup, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { FindingSeverity, StoryLead } from '../types';

const bridge = vi.hoisted(() => ({ leads: [] as ((leads: unknown) => void)[] }));

vi.mock('../ipc', () => ({
  formatNativeError: (reason: unknown) => String(reason),
  invokeCommand: (command: string) => {
    if (command === 'list_story_leads') return new Promise((resolve) => bridge.leads.push(resolve));
    if (command === 'build_lead_dossier') return new Promise(() => {});
    if (command === 'get_meeting_calendar') return new Promise(() => {});
    return Promise.resolve([]);
  },
}));

import OverviewWorkbench from '../components/workbench/OverviewWorkbench';
import { useAppStore } from '../store/useAppStore';

Element.prototype.scrollIntoView = vi.fn();

function lead(id: string, headline: string, severity: FindingSeverity, score: number, firstSeenAt: string): StoryLead {
  return {
    findingId: id,
    kind: 'CLAIM_CONTRADICTS_DATA',
    headline,
    severity,
    confidence: { kind: 'MEASURED', basisPoints: 7500 },
    subject: { portalKey: 'sd-city', agencyId: null, requestId: null, documentId: null, entityId: null },
    portalKey: 'sd-city',
    publicInterest: { totalBasisPoints: score, factors: [{ name: 'record contradicts statement', basisPoints: score, evidence: 'finding kind CLAIM_CONTRADICTS_DATA' }] },
    relatedFindingIds: [],
    firstSeenAt,
    lastSeenAt: firstSeenAt,
  };
}

afterEach(() => {
  cleanup();
  useAppStore.setState({ workbenchSelection: {}, sincePreset: 'lastOpened' });
  bridge.leads = [];
});

describe('overview ranking', () => {
  it('queues and numbers leads in the Story desk order after the Since filter', async () => {
    act(() => useAppStore.setState({ sincePreset: 'lastOpened', previousOpenAt: '2026-10-01T00:00:00Z' }));
    render(<OverviewWorkbench ledgerFigure={null} onOpenLedger={() => {}} onOpenFollowing={() => {}} />);
    await waitFor(() => expect(bridge.leads).toHaveLength(1));
    act(() => bridge.leads[0]([
      lead('a', 'Informational lead ranked first', 'INFO', 6700, '2026-10-05T00:00:00Z'),
      lead('old', 'Lead first seen before the window', 'CRITICAL', 6000, '2026-09-20T00:00:00Z'),
      lead('b', 'Critical lead ranked second', 'CRITICAL', 4300, '2026-10-06T00:00:00Z'),
      lead('c', 'Serious lead ranked third', 'SERIOUS', 3100, '2026-10-07T00:00:00Z'),
    ]));
    const queue = await screen.findByRole('list', { name: 'Leads by attention score' });
    const cards = within(queue).getAllByRole('button').map((card) => card.textContent ?? '');
    expect(cards).toHaveLength(3);
    expect(cards[0]).toContain('Informational lead ranked first');
    expect(cards[0]).toContain('Rank 1.');
    expect(cards[1]).toContain('Critical lead ranked second');
    expect(cards[1]).toContain('Rank 2.');
    expect(cards[2]).toContain('Serious lead ranked third');
    expect(screen.getByRole('region', { name: 'Selected lead' }).textContent).toContain('#1 of 3');
  });
});
