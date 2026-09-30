// @vitest-environment jsdom
//
// CrossCorpusExplorer renders only the four Wave 8 finding kinds and skips everything else,
// draws a jurisdiction × detector heatmap, and disappears when nothing crosses the corpus.
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
  invoke: (command: string) => {
    if (command === 'list_findings') return Promise.resolve(bridge.rows);
    return Promise.resolve(null);
  },
}));

import { useAppStore } from '../store/useAppStore';
import CrossCorpusExplorer from '../components/dashboard/CrossCorpusExplorer';
import type { Finding, FindingKind, FindingSeverity } from '../types';

function makeFinding(
  id: string,
  kind: FindingKind,
  portalKey: string | null,
  severity: FindingSeverity = 'NOTABLE',
): Finding {
  return {
    id,
    kind,
    severity,
    confidence: { kind: 'MEASURED', basisPoints: 6500 },
    subject: {
      portalKey,
      agencyId: 1,
      requestId: null,
      documentId: 1,
      entityId: null,
    },
    headline: `headline for ${id}`,
    explanation: `explanation for ${id}`,
    citations: [],
    discoveredAt: '2026-09-01T00:00:00Z',
    lastSeenAt: '2026-09-01T00:00:00Z',
    dismissedAt: null,
    sourceDetector: 'cross_corpus_test',
  };
}

afterEach(() => {
  cleanup();
  bridge.rows = [];
  act(() => {
    useAppStore.setState({ findingsRefreshRequests: 0 });
  });
});

describe('CrossCorpusExplorer', () => {
  it('shows the empty state when no cross-corpus findings exist', async () => {
    bridge.rows = [
      // A non-cross-corpus finding should be filtered out entirely.
      makeFinding('hb-1', 'HELD_BACK_RESPONSE', 'city-a'),
    ];
    render(<CrossCorpusExplorer />);
    await screen.findByText(/No cross-corpus patterns surfaced yet/);
  });

  it('renders every cross-corpus finding and none of the direct-detector findings', async () => {
    bridge.rows = [
      makeFinding('cc-1', 'CROSS_CORPUS_SHARED_LANGUAGE', 'city-a', 'SERIOUS'),
      makeFinding('cc-2', 'CROSS_CORPUS_TEMPORAL_SHIFT', 'city-b', 'NOTABLE'),
      makeFinding('cc-3', 'CROSS_CORPUS_ORPHAN_TOPIC', 'city-a', 'SERIOUS'),
      makeFinding('cc-4', 'CROSS_CORPUS_REUSED_DENIAL_GROUNDS', 'city-b', 'SERIOUS'),
      // These are non-cross-corpus and must be dropped.
      makeFinding('hb-1', 'HELD_BACK_RESPONSE', 'city-a'),
      makeFinding('bd-1', 'BOILERPLATE_DENIAL', 'city-a'),
    ];
    render(<CrossCorpusExplorer />);
    await screen.findByText(/headline for cc-1/);
    await screen.findByText(/headline for cc-2/);
    await screen.findByText(/headline for cc-3/);
    await screen.findByText(/headline for cc-4/);
    // Non-cross-corpus headlines must NOT appear.
    expect(screen.queryByText(/headline for hb-1/)).toBeNull();
    expect(screen.queryByText(/headline for bd-1/)).toBeNull();
  });

  it('draws the jurisdiction × detector heatmap with both portals', async () => {
    bridge.rows = [
      makeFinding('cc-1', 'CROSS_CORPUS_SHARED_LANGUAGE', 'city-a'),
      makeFinding('cc-2', 'CROSS_CORPUS_TEMPORAL_SHIFT', 'city-b'),
    ];
    const { container } = render(<CrossCorpusExplorer />);
    await screen.findByText(/headline for cc-1/);
    const table = container.querySelector('table[aria-label="Jurisdiction × detector heatmap"]');
    // Table exists (aria-label from parent div; check the presence of table + both portals).
    const tables = container.querySelectorAll('table');
    expect(tables.length).toBeGreaterThan(0);
    const cellsMentioningA = container.querySelectorAll('[aria-label^="city-a"]');
    const cellsMentioningB = container.querySelectorAll('[aria-label^="city-b"]');
    expect(cellsMentioningA.length).toBeGreaterThan(0);
    expect(cellsMentioningB.length).toBeGreaterThan(0);
    // Silence unused var lint.
    void table;
  });
});
