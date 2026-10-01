import workerUrl from 'pdfjs-dist/legacy/build/pdf.worker.min.mjs?url';

// The legacy build is the one that runs on the oldest WebKit this app supports (macOS 13).
export type PdfJs = typeof import('pdfjs-dist/legacy/build/pdf.mjs');

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
