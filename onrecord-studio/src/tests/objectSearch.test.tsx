// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, renderHook, waitFor } from '@testing-library/react';

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
      get length() { return entries.size; },
    },
  });
});


const pending = new Map<string, (rows: unknown[]) => void>();
const ipc = { holdRecords: true, findingsFailures: 0 };
const finding = { id: 'f1', headline: 'Budget memo withheld', severity: 'HIGH', kind: 'WITHHOLDING', dismissedAt: null };

vi.mock('../ipc', () => ({
  formatNativeError: String,
  invokeCommand: (command: string, args: { searchTerm?: string; query?: string }) => {
    if (command === 'query_audit_records_page') {
      if (!ipc.holdRecords) return Promise.resolve({ records: [], nextCursor: null, total: 0 });
      return new Promise((resolve) => pending.set(args.searchTerm!, (rows) => resolve({ records: rows, nextCursor: null, total: rows.length })));
    }
    if (command === 'list_findings') {
      if (ipc.findingsFailures > 0) {
        ipc.findingsFailures -= 1;
        return Promise.reject(new Error('busy'));
      }
      return Promise.resolve([finding]);
    }
    return Promise.resolve([]);
  },
}));

import { useObjectSearch } from '../components/dashboard/useObjectSearch';

const record = (title: string) => ({ requestId: 1, portalKey: 'p', agencyId: null, agencyName: 'Agency', title, descriptionPreview: '', status: 'Open', createdAt: '', closedAt: null, documentCount: 0, integrityFlags: [] });

afterEach(() => {
  cleanup();
  pending.clear();
  ipc.holdRecords = true;
  ipc.findingsFailures = 0;
});

describe('useObjectSearch', () => {
  it('drops results for a query the user has typed past', async () => {
    const { result, rerender } = renderHook(({ query }) => useObjectSearch(query), { initialProps: { query: 'shel' } });
    await waitFor(() => expect(pending.has('shel')).toBe(true));
    rerender({ query: 'shelter' });
    await waitFor(() => expect(pending.has('shelter')).toBe(true));
    await act(async () => pending.get('shelter')!([record('Shelter beds')]));
    await waitFor(() => expect(result.current.results.map((row) => row.label)).toEqual(['Shelter beds']));
    await act(async () => pending.get('shel')!([record('Stale result')]));
    expect(result.current.results.map((row) => row.label)).toEqual(['Shelter beds']);
  });

  it('retries a failed snapshot read instead of holding it as an empty corpus', async () => {
    ipc.holdRecords = false;
    ipc.findingsFailures = 1;
    const { result, rerender } = renderHook(({ query }) => useObjectSearch(query), { initialProps: { query: 'budget' } });
    await waitFor(() => expect(ipc.findingsFailures).toBe(0));
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(result.current.results).toEqual([]);
    rerender({ query: 'budget memo' });
    await waitFor(() => expect(result.current.results.map((row) => row.label)).toEqual(['Budget memo withheld']));
  });
});
