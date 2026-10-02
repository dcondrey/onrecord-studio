// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, renderHook, waitFor } from '@testing-library/react';

const pending = new Map<string, (rows: unknown[]) => void>();

vi.mock('../ipc', () => ({
  formatNativeError: String,
  invokeCommand: (command: string, args: { searchTerm?: string; query?: string }) => {
    if (command === 'query_audit_records_page') {
      return new Promise((resolve) => pending.set(args.searchTerm!, (rows) => resolve({ records: rows, nextCursor: null, total: rows.length })));
    }
    return Promise.resolve([]);
  },
}));

import { useObjectSearch } from '../components/dashboard/useObjectSearch';

const record = (title: string) => ({ requestId: 1, portalKey: 'p', agencyId: null, agencyName: 'Agency', title, descriptionPreview: '', status: 'Open', createdAt: '', closedAt: null, documentCount: 0, integrityFlags: [] });

afterEach(() => {
  cleanup();
  pending.clear();
});

describe('useObjectSearch', () => {
  it('drops results for a query the user has typed past', async () => {
    const { result, rerender } = renderHook(({ query }) => useObjectSearch(query), { initialProps: { query: 'shel' } });
    await waitFor(() => expect(pending.has('shel')).toBe(true));
    rerender({ query: 'shelter' });
    await waitFor(() => expect(pending.has('shelter')).toBe(true));
    await act(async () => pending.get('shelter')!([record('Shelter beds')]));
    await waitFor(() => expect(result.current.map((row) => row.label)).toEqual(['Shelter beds']));
    await act(async () => pending.get('shel')!([record('Stale result')]));
    expect(result.current.map((row) => row.label)).toEqual(['Shelter beds']);
  });
});
