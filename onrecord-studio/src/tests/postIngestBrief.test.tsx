// @vitest-environment jsdom
//
// The post-ingest brief opens after `syncStopped`. It shows the findings whose `discoveredAt`
// falls inside a widened window around the moment the overlay opened. A test doubles the
// bridge and checks both that the overlay renders findings when open, and that dismissing it
// clears the store's `postIngestBriefOpen` flag.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';

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
  rows: [] as unknown[],
  /** When set, the next list_findings call returns this instead of resolving with `rows`. */
  next: null as null | (() => Promise<unknown>),
  filters: [] as unknown[],
}));

vi.mock('@tauri-apps/api/core', () => ({
  isTauri: () => false,
  invoke: (command: string, args: { filter?: unknown }) => {
    if (command === 'list_findings') {
      bridge.filters.push(args.filter);
      const next = bridge.next;
      if (next) {
        bridge.next = null;
        return next();
      }
      return Promise.resolve(bridge.rows);
    }
    return Promise.resolve(null);
  },
}));

import { useAppStore } from '../store/useAppStore';
import PostIngestBrief from '../components/dashboard/PostIngestBrief';
import type { Finding } from '../types';

function sample(id: string, discoveredAt: string): Finding {
  return {
    id,
    kind: 'HELD_BACK_RESPONSE',
    severity: 'SERIOUS',
    confidence: { kind: 'MEASURED', basisPoints: 7000 },
    subject: {
      portalKey: 'p1',
      agencyId: 42,
      requestId: 7,
      documentId: 3,
      entityId: null,
    },
    headline: `Response held (${id})`,
    explanation: 'Document was edited days before it was released.',
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
  bridge.next = null;
  bridge.filters = [];
  act(() => {
    useAppStore.setState({ postIngestBriefOpen: false });
  });
});

describe('PostIngestBrief', () => {
  it('asks only for the recent window and lists only findings discovered in it', async () => {
    bridge.rows = [
      sample('recent', new Date().toISOString()),
      sample('older', '2020-01-01T00:00:00Z'),
    ];
    act(() => {
      useAppStore.setState({ postIngestBriefOpen: true });
    });
    render(<PostIngestBrief />);
    await screen.findByRole('region', { name: /finding/i });
    await screen.findByText(/Response held \(recent\)/);
    expect(screen.queryByText(/Response held \(older\)/)).toBeNull();
    await waitFor(() => expect(bridge.filters.length).toBeGreaterThan(0));
    const filter = bridge.filters[0] as { discoveredAfter?: string };
    const after = Date.parse(filter.discoveredAfter ?? '');
    expect(Number.isFinite(after)).toBe(true);
    expect(Date.now() - after).toBeGreaterThanOrEqual(15 * 60 * 1000 - 1000);
    expect(Date.now() - after).toBeLessThan(16 * 60 * 1000);
  });

  it('does not show the previous ingest\'s findings when reopened before its own load resolves', async () => {
    bridge.rows = [sample('first', new Date().toISOString())];
    act(() => {
      useAppStore.setState({ postIngestBriefOpen: true });
    });
    render(<PostIngestBrief />);
    await screen.findByText(/Response held \(first\)/);
    act(() => {
      useAppStore.setState({ postIngestBriefOpen: false });
    });
    expect(screen.queryByRole('region')).toBeNull();
    bridge.next = () => new Promise(() => {});
    act(() => {
      useAppStore.setState({ postIngestBriefOpen: true });
    });
    await screen.findByRole('region', { name: /finding/i });
    expect(screen.queryByText(/Response held \(first\)/)).toBeNull();
    expect(screen.getAllByRole('status').map((node) => node.textContent).join(' ')).toContain('Checking for new findings');
  });

  it('does not claim nothing surfaced when the findings load failed', async () => {
    bridge.next = () => Promise.reject(new Error('database busy'));
    act(() => {
      useAppStore.setState({ postIngestBriefOpen: true });
    });
    render(<PostIngestBrief />);
    await screen.findByRole('alert');
    expect(screen.queryByText(/No new findings/)).toBeNull();
    expect(screen.getByText(/could not be read/)).toBeTruthy();
  });

  it('shows the empty state when no findings arrived recently and dismisses', async () => {
    bridge.rows = [];
    act(() => {
      useAppStore.setState({ postIngestBriefOpen: true });
    });
    render(<PostIngestBrief />);
    await screen.findByText(/Import finished. No new findings/i);
    fireEvent.click(screen.getByRole('button', { name: /dismiss/i }));
    await waitFor(() => {
      expect(useAppStore.getState().postIngestBriefOpen).toBe(false);
    });
  });
});
