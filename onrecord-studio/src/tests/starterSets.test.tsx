// @vitest-environment jsdom
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
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
  started: [] as string[],
  failing: new Set<string>(),
  registry: null as unknown,
}));

vi.mock('@tauri-apps/api/core', () => ({
  isTauri: () => false,
  invoke: (command: string, args: { url?: string }) => {
    if (command === 'list_starter_sets') return Promise.resolve(bridge.registry);
    if (command === 'start_data_source') {
      const url = args.url ?? '';
      bridge.started.push(url);
      if (bridge.failing.has(url)) return Promise.reject(new Error('503 from publisher'));
      return Promise.resolve({
        source: {
          portalKey: `key-${bridge.started.length}`,
          displayName: url,
          canonicalUrl: url,
          originUrl: url,
          apiLayout: { requestsEndpoint: '', requestsPointer: '', metadataPointer: null, pageParameter: '', pageSizeParameter: '' },
          verifiedAt: '2026-09-30T00:00:00Z',
          sourceKind: 'WEB_CRAWL',
        },
        syncRunId: `run-${bridge.started.length}`,
        detectedKind: 'WEB_CRAWL',
        usedSavedGuidance: false,
      });
    }
    return Promise.reject(new Error(`unmocked ${command}`));
  },
}));

vi.mock('../components/charts/AnomalyRadar', () => ({ default: () => <div /> }));
vi.mock('../components/charts/LifecycleFunnel', () => ({ default: () => <div /> }));
vi.mock('../components/charts/PerformanceScatter', () => ({ default: () => <div /> }));
vi.mock('../components/charts/TemporalMap', () => ({ default: () => <div /> }));
vi.mock('../components/charts/AnalyticsUnavailable', () => ({ default: () => <div /> }));
vi.mock('../components/dashboard/MetricsGrid', () => ({ default: () => <div /> }));

import OverviewDashboard from '../components/dashboard/OverviewDashboard';
import StarterSets from '../components/dashboard/StarterSets';
import { useAppStore } from '../store/useAppStore';
import { useStarterRunStore } from '../store/useStarterRunStore';
import type { SavedSource, StarterEntry, StarterSet, StarterSetsReport } from '../types';
import { SAN_DIEGO_SOURCES, sanDiegoSourceForUrl } from '../utils/sanDiego';
import { addStarterEntries, entriesToAdd, entryIsSaved, normalizeStarterUrl } from '../utils/starterSets';

function entry(id: string, url: string): StarterEntry {
  return { id, kind: 'SOURCE', title: `Title ${id}`, url, what: `What ${id}.`, publisher: 'Publisher', termsNote: null, verifiedAsOf: '2026-09-30' };
}

function saved(url: string): SavedSource {
  return {
    registration: {
      portalKey: url,
      displayName: url,
      canonicalUrl: url,
      originUrl: url,
      apiLayout: { requestsEndpoint: '', requestsPointer: '', metadataPointer: null, pageParameter: '', pageSizeParameter: '' },
      verifiedAt: '2026-09-30T00:00:00Z',
      sourceKind: 'WEB_CRAWL',
    },
    lifecycleState: 'ACTIVE',
  } as SavedSource;
}

// Two real preset URLs, so the add path resolves their crawl policy exactly as a hand-chosen
// preset would.
const FIRST = 'https://www.sandiego.gov/iba/reports';
const SECOND = 'https://www.sandiego.gov/auditor/reports';
const SET: StarterSet = { id: 'test-set', title: 'Test set', summary: 'Summary.', entries: [entry('sd-iba', FIRST), entry('sd-auditor-reports', SECOND)] };

beforeEach(() => {
  bridge.started = [];
  bridge.failing = new Set();
  bridge.registry = { registryVersion: 1, sets: [SET] } satisfies StarterSetsReport;
  useAppStore.setState({ portals: [], workspaceInitializationStatus: 'ready', syncLogs: [] } as never);
  useStarterRunStore.setState({ runningSetId: null, statuses: {} });
});

afterEach(cleanup);

