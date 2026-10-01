// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import PdfPageView from '../components/documents/PdfPageView';

const invoke = vi.hoisted(() => vi.fn());
const pdfjs = vi.hoisted(() => ({ pages: new Map<number, { render: ReturnType<typeof vi.fn> }>(), gates: new Map<number, Promise<void>>() }));

vi.mock('../ipc', () => ({ invokeCommand: invoke, formatNativeError: String }));
vi.mock('pdfjs-dist/legacy/web/pdf_viewer.css', () => ({}));
vi.mock('../components/documents/pdfjsLoader', () => ({
  loadPdfjs: async () => ({
    TextLayer: class {
      render = async () => {};
      cancel = () => {};
    },
    getDocument: () => ({
      promise: Promise.resolve({
        numPages: 3,
        loadingTask: { destroy: async () => {} },
        getPage: async (number: number) => {
          await pdfjs.gates.get(number);
          const render = vi.fn(() => ({ promise: Promise.resolve(), cancel: () => {} }));
          pdfjs.pages.set(number, { render });
          return {
            getViewport: ({ scale }: { scale: number }) => ({ width: 100 * scale, height: 140 * scale }),
            render,
            streamTextContent: () => ({}),
            cleanup: () => true,
          };
        },
      }),
    }),
  }),
}));

const frame = (): ArrayBuffer => {
  const file = new TextEncoder().encode('%PDF-1.7');
  const header = new TextEncoder().encode(JSON.stringify({ byteLength: file.length, sha256: 'a'.repeat(64), recordedSha256: 'a'.repeat(64), matchesRecorded: true }));
  const bytes = new Uint8Array(file.length + header.length + 4);
  bytes.set(file, 0);
  bytes.set(header, file.length);
  new DataView(bytes.buffer).setUint32(file.length + header.length, header.length, false);
  return bytes.buffer;
};

beforeEach(() => {
  pdfjs.pages.clear();
  pdfjs.gates.clear();
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue({} as CanvasRenderingContext2D);
  invoke.mockImplementation((command: string) => {
    if (command === 'read_document_original') return Promise.resolve(frame());
    if (command === 'list_document_annotations') return Promise.resolve({ textAvailable: true, currentTextSha256: null, annotations: [] });
    return Promise.resolve([]);
  });
});

afterEach(() => {
  cleanup();
  invoke.mockReset();
  vi.restoreAllMocks();
});

const shown = (container: HTMLElement) => container.querySelector('[data-rendered-page]')?.getAttribute('data-rendered-page');

describe('PdfPageView', () => {
  it('announces the page and discards a page that finishes after the reader moved on', async () => {
    let release: () => void = () => {};
    pdfjs.gates.set(2, new Promise<void>((done) => { release = done; }));
    const { container } = render(<PdfPageView portalKey="sd" documentId={7} fileName="a.pdf" />);
    await waitFor(() => expect(shown(container)).toBe('1'));
    expect(screen.getByText('Page 1 of 3', { selector: '.sr-only' })).toBeTruthy();

    fireEvent.click(screen.getByRole('button', { name: 'Next page' }));
    fireEvent.click(screen.getByRole('button', { name: 'Next page' }));
    await waitFor(() => expect(shown(container)).toBe('3'));

    await act(async () => {
      release();
      await Promise.resolve();
    });
    await act(async () => {
      await new Promise((done) => setTimeout(done, 20));
    });
    expect(shown(container)).toBe('3');
    expect(pdfjs.pages.get(2)?.render).not.toHaveBeenCalled();
  });

  it('says so, and does not call the file verified, when its digest differs from the recorded one', async () => {
    invoke.mockImplementation((command: string) => {
      if (command !== 'read_document_original') return Promise.resolve(command === 'list_document_annotations' ? { textAvailable: true, currentTextSha256: null, annotations: [] } : []);
      const file = new TextEncoder().encode('%PDF');
      const header = new TextEncoder().encode(JSON.stringify({ byteLength: 4, sha256: 'b'.repeat(64), recordedSha256: 'a'.repeat(64), matchesRecorded: false }));
      const bytes = new Uint8Array(4 + header.length + 4);
      bytes.set(file, 0);
      bytes.set(header, 4);
      new DataView(bytes.buffer).setUint32(4 + header.length, header.length, false);
      return Promise.resolve(bytes.buffer);
    });
    render(<PdfPageView portalKey="sd" documentId={7} fileName="a.pdf" />);
    expect(await screen.findByText(/Not the verified original/)).toBeTruthy();
    expect(screen.queryByText(/Verified original/)).toBeNull();
  });
});
