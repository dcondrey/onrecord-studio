import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { PDFDocumentProxy } from 'pdfjs-dist/legacy/build/pdf.mjs';
import 'pdfjs-dist/legacy/web/pdf_viewer.css';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import type { ResolvedAnnotation } from '../../types';
import { annotationButton as button } from './annotationStyles';
import { KIND_LABELS, STATUS_LABELS } from './annotationLogic';
import { loadPdfjs } from './pdfjsLoader';
import { pageViewBridge, pageViewKey, usePageViewState } from './pageViewBridge';
import {
  cleanRegionText,
  normaliseClientBoxes,
  normaliseRectangle,
  parseOriginalFrame,
  percent,
  type ClientBox,
  type NormalisedRectangle,
  type OriginalHeader,
  type PagePoint,
} from './pageRegionLogic';

const ZOOM_STEPS = [0.5, 0.75, 1, 1.25, 1.5, 2, 3] as const;
const DEFAULT_ZOOM = 1;
const KEYBOARD_STEP = 0.01;
const MINIMUM_KEYBOARD_SIZE = 0.01;

type Load =
  | { status: 'loading' }
  | { status: 'error'; message: string }
  | { status: 'ready'; header: OriginalHeader; pageCount: number };

interface RenderedPage {
  pageNumber: number;
  zoom: number;
  width: number;
  height: number;
}

/** Text layer characters whose centre lies inside the box, in reading order of the layer. */
function textInsideBox(layer: HTMLElement, box: ClientBox): string {
  const parts: string[] = [];
  const walker = document.createTreeWalker(layer, NodeFilter.SHOW_TEXT);
  const range = document.createRange();
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    const text = node.textContent ?? '';
    range.selectNodeContents(node);
    const outer = range.getBoundingClientRect();
    if (outer.right < box.left || outer.left > box.right || outer.bottom < box.top || outer.top > box.bottom) continue;
    let run = '';
    let index = 0;
    for (const character of text) {
      range.setStart(node, index);
      range.setEnd(node, index + character.length);
      const at = range.getBoundingClientRect();
      const x = (at.left + at.right) / 2;
      const y = (at.top + at.bottom) / 2;
      if (x >= box.left && x <= box.right && y >= box.top && y <= box.bottom) run += character;
      index += character.length;
    }
    if (run !== '') parts.push(run);
  }
  return cleanRegionText(parts.join(' '));
}

function digestBanner(header: OriginalHeader) {
  if (header.matchesRecorded === true) {
    return { tone: 'text-emerald-200 border-emerald-400/30', text: `Verified original: this file's SHA-256 (${header.sha256.slice(0, 16)}…) matches the digest recorded when it was saved.` };
  }
  if (header.matchesRecorded === false) {
    return {
      tone: 'text-rose-200 border-rose-400/50',
      text: `Not the verified original: this file's SHA-256 is ${header.sha256}, but ${header.recordedSha256 ?? 'the recorded digest'} was recorded when it was saved. Treat what is shown as a different file.`,
    };
  }
  return { tone: 'text-amber-200 border-amber-400/30', text: `No digest was recorded for this document, so this file (SHA-256 ${header.sha256.slice(0, 16)}…) cannot be checked against one.` };
}

/**
 * The original PDF, one page at a time, with rectangles the reader drew on it. A rectangle locates
 * an area of the original file's page; it does not locate text in the extracted copy. Mount it
 * keyed by document. The page and its text layer are discarded unless they are still the page the
 * reader is on when they finish.
 */
/** Bundled under public/ so a PDF that names a standard font without embedding it draws with the right metrics offline. */
const STANDARD_FONT_URL = '/pdfjs/standard_fonts/';

