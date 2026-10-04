import { useEffect, useRef, useState } from 'react';
import type { PDFDocumentProxy } from 'pdfjs-dist/legacy/build/pdf.mjs';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import { parseOriginalFrame, type OriginalHeader } from './pageRegionLogic';
import { canvasScale, loadPdfjs, STANDARD_FONT_URL, WASM_URL } from './pdfjsLoader';
import { keptCopyKind, keptCopyText, shortDigest } from './sourceHistoryLogic';

type Load =
  | { status: 'loading' }
  | { status: 'error'; message: string }
  | { status: 'ready'; header: OriginalHeader; content: { kind: 'pdf'; pageCount: number } | { kind: 'text'; text: string; truncated: boolean } | { kind: 'binary' } };

/**
 * The bytes one source check kept, read through the same store gate and digest comparison as the
 * collected original, against the digest that check received. Mount it keyed by check.
 */
export default function KeptCopyView({ documentId, portalKey, checkId }: { documentId: number; portalKey: string; checkId: number }) {
  const loadGate = useLatestRequestGate();
  const renderGate = useLatestRequestGate();
  const pdfRef = useRef<PDFDocumentProxy | null>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [load, setLoad] = useState<Load>({ status: 'loading' });
  const [pageNumber, setPageNumber] = useState(1);

  useEffect(() => {
    const request = loadGate.begin();
    let held: PDFDocumentProxy | null = null;
    setLoad({ status: 'loading' });
    void (async () => {
      try {
        const frame = await invokeCommand('read_source_check_copy', { documentId, portalKey, checkId });
        if (!loadGate.isCurrent(request)) return;
        const { bytes, header } = parseOriginalFrame(frame);
        const kind = keptCopyKind(bytes);
        if (kind === 'pdf') {
          const pdfjs = await loadPdfjs();
          if (!loadGate.isCurrent(request)) return;
          const opened = await pdfjs.getDocument({ data: bytes, useWorkerFetch: false, standardFontDataUrl: STANDARD_FONT_URL, wasmUrl: WASM_URL }).promise;
          if (!loadGate.isCurrent(request)) {
            void opened.loadingTask.destroy();
            return;
          }
          held = opened;
          pdfRef.current = opened;
          setLoad({ status: 'ready', header, content: { kind: 'pdf', pageCount: opened.numPages } });
        } else if (kind === 'text') {
          setLoad({ status: 'ready', header, content: { kind: 'text', ...keptCopyText(bytes) } });
        } else {
          setLoad({ status: 'ready', header, content: { kind: 'binary' } });
        }
      } catch (reason) {
        if (loadGate.isCurrent(request)) setLoad({ status: 'error', message: formatNativeError(reason) });
      }
    })();
    return () => {
      loadGate.invalidate();
      pdfRef.current = null;
      void held?.loadingTask.destroy();
    };
  }, [checkId, documentId, loadGate, portalKey]);

  const isPdf = load.status === 'ready' && load.content.kind === 'pdf';
  useEffect(() => {
    const pdf = pdfRef.current;
    const canvas = canvasRef.current;
    if (!isPdf || !pdf || !canvas) return;
    const request = renderGate.begin();
    let renderTask: { cancel: () => void; promise: Promise<unknown> } | null = null;
    void (async () => {
      try {
        const page = await pdf.getPage(pageNumber);
        if (!renderGate.isCurrent(request)) return;
        const viewport = page.getViewport({ scale: 1 });
        const ratio = canvasScale(viewport.width, viewport.height, window.devicePixelRatio || 1);
        const context = canvas.getContext('2d');
        if (!context) return;
        canvas.width = Math.floor(viewport.width * ratio);
        canvas.height = Math.floor(viewport.height * ratio);
        canvas.style.width = `${viewport.width}px`;
        renderTask = page.render({ canvas, canvasContext: context, viewport, transform: ratio === 1 ? undefined : [ratio, 0, 0, ratio, 0, 0] });
        await renderTask.promise;
      } catch {
        // A cancelled or failed page draw leaves the canvas blank; the digest line above stands.
      }
    })();
    return () => {
      renderGate.invalidate();
      renderTask?.cancel();
    };
  }, [isPdf, pageNumber, renderGate]);

  if (load.status === 'loading') return <p role="status" className="text-muted">Opening kept copy…</p>;
  if (load.status === 'error') return <p role="alert" className="text-danger">The kept copy could not be opened: {load.message}</p>;
  const { header, content } = load;
  return (
    <div aria-label="Kept copy" className="mt-2 space-y-2 rounded-lg border border-line bg-surface/40 p-2">
      <p className={header.matchesRecorded === false ? 'text-danger' : 'text-fg-secondary'}>
        {header.byteLength.toLocaleString()} bytes, SHA-256 {shortDigest(header.sha256)}….{' '}
        {header.matchesRecorded === true
          ? 'Matches what this check received.'
          : header.matchesRecorded === false
            ? 'Does not match what this check received; the stored file changed after the check.'
            : 'No digest was recorded to compare with.'}
      </p>
      {content.kind === 'pdf' && (
        <>
          <div className="flex items-center gap-2">
            <button type="button" onClick={() => setPageNumber((page) => Math.max(1, page - 1))} disabled={pageNumber <= 1} className="rounded border border-control px-2 py-1 text-muted disabled:opacity-40">Previous page</button>
            <span className="font-mono text-muted">{pageNumber} / {content.pageCount}</span>
            <button type="button" onClick={() => setPageNumber((page) => Math.min(content.pageCount, page + 1))} disabled={pageNumber >= content.pageCount} className="rounded border border-control px-2 py-1 text-muted disabled:opacity-40">Next page</button>
          </div>
          <div className="max-h-[60vh] overflow-auto">
            <canvas ref={canvasRef} aria-label={`Kept copy page ${pageNumber}`} />
          </div>
        </>
      )}
      {content.kind === 'text' && (
        <>
          <pre className="max-h-[50vh] overflow-auto whitespace-pre-wrap break-words font-mono text-caption text-fg">{content.text}</pre>
          {content.truncated && <p className="text-muted">Showing the beginning of the file only.</p>}
        </>
      )}
      {content.kind === 'binary' && <p className="text-muted">This file type cannot be shown here.</p>}
    </div>
  );
}
