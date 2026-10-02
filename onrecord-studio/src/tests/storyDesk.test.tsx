// @vitest-environment jsdom
//
// A lead list requested before a findings refresh must not render after the refreshed list: the
// desk would rank stale findings under a "refreshed" heading.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, render, screen, waitFor } from '@testing-library/react';
import type { StoryLead } from '../types';

// Node 26's flag-gated `localStorage` shadows jsdom's, so replace it before the store import.
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
  pending: [] as ((leads: unknown) => void)[],
  availabilityRequests: [] as string[],
}));

vi.mock('../ipc', () => ({
  formatNativeError: (reason: unknown) => String(reason),
  invokeCommand: (command: string, args?: { findingId?: string }) => {
    if (command === 'build_lead_dossier') return new Promise(() => {});
    if (command === 'draft_availability_for_finding') {
      bridge.availabilityRequests.push(args?.findingId ?? '');
      return Promise.resolve({ followup: true, appeal: false });
    }
    if (command !== 'list_story_leads') return Promise.resolve([]);
    return new Promise((resolve) => bridge.pending.push(resolve));
  },
}));

import { useAppStore } from '../store/useAppStore';
import StoryDesk from '../components/dashboard/StoryDesk';

function lead(id: string, headline: string): StoryLead {
  return {
    findingId: id,
    kind: 'CLAIM_CONTRADICTS_DATA',
    headline,
    severity: 'SERIOUS',
    confidence: { kind: 'MEASURED', basisPoints: 7500 },
    subject: { portalKey: 'sd-city', agencyId: null, requestId: 7, documentId: 3, entityId: null },
    portalKey: 'sd-city',
    publicInterest: {
      totalBasisPoints: 5200,
      factors: [
        { name: 'money at stake', basisPoints: 2500, evidence: 'largest figure named: $1200000.00' },
        { name: 'record contradicts statement', basisPoints: 2000, evidence: 'finding kind CLAIM_CONTRADICTS_DATA' },
      ],
    },
    relatedFindingIds: ['other'],
    firstSeenAt: '2026-09-30T00:00:00Z',
    lastSeenAt: '2026-09-30T00:00:00Z',
  };
}

afterEach(() => {
  cleanup();
  bridge.pending = [];
  bridge.availabilityRequests = [];
});

describe('StoryDesk', () => {
  it('renders leads with their score and every factor and its evidence', async () => {
    render(<StoryDesk />);
    expect(screen.getByRole('status').textContent).toContain('Ranking leads');
    await waitFor(() => expect(bridge.pending).toHaveLength(1));
    act(() => bridge.pending[0]([lead('a', 'Stated total does not match the table')]));
    await screen.findByText('Stated total does not match the table');
    expect(screen.getByLabelText('Attention score').textContent).toBe('52.0');
    expect(screen.getByText(/largest figure named/)).toBeTruthy();
    expect(screen.getByText(/pattern signal/)).toBeTruthy();
    expect(screen.getByText(/1 related finding/)).toBeTruthy();
  });

  it('opens the dossier for the lead whose button was pressed', async () => {
    render(<StoryDesk />);
    await waitFor(() => expect(bridge.pending).toHaveLength(1));
    act(() => bridge.pending[0]([lead('a', 'First lead'), lead('b', 'Second lead')]));
    await screen.findByText('Second lead');
    act(() => {
      screen.getByRole('button', { name: 'Assemble dossier: Second lead' }).click();
    });
    expect(useAppStore.getState().inspectorStack).toEqual([{ kind: 'finding', id: 'b' }]);
    act(() => useAppStore.getState().closeInspector());
  });

  it('asks which drafts apply to a lead only once its draft toggle is opened', async () => {
    render(<StoryDesk />);
    await waitFor(() => expect(bridge.pending).toHaveLength(1));
    act(() => bridge.pending[0]([lead('a', 'First lead'), lead('b', 'Second lead')]));
    await screen.findByText('Second lead');
    await act(async () => {
      await Promise.resolve();
    });
    expect(bridge.availabilityRequests).toEqual([]);
    expect(screen.queryByRole('region', { name: 'Drafted artifacts' })).toBeNull();
    const summaries = screen.getAllByText('Draft a records request or appeal');
    expect(summaries).toHaveLength(2);
    act(() => {
      summaries[1].click();
    });
    await screen.findByRole('button', { name: /draft records request/i });
    expect(bridge.availabilityRequests).toEqual(['b']);
    expect(screen.getAllByRole('region', { name: 'Drafted artifacts' })).toHaveLength(1);
  });

  it('discards a lead list that resolves after a findings refresh superseded it', async () => {
    render(<StoryDesk />);
    await waitFor(() => expect(bridge.pending).toHaveLength(1));
    act(() => useAppStore.getState().requestFindingsRefresh());
    await waitFor(() => expect(bridge.pending).toHaveLength(2));
    act(() => bridge.pending[1]([lead('fresh', 'Fresh lead')]));
    await screen.findByText('Fresh lead');
    await act(async () => {
      bridge.pending[0]([lead('stale', 'Stale lead')]);
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(screen.queryByText('Stale lead')).toBeNull();
    expect(screen.getByText('Fresh lead')).toBeTruthy();
  });
});