export default function PdfPageView({ portalKey, documentId, fileName }: { portalKey: string; documentId: number; fileName: string }) {
  const key = pageViewKey(portalKey, documentId);
  const bridge = usePageViewState(key);
  const loadGate = useLatestRequestGate();
  const renderGate = useLatestRequestGate();
  const listGate = useLatestRequestGate();
  const resolveGate = useLatestRequestGate();
  const pdfRef = useRef<PDFDocumentProxy | null>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const pageRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const textLayerRef = useRef<HTMLDivElement>(null);
  const draftRef = useRef<HTMLDivElement>(null);
  const handledJump = useRef(bridge.jump);
  const [load, setLoad] = useState<Load>({ status: 'loading' });
  const [pageNumber, setPageNumber] = useState(1);
  const [pageInput, setPageInput] = useState('1');
  const [zoom, setZoom] = useState(DEFAULT_ZOOM);
  const [rendered, setRendered] = useState<RenderedPage | null>(null);
  const [renderError, setRenderError] = useState<string | null>(null);
  const [mode, setMode] = useState<'text' | 'draw'>('text');
  const [drag, setDrag] = useState<{ from: PagePoint; to: PagePoint } | null>(null);
  const [draft, setDraft] = useState<NormalisedRectangle | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [marks, setMarks] = useState<ResolvedAnnotation[]>([]);

  const pageCount = load.status === 'ready' ? load.pageCount : 0;

  useEffect(() => {
    const request = loadGate.begin();
    let held: PDFDocumentProxy | null = null;
    setLoad({ status: 'loading' });
    void (async () => {
      try {
        const frame = await invokeCommand('read_document_original', { documentId, portalKey });
        if (!loadGate.isCurrent(request)) return;
        const { bytes, header } = parseOriginalFrame(frame);
        const pdfjs = await loadPdfjs();
        if (!loadGate.isCurrent(request)) return;
        const opened = await pdfjs.getDocument({ data: bytes, useWorkerFetch: false, standardFontDataUrl: STANDARD_FONT_URL }).promise;
        if (!loadGate.isCurrent(request)) {
          void opened.loadingTask.destroy();
          return;
        }
        held = opened;
        pdfRef.current = opened;
        setLoad({ status: 'ready', header, pageCount: opened.numPages });
      } catch (reason) {
        if (loadGate.isCurrent(request)) setLoad({ status: 'error', message: formatNativeError(reason) });
      }
    })();
    return () => {
      loadGate.invalidate();
      pdfRef.current = null;
      void held?.loadingTask.destroy();
    };
  }, [documentId, loadGate, portalKey]);

  useEffect(() => {
    const request = listGate.begin();
    invokeCommand('list_document_annotations', { portalKey, documentId })
      .then((result) => {
        if (listGate.isCurrent(request)) setMarks(result.annotations.filter((item) => item.pageRegion !== undefined));
      })
      .catch(() => {
        if (listGate.isCurrent(request)) setMarks([]);
      });
  }, [bridge.revision, documentId, listGate, portalKey]);

  const originalSha256 = load.status === 'ready' ? load.header.sha256 : null;
  useEffect(() => {
    if (originalSha256 === null || pageCount === 0) return;
    const request = resolveGate.begin();
    invokeCommand('resolve_page_regions', { portalKey, documentId, originalSha256, pageCount })
      .then((resolved) => {
        if (resolveGate.isCurrent(request)) pageViewBridge.update(key, { states: Object.fromEntries(resolved.map((item) => [item.id, item.state])) });
      })
      .catch(() => {
        if (resolveGate.isCurrent(request)) pageViewBridge.update(key, { states: {} });
      });
  }, [documentId, key, marks, originalSha256, pageCount, portalKey, resolveGate]);

  useEffect(() => {
    pageViewBridge.update(key, { open: true });
    return () => pageViewBridge.reset(key);
  }, [key]);

  useEffect(() => {
    const pdf = pdfRef.current;
    const canvas = canvasRef.current;
    const layer = textLayerRef.current;
    if (load.status !== 'ready' || !pdf || !canvas || !layer) return;
    const request = renderGate.begin();
    let renderTask: { cancel: () => void; promise: Promise<unknown> } | null = null;
    let textLayer: { cancel: () => void } | null = null;
    let page: { cleanup: () => unknown } | null = null;
    setRenderError(null);
    void (async () => {
      try {
        const pdfjs = await loadPdfjs();
        const opened = await pdf.getPage(pageNumber);
        page = opened;
        if (!renderGate.isCurrent(request)) return;
        const viewport = opened.getViewport({ scale: zoom });
        const ratio = window.devicePixelRatio || 1;
        const context = canvas.getContext('2d');
        if (!context) throw new Error('This view cannot draw to a canvas.');
        canvas.width = Math.floor(viewport.width * ratio);
        canvas.height = Math.floor(viewport.height * ratio);
        renderTask = opened.render({ canvas, canvasContext: context, viewport, transform: ratio === 1 ? undefined : [ratio, 0, 0, ratio, 0, 0] });
        await renderTask.promise;
        if (!renderGate.isCurrent(request)) return;
        layer.replaceChildren();
        // pdf_viewer.css sizes the text layer's text from this variable, which pdf.js's own page
        // element normally sets and this view does not use.
        layer.style.setProperty('--total-scale-factor', String(zoom));
        const layerBuilder = new pdfjs.TextLayer({ textContentSource: opened.streamTextContent(), container: layer, viewport });
        textLayer = layerBuilder;
        await layerBuilder.render();
        if (!renderGate.isCurrent(request)) return;
        setRendered({ pageNumber, zoom, width: viewport.width, height: viewport.height });
      } catch (reason) {
        // A cancelled render is the reader moving on, not a failure to report.
        if (renderGate.isCurrent(request) && !(reason instanceof Error && reason.name === 'RenderingCancelledException')) {
          setRenderError(formatNativeError(reason));
        }
      }
    })();
    return () => {
      renderGate.invalidate();
      renderTask?.cancel();
      textLayer?.cancel();
      void page?.cleanup();
    };
  }, [load.status, pageNumber, renderGate, zoom]);

  const goTo = useCallback(
    (next: number) => {
      if (pageCount === 0) return;
      const target = Math.min(pageCount, Math.max(1, Math.trunc(next)));
      setPageNumber(target);
      setPageInput(String(target));
      setDrag(null);
      setDraft(null);
      setNotice(null);
      scrollRef.current?.scrollTo?.({ top: 0 });
    },
    [pageCount],
  );

  useEffect(() => {
    if (bridge.jump === handledJump.current) return;
    handledJump.current = bridge.jump;
    const target = marks.find((item) => item.id === bridge.selectedId)?.pageRegion;
    if (target) goTo(target.pageIndex + 1);
  }, [bridge.jump, bridge.selectedId, goTo, marks]);

  const zoomBy = (direction: 1 | -1) => {
    const index = ZOOM_STEPS.findIndex((step) => step >= zoom - 1e-9);
    const current = index === -1 ? ZOOM_STEPS.length - 1 : index;
    setZoom(ZOOM_STEPS[Math.min(ZOOM_STEPS.length - 1, Math.max(0, current + direction))]);
  };

  const fitWidth = () => {
    const available = scrollRef.current?.clientWidth;
    if (!rendered || !available) return;
    setZoom(Math.min(3, Math.max(0.25, ((available - 24) / rendered.width) * rendered.zoom)));
  };

  const finish = (rectangle: NormalisedRectangle, quote?: string) => {
    const layer = textLayerRef.current;
    const page = pageRef.current?.getBoundingClientRect();
    let text = quote;
    if (text === undefined) {
      text = layer && page
        ? textInsideBox(layer, {
            left: page.left + rectangle.x * page.width,
            top: page.top + rectangle.y * page.height,
            right: page.left + (rectangle.x + rectangle.width) * page.width,
            bottom: page.top + (rectangle.y + rectangle.height) * page.height,
          })
        : '';
    }
    pageViewBridge.update(key, { pending: { rectangle: { pageIndex: pageNumber - 1, ...rectangle }, quote: text } });
    setNotice(text === ''
      ? `Region on page ${pageNumber} marked. It holds no text layer text (an image or a scan), so nothing is quoted. Save it from the annotations panel.`
      : `Region on page ${pageNumber} marked with its text. Save it from the annotations panel.`);
  };

  const pointFor = (event: React.PointerEvent<HTMLDivElement>): { point: PagePoint; box: DOMRect } => {
    const box = event.currentTarget.getBoundingClientRect();
    return { point: { x: event.clientX - box.left, y: event.clientY - box.top }, box };
  };

  const markSelectedText = () => {
    const selection = window.getSelection();
    const layer = textLayerRef.current;
    const page = pageRef.current?.getBoundingClientRect();
    if (!selection || selection.rangeCount === 0 || selection.isCollapsed || !layer || !page) {
      setNotice('Select text on the page first.');
      return;
    }
    const range = selection.getRangeAt(0);
    if (!layer.contains(range.commonAncestorContainer)) {
      setNotice('Select text on the page first.');
      return;
    }
    const rectangle = normaliseClientBoxes(Array.from(range.getClientRects()), page);
    if (!rectangle) {
      setNotice('That selection has no area on the page.');
      return;
    }
    finish(rectangle, cleanRegionText(selection.toString()));
  };

  const startDraft = () => {
    setMode('text');
    setDraft({ x: 0.4, y: 0.45, width: 0.2, height: 0.1 });
    setNotice(null);
    requestAnimationFrame(() => draftRef.current?.focus());
  };

  const keyOnDraft = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (!draft) return;
    if (event.key === 'Escape') {
      event.preventDefault();
      setDraft(null);
      return;
    }
    if (event.key === 'Enter') {
      event.preventDefault();
      finish(draft);
      setDraft(null);
      return;
    }
    const delta: Record<string, [number, number]> = { ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, -1], ArrowDown: [0, 1] };
    const move = delta[event.key];
    if (!move) return;
    event.preventDefault();
    const step = KEYBOARD_STEP;
    const next = event.shiftKey
      ? { ...draft, width: Math.max(MINIMUM_KEYBOARD_SIZE, draft.width + move[0] * step), height: Math.max(MINIMUM_KEYBOARD_SIZE, draft.height + move[1] * step) }
      : { ...draft, x: draft.x + move[0] * step, y: draft.y + move[1] * step };
    const width = Math.min(next.width, 1);
    const height = Math.min(next.height, 1);
    const x = Math.min(1 - width, Math.max(0, next.x));
    const y = Math.min(1 - height, Math.max(0, next.y));
    setDraft({ x: Math.round(x * 1e6) / 1e6, y: Math.round(y * 1e6) / 1e6, width: Math.round(width * 1e6) / 1e6, height: Math.round(height * 1e6) / 1e6 });
  };

  const keyOnViewer = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (event.target !== event.currentTarget) return;
    const box = event.currentTarget;
    const atBottom = box.scrollTop + box.clientHeight >= box.scrollHeight - 1;
    const atTop = box.scrollTop <= 0;
    if (event.key === 'PageDown' && atBottom) goTo(pageNumber + 1);
    else if (event.key === 'PageUp' && atTop) goTo(pageNumber - 1);
    else if (event.key === 'Home' && atTop) goTo(1);
    else if (event.key === 'End' && atBottom) goTo(pageCount);
    else if (event.key === '+' || event.key === '=') zoomBy(1);
    else if (event.key === '-') zoomBy(-1);
    else return;
    event.preventDefault();
  };

  const here = marks.filter((item) => item.pageRegion?.pageIndex === pageNumber - 1);
  const missing = useMemo(() => marks.filter((item) => bridge.states[item.id] === 'ORPHANED'), [bridge.states, marks]);
  const rubber = drag && pageRef.current
    ? normaliseRectangle(drag.from, drag.to, { width: pageRef.current.clientWidth, height: pageRef.current.clientHeight })
    : null;
  const banner = load.status === 'ready' ? digestBanner(load.header) : null;
  const shown = rendered?.pageNumber === pageNumber;

  return (
    <section aria-label={`Page view of ${fileName}`} className="flex min-h-0 min-w-0 flex-1 flex-col border-b border-slate-800 text-[10px] text-slate-300">
      {banner && <p role="status" className={`border-b px-3 py-1.5 ${banner.tone}`}>{banner.text}</p>}
      {load.status === 'loading' && <p role="status" className="p-3 text-slate-400">Opening the original file…</p>}
      {load.status === 'error' && <p role="alert" className="p-3 text-rose-300">The page view could not open this file: {load.message}</p>}
      {load.status === 'ready' && (
        <>
          <div className="flex flex-wrap items-center gap-2 border-b border-slate-900 px-3 py-2">
            <button type="button" className={button} disabled={pageNumber <= 1} onClick={() => goTo(pageNumber - 1)}>Previous page</button>
            <label className="flex items-center gap-1">
              <span>Page</span>
              <input
                type="number"
                min={1}
                max={pageCount}
                value={pageInput}
                onChange={(event) => setPageInput(event.target.value)}
                onBlur={() => goTo(Number(pageInput) || pageNumber)}
                onKeyDown={(event) => { if (event.key === 'Enter') goTo(Number(pageInput) || pageNumber); }}
                className="focus-ring w-14 rounded border border-slate-700 bg-slate-950 px-1 py-0.5 text-slate-100"
              />
              <span aria-hidden="true">of {pageCount}</span>
            </label>
            <button type="button" className={button} disabled={pageNumber >= pageCount} onClick={() => goTo(pageNumber + 1)}>Next page</button>
            <span className="sr-only" role="status" aria-live="polite">Page {pageNumber} of {pageCount}</span>
            <span className="mx-1 text-slate-400" aria-hidden="true">|</span>
            <button type="button" className={button} aria-label="Zoom out" disabled={zoom <= ZOOM_STEPS[0]} onClick={() => zoomBy(-1)}>-</button>
            <span className="min-w-10 text-center font-mono" aria-label={`Zoom ${Math.round(zoom * 100)} percent`}>{Math.round(zoom * 100)}%</span>
            <button type="button" className={button} aria-label="Zoom in" disabled={zoom >= ZOOM_STEPS[ZOOM_STEPS.length - 1]} onClick={() => zoomBy(1)}>+</button>
            <button type="button" className={button} onClick={fitWidth}>Fit width</button>
            <span className="mx-1 text-slate-400" aria-hidden="true">|</span>
            <fieldset className="flex items-center gap-2">
              <legend className="sr-only">Pointer mode</legend>
              <label className="flex items-center gap-1"><input type="radio" name={`mode-${key}`} checked={mode === 'text'} onChange={() => setMode('text')} />Select text</label>
              <label className="flex items-center gap-1"><input type="radio" name={`mode-${key}`} checked={mode === 'draw'} onChange={() => setMode('draw')} />Draw region</label>
            </fieldset>
            <button type="button" className={button} onClick={markSelectedText}>Mark selected text</button>
            <button type="button" className={button} onClick={startDraft}>Mark region with keyboard</button>
          </div>
          <p className="px-3 py-1 text-slate-400">
            The page is drawn from the original file. Its selectable text is the PDF's own text layer, which can differ from the extracted text stored for search. A rectangle locates an area of this page of the original file.
          </p>
          <div
            ref={scrollRef}
            tabIndex={0}
            role="group"
            aria-label={`Page ${pageNumber} of ${pageCount}. Page Down at the end of a page goes to the next page. Plus and minus change the zoom.`}
            onKeyDown={keyOnViewer}
            className="focus-ring min-h-0 flex-1 overflow-auto bg-slate-900 p-3"
          >
            <div
              ref={pageRef}
              data-rendered-page={rendered?.pageNumber ?? ''}
              style={rendered ? { width: rendered.width, height: rendered.height } : undefined}
              className="relative mx-auto bg-white"
            >
              <canvas ref={canvasRef} className="absolute inset-0 h-full w-full" aria-hidden="true" />
              <div ref={textLayerRef} className={`textLayer ${mode === 'draw' ? 'pointer-events-none' : ''}`} />
              {shown && here.map((item) => {
                const region = item.pageRegion;
                if (!region) return null;
                const state = bridge.states[item.id];
                const changed = state === 'ORIGINAL_CHANGED';
                const color = item.kind === 'ISSUE' ? 'border-cyan-500 text-cyan-900 bg-cyan-100' : item.kind === 'HIGHLIGHT' ? 'border-yellow-500 text-yellow-900 bg-yellow-100' : 'border-slate-500 text-slate-900 bg-slate-200';
                const dash = item.status === 'CONFIRMED' ? 'border-solid' : item.status === 'DISPUTED' ? 'border-dotted' : 'border-dashed';
                const label = `${KIND_LABELS[item.kind]}${item.tag ? `: ${item.tag}` : ''}, ${STATUS_LABELS[item.status].toLowerCase()}${changed ? ', original changed' : ''}`;
                return (
                  <div
                    key={item.id}
                    data-region-id={item.id}
                    style={{ left: `${region.x * 100}%`, top: `${region.y * 100}%`, width: `${region.width * 100}%`, height: `${region.height * 100}%` }}
                    className={`pointer-events-none absolute ${changed ? 'border-4 border-double border-amber-600' : `border-2 ${dash} ${color.split(' ')[0]}`} ${bridge.selectedId === item.id ? 'outline outline-2 outline-offset-2 outline-indigo-600' : ''}`}
                  >
                    <button
                      type="button"
                      aria-label={`${label}. Show in the annotations list.`}
                      aria-pressed={bridge.selectedId === item.id}
                      onClick={() => pageViewBridge.update(key, { selectedId: item.id })}
                      className={`focus-ring pointer-events-auto absolute left-0 top-0 max-w-full -translate-y-full truncate rounded-t px-1 text-[9px] font-semibold ${changed ? 'bg-amber-200 text-amber-950' : color.split(' ').slice(1).join(' ')}`}
                    >
                      {label}
                    </button>
                  </div>
                );
              })}
              {shown && bridge.pending && bridge.pending.rectangle.pageIndex === pageNumber - 1 && (
                <div
                  data-testid="pending-region"
                  className="pointer-events-none absolute border-2 border-dotted border-indigo-600 bg-indigo-400/15"
                  style={{ left: `${bridge.pending.rectangle.x * 100}%`, top: `${bridge.pending.rectangle.y * 100}%`, width: `${bridge.pending.rectangle.width * 100}%`, height: `${bridge.pending.rectangle.height * 100}%` }}
                >
                  <span className="absolute left-0 top-0 -translate-y-full whitespace-nowrap rounded-t bg-indigo-200 px-1 text-[9px] font-semibold text-indigo-950">Unsaved region</span>
                </div>
              )}
              {drag && rubber && (
                <div
                  aria-hidden="true"
                  className="pointer-events-none absolute border-2 border-dashed border-indigo-600 bg-indigo-400/20"
                  style={{ left: `${rubber.x * 100}%`, top: `${rubber.y * 100}%`, width: `${rubber.width * 100}%`, height: `${rubber.height * 100}%` }}
                />
              )}
              {draft && (
                <div
                  ref={draftRef}
                  tabIndex={0}
                  role="group"
                  aria-label={`Region being marked on page ${pageNumber}: left ${percent(draft.x)}, top ${percent(draft.y)}, width ${percent(draft.width)}, height ${percent(draft.height)}. Arrow keys move it, Shift with arrow keys resizes it, Enter keeps it, Escape cancels.`}
                  onKeyDown={keyOnDraft}
                  className="focus-ring absolute border-2 border-dashed border-indigo-600 bg-indigo-400/20"
                  style={{ left: `${draft.x * 100}%`, top: `${draft.y * 100}%`, width: `${draft.width * 100}%`, height: `${draft.height * 100}%` }}
                />
              )}
              {mode === 'draw' && (
                <div
                  data-testid="region-draw-surface"
                  className="absolute inset-0 cursor-crosshair touch-none"
                  onPointerDown={(event) => {
                    event.currentTarget.setPointerCapture?.(event.pointerId);
                    const { point } = pointFor(event);
                    setNotice(null);
                    setDrag({ from: point, to: point });
                  }}
                  onPointerMove={(event) => {
                    if (!drag) return;
                    const { point } = pointFor(event);
                    setDrag({ from: drag.from, to: point });
                  }}
                  onPointerUp={(event) => {
                    if (!drag) return;
                    const { point, box } = pointFor(event);
                    const rectangle = normaliseRectangle(drag.from, point, { width: box.width, height: box.height });
                    setDrag(null);
                    if (rectangle) finish(rectangle);
                    else setNotice('Drag a larger rectangle.');
                  }}
                  onPointerCancel={() => setDrag(null)}
                />
              )}
            </div>
          </div>
          {renderError && <p role="alert" className="px-3 py-1 text-rose-300">This page could not be drawn: {renderError}</p>}
          {notice && <p role="status" className="px-3 py-1 text-emerald-300">{notice}</p>}
          {missing.length > 0 && (
            <p role="status" className="px-3 py-1 text-rose-200">
              {missing.length} saved region{missing.length === 1 ? ' refers' : 's refer'} to a page this file does not have (it has {pageCount}). They are listed as orphaned in the annotations panel and are not shown on any page.
            </p>
          )}
        </>
      )}
    </section>
  );
}
