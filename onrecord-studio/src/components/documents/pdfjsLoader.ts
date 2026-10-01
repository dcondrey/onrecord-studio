// The legacy build is the one that runs on the oldest WebKit this app supports (macOS 13).
export type PdfJs = typeof import('pdfjs-dist/legacy/build/pdf.mjs');

let loading: Promise<PdfJs> | null = null;

/**
 * pdf.js and its worker are fetched only when a PDF page view is opened, and both ship inside the
 * application bundle: nothing is requested from the network.
 */
export function loadPdfjs(): Promise<PdfJs> {
  loading ??= (async () => {
    const [library, worker] = await Promise.all([
      import('pdfjs-dist/legacy/build/pdf.mjs'),
      import('pdfjs-dist/legacy/build/pdf.worker.min.mjs?url'),
    ]);
    library.GlobalWorkerOptions.workerSrc = worker.default;
    return library;
  })().catch((error: unknown) => {
    loading = null;
    throw error;
  });
  return loading;
}
