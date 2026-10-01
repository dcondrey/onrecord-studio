// @vitest-environment jsdom
//
// Search shows the exact number of matches and reaches every one of them by paging. A page that
// belongs to a query the reader has since replaced must never be appended to the new results: the
// list would then present rows the new query never matched as its own matches.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen, waitFor } from '@testing-library/react';

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
  held: new Map<string, (value: unknown) => void>(),
  calls: [] as { command: string; args: Record<string, unknown> }[],
}));

function record(id: number, title: string) {
  return {
    requestId: id,
    portalKey: 'example-gov',
    agencyId: 7,
    agencyName: 'Public Works',
    title,
    descriptionPreview: 'Responsive records',
    status: 'closed',
    createdAt: '2026-08-01T00:00:00Z',
    closedAt: null,
    documentCount: 0,
    integrityFlags: [],
  };
}

vi.mock('@tauri-apps/api/core', () => ({
  isTauri: () => false,
  invoke: (command: string, args: Record<string, unknown> = {}) => {
    bridge.calls.push({ command, args });
    if (command === 'query_audit_records_page') {
      if (args.cursor) return new Promise((resolve) => bridge.held.set(String(args.cursor), resolve));
      return Promise.resolve(
        args.searchTerm === 'zebra'
          ? { records: [record(900, 'Zebra request')], nextCursor: null, total: 1 }
          : { records: [record(1, 'First request'), record(2, 'Second request')], nextCursor: 'c1', total: 12431 },
      );
    }
    if (command === 'count_audit_records') return Promise.resolve({ total: 3 });
    if (command === 'get_request_statutory_extension') return Promise.resolve(null);
    if (command === 'get_affiliation_disclosure') {
      return Promise.resolve({ status: 'UNSET', sentence: null, affiliations: [], ownPortalKeys: [], ownAgencies: [], ownEntityIds: [] });
    }
    if (command === 'get_profile') return Promise.resolve({ status: 'UNSET', profile: null });
    return Promise.resolve([]);
  },
}));

import MainLayout from '../components/dashboard/MainLayout';
import { useAppStore } from '../store/useAppStore';

beforeEach(() => {
  bridge.held.clear();
  bridge.calls.length = 0;
  useAppStore.setState({
    syncingPortalKeys: [],
    workspaceView: 'evidence',
    activePortalKey: null,
    searchTerm: '',
    selectedAgencyId: null,
    selectedStatus: null,
    selectedPolicyDeviation: null,
    filteredResults: [],
    filteredResultsTotal: 0,
    searchPaging: null,
    searchError: null,
    selectedRequestId: null,
    selectedRequestPortalKey: null,
    activeLegalFindings: [],
    agencyProfiles: [],
  });
});
afterEach(cleanup);

describe('search paging', () => {
  it('shows the exact total, not the rows loaded, and appends the next page by cursor', async () => {
    render(<MainLayout />);
    await screen.findByText('First request');
    expect((await screen.findAllByText('12,431')).length).toBeGreaterThan(0);
    expect(screen.getByText(/Showing 2 of 12,431 matches/)).toBeTruthy();
    expect(screen.queryByText(/500\+/)).toBeNull();

    // The window is at the end of the loaded rows, so the next page is requested by its cursor.
    await waitFor(() => expect(bridge.held.has('c1')).toBe(true));
    expect(bridge.calls.find((call) => call.args.cursor === 'c1')?.args).toMatchObject({ searchTerm: '', pageSize: 200 });
    bridge.held.get('c1')?.({ records: [record(3, 'Third request')], nextCursor: null, total: null });
    await screen.findByText('Third request');
    // The list ended at 3 rows while the first count said 12,431: the count is re-read rather than
    // the first total being kept as if it still described the corpus.
    await waitFor(() => expect(bridge.calls.some((call) => call.command === 'count_audit_records')).toBe(true));
    await waitFor(() => expect(useAppStore.getState().filteredResultsTotal).toBe(3));
    expect(screen.queryByText(/The records changed/)).toBeNull();
    expect(screen.queryByText(/Showing 3 of/)).toBeNull();
  });

  it('drops a page that lands after a different query replaced the list', async () => {
    render(<MainLayout />);
    await screen.findByText('First request');
    await waitFor(() => expect(bridge.held.has('c1')).toBe(true));

    useAppStore.getState().setSearchFilters('zebra', null, null, null);
    await screen.findByText('Zebra request');

    bridge.held.get('c1')?.({ records: [record(3, 'STALE ROW')], nextCursor: null, total: null });
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(screen.queryByText('STALE ROW')).toBeNull();
    expect(useAppStore.getState().filteredResults.map((result) => result.requestId)).toEqual([900]);
  });
});
