// @vitest-environment jsdom
import { act, cleanup, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import DocumentAnnotations, { annotationsToMarkdown } from '../components/documents/DocumentAnnotations';
import type { DocumentAnnotationList, ResolvedAnnotation } from '../types';

const invoke = vi.hoisted(() => vi.fn());
vi.mock('../ipc', () => ({ invokeCommand: invoke, formatNativeError: String }));
vi.stubGlobal('__APP_VERSION__', '1.0.0');
afterEach(() => {
  cleanup();
  invoke.mockReset();
});

const annotation = (documentId: number, note: string, patch: Partial<ResolvedAnnotation> = {}): ResolvedAnnotation => ({
  id: `a${documentId}`,
  portalKey: 'sd',
  documentId,
  kind: 'NOTE',
  characterOffset: null,
  quote: '',
  prefix: '',
  suffix: '',
  textSha256: null,
  contextUnanchored: false,
  note,
  tag: null,
  author: 'Maria',
  status: 'OPEN',
  findingId: null,
  importedFromKey: null,
  createdAt: '2026-01-01T00:00:00Z',
  anchor: null,
  ...patch,
});

const listOf = (...annotations: ResolvedAnnotation[]): DocumentAnnotationList => ({
  textAvailable: true,
  currentTextSha256: 'f'.repeat(64),
  annotations,
});

const panel = (documentId: number) => (
  <DocumentAnnotations
    portalKey="sd"
    documentId={documentId}
    fileName={`doc-${documentId}.pdf`}
    readSelection={() => null}
    onJump={() => true}
  />
);

describe('DocumentAnnotations', () => {
  it("never shows a slow document's notes after the viewer moved to another document", async () => {
    let resolveFirst: (list: DocumentAnnotationList) => void = () => {};
    invoke.mockImplementation((command: string, args: { documentId?: number }) => {
      if (command === 'get_annotation_settings') return Promise.resolve({ reviewer: 'Maria', codebook: [] });
      if (command === 'list_findings') return Promise.resolve([]);
      return args.documentId === 1
        ? new Promise((resolve) => { resolveFirst = resolve; })
        : Promise.resolve(listOf(annotation(2, 'note on document two')));
    });
    const { rerender } = render(panel(1));
    rerender(panel(2));
    await waitFor(() => expect(screen.getByText('note on document two')).toBeTruthy());
    await act(async () => resolveFirst(listOf(annotation(1, 'note on document one'))));
    expect(screen.queryByText('note on document one')).toBeNull();
    expect(screen.getByText('note on document two')).toBeTruthy();
  });

  it('lists an orphan with its stored quote and offers no jump to a passage that was not found', async () => {
    const orphan = annotation(1, '', {
      kind: 'HIGHLIGHT',
      quote: 'the vanished sentence',
      characterOffset: 40,
      anchor: { state: 'ORPHANED', sourceOffset: null, sourceLength: null, reason: 'The quoted text is no longer in the document.' },
    });
    const anchored = annotation(2, '', {
      id: 'kept',
      kind: 'HIGHLIGHT',
      quote: 'still here',
      characterOffset: 5,
      anchor: { state: 'ANCHORED', sourceOffset: 5, sourceLength: 10, reason: null },
    });
    invoke.mockImplementation((command: string) => {
      if (command === 'get_annotation_settings') return Promise.resolve({ reviewer: 'Maria', codebook: [] });
      if (command === 'list_findings') return Promise.resolve([]);
      return Promise.resolve(listOf(orphan, anchored));
    });
    render(panel(1));
    const orphans = await screen.findByRole('list', { name: 'Orphaned annotations' });
    expect(orphans.textContent).toContain('the vanished sentence');
    expect(orphans.textContent).toContain('not located in the current text');
    expect(orphans.textContent).not.toContain('Go to passage');
    expect(screen.getByRole('list', { name: 'Saved annotations' }).textContent).toContain('Go to passage');
  });

  it('exports every annotation with its location and the reviewer-only caveat', () => {
    const markdown = annotationsToMarkdown('a.pdf', 'sd', 1, [
      annotation(1, 'check this', {
        kind: 'ISSUE',
        tag: 'amendment',
        quote: 'was amended',
        anchor: { state: 'ANCHORED', sourceOffset: 10, sourceLength: 11, reason: null },
      }),
    ]);
    expect(markdown).toContain('**Issue code: amendment** (character 10, Maria, open)');
    expect(markdown).toContain('> was amended');
    expect(markdown).toContain('not part of the record');
  });
});
