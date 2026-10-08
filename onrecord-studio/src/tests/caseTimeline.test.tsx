// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import CaseTimeline from '../components/dashboard/CaseTimeline';
import { useAppStore } from '../store/useAppStore';
import type { Finding } from '../types';
import EvidenceTimeline, { DENSE_LANE } from '../components/charts/figures/EvidenceTimeline';
import type { TimelineItem } from '../utils/timeline';

// Node 26's flag-gated `localStorage` shadows jsdom's, so replace it before the store is imported.
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

const invoke = vi.hoisted(() => vi.fn());
vi.mock('../ipc', () => ({ invokeCommand: invoke, formatNativeError: String }));
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => true }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn() }));
afterEach(() => {
  cleanup();
  invoke.mockReset();
});

describe('CaseTimeline', () => {
  it('names a source that failed to load and still shows the others', async () => {
    invoke.mockImplementation((command: string) => {
      if (command === 'list_findings') return Promise.reject(new Error('index unavailable'));
      if (command === 'list_watch_changes') {
        return Promise.resolve([
          { id: 'w1', watchId: 'x', dimension: 'DOCUMENTS', changeKind: 'NEW_DOCUMENT', subject: 'Budget memo', oldValue: null, newValue: 'added', oldAsOf: null, newAsOf: '2026-09-01T00:00:00Z', basis: 'OBSERVED', detectedAt: '2026-09-02T00:00:00Z', portalKey: null },
        ]);
      }
      if (command === 'get_casework_state') return Promise.resolve({ cases: [], events: [], leads: [], availableEvidence: [] });
      return Promise.resolve([]);
    });
    render(<CaseTimeline />);
    await waitFor(() => expect(screen.getAllByText(/new document: Budget memo/i).length).toBeGreaterThan(0));
    expect(screen.getByRole('alert').textContent).toMatch(/Findings could not be read.*missing from this timeline.*index unavailable/);
    expect(screen.getByText('Noticed by this app')).toBeTruthy();
  });

  it('keeps the inspectable findings from the latest load when an earlier load resolves last', async () => {
    const finding = (id: string): Finding => ({
      id, kind: 'HELD_BACK_RESPONSE', severity: 'NOTABLE', confidence: { kind: 'MEASURED', basisPoints: 5000 },
      subject: { portalKey: 'p', agencyId: null, requestId: 1, documentId: null, entityId: null },
      headline: `headline ${id}`, explanation: `explanation ${id}`, citations: [],
      discoveredAt: '2026-09-01T00:00:00Z', lastSeenAt: '2026-09-01T00:00:00Z', dismissedAt: null, sourceDetector: 'test',
    });
    let releaseFirst: (rows: Finding[]) => void = () => {};
    let findingCalls = 0;
    invoke.mockImplementation((command: string) => {
      if (command === 'list_findings') {
        findingCalls += 1;
        return findingCalls === 1 ? new Promise((resolve) => { releaseFirst = resolve; }) : Promise.resolve([finding('new')]);
      }
      if (command === 'get_casework_state') return Promise.resolve({ cases: [], events: [], leads: [], availableEvidence: [] });
      return Promise.resolve([]);
    });
    render(<CaseTimeline />);
    await waitFor(() => expect(findingCalls).toBe(1));
    act(() => useAppStore.getState().requestFindingsRefresh());
    await screen.findByText('headline new');
    await act(async () => releaseFirst([finding('old')]));
    expect(screen.queryByText('headline old')).toBeNull();
    expect(screen.getByRole('button', { name: 'Open finding' })).toBeTruthy();
  });

  it('keeps the overview wider than the From/To range, and a drag on it sets both inputs', async () => {
    const change = (id: string, at: string) => ({ id, watchId: 'x', dimension: 'DOCUMENTS', changeKind: 'NEW_DOCUMENT', subject: `memo ${id}`, oldValue: null, newValue: 'added', oldAsOf: null, newAsOf: at, basis: 'OBSERVED', detectedAt: at, portalKey: null });
    invoke.mockImplementation((command: string) => {
      if (command === 'list_watch_changes') return Promise.resolve([change('early', '2026-01-10T00:00:00Z'), change('late', '2026-09-10T00:00:00Z')]);
      if (command === 'get_casework_state') return Promise.resolve({ cases: [], events: [], leads: [], availableEvidence: [] });
      return Promise.resolve([]);
    });
    render(<CaseTimeline />);
    await waitFor(() => expect(screen.getAllByRole('img', { name: /memo early/ }).length).toBe(1));
    const [fromInput, toInput] = document.querySelectorAll<HTMLInputElement>('input[type="date"]');
    fireEvent.change(fromInput, { target: { value: '2026-06-01' } });
    // The list drops the early change; the overview still places it, so the range can widen again.
    expect(screen.queryByText(/memo early/i, { selector: 'p' })).toBeNull();
    expect(screen.getByRole('img', { name: /memo early/ })).toBeTruthy();
    const brush = document.querySelector('svg rect[style*="col-resize"]') as SVGRectElement;
    fireEvent.pointerDown(brush, { clientX: 150, pointerId: 1 });
    fireEvent.pointerMove(brush, { clientX: 400, pointerId: 1 });
    fireEvent.pointerUp(brush, { clientX: 400, pointerId: 1 });
    expect(fromInput.value).toMatch(/^2026-0[1-4]-\d{2}$/);
    expect(toInput.value).toMatch(/^2026-\d{2}-\d{2}$/);
    expect(toInput.value > fromInput.value).toBe(true);
  });
});

