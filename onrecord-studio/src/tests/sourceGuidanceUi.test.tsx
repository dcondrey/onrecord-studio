// @vitest-environment jsdom
//
// Exercises the only UI path that persists source-scoping rules: automatic discovery fails,
// guided repair opens, and the operator supplies filters with the seed URL.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';

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
  calls: [] as { command: string; args: Record<string, unknown> }[],
}));

vi.mock('@tauri-apps/api/core', () => ({
  isTauri: () => false,
  invoke: (command: string, args: Record<string, unknown>) => {
    bridge.calls.push({ command, args });
    if (command === 'start_data_source' && args.guidance === null) {
      return Promise.reject(new Error('automatic discovery failed'));
    }
    if (command === 'preview_data_source') {
      return Promise.resolve({
        source: {
          portalKey: 'example-gov',
          displayName: 'Example',
          canonicalUrl: 'https://example.gov',
          sourceKind: 'WEB_CRAWL',
        },
        detectedKind: 'WEB_CRAWL',
        confidenceBasisPoints: 8_500,
        candidateLinks: [
          {
            url: 'https://cdn.example.gov/records/',
            label: 'Records',
            downloadCandidate: true,
            followCandidate: true,
          },
        ],
        warnings: ['A guided seed is required.'],
        repairAvailable: true,
        selectorPreviewApplied: false,
      });
    }
    return Promise.resolve({
      source: {
        portalKey: 'example-gov',
        displayName: 'Example',
        canonicalUrl: 'https://example.gov',
        sourceKind: 'WEB_CRAWL',
      },
      syncRunId: 'run-guided',
      detectedKind: 'WEB_CRAWL',
      usedSavedGuidance: false,
    });
  },
}));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: () => Promise.resolve(null) }));

import Header from '../components/dashboard/Header';
import { useAppStore } from '../store/useAppStore';

describe('guided source scoping', () => {
  beforeEach(() => {
    bridge.calls.length = 0;
    useAppStore.setState({ portalUrlDraft: '', portals: [], syncingPortalKeys: [], syncLogs: [] });
    render(<Header />);
  });

  afterEach(cleanup);

  it('sends validated source filters with the guided connector recipe', async () => {
    fireEvent.change(screen.getByLabelText('Public data source URL'), {
      target: { value: 'https://example.gov/records' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Add' }));

    await screen.findByRole('dialog', { name: /Show OnRecord where the data lives/ });
    fireEvent.click(screen.getByText('Scope imported documents'));
    fireEvent.change(screen.getByLabelText(/^Filename include globs/), {
      target: { value: '*.pdf, *.docx' },
    });
    fireEvent.change(screen.getByLabelText(/^Content exclude patterns/), {
      target: { value: String.raw`\bSSN\b` },
    });
    fireEvent.change(screen.getByLabelText(/^Organization include/), {
      target: { value: 'Parks, Public Works' },
    });
    fireEvent.change(screen.getByLabelText(/^Minimum file size \(bytes\)/), {
      target: { value: '1024' },
    });
    fireEvent.change(screen.getByLabelText(/^Document date from/), {
      target: { value: '2024-01-01' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Use this data source' }));

    await waitFor(() => {
      expect(bridge.calls.filter((call) => call.command === 'start_data_source')).toHaveLength(2);
    });
    const guidedCall = bridge.calls.filter((call) => call.command === 'start_data_source')[1];
    expect(guidedCall.args.guidance).toMatchObject({
      additionalSeedUrls: ['https://cdn.example.gov/records/'],
      includeFilenameGlobs: ['*.pdf', '*.docx'],
      excludeContentPatterns: [String.raw`\bSSN\b`],
      includeOrganizations: ['Parks', 'Public Works'],
      minimumFileSizeBytes: 1024,
      maximumFileSizeBytes: null,
      documentDateFrom: '2024-01-01',
      documentDateTo: null,
    });
  });
});
