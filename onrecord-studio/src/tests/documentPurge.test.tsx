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
}));

vi.mock('@tauri-apps/api/core', () => ({
  isTauri: () => false,
  invoke: (command: string, args: Record<string, unknown>) => {
    bridge.calls.push({ command, args });
    // MainLayout mounts SearchFilter, whose debounced query overwrites `filteredResults` with
    // whatever the bridge returns. Returning [] here empties the record list mid-test and takes
    // the document panel with it, which is a fixture problem rather than a product one.
    if (command === 'query_audit_records') return Promise.resolve([recordFor(42), recordFor(99)]);
    if (command === 'list_request_documents') {
      const requestId = Number(args.requestId);
      return Promise.resolve(requestId === 42
        ? [
          documentSummary(1, 'permit-log.pdf'),
          documentSummary(2, 'inspection-notes.pdf'),
        ]
        : [documentSummary(9, 'other-record.pdf')]);
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

function mount(selectedRequestId: number) {
  useAppStore.setState({
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
});
