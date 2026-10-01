// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import AnnotationSearch from '../components/documents/AnnotationSearch';
import type { AnnotationSearchPage, AnnotationSearchRow } from '../types';

const invoke = vi.hoisted(() => vi.fn());
vi.mock('../ipc', () => ({ invokeCommand: invoke, formatNativeError: String }));
afterEach(() => {
  cleanup();
  invoke.mockReset();
});

const row = (id: string, quote: string, patch: Partial<AnnotationSearchRow> = {}): AnnotationSearchRow => ({
  id,
  portalKey: 'sd',
  documentId: 7,
  kind: 'HIGHLIGHT',
  characterOffset: 5,
  quote,
  prefix: '',
  suffix: '',
  textSha256: null,
  contextUnanchored: false,
  note: '',
  tag: null,
  author: 'Maria',
  status: 'OPEN',
  findingId: null,
  importedFromKey: null,
  createdAt: '2026-01-01T00:00:00Z',
  anchor: { state: 'ANCHORED', sourceOffset: 5, sourceLength: quote.length, reason: null },
  requestId: 42,
  fileName: 'contract.pdf',
  ...patch,
});

const page = (rows: AnnotationSearchRow[], nextCursor: string | null = null): AnnotationSearchPage => ({ rows, nextCursor });

const deferred = () => {
  let resolve: (value: AnnotationSearchPage) => void = () => {};
  const promise = new Promise<AnnotationSearchPage>((done) => { resolve = done; });
  return { promise, resolve };
};

const search = () => fireEvent.click(screen.getByRole('button', { name: /^Search$/ }));
const setText = (value: string) => fireEvent.change(screen.getByLabelText('Text in quote or note'), { target: { value } });

describe('AnnotationSearch', () => {
  it('drops a slow earlier search that settles after a newer one', async () => {
    const slow = deferred();
    invoke
      .mockReturnValueOnce(slow.promise)
      .mockReturnValueOnce(Promise.resolve(page([row('new', 'second filter hit')])));
    render(<AnnotationSearch onOpen={() => {}} />);
    setText('first');
    search();
    setText('second');
    search();
    await screen.findByText('second filter hit');
    await act(async () => slow.resolve(page([row('old', 'first filter hit')])));
    expect(screen.queryByText('first filter hit')).toBeNull();
    expect(screen.getByText('second filter hit')).toBeTruthy();
  });

  it('drops a slow "show more" for a filter that has since changed', async () => {
    const slowMore = deferred();
    invoke
      .mockReturnValueOnce(Promise.resolve(page([row('a', 'first page hit')], 'cursor-1')))
      .mockReturnValueOnce(slowMore.promise)
      .mockReturnValueOnce(Promise.resolve(page([row('b', 'other filter hit')])));
    render(<AnnotationSearch onOpen={() => {}} />);
    setText('one');
    search();
    await screen.findByText('first page hit');
    fireEvent.click(screen.getByRole('button', { name: 'Show more' }));
    setText('two');
    search();
    await screen.findByText('other filter hit');
    await act(async () => slowMore.resolve(page([row('c', 'stale second page hit')])));
    expect(screen.queryByText('stale second page hit')).toBeNull();
    expect(screen.queryByText('first page hit')).toBeNull();
  });

  it('opens an anchored passage at its position and offers no passage jump for an orphan', async () => {
    const onOpen = vi.fn();
    invoke.mockResolvedValue(page([
      row('a', 'found text', { anchor: { state: 'ANCHORED', sourceOffset: 5, sourceLength: 10, reason: null } }),
      row('o', 'lost text', { anchor: { state: 'ORPHANED', sourceOffset: null, sourceLength: null, reason: 'The quoted text is no longer in the document.' } }),
    ]));
    render(<AnnotationSearch onOpen={onOpen} />);
    search();
    fireEvent.click(await screen.findByRole('button', { name: 'Open at passage' }));
    expect(onOpen).toHaveBeenCalledWith({ portalKey: 'sd', requestId: 42, documentId: 7, characterOffset: 5 });
    expect(screen.getAllByRole('button', { name: 'Open at passage' })).toHaveLength(1);
    fireEvent.click(screen.getByRole('button', { name: 'Open document' }));
    expect(onOpen).toHaveBeenLastCalledWith({ portalKey: 'sd', requestId: 42, documentId: 7, characterOffset: 0 });
  });
});