describe('shipped registry', () => {
  const source = readFileSync(join(import.meta.dirname, '../../src-tauri/src/ingestion/starter_sets.rs'), 'utf8');
  const shipped = [...source.matchAll(/entry\(\s*"([^"]+)",\s*(?:Source|RecordRequestTarget),\s*"[^"]*",\s*"([^"]+)"/g)].map((m) => ({ id: m[1], url: m[2] }));

  it('reads every entry', () => {
    expect(shipped.length).toBeGreaterThan(10);
    // A literal the parse above missed would otherwise escape every check below.
    const registryText = source.slice(0, source.indexOf('#[cfg(test)]'));
    expect(shipped.length).toBe((registryText.match(/^\s+entry\(/gm) ?? []).length);
  });

  it('names only https built-in presets, by id and address, with no duplicates', () => {
    const ids = new Set<string>();
    const urls = new Set<string>();
    for (const { id, url } of shipped) {
      expect(url.startsWith('https://'), id).toBe(true);
      const preset = SAN_DIEGO_SOURCES.find((candidate) => candidate.id === id);
      expect(preset, `${id} has no preset`).toBeDefined();
      expect(sanDiegoSourceForUrl(url)?.id, `${id} url does not resolve to its preset`).toBe(id);
      expect(preset?.access ?? 'public', `${id} is export-only and cannot be fetched`).toBe('public');
      expect(ids.has(id), `duplicate id ${id}`).toBe(false);
      expect(urls.has(normalizeStarterUrl(url)), `duplicate url ${url}`).toBe(false);
      ids.add(id);
      urls.add(normalizeStarterUrl(url));
    }
  });
});

describe('re-adding', () => {
  it('skips a source already saved under its address, with or without a trailing slash', () => {
    const library = [saved(`${FIRST}/`)];
    expect(entryIsSaved(SET.entries[0], library)).toBe(true);
    expect(entriesToAdd(SET, library).map((e) => e.id)).toEqual(['sd-auditor-reports']);
    expect(entriesToAdd(SET, [...library, saved(SECOND)])).toEqual([]);
  });
});

describe('addStarterEntries', () => {
  it('marks a failing entry unavailable with its reason and still runs the rest', async () => {
    const results: string[] = [];
    await addStarterEntries(SET.entries, {
      isCurrent: () => true,
      start: async (e) => {
        if (e.id === 'sd-iba') throw new Error('503 from publisher');
        return { source: saved(e.url).registration, syncRunId: 'r', detectedKind: 'WEB_CRAWL', usedSavedGuidance: false };
      },
      onResult: (e, outcome) => results.push(`${e.id}:${outcome.state}:${outcome.state === 'unavailable' ? outcome.reason : ''}`),
    });
    expect(results).toEqual(['sd-iba:unavailable:503 from publisher', 'sd-auditor-reports:started:']);
  });

  it('starts nothing further once the run is no longer current', async () => {
    let current = true;
    const started: string[] = [];
    await addStarterEntries(SET.entries, {
      isCurrent: () => current,
      start: async (e) => {
        started.push(e.id);
        current = false;
        return { source: saved(e.url).registration, syncRunId: 'r', detectedKind: 'WEB_CRAWL', usedSavedGuidance: false };
      },
      onResult: () => undefined,
    });
    expect(started).toEqual(['sd-iba']);
  });
});

describe('StarterSets panel', () => {
  it('fetches nothing on render or on choosing a set, and names the sites before anything starts', async () => {
    render(<StarterSets />);
    const add = await screen.findByRole('button', { name: /Add 2 sources/ });
    expect(bridge.started).toEqual([]);
    fireEvent.click(add);
    expect(bridge.started).toEqual([]);
    expect(screen.getByText(/contacts www\.sandiego\.gov/)).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(bridge.started).toEqual([]);
  });

  it('adds on confirmation, reports a failed source as unavailable, and a second add starts only what is missing', async () => {
    bridge.failing.add(SECOND);
    render(<StarterSets />);
    fireEvent.click(await screen.findByRole('button', { name: /Add 2 sources/ }));
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Fetch 2 sources' }));
    });
    await waitFor(() => expect(screen.getByRole('alert').textContent).toContain('Unavailable: 503 from publisher'));
    expect(bridge.started).toEqual([FIRST, SECOND]);
    expect(useAppStore.getState().portals.map((p) => p.registration.canonicalUrl)).toEqual([FIRST]);

    bridge.failing.clear();
    fireEvent.click(await screen.findByRole('button', { name: /Add 1 source/ }));
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Fetch 1 source' }));
    });
    await waitFor(() => expect(bridge.started).toEqual([FIRST, SECOND, SECOND]));
    await screen.findByText('Every source in this set is already in your library.');
  });

  it('does not keep saying Started for a source that is no longer in the library', async () => {
    useStarterRunStore.setState({
      statuses: { 'sd-iba': { state: 'started', source: saved(FIRST).registration } },
    });
    render(<StarterSets />);
    await screen.findByRole('button', { name: /Add 2 sources/ });
    expect(screen.queryByText('Started')).toBeNull();
    act(() => useAppStore.setState({ portals: [saved(FIRST)] } as never));
    expect(screen.getByText('Started')).toBeTruthy();
  });
});

describe('first-run empty state', () => {
  it('keeps adding after the first source unmounts the empty-state panel, and keeps the failure visible', async () => {
    const THIRD = 'https://www.sandiego.gov/cpp/reports';
    bridge.registry = {
      registryVersion: 1,
      sets: [{ ...SET, entries: [...SET.entries, entry('sd-cpp-resources', THIRD)] }],
    } satisfies StarterSetsReport;
    bridge.failing.add(SECOND);
    render(<OverviewDashboard />);
    fireEvent.click(await screen.findByRole('button', { name: /Add 3 sources/ }));
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Fetch 3 sources' }));
    });
    await waitFor(() => expect(bridge.started).toEqual([FIRST, SECOND, THIRD]));
    await waitFor(() => expect(useStarterRunStore.getState().runningSetId).toBeNull());
    expect(useAppStore.getState().portals.map((p) => p.registration.canonicalUrl)).toEqual([FIRST, THIRD].sort());
    expect(await screen.findByText('Unavailable: 503 from publisher')).toBeTruthy();
  });
});
