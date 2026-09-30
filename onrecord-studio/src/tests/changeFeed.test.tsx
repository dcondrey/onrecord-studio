// @vitest-environment jsdom
//
// ChangeFeed reports two numbers: findings arrived since `lastFindingsSeenAt`, and lapsing
// deadlines from the analytics profiles. Both need to render honestly — no findings means the
// strip disappears entirely rather than reporting zero.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, render, screen } from '@testing-library/react';

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

const bridge = vi.hoisted(() => ({ rows: [] as unknown[] }));

vi.mock('@tauri-apps/api/core', () => ({
  isTauri: () => false,
  invoke: (command: string, args: { filter?: { discoveredAfter?: string } | null }) => {
    // Counts the sample rows the way the backend's `discoveredAfter` predicate would.
    if (command === 'count_findings') {
      const after = args.filter?.discoveredAfter;
      const cutoff = after ? Date.parse(after) : null;
      const rows = bridge.rows as { discoveredAt: string }[];
      return Promise.resolve(
        cutoff === null ? rows.length : rows.filter((row) => Date.parse(row.discoveredAt) > cutoff).length,
      );
    }
    return Promise.resolve(null);
  },
}));

import { useAppStore } from '../store/useAppStore';
import ChangeFeed from '../components/dashboard/ChangeFeed';
import type { Finding } from '../types';

function sample(id: string, discoveredAt: string): Finding {
  return {
    id,
    kind: 'HELD_BACK_RESPONSE',
    severity: 'NOTABLE',
    confidence: { kind: 'MEASURED', basisPoints: 5000 },
    subject: {
      portalKey: 'p1',
      agencyId: null,
      requestId: null,
      documentId: null,
      entityId: null,
    },
    headline: id,
    explanation: '',
    citations: [],
    discoveredAt,
    lastSeenAt: discoveredAt,
    dismissedAt: null,
    sourceDetector: 'held_back_response',
  };
}

afterEach(() => {
  cleanup();
  bridge.rows = [];
  act(() => {
    useAppStore.setState({ lastFindingsSeenAt: null, agencyProfiles: [] });
  });
});

describe('ChangeFeed', () => {
  it('renders nothing when there are neither findings nor profiles', async () => {
    const { container } = render(<ChangeFeed />);
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(container.querySelector('section')).toBeNull();
  });

  it('reports every finding when `lastFindingsSeenAt` is null', async () => {
    bridge.rows = [
      sample('a', '2026-09-01T00:00:00Z'),
      sample('b', '2026-09-02T00:00:00Z'),
    ];
    render(<ChangeFeed />);
    await screen.findByText(/2 new findings since you last opened/);
  });

  it('counts only findings after the watermark', async () => {
    bridge.rows = [
      sample('old', '2026-01-01T00:00:00Z'),
      sample('new', '2026-09-15T00:00:00Z'),
    ];
    act(() => {
      useAppStore.setState({ lastFindingsSeenAt: '2026-09-10T00:00:00Z' });
    });
    render(<ChangeFeed />);
    await screen.findByText(/1 new finding since you last opened/);
  });

  it('keeps counting against the watermark captured when its load began', async () => {
    bridge.rows = [sample('new', '2026-09-15T00:00:00Z')];
    render(<ChangeFeed />);
    await screen.findByText(/1 new finding since you last opened/);
    // FindingsInbox marks the findings seen once its own load of the same refresh settles.
    act(() => {
      useAppStore.setState({ lastFindingsSeenAt: '2026-09-20T00:00:00Z' });
    });
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(screen.getByText(/1 new finding since you last opened/)).toBeTruthy();
  });
});

describe('ChangeFeed statutory violations', () => {
  it('shows the counted violations from the agency profiles', async () => {
    bridge.rows = [sample('a', '2026-09-01T00:00:00Z')];
    act(() => {
      useAppStore.setState({
        agencyProfiles: [
          { portalSubdomain: 'p1', statutoryViolationCount: 2 },
          { portalSubdomain: 'p2', statutoryViolationCount: 1 },
        ] as never,
      });
    });
    render(<ChangeFeed />);
    await screen.findByText('3 statutory deadline violations counted.');
  });

  it('omits the button when no violation is counted', async () => {
    bridge.rows = [sample('a', '2026-09-01T00:00:00Z')];
    act(() => {
      useAppStore.setState({ agencyProfiles: [{ portalSubdomain: 'p1', statutoryViolationCount: 0 }] as never });
    });
    render(<ChangeFeed />);
    await screen.findByText(/new finding/);
    expect(screen.queryByText(/statutory deadline/)).toBeNull();
  });
});
