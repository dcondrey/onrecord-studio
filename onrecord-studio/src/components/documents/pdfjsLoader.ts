import workerUrl from 'pdfjs-dist/legacy/build/pdf.worker.min.mjs?url';

// The legacy build is the one that runs on the oldest WebKit this app supports (macOS 13).
export type PdfJs = typeof import('pdfjs-dist/legacy/build/pdf.mjs');

/** Bundled under public/ so a PDF that names a standard font without embedding it draws with the right metrics offline. */
export const STANDARD_FONT_URL = '/pdfjs/standard_fonts/';
/**
 * CCITT, JBIG2 and JPEG 2000 images (the encodings of scanned records) and ICC colour decode only
 * through these modules; without them pdf.js draws the page with the image silently missing. The
 * `_nowasm_fallback.js` files beside them cover a WebKit that refuses WebAssembly.
 */
export const WASM_URL = '/pdfjs/wasm/';
/**
 * Backing-store pixel budget for one page. A tall MediaBox at zoom 3 on a Retina display asks
 * for billions of pixels, which WebKit answers with a blank canvas or a hung webview.
 */
const MAXIMUM_CANVAS_PIXELS = 16_777_216;

/** The device scale that keeps a page's backing store within the pixel budget. */
export function canvasScale(width: number, height: number, devicePixelRatio: number): number {
  const area = width * height;
  if (area <= 0) return devicePixelRatio;
  return Math.min(devicePixelRatio, Math.sqrt(MAXIMUM_CANVAS_PIXELS / area));
}

let loading: Promise<PdfJs> | null = null;

/**
 * pdf.js is fetched only when a PDF page view is opened (this module is itself loaded lazily), and
 * the library and its worker both ship inside the application bundle: nothing is requested from
 * the network. The worker URL is a static import: a dynamic `?url` import resolved to no default
 * export under the dev server.
 */
export function loadPdfjs(): Promise<PdfJs> {
  loading ??= import('pdfjs-dist/legacy/build/pdf.mjs')
    .then((library) => {
      library.GlobalWorkerOptions.workerSrc = workerUrl;
      return library;
    })
    .catch((error: unknown) => {
      loading = null;
      throw error;
    });
  return loading;
}
