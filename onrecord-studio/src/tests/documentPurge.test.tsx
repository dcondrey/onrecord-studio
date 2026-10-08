// @vitest-environment jsdom
//
// Per-document deletion. The failure that matters is not the delete itself but its outcome landing
// on the wrong record: a purge started while request 42 was open and resolving after request 99 is
// selected would report a deletion, or a deletion failure, about evidence it never touched.
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
  calls: [] as { command: string; args: Record<string, unknown> }[],
  releasePurge: null as ((value: unknown) => void) | null,
  rejectPurge: null as ((reason: unknown) => void) | null,
  holdPurge: false,
  releaseChunk: null as ((value: unknown) => void) | null,
}));

vi.mock('@tauri-apps/api/core', () => ({
  isTauri: () => false,
  invoke: (command: string, args: Record<string, unknown>) => {
    bridge.calls.push({ command, args });
    // MainLayout mounts SearchFilter, whose debounced query overwrites `filteredResults` with
    // whatever the bridge returns. Returning [] here empties the record list mid-test and takes
    // the document panel with it, which is a fixture problem rather than a product one.
    // A request with no recorded statutory extension. The command returns the record or
    // null, never a collection, and the panel reads its fields.
    if (command === 'get_request_statutory_extension') return Promise.resolve(null);
    if (command === 'query_audit_records_page') return Promise.resolve({ records: [recordFor(42), recordFor(99)], nextCursor: null, total: 2 });
    if (command === 'list_request_documents') {
      const requestId = Number(args.requestId);
      return Promise.resolve(requestId === 42
        ? [
          documentSummary(1, 'permit-log.pdf'),
          documentSummary(2, 'inspection-notes.pdf'),
        ]
        : [documentSummary(9, 'other-record.pdf')]);
    }
    if (command === 'get_document_text_chunk') {
      return new Promise((resolve) => { bridge.releaseChunk = resolve; });
    }
    if (command === 'purge_document') {
      if (bridge.holdPurge) {
        return new Promise((resolve, reject) => {
          bridge.releasePurge = resolve;
          bridge.rejectPurge = reject;
        });
      }
      return Promise.resolve({
        documentId: Number(args.documentId),
        sourceKey: String(args.sourceKey),
        localPath: '/var/data/example-gov/permit-log.pdf',
        searchRowsRemoved: 4,
        provenanceRowsRemoved: 1,
      });
    }
    return Promise.resolve([]);
  },
}));

import MainLayout from '../components/dashboard/MainLayout';
import { useAppStore } from '../store/useAppStore';
import type { RequestDocumentSummary, RequestSearchResult } from '../types';

function documentSummary(documentId: number, fileName: string): RequestDocumentSummary {
  return {
    documentId,
    portalKey: 'example-gov',
    fileName,
    fileSize: 2048,
    contentType: 'application/pdf',
    visibility: 'public',
    downloadStatus: 'DOWNLOADED',
    rawFileSha256: null,
    extractedTextSha256: null,
    extractionStatus: 'SEARCHABLE',
    extractionFormat: 'pdf',
    extractionDeferredReason: null,
  };
}

function recordFor(requestId: number): RequestSearchResult {
  return {
    requestId,
    portalKey: 'example-gov',
    agencyId: 7,
    agencyName: 'Public Works',
    title: `Record ${requestId}`,
    descriptionPreview: 'Responsive records',
    status: 'closed',
    createdAt: '2026-08-01T00:00:00Z',
    closedAt: '2026-08-10T00:00:00Z',
    documentCount: 2,
    integrityFlags: [],
  };
}

function mount(selectedRequestId: number, syncingPortalKeys: string[] = []) {
  useAppStore.setState({
    syncingPortalKeys,
    workspaceView: 'evidence',
    activePortalKey: null,
    searchTerm: '',
    selectedAgencyId: null,
    selectedStatus: null,
    selectedPolicyDeviation: null,
    filteredResults: [recordFor(42), recordFor(99)],
    searchError: null,
    selectedRequestId,
    selectedRequestPortalKey: 'example-gov',
    activeLegalFindings: [],
    agencyProfiles: [],
  });
  render(<MainLayout />);
}

