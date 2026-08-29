import { useCallback, useEffect, useState } from 'react';
import SearchFilter from '../filters/SearchFilter';
import { useAppStore } from '../../store/useAppStore';
import { cn } from '../../utils/cn';
import DocumentViewer from '../documents/DocumentViewer';
import UnredactorPanel from '../documents/UnredactorPanel';
import LegalAuthorityManager from '../legal/LegalAuthorityManager';
import LegalCompliancePanel from '../legal/LegalCompliancePanel';
import { invokeCommand } from '../../ipc';
import type {
  DocumentTextChunk,
  LegalBriefExportResult,
  RequestDocumentSummary,
  RequestSearchResult,
} from '../../types';
import OverviewDashboard from './OverviewDashboard';
import ActivityDashboard from './ActivityDashboard';
import PatternExplorer from './PatternExplorer';
import Icon from '../ui/icons';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import ExtractionTerminal from '../documents/ExtractionTerminal';
import MetadataCard from '../documents/MetadataCard';

export default function MainLayout() {
  // Request selection lives in the shared workspace store and survives view switches. The open
  // document chunk is session-local and is deliberately cleared when the selected request changes.
  const workspaceView = useAppStore((state) => state.workspaceView);
  const results = useAppStore((state) => state.filteredResults);
  const searchError = useAppStore((state) => state.searchError);
  const selectedRequestId = useAppStore((state) => state.selectedRequestId);
  const selectedRequestPortalKey = useAppStore((state) => state.selectedRequestPortalKey);
  const setSelectedRequest = useAppStore((state) => state.setSelectedRequest);
  const [exportState, setExportState] = useState<{ key: string; isExporting: boolean; result: LegalBriefExportResult | null; error: string | null } | null>(null);
  const [documents, setDocuments] = useState<RequestDocumentSummary[]>([]);
  const [documentError, setDocumentError] = useState<string | null>(null);
  const [activeDocument, setActiveDocument] = useState<{ summary: RequestDocumentSummary; chunk: DocumentTextChunk } | null>(null);
  const [pendingEvidenceLink, setPendingEvidenceLink] = useState(readEvidenceLink);
  const [isLegalLibraryOpen, setLegalLibraryOpen] = useState(false);
  const closeLegalLibrary = useCallback(() => setLegalLibraryOpen(false), []);
  const selected = results.find(
    (result) => result.requestId === selectedRequestId && result.portalKey === selectedRequestPortalKey,
  ) ?? null;
  const documentListGate = useLatestRequestGate();
  const documentOpenGate = useLatestRequestGate();
  const exportGate = useLatestRequestGate();

  useEffect(() => {
    if (pendingEvidenceLink) {
      setSelectedRequest(pendingEvidenceLink.requestId, pendingEvidenceLink.portalKey);
    }
  }, [pendingEvidenceLink, setSelectedRequest]);

  useEffect(() => {
    const request = documentListGate.begin();
    documentOpenGate.invalidate();
    setDocuments([]);
    setDocumentError(null);
    setActiveDocument(null);
    const target = selected ?? (pendingEvidenceLink ? {
      requestId: pendingEvidenceLink.requestId,
      portalKey: pendingEvidenceLink.portalKey,
    } : null);
    if (!target) return documentListGate.invalidate;
    void invokeCommand('list_request_documents', {
      requestId: target.requestId,
      portalKey: target.portalKey,
    }).then((values) => {
      if (documentListGate.isCurrent(request)) setDocuments(values);
    }).catch((reason) => {
      if (documentListGate.isCurrent(request)) setDocumentError(String(reason));
    });
    return documentListGate.invalidate;
  }, [documentListGate, documentOpenGate, pendingEvidenceLink, selected?.portalKey, selected?.requestId]);

  useEffect(() => {
    if (!activeDocument) return;
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === 'Escape') setActiveDocument(null);
    };
    window.addEventListener('keydown', closeOnEscape);
    return () => window.removeEventListener('keydown', closeOnEscape);
  }, [activeDocument]);

  const openDocument = async (summary: RequestDocumentSummary, characterOffset = 0) => {
    setDocumentError(null);
    // Identity is the request, not the document: two rapid clicks resolve out of order and reopen
    // the older document, and a chunk from request A resolving after B is selected opens A's text
    // under B's header. Both are false attributions of evidence.
    const request = documentOpenGate.begin();
    try {
      const chunk = await invokeCommand('get_document_text_chunk', {
        documentId: summary.documentId,
        portalKey: summary.portalKey,
        characterOffset,
        characterLimit: 100000,
      });
      if (!documentOpenGate.isCurrent(request)) return;
      setActiveDocument({ summary, chunk });
    } catch (reason) {
      if (!documentOpenGate.isCurrent(request)) return;
      setDocumentError(String(reason));
    }
  };

  useEffect(() => {
    if (!pendingEvidenceLink || documents.length === 0) return;
    const summary = documents.find((document) =>
      document.documentId === pendingEvidenceLink.documentId
      && document.portalKey === pendingEvidenceLink.portalKey
    );
    if (!summary) {
      setDocumentError(`Evidence link document ${pendingEvidenceLink.portalKey}/${pendingEvidenceLink.documentId} is not part of request ${pendingEvidenceLink.requestId}.`);
      setPendingEvidenceLink(null);
      return;
    }
    void openDocument(summary, pendingEvidenceLink.characterOffset);
    setPendingEvidenceLink(null);
  }, [documents, pendingEvidenceLink]);

  const exportBrief = async (record: RequestSearchResult) => {
    const request = exportGate.begin();
    const key = `${record.portalKey}-${record.requestId}`;
    setExportState({ key, isExporting: true, result: null, error: null });
    try {
      const result = await invokeCommand('export_legal_brief', {
        requestId: record.requestId,
        portalKey: record.portalKey,
        targetPath: null,
        authorName: null,
      });
      if (exportGate.isCurrent(request)) {
        setExportState({ key, isExporting: false, result, error: null });
      }
    } catch (reason) {
      if (exportGate.isCurrent(request)) {
        setExportState({ key, isExporting: false, result: null, error: String(reason) });
      }
    }
  };

  if (workspaceView === 'overview') {
    return <main className="flex min-h-0 flex-1 overflow-hidden"><OverviewDashboard /></main>;
  }

  if (workspaceView === 'activity') {
    return <main className="flex min-h-0 flex-1 overflow-hidden"><ActivityDashboard /></main>;
  }

  if (workspaceView === 'patterns') {
    return <main className="flex min-h-0 flex-1 overflow-hidden"><PatternExplorer /></main>;
  }

  return (
    <main className="flex min-h-0 flex-1 flex-col overflow-hidden">
      <div className="flex shrink-0 items-center justify-between px-5 pb-1 pt-4 lg:px-6">
        <div>
          <div className="app-eyebrow flex items-center gap-2 text-indigo-300/60"><Icon name="evidence" className="h-3.5 w-3.5" /> Evidence review</div>
          <h2 className="mt-1.5 text-[26px] font-semibold tracking-[-0.035em] text-slate-100 sm:text-[30px]">Search, inspect, and build the record.</h2>
        </div>
        <div className="hidden items-center gap-2 text-[9px] text-slate-400 sm:flex"><Icon name="shield" className="h-3.5 w-3.5 text-emerald-400/60" /> Sensitive fields are masked locally</div>
      </div>
      <SearchFilter />
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="mx-auto grid min-h-[620px] w-full max-w-[1800px] grid-cols-1 gap-4 p-4 lg:p-5 xl:grid-cols-[minmax(0,1.55fr)_minmax(320px,0.75fr)]">
          <section className="app-panel flex min-h-[520px] flex-col overflow-hidden">
            <div className="flex items-center justify-between border-b border-slate-800 px-4 py-3">
              <div>
                <h2 className="text-xs font-semibold text-slate-200">Evidence workbench</h2>
                <p className="mt-0.5 text-[9px] text-slate-400">Request metadata is PII-masked before it reaches this view.</p>
              </div>
              <div className="flex items-center gap-2">
                <button type="button" onClick={() => setLegalLibraryOpen(true)} className="rounded border border-indigo-400/20 bg-indigo-500/[0.07] px-2 py-1 text-[9px] font-semibold text-indigo-200/80 transition hover:border-indigo-400/35 hover:bg-indigo-500/10">Legal authority library</button>
                <span className="rounded border border-slate-800 bg-slate-950 px-2 py-1 font-mono text-[9px] text-slate-400">{results.length.toLocaleString()} records</span>
              </div>
            </div>
            <div className="min-h-0 flex-1 overflow-y-auto">
              {results.length === 0 ? (
                <EmptyResults error={searchError} />
              ) : (
                <div className="divide-y divide-slate-800/70">
                  {results.map((result) => {
                    const isSelected = selectedRequestId === result.requestId && selectedRequestPortalKey === result.portalKey;
                    return (
                      <button
                        key={`${result.portalKey}-${result.requestId}`}
                        type="button"
                        onClick={() => setSelectedRequest(result.requestId, result.portalKey)}
                        className={cn(
                          'grid w-full grid-cols-[minmax(0,1fr)_120px] gap-4 px-4 py-3 text-left outline-none transition hover:bg-slate-900/80 focus:bg-slate-900/80',
                          isSelected && 'bg-indigo-500/[0.08] ring-1 ring-inset ring-indigo-400/25',
                        )}
                      >
                        <div className="min-w-0">
                          <div className="flex items-center gap-2">
                            <span className="font-mono text-[9px] text-slate-400">{result.portalKey} / {result.requestId}</span>
                            <StatusBadge status={result.status} />
                          </div>
                          <div className="mt-1 truncate text-xs font-medium text-slate-200">{result.title || 'Untitled public-records request'}</div>
                          <div className="mt-1 line-clamp-2 text-[10px] leading-relaxed text-slate-400">{result.descriptionPreview || 'No public description supplied.'}</div>
                          {result.integrityFlags.length > 0 && (
                            <div className="mt-2 flex flex-wrap gap-1">
                              {result.integrityFlags.slice(0, 4).map((flag) => <PolicyBadge key={flag} flag={flag} />)}
                            </div>
                          )}
                        </div>
                        <div className="text-right font-mono text-[9px] text-slate-400">
                          <div className="truncate text-slate-400">{result.agencyName}</div>
                          <div className="mt-2">{formatDate(result.createdAt)}</div>
                          <div className="mt-1">{result.documentCount} document{result.documentCount === 1 ? '' : 's'}</div>
                        </div>
                      </button>
                    );
                  })}
                </div>
              )}
            </div>
          </section>

          <div className="grid min-h-0 grid-rows-[auto_minmax(220px,1fr)] gap-4">
            <section className="app-panel p-4">
              <div className="text-[9px] font-bold uppercase tracking-[0.18em] text-slate-400">Case context</div>
              {selected ? (
                <div className="mt-3">
                  <div className="flex items-start justify-between gap-3">
                    <div className="min-w-0">
                      <h2 className="text-sm font-semibold leading-snug text-slate-100">{selected.title || 'Untitled request'}</h2>
                      <p className="mt-1 text-[10px] text-slate-400">{selected.agencyName} · {selected.portalKey}</p>
                    </div>
                    <StatusBadge status={selected.status} />
                  </div>
                  <dl className="mt-4 grid grid-cols-2 gap-x-4 gap-y-3 border-t border-slate-800 pt-3 text-[10px]">
                    <CaseField label="Request ID" value={String(selected.requestId)} />
                    <CaseField label="Documents" value={String(selected.documentCount)} />
                    <CaseField label="Created" value={formatDate(selected.createdAt)} />
                    <CaseField label="Closed" value={selected.closedAt ? formatDate(selected.closedAt) : 'Not recorded'} />
                  </dl>
                  <div className="mt-4 rounded-lg border border-slate-800 bg-slate-950/60 p-3 text-[10px] leading-relaxed text-slate-400">
                    {selected.descriptionPreview || 'No public description supplied.'}
                  </div>
                  <LegalCompliancePanel record={selected} onOpenLibrary={() => setLegalLibraryOpen(true)} />
                  {documents.length > 0 && (
                    <div className="mt-3">
                      <div className="text-[8px] font-semibold uppercase tracking-wider text-slate-400">Public documents</div>
                      <div className="mt-1.5 max-h-64 space-y-1.5 overflow-y-auto pr-1">
                        {documents.map((document) => {
                          const canOpen = document.extractionStatus === 'SEARCHABLE'
                            || document.extractionStatus === 'SALVAGED';
                          const stateLabel = formatExtractionState(document);
                          return (
                            <button
                              key={document.documentId}
                              type="button"
                              disabled={!canOpen}
                              title={document.extractionDeferredReason ?? stateLabel}
                              onClick={() => void openDocument(document)}
                              className="flex w-full items-center justify-between gap-3 rounded-lg border border-slate-800 bg-slate-950/50 px-3 py-2 text-left transition hover:border-indigo-400/30 hover:bg-indigo-500/[0.05] disabled:cursor-default disabled:opacity-65 disabled:hover:border-slate-800 disabled:hover:bg-slate-950/50"
                            >
                              <span className="min-w-0">
                                <span className="block truncate text-[10px] font-medium text-slate-300">{document.fileName}</span>
                                <span className="mt-0.5 block truncate text-[8px] text-slate-400">{stateLabel}</span>
                              </span>
                              <span className="shrink-0 font-mono text-[8px] text-slate-400">{formatFileSize(document.fileSize)}</span>
                            </button>
                          );
                        })}
                      </div>
                    </div>
                  )}
                  {documentError && <div role="alert" className="mt-2 text-[9px] leading-relaxed text-rose-300">Document view unavailable: {documentError}</div>}
                  {hasReportableFinding(selected) && (
                    <div className="mt-3">
                      <button
                        type="button"
                        disabled={exportState?.key === `${selected.portalKey}-${selected.requestId}` && exportState.isExporting}
                        onClick={() => void exportBrief(selected)}
                        className="rounded-lg border border-amber-400/25 bg-amber-500/10 px-3 py-2 text-[10px] font-semibold text-amber-200 transition hover:border-amber-300/40 hover:bg-amber-500/15 disabled:cursor-wait disabled:opacity-60"
                      >
                        {exportState?.key === `${selected.portalKey}-${selected.requestId}` && exportState.isExporting ? 'Compiling protected draft…' : 'Generate administrative appeal draft'}
                      </button>
                      {exportState?.key === `${selected.portalKey}-${selected.requestId}` && exportState.result && (
                        <div className="mt-2 break-all font-mono text-[8px] leading-relaxed text-emerald-300">
                          <div>Saved immutable report: {exportState.result.outputPath}</div>
                          <div>Signed evidence package: {exportState.result.packagePath}</div>
                          <div>Signer {exportState.result.signatureAlgorithm}: {exportState.result.signerKeyId}</div>
                        </div>
                      )}
                      {exportState?.key === `${selected.portalKey}-${selected.requestId}` && exportState.error && (
                        <div role="alert" className="mt-2 text-[9px] leading-relaxed text-rose-300">{exportState.error}</div>
                      )}
                    </div>
                  )}
                </div>
              ) : (
                <div className="mt-3 rounded-lg border border-dashed border-slate-800 px-4 py-6 text-center text-[10px] leading-relaxed text-slate-400">Select a record to inspect its evidence summary and compliance markers.</div>
              )}
            </section>

            <div className="min-h-[220px]"><ExtractionTerminal /></div>
          </div>
        </div>
      </div>
      {activeDocument && (
        <div className="app-backdrop fixed inset-0 z-50 grid place-items-center p-8 backdrop-blur-sm" role="dialog" aria-modal="true" aria-label={`Document ${activeDocument.summary.fileName}`}>
          <div className={cn('relative h-[min(760px,88vh)]', isPdfDocument(activeDocument.summary) ? 'w-[min(1440px,96vw)]' : 'w-[min(1080px,92vw)]')}>
            <button type="button" autoFocus onClick={() => setActiveDocument(null)} className="absolute -top-7 right-0 z-10 rounded-md border border-slate-700 bg-slate-950/90 px-2 py-1 text-[9px] font-semibold text-slate-300 shadow-lg hover:border-slate-500">Close</button>
            <div className="grid h-full grid-cols-[minmax(0,1fr)_360px] gap-4">
              <DocumentViewer
                documentId={activeDocument.summary.documentId}
                requestId={selectedRequestId ?? undefined}
                portalKey={activeDocument.summary.portalKey}
                fileName={activeDocument.summary.fileName}
                textContent={activeDocument.chunk.maskedText}
                fileSizeFormatted={formatFileSize(activeDocument.summary.fileSize)}
                textPositionLabel={`${activeDocument.chunk.characterOffset.toLocaleString()}–${Math.min(activeDocument.chunk.characterOffset + activeDocument.chunk.returnedCharacterCount, activeDocument.chunk.totalCharacterCount).toLocaleString()} of ${activeDocument.chunk.totalCharacterCount.toLocaleString()} characters`}
                characterOffset={activeDocument.chunk.characterOffset}
                knownMaskedFindingCount={activeDocument.chunk.maskedFindingCount}
                onNextChunk={activeDocument.chunk.nextCharacterOffset === null ? undefined : () => void openDocument(activeDocument.summary, activeDocument.chunk.nextCharacterOffset ?? 0)}
              />
              <aside className="min-h-0 space-y-4 overflow-y-auto" aria-label="Document evidence details">
                <MetadataCard
                  documentId={activeDocument.summary.documentId}
                  portalKey={activeDocument.summary.portalKey}
                  sha256={activeDocument.summary.rawFileSha256}
                  contentType={activeDocument.summary.contentType}
                  visibility={activeDocument.summary.visibility}
                  downloadStatus={activeDocument.summary.downloadStatus}
                  fileSize={activeDocument.summary.fileSize}
                />
                {isPdfDocument(activeDocument.summary) && <UnredactorPanel documentId={activeDocument.summary.documentId} portalKey={activeDocument.summary.portalKey} />}
              </aside>
            </div>
          </div>
        </div>
      )}
      <LegalAuthorityManager isOpen={isLegalLibraryOpen} onClose={closeLegalLibrary} />
    </main>
  );
}

