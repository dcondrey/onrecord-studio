// @vitest-environment jsdom
//
// The BriefingWorkspace used to announce "The app never upgrades a lead automatically" and
// require every entry to be user-typed. Wave 2E overrides that: on mount with an empty saved
// briefing, the workspace pulls the top findings and drafts them as leads. The user still
// promotes; the app just fills the starting page.
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

const bridge = vi.hoisted(() => ({ rows: [] as unknown[], shouldFail: false }));

vi.mock('@tauri-apps/api/core', () => ({
  isTauri: () => false,
  invoke: (command: string) => {
    if (command === 'list_findings') {
      if (bridge.shouldFail) return Promise.reject(new Error('list_findings unavailable'));
      return Promise.resolve(bridge.rows);
    }
    return Promise.resolve(null);
  },
}));

import { useAppStore } from '../store/useAppStore';
import BriefingWorkspace from '../components/dashboard/BriefingWorkspace';
import type { Finding } from '../types';
import { createDefaultBriefingProject } from '../utils/briefing';

function withRequestSubject(id: string): Finding {
  return {
    id,
    kind: 'HELD_BACK_RESPONSE',
    severity: 'SERIOUS',
    confidence: { kind: 'MEASURED', basisPoints: 7000 },
    subject: {
      portalKey: 'city-of-x',
      agencyId: 42,
      requestId: 999,
      documentId: 3,
      entityId: null,
    },
    headline: `Response held for ${id}`,
    explanation: 'PDF ModDate predates release by 9 days.',
    citations: [
      {
        portalKey: 'city-of-x',
        documentId: 3,
        characterStart: null,
        characterEnd: null,
        quote: null,
        label: 'PDF ModDate',
      },
    ],
    discoveredAt: '2026-09-27T12:00:00Z',
    lastSeenAt: '2026-09-27T12:00:00Z',
    dismissedAt: null,
    sourceDetector: 'held_back_response',
  };
}

afterEach(() => {
  cleanup();
  bridge.rows = [];
  bridge.shouldFail = false;
  act(() => {
    useAppStore.setState({ briefingProject: createDefaultBriefingProject() });
  });
});

describe('BriefingWorkspace auto-draft', () => {
  it('promotion header no longer declares the workspace anti-autonomous', async () => {
    render(<BriefingWorkspace />);
    await waitFor(() => {
      expect(screen.queryByText('The app never upgrades a lead automatically.')).toBeNull();
    });
    expect(screen.getAllByText(/auto-drafts a starting briefing/i).length).toBeGreaterThan(0);
  });

  it('auto-populates an empty briefing from findings with request anchors', async () => {
    bridge.rows = [withRequestSubject('finding-1'), withRequestSubject('finding-2')];
    render(<BriefingWorkspace />);
    await waitFor(() => {
      expect(useAppStore.getState().briefingProject.evidence.length).toBeGreaterThan(0);
    });
    const evidence = useAppStore.getState().briefingProject.evidence;
    expect(evidence.every((item) => item.id.startsWith('auto-draft:'))).toBe(true);
    expect(screen.getAllByText(/Auto-drafted from your top findings/i).length).toBeGreaterThan(0);
  });

  it('does not resurrect evidence the user deleted after the component remounts', async () => {
    bridge.rows = [withRequestSubject('finding-1'), withRequestSubject('finding-2')];
    const { unmount } = render(<BriefingWorkspace />);
    await waitFor(() => {
      expect(useAppStore.getState().briefingProject.evidence.length).toBeGreaterThan(0);
    });

    act(() => {
      for (const item of useAppStore.getState().briefingProject.evidence) {
        useAppStore.getState().removeBriefingEvidence(item.id);
      }
    });
    expect(useAppStore.getState().briefingProject.evidence).toHaveLength(0);
    unmount();

    render(<BriefingWorkspace />);
    // Give the effect time to run if it were going to re-fire.
    await new Promise((resolve) => setTimeout(resolve, 30));
    expect(useAppStore.getState().briefingProject.evidence).toHaveLength(0);
  });

  it('retries the auto-draft on the next mount after a failed fetch, unlike a completed draft', async () => {
    bridge.shouldFail = true;
    const { unmount } = render(<BriefingWorkspace />);
    await waitFor(() => {
      expect(screen.getByText('Auto-draft unavailable — start manually.')).toBeTruthy();
    });
    // The failed attempt must not have burned the one chance to auto-draft.
    expect(useAppStore.getState().briefingProject.autoDrafted).toBe(false);
    unmount();

    bridge.shouldFail = false;
    bridge.rows = [withRequestSubject('finding-retry')];
    render(<BriefingWorkspace />);
    await waitFor(() => {
      expect(useAppStore.getState().briefingProject.evidence.length).toBeGreaterThan(0);
    });
    expect(useAppStore.getState().briefingProject.autoDrafted).toBe(true);
  });

  it('skips findings without a request anchor rather than fabricating one', async () => {
    const anchorless = withRequestSubject('anchorless');
    anchorless.subject = { ...anchorless.subject, requestId: null };
    bridge.rows = [anchorless];
    render(<BriefingWorkspace />);
    // Give the effect time to run without adding anything.
    await new Promise((resolve) => setTimeout(resolve, 30));
    expect(useAppStore.getState().briefingProject.evidence.length).toBe(0);
  });

  it('stops claiming a draft is in progress once the draft has actually finished with nothing to show', async () => {
    bridge.rows = [];
    render(<BriefingWorkspace />);
    await waitFor(() => {
      expect(useAppStore.getState().briefingProject.autoDrafted).toBe(true);
    });
    // The fetch has completed with zero findings: nothing is in flight any more, so the
    // workspace must not still tell the user it is drafting.
    await waitFor(() => {
      expect(screen.queryByText('Drafting from detector findings…')).toBeNull();
    });
    expect(screen.getByText('No findings drafted — start manually.')).toBeTruthy();
  });

  it('stops claiming a draft is in progress on a remount after the user deleted every drafted item', async () => {
    bridge.rows = [withRequestSubject('finding-1')];
    const { unmount } = render(<BriefingWorkspace />);
    await waitFor(() => {
      expect(useAppStore.getState().briefingProject.evidence.length).toBeGreaterThan(0);
    });
    act(() => {
      for (const item of useAppStore.getState().briefingProject.evidence) {
        useAppStore.getState().removeBriefingEvidence(item.id);
      }
    });
    unmount();

    render(<BriefingWorkspace />);
    // The persisted flag blocks the effect from ever calling list_findings again this
    // session, so the "drafting" message would otherwise be a permanent, false claim.
    await new Promise((resolve) => setTimeout(resolve, 30));
    expect(screen.queryByText('Drafting from detector findings…')).toBeNull();
    expect(screen.getByText('No findings drafted — start manually.')).toBeTruthy();
  });
});
