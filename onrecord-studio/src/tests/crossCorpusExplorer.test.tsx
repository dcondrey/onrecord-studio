// @vitest-environment jsdom
//
import { afterEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';

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

const bridge = vi.hoisted(() => ({ rows: [] as unknown[], fail: false, total: null as number | null }));

vi.mock('@tauri-apps/api/core', () => ({
  isTauri: () => false,
  invoke: (command: string) => {
    if (command === 'list_findings') return bridge.fail ? Promise.reject(new Error('index offline')) : Promise.resolve(bridge.rows);
    if (command === 'count_findings') return Promise.resolve(bridge.total);
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
  bridge.fail = false;
  act(() => {
    useAppStore.setState({ findingsRefreshRequests: 0 });
  });
});

describe('CrossCorpusExplorer', () => {
  it('shows the empty state when no cross-corpus findings exist', async () => {
    bridge.rows = [
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
      makeFinding('hb-1', 'HELD_BACK_RESPONSE', 'city-a'),
      makeFinding('bd-1', 'BOILERPLATE_DENIAL', 'city-a'),
    ];
    render(<CrossCorpusExplorer />);
    await screen.findByText(/headline for cc-1/);
    await screen.findByText(/headline for cc-2/);
    await screen.findByText(/headline for cc-3/);
    await screen.findByText(/headline for cc-4/);
    expect(screen.queryByText(/headline for hb-1/)).toBeNull();
    expect(screen.queryByText(/headline for bd-1/)).toBeNull();
  });

  it('counts findings per detector and lists only detectors that fired', async () => {
    bridge.rows = [
      makeFinding('cc-1', 'CROSS_CORPUS_SHARED_LANGUAGE', 'city-a'),
      makeFinding('cc-2', 'CROSS_CORPUS_SHARED_LANGUAGE', 'city-a'),
      makeFinding('cc-3', 'CROSS_CORPUS_TEMPORAL_SHIFT', 'city-b'),
    ];
    render(<CrossCorpusExplorer />);
    await screen.findByRole('grid', { name: 'Cross-corpus findings by jurisdiction' });
    expect(screen.getByRole('gridcell', { name: /^city-a, shared language: 2 findings/ }).textContent).toContain('2');
    expect(screen.getByRole('gridcell', { name: /^city-b, temporal shift: 1 finding/ })).toBeTruthy();
    expect(screen.getByRole('gridcell', { name: /^city-a, orphan topic: 0 findings/ })).toBeTruthy();
    expect(screen.getByText('3 cross-corpus findings across 2 jurisdictions; most from shared language (2).')).toBeTruthy();
    // The full detector name stays reachable however the header is shortened.
    expect(screen.getByRole('columnheader', { name: 'Reused grounds' })).toBeTruthy();

    fireEvent.keyDown(screen.getByRole('gridcell', { name: /^city-b, temporal shift/ }), { key: 'Enter' });
    expect(screen.getByText(/headline for cc-3/)).toBeTruthy();
    expect(screen.queryByText(/headline for cc-1/)).toBeNull();
  });

  it('withholds the counts while only a first page of findings is loaded, and draws them once all are', async () => {
    bridge.rows = Array.from({ length: 200 }, (_, i) => makeFinding(`cc-${i}`, i % 2 ? 'CROSS_CORPUS_SHARED_LANGUAGE' : 'HELD_BACK_RESPONSE', 'city-a'));
    bridge.total = 200;
    render(<CrossCorpusExplorer />);
    const loadAll = await screen.findByRole('button', { name: 'Load all to compare' });
    expect(screen.queryByRole('grid')).toBeNull();
    await act(async () => { fireEvent.click(loadAll); });
    await screen.findByRole('grid', { name: 'Cross-corpus findings by jurisdiction' });
    expect(screen.getByRole('gridcell', { name: /^city-a, shared language: 100 findings/ })).toBeTruthy();
  });

  it('reports a failed load instead of claiming no patterns exist', async () => {
    bridge.fail = true;
    render(<CrossCorpusExplorer />);
    await screen.findByText(/Cross-corpus findings could not be read/);
    expect(screen.queryByText(/No cross-corpus patterns surfaced yet/)).toBeNull();
  });
});
