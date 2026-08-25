// @vitest-environment jsdom
//
// Pins the wiring between the crawl settings dialog and `start_data_source`. The regression this
// guards is subtle: `start_data_source` treats *any* present crawl policy as an explicit recipe
// and stops loading the source's saved connector recipe, so an untouched dialog must send null
// rather than a default-valued object.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';

// See the note in App.test.tsx: Node 26's flag-gated `localStorage` shadows jsdom's, and
// `useAppStore` reads it at module scope.
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

const bridge = vi.hoisted(() => ({ calls: [] as { command: string; args: Record<string, unknown> }[] }));

vi.mock('@tauri-apps/api/core', () => ({
  isTauri: () => false,
  invoke: (command: string, args: Record<string, unknown>) => {
    bridge.calls.push({ command, args });
    return Promise.resolve({
      source: {
        portalKey: 'example-gov',
        displayName: 'Example',
        canonicalUrl: 'https://example.gov',
        sourceKind: 'PUBLIC_WEBSITE',
      },
      syncRunId: 'run-1',
      detectedKind: 'PUBLIC_WEBSITE',
      usedSavedGuidance: false,
    });
  },
}));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: () => Promise.resolve(null) }));

import Header from '../components/dashboard/Header';
import { useAppStore } from '../store/useAppStore';

const DEFAULT_RATE_LIMIT = { burstCapacity: 5, refillTokensPerSecond: 1 };

async function startSource(url: string) {
  fireEvent.change(screen.getByLabelText('Public data source URL'), { target: { value: url } });
  fireEvent.click(screen.getByRole('button', { name: 'Add' }));
  await waitFor(() => expect(bridge.calls).toHaveLength(1));
  return bridge.calls[0];
}

describe('crawl settings', () => {
  beforeEach(() => {
    bridge.calls.length = 0;
    useAppStore.setState({ portalUrlDraft: '', portals: [], syncingPortalKeys: [], syncLogs: [] });
    render(<Header />);
  });

  afterEach(cleanup);

  it('sends no crawl policy while the settings are untouched', async () => {
    const call = await startSource('https://example.gov/records');
    expect(call.command).toBe('start_data_source');
    expect(call.args.crawlPolicy).toBeNull();
    expect(call.args.rateLimitPolicy).toEqual(DEFAULT_RATE_LIMIT);
  });

  it('sends the full policy once a limit is customized', async () => {
    fireEvent.click(screen.getByRole('button', { name: 'Crawl settings' }));
    fireEvent.change(screen.getByLabelText(/Crawl depth/), { target: { value: '16' } });
    fireEvent.click(screen.getByRole('button', { name: 'Done' }));

    const call = await startSource('https://example.gov/records');
    expect(call.args.crawlPolicy).toEqual({
      maximumDepth: 16,
      maximumPages: 5_000,
      maximumResources: 100_000,
      followSupportedExternalPlatforms: true,
    });
    expect(call.args.rateLimitPolicy).toEqual(DEFAULT_RATE_LIMIT);
  });

  it('refuses to start on an out-of-range limit instead of sending it', async () => {
    fireEvent.click(screen.getByRole('button', { name: 'Crawl settings' }));
    fireEvent.change(screen.getByLabelText(/Crawl depth/), { target: { value: '99' } });
    fireEvent.click(screen.getByRole('button', { name: 'Close crawl settings' }));

    fireEvent.change(screen.getByLabelText('Public data source URL'), {
      target: { value: 'https://example.gov/records' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Add' }));

    expect(bridge.calls).toHaveLength(0);
    const alerts = await screen.findAllByRole('alert');
    expect(
      alerts.some((alert) => alert.textContent?.includes('Crawl depth must be between 0 and 32.')),
    ).toBe(true);
  });
});
