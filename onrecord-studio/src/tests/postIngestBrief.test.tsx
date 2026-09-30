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
}));

vi.mock('@tauri-apps/api/core', () => ({
  isTauri: () => false,
  invoke: (command: string) => {
    if (command === 'list_findings') return Promise.resolve(bridge.rows);
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
  act(() => {
    useAppStore.setState({ postIngestBriefOpen: false });
  });
});

describe('PostIngestBrief', () => {
  it('renders nothing when the overlay is closed', () => {
    render(<PostIngestBrief />);
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('opens and lists findings discovered in the recent window', async () => {
    bridge.rows = [
      sample('recent', new Date().toISOString()),
      sample('older', '2020-01-01T00:00:00Z'),
    ];
    act(() => {
      useAppStore.setState({ postIngestBriefOpen: true });
    });
    render(<PostIngestBrief />);
    await screen.findByRole('dialog');
    await screen.findByText(/Response held \(recent\)/);
    expect(screen.queryByText(/Response held \(older\)/)).toBeNull();
  });

  it('shows the empty state when no findings arrived recently and dismisses', async () => {
    bridge.rows = [];
    act(() => {
      useAppStore.setState({ postIngestBriefOpen: true });
    });
    render(<PostIngestBrief />);
    await screen.findByText(/Ingest complete. Nothing notable/i);
    fireEvent.click(screen.getByRole('button', { name: /dismiss/i }));
    await waitFor(() => {
      expect(useAppStore.getState().postIngestBriefOpen).toBe(false);
    });
  });
});