describe('EvidenceTimeline', () => {
  const item = (id: string, at: string, overrides: Partial<TimelineItem> = {}): TimelineItem => ({ id, source: 'WATCH', at, dateMeaning: 'OCCURRED', title: `item ${id}`, detail: null, provenance: 'watch_changes', evidence: null, findingId: null, ...overrides });
  const base = { caseLanes: [], from: '', to: '', onRange: () => {}, activeLane: null, onLane: () => {}, onOpen: () => {}, highlight: null, today: Date.parse('2026-10-08T00:00:00Z') };

  it('shows a failed source as could not load, a capped one with its limit, a dense lane as weekly cells, and both contradicting dates', () => {
    const dense = Array.from({ length: DENSE_LANE + 1 }, (_, i) => item(`w${i}`, new Date(Date.UTC(2026, 0, 5) + (i % 40) * 86_400_000).toISOString()));
    render(
      <EvidenceTimeline
        {...base}
        items={dense}
        lanes={[{ source: 'FINDING', state: 'failed', cappedAt: null }, { source: 'WATCH', state: 'ready', cappedAt: 200 }]}
        contradictions={[{ id: 'c1', subject: 'Request 26-114 filed', readings: [{ date: '2026-02-01', basis: 'portal record' }, { date: '2026-02-09', basis: 'your entry' }] }]}
        listed={{ items: dense, unreadable: 2 }}
      />,
    );
    expect(document.querySelector('[data-absence="not-recorded"]')?.textContent).toMatch(/could not load$/);
    expect(screen.getByText('first 200 shown')).toBeTruthy();
    // No week is a zero-shaded cell: only weeks that hold items get one, and no item is drawn as its own mark.
    expect(document.querySelectorAll('rect[style*="--viz-seq-"]').length).toBe(6);
    expect(screen.queryByRole('img', { name: /item w0/ })).toBeNull();
    expect(screen.getByRole('img', { name: 'Request 26-114 filed: Feb 1, 2026 (portal record) ≠ Feb 9, 2026 (your entry). Both are shown as recorded.' })).toBeTruthy();
    expect(screen.getByText('2 undated, not placed')).toBeTruthy();
    expect(document.body.innerHTML).not.toContain('--viz-counted');
  });
});
