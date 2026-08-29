// @vitest-environment jsdom
//
// A recorded statutory extension moves the determination deadline the evaluator applies, so the
// recording surface has two obligations these tests pin. It must never present a failed read as
// "no extension recorded", because the form beside that sentence offers to replace evidence it
// never saw. And a read issued for one request must never resolve onto another.
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

const extensionResponses: Array<{
  resolve: (value: unknown) => void;
  reject: (reason: unknown) => void;
}> = [];

vi.mock('@tauri-apps/api/core', () => ({
  isTauri: () => false,
  invoke: (command: string) => {
    if (command === 'get_request_statutory_extension') {
      return new Promise((resolve, reject) => {
        extensionResponses.push({ resolve, reject });
      });
    }
    if (command === 'list_request_legal_findings') return Promise.resolve([]);
    return Promise.resolve([]);
  },
}));

import LegalCompliancePanel from '../components/legal/LegalCompliancePanel';
import { useAppStore } from '../store/useAppStore';
import type { RequestSearchResult, StatutoryExtensionRecord } from '../types';

function requestRecord(requestId: number, title: string): RequestSearchResult {
  return {
    requestId,
    portalKey: 'example-gov',
    agencyId: 7,
    agencyName: 'Public Works',
    title,
    descriptionPreview: 'Responsive records',
    status: 'closed',
    createdAt: '2026-08-01T00:00:00Z',
    closedAt: '2026-08-10T00:00:00Z',
    documentCount: 2,
    integrityFlags: [],
  };
}

const firstRecordExtension: StatutoryExtensionRecord = {
  noticeAt: '2026-08-05T00:00:00.000Z',
  expectedDeterminationAt: '2026-08-19T00:00:00.000Z',
  reason: 'VOLUMINOUS_RECORDS',
  noticeSource: 'Agency letter for the first request',
};

describe('statutory extension recording', () => {
  beforeEach(() => {
    extensionResponses.length = 0;
    useAppStore.setState({ activeLegalFindings: [], legalAuthorities: [] });
  });

  afterEach(() => {
    cleanup();
    vi.clearAllMocks();
  });

  it('reports a failed read as unavailable rather than as no extension', async () => {
    render(<LegalCompliancePanel record={requestRecord(42, 'First')} onOpenLibrary={() => {}} />);
    await waitFor(() => expect(extensionResponses).toHaveLength(1));
    extensionResponses[0].reject(new Error('database busy'));

    await waitFor(() =>
      expect(screen.getByText(/recorded extension could not be read/i)).toBeTruthy(),
    );
    expect(screen.queryByText(/No extension recorded/i)).toBeNull();
    // The recording control is the danger: the write is an upsert, so offering it here would let
    // an operator replace a notice the panel failed to read.
    expect(screen.queryByRole('button', { name: /Record notice|Replace notice/i })).toBeNull();
  });

  it('discards an extension read that resolves after the viewer moved to another request', async () => {
    const { rerender } = render(
      <LegalCompliancePanel record={requestRecord(42, 'First')} onOpenLibrary={() => {}} />,
    );
    await waitFor(() => expect(extensionResponses).toHaveLength(1));

    rerender(
      <LegalCompliancePanel record={requestRecord(43, 'Second')} onOpenLibrary={() => {}} />,
    );
    await waitFor(() => expect(extensionResponses).toHaveLength(2));

    // Only the superseded read resolves. The read for request 43 is left pending on purpose: if
    // both resolved, the second would overwrite the first and the assertion would hold whether or
    // not the gate exists. With it pending, request 42's extension is the only value that can
    // reach the panel, so publishing it is observable.
    extensionResponses[0].resolve(firstRecordExtension);
    await Promise.resolve();

    await waitFor(() => expect(screen.getByText(/Reading recorded extension/i)).toBeTruthy());
    expect(screen.queryByText(/Agency letter for the first request/)).toBeNull();
    expect(screen.queryByText(/No extension recorded/i)).toBeNull();
  });
});