describe('document purge', () => {
  beforeEach(() => {
    bridge.calls.length = 0;
    bridge.holdPurge = false;
    bridge.releasePurge = null;
    bridge.rejectPurge = null;
    bridge.releaseChunk = null;
  });

  afterEach(cleanup);

  it('confirms before deleting, then removes only the document it deleted', async () => {
    mount(42);
    await screen.findByRole('button', { name: 'Delete document permit-log.pdf' });

    fireEvent.click(screen.getByRole('button', { name: 'Delete document permit-log.pdf' }));
    expect(bridge.calls.some((call) => call.command === 'purge_document')).toBe(false);
    expect(screen.getByRole('heading', { name: 'Delete permit-log.pdf?' })).toBeTruthy();

    fireEvent.click(screen.getByRole('button', { name: 'Delete this document' }));
    await waitFor(() => expect(bridge.calls.some((call) => call.command === 'purge_document')).toBe(true));
    expect(bridge.calls.find((call) => call.command === 'purge_document')?.args).toEqual({
      sourceKey: 'example-gov',
      documentId: 1,
    });
    await waitFor(() => expect(screen.queryByText('permit-log.pdf')).toBeNull());
    expect(screen.getByText('inspection-notes.pdf')).toBeTruthy();
  });

  // A purge while the crawl runs is a delete that undoes itself: the frontier is rebuilt per run
  // and nothing records that the document was destroyed, so the same file comes back inside the
  // same sync. Declining is the honest outcome; the tooltip says what to do instead.
  it('refuses to delete a document while its source is still synchronizing', async () => {
    mount(42, ['example-gov']);
    const control = await screen.findByRole('button', { name: 'Delete document permit-log.pdf' });

    expect(control).toHaveProperty('disabled', true);
    expect(control.getAttribute('title')).toContain('Pause this source');
    fireEvent.click(control);
    expect(screen.queryByRole('heading', { name: 'Delete permit-log.pdf?' })).toBeNull();
    expect(bridge.calls.some((call) => call.command === 'purge_document')).toBe(false);
  });

  // Proven by deleting the `documentOwnerRef.current?.requestId !== owner?.requestId` check from
  // the catch arm of `purgeDocument` in MainLayout.tsx: request 42's failure then renders as
  // "Document view unavailable" under record 99, which never had that document.
  it('does not report a purge failure from one record against another', async () => {
    bridge.holdPurge = true;
    mount(42);
    await screen.findByRole('button', { name: 'Delete document permit-log.pdf' });
    fireEvent.click(screen.getByRole('button', { name: 'Delete document permit-log.pdf' }));
    fireEvent.click(screen.getByRole('button', { name: 'Delete this document' }));
    await waitFor(() => expect(bridge.rejectPurge).not.toBeNull());

    act(() => useAppStore.setState({ selectedRequestId: 99 }));
    await screen.findByText('other-record.pdf');

    act(() => bridge.rejectPurge?.(new Error('evidence store is locked')));
    await waitFor(() => expect(screen.getByText('other-record.pdf')).toBeTruthy());
    expect(screen.queryByText(/evidence store is locked/)).toBeNull();
  });

  // Proven by deleting the `documentOpenGate.isCurrent(request)` check from the success arm of
  // `openDocument` in MainLayout.tsx: record 42's text then opens in a viewer over record 99.
  it('opens the viewer at once on a loading state, and drops a chunk that lands after the record changed', async () => {
    mount(42);
    fireEvent.click(await screen.findByRole('button', { name: /^permit-log\.pdf/ }));

    const viewer = screen.getByRole('dialog');
    expect(viewer.textContent).toContain('permit-log.pdf');
    expect(screen.getByText('Reading the extracted text').closest('[aria-busy="true"]')).not.toBeNull();
    await waitFor(() => expect(bridge.releaseChunk).not.toBeNull());

    act(() => useAppStore.setState({ selectedRequestId: 99 }));
    await screen.findByText('other-record.pdf');
    expect(screen.queryByRole('dialog')).toBeNull();

    await act(async () => bridge.releaseChunk?.({
      maskedText: 'TEXT OF RECORD 42',
      characterOffset: 0,
      returnedCharacterCount: 17,
      totalCharacterCount: 17,
      nextCharacterOffset: null,
      maskedFindingCount: 0,
    }));
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(screen.queryByText('TEXT OF RECORD 42')).toBeNull();
  });
});
