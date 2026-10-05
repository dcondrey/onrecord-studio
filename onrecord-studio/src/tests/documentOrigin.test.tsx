// @vitest-environment jsdom
//
// "Added by you, not downloaded from an agency source" is a statement about one file, and so are
// the collected files it matches. An origin report requested for document A that resolves after
// the viewer moved to document B would label B as hand-added and list A's matches under it. This
// proves the `originGate` guard in DocumentViewer.tsx (see CLAUDE.md's async-identity-gate rule),
// and that a comparison which did not run is never rendered as one that found nothing.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen } from '@testing-library/react';
import DocumentViewer from '../components/documents/DocumentViewer';
import type { DocumentInternalMetadata, DocumentOriginReport } from '../types';

const bridge = vi.hoisted(() => ({
  reports: new Map<number, unknown>(),
  metadata: new Map<number, unknown>(),
  gates: new Map<number, Promise<void>>(),
}));

vi.mock('../ipc', () => ({
  invokeCommand: (command: string, args?: { documentId?: number }) => {
    const documentId = args?.documentId ?? 0;
    if (command === 'verify_document_provenance') return Promise.resolve(null);
    if (command === 'get_document_entity_truncations') return Promise.resolve([]);
    if (command === 'describe_document_origin') {
      if (!bridge.reports.has(documentId)) return Promise.reject(new Error('core unavailable'));
      const answer = bridge.reports.get(documentId);
      const gate = bridge.gates.get(documentId);
      return gate ? gate.then(() => answer) : Promise.resolve(answer);
    }
    if (command === 'read_document_internal_metadata') return Promise.resolve(bridge.metadata.get(documentId));
    return Promise.reject(new Error(`unexpected command ${command}`));
  },
}));

const held = (documentId: number) => ({
  portalKey: 'city', sourceName: 'City records portal', requestId: 7, requestLabel: '24-100', documentId, fileName: `release-${documentId}.pdf`,
});

function added(documentId: number, overrides: Partial<NonNullable<DocumentOriginReport['heldRelation']>> = {}): DocumentOriginReport {
  return {
    documentId,
    portalKey: 'example',
    origin: 'ADDED_BY_USER',
    sourceKind: 'LOCAL_FILE',
    heldRelation: {
      exactCopies: { state: 'COMPARED', matches: [held(documentId * 10)], truncated: false, comparedCount: 40 },
      sameText: { state: 'COMPARED', matches: [], truncated: false, comparedCount: 38 },
      nearDuplicates: { state: 'NOT_COMPARED', reason: 'NOT_IN_SIMILARITY_INDEX' },
      ...overrides,
    },
  };
}

const collected = (documentId: number): DocumentOriginReport => ({
  documentId, portalKey: 'example', origin: 'COLLECTED_FROM_SOURCE', sourceKind: 'NEXT_REQUEST', heldRelation: null,
});

const viewer = (documentId: number) => (
  <DocumentViewer documentId={documentId} portalKey="example" fileName="memo.pdf" textContent="" fileSizeFormatted="1 KB" originalAvailable={false} />
);

afterEach(() => {
  cleanup();
  bridge.reports.clear();
  bridge.metadata.clear();
  bridge.gates.clear();
});

describe('document origin', () => {
  it('discards an origin report that resolves after the viewer moved to another document', async () => {
    bridge.reports.set(7, added(7));
    bridge.reports.set(8, collected(8));
    let release = () => {};
    bridge.gates.set(7, new Promise<void>((resolve) => { release = () => resolve(); }));

    const view = render(viewer(7));
    view.rerender(viewer(8));
    release();
    await new Promise((resolve) => setTimeout(resolve, 20));

    // Document 8 was collected from a source. Document 7's label and match must not appear on it.
    expect(screen.queryByLabelText('File origin')).toBeNull();
    expect(screen.queryByText(/Added by you/)).toBeNull();
    expect(screen.queryByText(/release-70\.pdf/)).toBeNull();
  });

  it('labels a hand-added file and tells a comparison that did not run from one that found nothing', async () => {
    bridge.reports.set(7, added(7));
    const metadata: DocumentInternalMetadata = {
      state: 'READ',
      format: 'PDF',
      fields: [{ name: 'Info Producer', value: 'Acme PDF 2.1' }, { name: 'Info CreationDate', value: null }],
    };
    bridge.metadata.set(7, metadata);
    render(viewer(7));

    const panel = await screen.findByLabelText('File origin');
    expect(panel.textContent).toContain('Added by you from a local file. Not downloaded from an agency source.');
    expect(panel.textContent).toContain('It does not say this file is authentic, and no match does not say it is fabricated.');
    expect(panel.textContent).toContain('City records portal · request 24-100 · release-70.pdf (document 70)');
    expect(panel.textContent).toContain('Same extracted text, not an exact copy: no match among 38 collected files with extracted text.');
    expect(panel.textContent).toContain('Similar text: not compared, the similarity index does not hold this file’s text yet.');

    await screen.findByText('Acme PDF 2.1');
    expect(panel.textContent).toContain('written by whoever made the file and can be set to anything');
    expect(screen.getByText('Info CreationDate').nextElementSibling?.textContent).toBe('not recorded');
  });

  it('says so when the origin could not be determined instead of showing the file unlabelled', async () => {
    render(viewer(9));
    expect((await screen.findByRole('alert')).textContent).toContain('Could not determine whether this file was collected from a source or added by hand.');
  });
});