function readEvidenceLink() {
  const parameters = new URLSearchParams(window.location.hash.replace(/^#/, ''));
  const portalKey = parameters.get('portal')?.trim();
  const requestValue = parameters.get('request');
  const documentValue = parameters.get('document');
  if (!portalKey || requestValue === null || documentValue === null) return null;
  const requestId = Number(requestValue);
  const documentId = Number(documentValue);
  const characterOffset = Number(parameters.get('offset') ?? 0);
  if (!Number.isSafeInteger(requestId) || requestId < 0 || !Number.isSafeInteger(documentId) || documentId < 0 || !Number.isSafeInteger(characterOffset) || characterOffset < 0) return null;
  return { portalKey, requestId, documentId, characterOffset };
}

function EmptyResults({ error }: { error: string | null }) {
  return (
    <div className="grid h-full min-h-72 place-items-center p-8 text-center">
      <div className="max-w-xs">
        <div className="mx-auto grid h-9 w-9 place-items-center rounded-lg border border-slate-800 bg-slate-950 text-slate-400">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" className="h-4 w-4" aria-hidden="true"><path strokeLinecap="round" strokeLinejoin="round" d="M3 7.5h18M5.25 3.75h13.5A2.25 2.25 0 0 1 21 6v12a2.25 2.25 0 0 1-2.25 2.25H5.25A2.25 2.25 0 0 1 3 18V6a2.25 2.25 0 0 1 2.25-2.25Z" /></svg>
        </div>
        <h2 className="mt-3 text-xs font-semibold text-slate-400">{error ? 'Search unavailable' : 'No matching evidence'}</h2>
        <p className="mt-1 text-[10px] leading-relaxed text-slate-400">
          {error
            ? 'The query did not complete, so there is no result set to interpret. Retry before drawing a conclusion from these filters.'
            : 'Add or synchronize a source, or broaden the active department, status, and policy filters.'}
        </p>
      </div>
    </div>
  );
}

function StatusBadge({ status }: { status: string }) {
  const closed = status.toLowerCase() === 'closed';
  return <span className={cn('rounded-full border px-1.5 py-0.5 text-[8px] font-semibold uppercase tracking-wider', closed ? 'border-slate-700 bg-slate-800/70 text-slate-400' : 'border-emerald-500/20 bg-emerald-500/10 text-emerald-300')}>{status}</span>;
}

function PolicyBadge({ flag }: { flag: string }) {
  return <span className="rounded border border-amber-500/20 bg-amber-500/[0.07] px-1.5 py-0.5 text-[8px] font-medium text-amber-300">{flag.split('_').join(' ')}</span>;
}

function CaseField({ label, value }: { label: string; value: string }) {
  return <div><dt className="text-[8px] font-semibold uppercase tracking-wider text-slate-400">{label}</dt><dd className="mt-1 font-mono text-slate-400">{value}</dd></div>;
}

function formatDate(value: string) {
  const date = new Date(value);
  return Number.isNaN(date.valueOf()) ? value : new Intl.DateTimeFormat(undefined, { month: 'short', day: 'numeric', year: 'numeric' }).format(date);
}

function formatFileSize(value: number | null) {
  if (value === null) return 'Size unavailable';
  if (value < 1024) return `${value} B`;
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KB`;
  return `${(value / (1024 * 1024)).toFixed(1)} MB`;
}

function isPdfDocument(document: RequestDocumentSummary) {
  return document.contentType?.toLocaleLowerCase() === 'application/pdf'
    || document.fileName.toLocaleLowerCase().endsWith('.pdf');
}

function formatExtractionState(document: RequestDocumentSummary) {
  if (document.extractionStatus === 'SEARCHABLE') return 'Searchable text';
  if (document.extractionStatus === 'SALVAGED') return 'Searchable text · recovered';
  if (document.extractionStatus === 'EXPANDED') return `Container expanded${document.extractionFormat ? ` · ${document.extractionFormat}` : ''}`;
  if (document.extractionStatus === 'DEFERRED') return `Not searchable${document.extractionFormat ? ` · ${document.extractionFormat}` : ''}`;
  if (document.extractionStatus === 'FAILED') return 'Extraction failed';
  if (document.extractionStatus === 'DOWNLOADED') return 'Downloaded · no searchable text recorded';
  return 'Extraction pending';
}

function hasReportableFinding(record: RequestSearchResult) {
  return record.integrityFlags.some((flag) => [
    'STATUTORY_LATENCY_VIOLATION',
    'STATUTORY_EXTENSION_DEFECTIVE',
    'SUSPICIOUSLY_FAST_CLOSURE',
  ].includes(flag));
}
