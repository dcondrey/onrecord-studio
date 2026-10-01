// @vitest-environment jsdom
import { act, cleanup, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import DocumentAnnotations, { annotationsToMarkdown } from '../components/documents/DocumentAnnotations';
import type { DocumentAnnotation } from '../types';

const invoke = vi.hoisted(() => vi.fn());
vi.mock('../ipc', () => ({ invokeCommand: invoke, formatNativeError: String }));
afterEach(() => {
  cleanup();
  invoke.mockReset();
});

const annotation = (documentId: number, note: string): DocumentAnnotation => ({
  id: `a${documentId}`,
  portalKey: 'sd',
  documentId,
  kind: 'NOTE',
  characterOffset: 10,
  quote: '',
  note,
  tag: null,
  createdAt: '2026-01-01T00:00:00Z',
});

const panel = (documentId: number) => (
  <DocumentAnnotations
    portalKey="sd"
    documentId={documentId}
    fileName={`doc-${documentId}.pdf`}
    readSelection={() => null}
    currentOffset={0}
    onJump={() => true}
  />
);

describe('DocumentAnnotations', () => {
  it("never shows a slow document's notes after the viewer moved to another document", async () => {
    let resolveFirst: (rows: DocumentAnnotation[]) => void = () => {};
    invoke.mockImplementation((_command: string, args: { documentId: number }) =>
      args.documentId === 1
        ? new Promise((resolve) => { resolveFirst = resolve; })
        : Promise.resolve([annotation(2, 'note on document two')]),
    );
    const { rerender } = render(panel(1));
    rerender(panel(2));
    await waitFor(() => expect(screen.getByText('note on document two')).toBeTruthy());
    await act(async () => resolveFirst([annotation(1, 'note on document one')]));
    expect(screen.queryByText('note on document one')).toBeNull();
    expect(screen.getByText('note on document two')).toBeTruthy();
  });

  it('exports every annotation with its location and the reviewer-only caveat', () => {
    const markdown = annotationsToMarkdown('a.pdf', 'sd', 1, [
      { ...annotation(1, 'check this'), kind: 'ISSUE', tag: 'amendment', quote: 'was amended' },
    ]);
    expect(markdown).toContain('**Issue code: amendment** at character 10');
    expect(markdown).toContain('> was amended');
    expect(markdown).toContain('not part of the record');
  });
});
