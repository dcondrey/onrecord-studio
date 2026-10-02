import { useCallback, useEffect, useRef, useState } from 'react';
import SearchFilter from '../filters/SearchFilter';
import { useAppStore } from '../../store/useAppStore';
import { useScrollRestore } from '../../hooks/useScrollRestore';
import ObjectLink from '../objects/ObjectLink';
import CaseScopeBar from './CaseScopeBar';
import { useCaseScope } from '../../hooks/useCaseScope';
import { cn } from '../../utils/cn';
import DocumentViewer from '../documents/DocumentViewer';
import AnnotationSearch from '../documents/AnnotationSearch';
import UnredactorPanel from '../documents/UnredactorPanel';
import LegalAuthorityManager from '../legal/LegalAuthorityManager';
import LegalCompliancePanel from '../legal/LegalCompliancePanel';
import { isTauri } from '@tauri-apps/api/core';
import { invokeCommand } from '../../ipc';
import StatutoryDeadlineAppeal from './StatutoryDeadlineAppeal';
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
import BriefingWorkspace from './BriefingWorkspace';
import StoryDesk from './StoryDesk';
import AgencyComparison from './AgencyComparison';
import MeetingsView from './MeetingsView';
import { documentBriefingEvidence, requestBriefingEvidence } from '../../utils/briefing';
import DialogBackdrop from '../ui/DialogBackdrop';
import AnalysisWorkbench from './AnalysisWorkbench';
import CaseTimeline from './CaseTimeline';
import { formatMatchCount, MATCH_BASIS_LABELS, matchBasis, SEARCH_PAGE_SIZE } from '../../utils/searchResults';
import VirtualList from '../ui/VirtualList';

/** Fixed so the list can be windowed from the scroll offset; the row clamps its text to fit. */
const RESULT_ROW_HEIGHT = 148;

export default function MainLayout() {
  // Request selection lives in the shared workspace store and survives view switches. The open
  // document chunk is session-local and is deliberately cleared when the selected request changes.
  const workspaceView = useAppStore((state) => state.workspaceView);
  useScrollRestore();
  const evidenceMode = useAppStore((state) => state.evidenceMode);
  const setEvidenceMode = useAppStore((state) => state.setEvidenceMode);
  const setWorkspaceView = useAppStore((state) => state.setWorkspaceView);
  const evidenceRefreshRequests = useAppStore((state) => state.evidenceRefreshRequests);
  const results = useAppStore((state) => state.filteredResults);
  const resultsTerm = useAppStore((state) => state.filteredResultsTerm);
  const resultsTotal = useAppStore((state) => state.filteredResultsTotal);
  const searchPaging = useAppStore((state) => state.searchPaging);
  const searchError = useAppStore((state) => state.searchError);
  const selectedRequestId = useAppStore((state) => state.selectedRequestId);
  const selectedRequestPortalKey = useAppStore((state) => state.selectedRequestPortalKey);
  const setSelectedRequest = useAppStore((state) => state.setSelectedRequest);
  const syncingPortalKeys = useAppStore((state) => state.syncingPortalKeys);
  const portals = useAppStore((state) => state.portals);
  const briefingEvidence = useAppStore((state) => state.briefingProject.evidence);
  const addBriefingEvidence = useAppStore((state) => state.addBriefingEvidence);
  const [exportState, setExportState] = useState<{ key: string; isExporting: boolean; result: LegalBriefExportResult | null; error: string | null } | null>(null);
  const [documents, setDocuments] = useState<RequestDocumentSummary[]>([]);
  const [documentError, setDocumentError] = useState<string | null>(null);
  const [activeDocument, setActiveDocument] = useState<{ summary: RequestDocumentSummary; chunk: DocumentTextChunk } | null>(null);
  const [pendingEvidenceLink, setPendingEvidenceLink] = useState(readEvidenceLink);
  const requestedEvidenceLink = useAppStore((state) => state.requestedEvidenceLink);
  const consumeEvidenceLink = useAppStore((state) => state.consumeEvidenceLink);
  // Any surface may ask for a document; the evidence view owns opening it.
  useEffect(() => {
    if (!requestedEvidenceLink) return;
    const link = consumeEvidenceLink();
    if (link) setPendingEvidenceLink(link);
  }, [consumeEvidenceLink, requestedEvidenceLink]);
  // Held as the summary rather than an id, so the confirmation names the file the operator clicked
  // even if the list refreshes under it.
  const [pendingDocumentPurge, setPendingDocumentPurge] = useState<RequestDocumentSummary | null>(null);
  const [isPurgingDocument, setPurgingDocument] = useState(false);
  // The request the loaded document list belongs to. A purge started for request A that resolves
  // after B is selected must not write A's outcome, or its error, into B's panel.
  const documentOwnerRef = useRef<{ requestId: number; portalKey: string } | null>(null);
  const [isLegalLibraryOpen, setLegalLibraryOpen] = useState(false);
  const closeLegalLibrary = useCallback(() => setLegalLibraryOpen(false), []);
  const selected = results.find(
    (result) => result.requestId === selectedRequestId && result.portalKey === selectedRequestPortalKey,
  ) ?? null;
  // A later page belongs to the query that was showing when it was requested. The generation is
  // that identity: a page that lands after a new query replaced the list is dropped, not appended.
  const loadMoreResults = useCallback(async (retry = false) => {
    const { searchPaging: paging, ...store } = useAppStore.getState();
    if (!paging || paging.nextCursor === null || paging.loadingMore || (paging.moreError !== null && !retry)) return;
    const { generation, args, nextCursor } = paging;
    const stillCurrent = () => useAppStore.getState().searchPaging?.generation === generation;
    store.setSearchPageState(generation, { loadingMore: true, moreError: null });
    try {
      const page = await invokeCommand('query_audit_records_page', { ...args, cursor: nextCursor, pageSize: SEARCH_PAGE_SIZE });
      if (!stillCurrent()) return;
      store.appendFilteredResults(generation, page.records, page.nextCursor);
      const after = useAppStore.getState();
      if (page.nextCursor === null && after.filteredResults.length !== after.filteredResultsTotal) {
        // The corpus changed while the pages were read; state the current total rather than the first one.
        const count = await invokeCommand('count_audit_records', args).catch(() => null);
        if (count && stillCurrent()) store.setFilteredResultsTotal(generation, count.total);
      }
    } catch (reason) {
      if (stillCurrent()) store.setSearchPageState(generation, { loadingMore: false, moreError: String(reason) });
    }
  }, []);
  const documentListGate = useLatestRequestGate();
  const documentOpenGate = useLatestRequestGate();
  const exportGate = useLatestRequestGate();
  const selectedSourceUrl = selected
    ? portals.find(({ registration }) => registration.portalKey === selected.portalKey)?.registration.canonicalUrl ?? null
    : null;

  useEffect(() => {
    if (pendingEvidenceLink) {
      setSelectedRequest(pendingEvidenceLink.requestId, pendingEvidenceLink.portalKey);
      setWorkspaceView('evidence');
    }
  }, [pendingEvidenceLink, setSelectedRequest, setWorkspaceView]);

  useEffect(() => {
    const request = documentListGate.begin();
    const target = selectedRequestId !== null && selectedRequestPortalKey
      ? { requestId: selectedRequestId, portalKey: selectedRequestPortalKey }
      : null;
    const owner = documentOwnerRef.current;
    if (owner?.requestId !== target?.requestId || owner?.portalKey !== target?.portalKey) {
      documentOpenGate.invalidate();
      setDocuments([]);
      setDocumentError(null);
      setActiveDocument(null);
      setPendingDocumentPurge(null);
    }
    documentOwnerRef.current = target;
    if (!target) return documentListGate.invalidate;
    void invokeCommand('list_request_documents', {
      requestId: target.requestId,
      portalKey: target.portalKey,
    }).then((values) => {
      if (documentListGate.isCurrent(request)) { setDocuments(values); setDocumentError(null); }
    }).catch((reason) => {
      if (documentListGate.isCurrent(request)) setDocumentError(String(reason));
    });
    return documentListGate.invalidate;
  }, [documentListGate, documentOpenGate, evidenceRefreshRequests, selectedRequestPortalKey, selectedRequestId]);

  const purgeDocument = async () => {
    const summary = pendingDocumentPurge;
    // The request this purge was started under, compared again on arrival before anything is
    // written. See the note on `documentOwnerRef`.
    const owner = documentOwnerRef.current;
    if (!summary) return;
    setDocumentError(null);
    setPurgingDocument(true);
    try {
      const purge = await invokeCommand('purge_document', {
        sourceKey: summary.portalKey,
        documentId: summary.documentId,
      });
      if (documentOwnerRef.current?.requestId !== owner?.requestId
        || documentOwnerRef.current?.portalKey !== owner?.portalKey) return;
      setDocuments((current) => current.filter((document) =>
        document.documentId !== purge.documentId || document.portalKey !== purge.sourceKey));
      setActiveDocument((current) =>
        current
        && current.summary.documentId === purge.documentId
        && current.summary.portalKey === purge.sourceKey
          ? null
          : current);
      setPendingDocumentPurge(null);
    } catch (reason) {
      if (documentOwnerRef.current?.requestId !== owner?.requestId
        || documentOwnerRef.current?.portalKey !== owner?.portalKey) return;
      setDocumentError(String(reason));
    } finally {
      setPurgingDocument(false);
    }
  };

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
    // `documents` still belongs to the previously selected request until the list effect has
    // switched owners; looking the target up in it would reject a valid link.
    const owner = documentOwnerRef.current;
    if (owner?.requestId !== pendingEvidenceLink.requestId || owner?.portalKey !== pendingEvidenceLink.portalKey) return;
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

  if (workspaceView === 'analysis') {
    return <AnalysisWorkbench />;
  }

  if (workspaceView === 'briefing') {
    return <BriefingWorkspace />;
  }

  if (workspaceView === 'story') {
    return <StoryDesk />;
  }

  if (workspaceView === 'meetings') {
    return <MeetingsView />;
  }

  if (workspaceView === 'compare') {
    return <AgencyComparison />;
  }

  return (
    <main className="flex min-h-0 flex-1 flex-col overflow-hidden">
      <div className="flex shrink-0 items-center justify-between px-5 pb-1 pt-4 lg:px-6">
        <div>
          <div className="app-eyebrow flex items-center gap-2 text-highlight"><Icon name="evidence" className="h-3.5 w-3.5" /> Evidence review</div>
          <h2 className="mt-1.5 text-[26px] font-semibold tracking-[-0.035em] text-fg sm:text-[30px]">Search, inspect, and build the record.</h2>
        </div>
        <div className="flex items-center gap-3">
          <div role="group" aria-label="Evidence view" className="flex gap-1 text-label">
            {(['records', 'timeline'] as const).map((mode) => (
              <button
                key={mode}
                type="button"
                aria-pressed={evidenceMode === mode}
                onClick={() => setEvidenceMode(mode)}
                className={cn('focus-ring rounded-md border px-2.5 py-1 font-semibold', evidenceMode === mode ? 'border-accent/60 text-accent' : 'border-control text-muted')}
              >
                {mode === 'records' ? 'Records' : 'Timeline'}
              </button>
            ))}
          </div>
          <div className="hidden items-center gap-2 text-caption text-muted sm:flex"><Icon name="shield" className="h-3.5 w-3.5 text-success" /> Sensitive fields are masked locally</div>
        </div>
      </div>
      {evidenceMode === 'timeline' ? <CaseTimeline /> : <>
      <CaseRequests />
      <SearchFilter />
      <AnnotationSearch onOpen={setPendingEvidenceLink} />
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="mx-auto grid min-h-[620px] w-full max-w-[1800px] grid-cols-1 gap-4 p-4 lg:p-5 xl:grid-cols-[minmax(0,1.55fr)_minmax(320px,0.75fr)]">
          <section className="app-panel flex min-h-[520px] flex-col overflow-hidden">
            <div className="flex items-center justify-between border-b border-line px-4 py-3">
              <div>
                <h2 className="text-label leading-4 font-semibold text-fg">Evidence workbench</h2>
                <p className="mt-0.5 text-caption text-muted">Request metadata is PII-masked before it reaches this view.</p>
              </div>
              <div className="flex items-center gap-2">
                <button type="button" onClick={() => setLegalLibraryOpen(true)} className="whitespace-nowrap rounded border border-highlight/30 bg-highlight/10 px-2 py-1 text-caption font-semibold text-highlight transition hover:border-highlight/30 hover:bg-highlight/10">Legal authority library</button>
                <span className="rounded border border-line bg-canvas px-2 py-1 font-mono text-caption text-muted">{formatMatchCount(resultsTotal)} records</span>
              </div>
            </div>
            {resultsTotal > results.length && (
              <p role="status" className="border-b border-line px-4 py-2 text-label text-muted">
                Showing {formatMatchCount(results.length)} of {formatMatchCount(resultsTotal)} matches. More load as you scroll, or use Load more below the list.
              </p>
            )}
            {searchPaging?.nextCursor === null && results.length !== resultsTotal && (
              <p role="status" className="border-b border-warning/30 bg-warning/5 px-4 py-2 text-label text-warning">
                The records changed while the pages loaded, so the list holds {formatMatchCount(results.length)} and the count is {formatMatchCount(resultsTotal)}. Search again for a consistent view.
              </p>
            )}
            <div className="flex min-h-0 flex-1 flex-col">
              {results.length === 0 ? (
                <EmptyResults error={searchError} sourceCount={portals.length} />
              ) : (
                <VirtualList
                  items={results}
                  rowHeight={RESULT_ROW_HEIGHT}
                  ariaLabel="Search results"
                  className="h-[60vh] min-h-[360px]"
                  resetKey={String(searchPaging?.generation ?? 'complete')}
                  initialActiveIndex={Math.max(0, results.findIndex((result) => result.requestId === selectedRequestId && result.portalKey === selectedRequestPortalKey))}
                  getKey={(result) => `${result.portalKey}-${result.requestId}`}
                  onNearEnd={() => void loadMoreResults()}
                  footer={searchPaging?.nextCursor != null ? (
                    <div className="flex items-center gap-2 px-4 py-3 text-label text-muted">
                      {searchPaging.moreError !== null && (
                        <span role="alert" className="text-danger">The next page could not be read: {searchPaging.moreError}</span>
                      )}
                      <button
                        type="button"
                        disabled={searchPaging.loadingMore}
                        onClick={() => void loadMoreResults(true)}
                        className="focus-ring rounded border border-control px-2.5 py-1 font-semibold text-fg hover:border-accent/60 disabled:opacity-60"
                      >
                        {searchPaging.loadingMore ? 'Loading…' : searchPaging.moreError !== null ? 'Retry' : 'Load more'}
                      </button>
                    </div>
                  ) : undefined}
                  renderRow={(result, { tabIndex }) => {
                    const isSelected = selectedRequestId === result.requestId && selectedRequestPortalKey === result.portalKey;
                    return (
                      <button
                        type="button"
                        tabIndex={tabIndex}
                        onClick={() => setSelectedRequest(result.requestId, result.portalKey)}
                        className={cn(
                          'grid h-full w-full grid-cols-[minmax(0,1fr)_120px] gap-4 overflow-hidden border-b border-control px-4 py-3 text-left outline-none transition hover:bg-surface/70 focus:bg-surface/70',
                          isSelected && 'bg-highlight/10 ring-1 ring-inset ring-highlight/30',
                        )}
                      >
                        <div className="min-w-0">
                          <div className="flex items-center gap-2">
                            <span className="font-mono text-caption text-muted">{result.portalKey} / {result.requestId}</span>
                            <StatusBadge status={result.status} />
                          </div>
                          <div className="mt-1 truncate text-label leading-4 font-medium text-fg">{result.title || 'Untitled public-records request'}</div>
                          <div className="mt-1 line-clamp-2 text-label leading-relaxed text-muted">{result.descriptionPreview || 'No public description supplied.'}</div>
                          {(() => {
                            const basis = matchBasis(result, resultsTerm);
                            return basis && <p className="mt-1 text-caption uppercase tracking-wider text-muted">{MATCH_BASIS_LABELS[basis]}</p>;
                          })()}
                          {result.integrityFlags.length > 0 && (
                            <div className="mt-2 flex flex-wrap gap-1">
                              {result.integrityFlags.slice(0, 4).map((flag) => <PolicyBadge key={flag} flag={flag} />)}
                            </div>
                          )}
                        </div>
                        <div className="text-right font-mono text-caption text-muted">
                          <div className="truncate text-muted">{result.agencyName}</div>
                          <div className="mt-2">{formatDate(result.createdAt)}</div>
                          <div className="mt-1">{result.documentCount} document{result.documentCount === 1 ? '' : 's'}</div>
                        </div>
                      </button>
                    );
                  }}
                />
              )}
            </div>
          </section>

          <div className="grid min-h-0 grid-rows-[auto_minmax(220px,1fr)] gap-4">
            <section className="app-panel p-4">
              <div className="text-caption font-bold uppercase tracking-[0.18em] text-muted">Case context</div>
              {selected ? (
                <div className="mt-3">
                  <div className="flex items-start justify-between gap-3">
                    <div className="min-w-0">
                      <h2 className="text-sm font-semibold leading-snug text-fg">{selected.title || 'Untitled request'}</h2>
                      <p className="mt-1 text-label text-muted">
                        {selected.agencyId === null
                          ? selected.agencyName
                          : <ObjectLink target={{ kind: 'agency', portalKey: selected.portalKey, agencyId: selected.agencyId, label: selected.agencyName }}>{selected.agencyName}</ObjectLink>}
                        {' · '}{selected.portalKey}
                        {' · '}<ObjectLink target={{ kind: 'request', portalKey: selected.portalKey, requestId: selected.requestId, label: selected.title || `Request ${selected.requestId}` }}>Findings and documents</ObjectLink>
                      </p>
                    </div>
                    <div className="flex shrink-0 flex-col items-end gap-2">
                      <StatusBadge status={selected.status} />
                      <button
                        type="button"
                        disabled={briefingEvidence.some((item) => item.id === `request:${selected.portalKey}:${selected.requestId}`)}
                        onClick={() => addBriefingEvidence(requestBriefingEvidence(selected, selectedSourceUrl))}
                        className="focus-ring rounded-lg border border-highlight/30 bg-highlight/10 px-2.5 py-1.5 text-caption font-semibold text-highlight disabled:cursor-default disabled:border-success/30 disabled:bg-success/5 disabled:text-success"
                      >
                        {briefingEvidence.some((item) => item.id === `request:${selected.portalKey}:${selected.requestId}`) ? 'Saved to briefing' : 'Save lead to briefing'}
                      </button>
                    </div>
                  </div>
                  <dl className="mt-4 grid grid-cols-2 gap-x-4 gap-y-3 border-t border-line pt-3 text-label">
                    <CaseField label="Request ID" value={String(selected.requestId)} />
                    <CaseField label="Documents" value={String(selected.documentCount)} />
                    <CaseField label="Created" value={formatDate(selected.createdAt)} />
                    <CaseField label="Closed" value={selected.closedAt ? formatDate(selected.closedAt) : 'Not recorded'} />
                  </dl>
                  <div className="mt-4 rounded-lg border border-line bg-canvas/70 p-3 text-label leading-relaxed text-muted">
                    {selected.descriptionPreview || 'No public description supplied.'}
                  </div>
                  <LegalCompliancePanel record={selected} onOpenLibrary={() => setLegalLibraryOpen(true)} />
                  {documents.length > 0 && (
                    <div className="mt-3">
                      <div className="text-caption font-semibold uppercase tracking-wider text-muted">Public documents</div>
                      <div className="mt-1.5 max-h-64 space-y-1.5 overflow-y-auto pr-1">
                        {documents.map((document) => {
                          const canOpen = document.extractionStatus === 'SEARCHABLE'
                            || document.extractionStatus === 'SALVAGED';
                          const stateLabel = formatExtractionState(document);
                          return (
                            <div key={document.documentId} className="flex items-stretch gap-1">
                              <button
                                type="button"
                                disabled={!canOpen}
                                title={document.extractionDeferredReason ?? stateLabel}
                                onClick={() => void openDocument(document)}
                                className="flex min-w-0 flex-1 items-center justify-between gap-3 rounded-lg border border-control bg-canvas/70 px-3 py-2 text-left transition hover:border-highlight/30 hover:bg-highlight/5 disabled:cursor-default disabled:opacity-65 disabled:hover:border-control disabled:hover:bg-canvas/70"
                              >
                                <span className="min-w-0">
                                  <span className="block truncate text-label font-medium text-fg-secondary">{document.fileName}</span>
                                  <span className="mt-0.5 block truncate text-caption text-muted">{stateLabel}</span>
                                </span>
                                <span className="shrink-0 font-mono text-caption text-muted">{formatFileSize(document.fileSize)}</span>
                              </button>
                              {/* Its own control, never a modifier on the open action, and never a
                                  swipe or a long press. Destroying a piece of evidence is not a
                                  variation on reading it.

                                  Refused while the source is synchronizing. Purging one document
                                  stops nothing: the crawl that produced it is still running, holds
                                  no record that the document was deleted, and would re-download it
                                  within the same run. A delete that silently undoes itself is a
                                  worse claim about evidence than one that declines. */}
                              <button
                                type="button"
                                disabled={syncingPortalKeys.includes(document.portalKey)}
                                title={syncingPortalKeys.includes(document.portalKey)
                                  ? 'Pause this source before deleting a document; a running crawl would download it again'
                                  : `Delete ${document.fileName} and everything derived from it`}
                                aria-label={`Delete document ${document.fileName}`}
                                onClick={() => setPendingDocumentPurge(document)}
                                className="focus-ring grid w-8 shrink-0 place-items-center rounded-lg border border-control bg-canvas/70 text-muted transition hover:border-danger/30 hover:text-danger disabled:opacity-40 disabled:hover:border-control disabled:hover:text-muted"
                              >
                                <Icon name="trash" className="h-3 w-3" />
                              </button>
                            </div>
                          );
                        })}
                      </div>
                    </div>
                  )}
                  {documentError && <div role="alert" className="mt-2 text-caption leading-relaxed text-danger">Document view unavailable: {documentError}</div>}
                  {isTauri() && selected.integrityFlags.includes('STATUTORY_LATENCY_VIOLATION') && (
                    <div className="mt-3">
                      <StatutoryDeadlineAppeal portalKey={selected.portalKey} requestId={selected.requestId} />
                    </div>
                  )}
                  {hasReportableFinding(selected) && (
                    <div className="mt-3">
                      <button
                        type="button"
                        disabled={exportState?.key === `${selected.portalKey}-${selected.requestId}` && exportState.isExporting}
                        onClick={() => void exportBrief(selected)}
                        className="rounded-lg border border-warning/30 bg-warning/10 px-3 py-2 text-label font-semibold text-warning transition hover:border-warning/30 hover:bg-warning/20 disabled:cursor-wait disabled:opacity-60"
                      >
                        {exportState?.key === `${selected.portalKey}-${selected.requestId}` && exportState.isExporting ? 'Compiling protected draft…' : 'Generate administrative appeal draft'}
                      </button>
                      {exportState?.key === `${selected.portalKey}-${selected.requestId}` && exportState.result && (
                        <div className="mt-2 break-all font-mono text-caption leading-relaxed text-success">
                          <div>Saved immutable report: {exportState.result.outputPath}</div>
                          <div>Signed evidence package: {exportState.result.packagePath}</div>
                          <div>Signer {exportState.result.signatureAlgorithm}: {exportState.result.signerKeyId}</div>
                        </div>
                      )}
                      {exportState?.key === `${selected.portalKey}-${selected.requestId}` && exportState.error && (
                        <div role="alert" className="mt-2 text-caption leading-relaxed text-danger">{exportState.error}</div>
                      )}
                    </div>
                  )}
                </div>
              ) : (
                <div className="mt-3 rounded-lg border border-dashed border-line px-4 py-6 text-center text-label leading-relaxed text-muted">Select a record to inspect its evidence summary and compliance markers.</div>
              )}
            </section>

            <div className="min-h-[220px]"><ExtractionTerminal /></div>
          </div>
        </div>
      </div>
      </>}
      {pendingDocumentPurge && (
        <DialogBackdrop labelledBy="purge-document-title" onClose={() => setPendingDocumentPurge(null)} className="app-backdrop fixed inset-0 z-[100] grid place-items-center p-4 backdrop-blur-sm">
          <div className="app-panel w-full max-w-md overflow-hidden shadow-2xl shadow-black/40">
            <div className="border-b border-line p-4">
              <div className="app-eyebrow text-danger">Evidence</div>
              <h2 id="purge-document-title" className="mt-1 break-all text-sm font-semibold text-fg">Delete {pendingDocumentPurge.fileName}?</h2>
            </div>
            <div className="space-y-2 p-4 text-label leading-relaxed text-muted">
              <p className="text-fg-secondary">Destroys the {formatFileSize(pendingDocumentPurge.fileSize)} file on disk, its extracted text, and every search, provenance, and integrity record derived from it.</p>
              <p>The request this document belongs to is kept, and so is every other document under it. This cannot be undone from inside the application.</p>
              {/* No tombstone survives the delete: the crawl frontier is rebuilt per sync run and
                  the only record that this document was ever ingested is the row being destroyed.
                  Saying so is the difference between a deletion and a deletion the operator thinks
                  is permanent. */}
              <p>Synchronizing this source again can download the same file back. Remove the source to stop that.</p>
            </div>
            <div className="flex items-center justify-end gap-2 border-t border-line px-4 py-3">
              <button type="button" onClick={() => setPendingDocumentPurge(null)} className="rounded-lg px-3 py-2 text-caption font-semibold text-muted hover:text-fg-secondary">Cancel</button>
              <button type="button" disabled={isPurgingDocument} onClick={() => void purgeDocument()} className="focus-ring rounded-lg border border-danger/30 bg-danger/20 px-4 py-2 text-caption font-semibold text-danger disabled:opacity-60">{isPurgingDocument ? 'Deleting…' : 'Delete this document'}</button>
            </div>
          </div>
        </DialogBackdrop>
      )}
      {activeDocument && (
        <DialogBackdrop label={`Document ${activeDocument.summary.fileName}`} onClose={() => setActiveDocument(null)} className="app-backdrop fixed inset-0 z-50 grid place-items-center p-8 backdrop-blur-sm">
          <div className={cn('relative h-[min(760px,88vh)]', isPdfDocument(activeDocument.summary) ? 'w-[min(1440px,96vw)]' : 'w-[min(1080px,92vw)]')}>
            <button type="button" autoFocus onClick={() => setActiveDocument(null)} className="absolute -top-7 right-0 z-10 rounded-md border border-control bg-canvas px-2 py-1 text-caption font-semibold text-fg-secondary shadow-lg hover:border-control">Close</button>
            <div className="grid h-full grid-cols-[minmax(0,1fr)_360px] gap-4">
              <DocumentViewer
                documentId={activeDocument.summary.documentId}
                requestId={selectedRequestId ?? undefined}
                portalKey={activeDocument.summary.portalKey}
                fileName={activeDocument.summary.fileName}
                originalAvailable={!activeDocument.summary.extractionDeferredReason?.startsWith('REFERENCE_ONLY:')}
                textContent={activeDocument.chunk.maskedText}
                fileSizeFormatted={formatFileSize(activeDocument.summary.fileSize)}
                textPositionLabel={`${activeDocument.chunk.characterOffset.toLocaleString()}–${Math.min(activeDocument.chunk.characterOffset + activeDocument.chunk.returnedCharacterCount, activeDocument.chunk.totalCharacterCount).toLocaleString()} of ${activeDocument.chunk.totalCharacterCount.toLocaleString()} characters`}
                characterOffset={activeDocument.chunk.characterOffset}
                knownMaskedFindingCount={activeDocument.chunk.maskedFindingCount}
                onPreviousChunk={activeDocument.chunk.characterOffset > 0 ? () => void openDocument(activeDocument.summary, Math.max(0, activeDocument.chunk.characterOffset - 100000)) : undefined}
                onNextChunk={activeDocument.chunk.nextCharacterOffset === null ? undefined : () => void openDocument(activeDocument.summary, activeDocument.chunk.nextCharacterOffset ?? 0)}
                onSaveEvidence={({ excerpt, characterOffset }) => {
                  if (!selected) return;
                  addBriefingEvidence(documentBriefingEvidence({
                    portalKey: activeDocument.summary.portalKey,
                    requestId: selected.requestId,
                    documentId: activeDocument.summary.documentId,
                    fileName: activeDocument.summary.fileName,
                    characterOffset,
                    sourceUrl: selectedSourceUrl,
                    agencyName: selected.agencyName,
                    requestTitle: selected.title,
                    excerpt,
                    rawFileSha256: activeDocument.summary.rawFileSha256,
                  }));
                }}
              />
              <aside className="min-h-0 space-y-4 overflow-y-auto" aria-label="Document evidence details">
                {activeDocument.summary.extractionDeferredReason?.startsWith('REFERENCE_ONLY:') && <p className="rounded border border-warning/30 p-3 text-label leading-4 text-warning">{activeDocument.summary.extractionDeferredReason.slice('REFERENCE_ONLY:'.length)}</p>}
                {activeDocument.summary.originalSourceUri && <a className="block break-all text-label leading-4 text-accent underline" href={activeDocument.summary.originalSourceUri} target="_blank" rel="noreferrer">View publisher’s original file</a>}
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
        </DialogBackdrop>
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

/** With a case active, the requests holding its sources, one click from their documents and findings. */
function CaseRequests() {
  const caseScope = useCaseScope();
  if (caseScope.status === 'off') return null;
  return (
    <div className="mx-5 mt-2 space-y-2 lg:mx-6">
      <CaseScopeBar state={caseScope} noun="requests" listOnly />
      {caseScope.status === 'ready' && (
        caseScope.value.scope.requests.length === 0 ? (
          <p className="text-label text-muted">None of this case's sources is a stored document, so no request is linked to it.</p>
        ) : (
          <ul aria-label="Requests in the active case" className="motion-stagger flex flex-wrap gap-2">
            {caseScope.value.scope.requests.map((request) => (
              <li key={`${request.portalKey}:${request.requestId}`} className="rounded-lg border border-line bg-surface/60 px-2.5 py-1 text-label">
                <ObjectLink target={{ kind: 'request', portalKey: request.portalKey, requestId: request.requestId, label: `Request ${request.requestId}` }}>
                  Request {request.requestId}
                </ObjectLink>
                <span className="text-muted"> · {request.portalKey}</span>
              </li>
            ))}
          </ul>
        )
      )}
    </div>
  );
}

function EmptyResults({ error, sourceCount }: { error: string | null; sourceCount: number }) {
  const searchTerm = useAppStore((state) => state.searchTerm);
  const agencyId = useAppStore((state) => state.selectedAgencyId);
  const status = useAppStore((state) => state.selectedStatus);
  const policy = useAppStore((state) => state.selectedPolicyDeviation);
  const activePortalKey = useAppStore((state) => state.activePortalKey);
  const setSearchFilters = useAppStore((state) => state.setSearchFilters);
  const setActivePortal = useAppStore((state) => state.setActivePortal);
  const narrowing = [
    searchTerm.trim() && { label: `the text "${searchTerm.trim()}"`, remove: () => setSearchFilters('', agencyId, status, policy) },
    agencyId !== null && { label: 'the department filter', remove: () => setSearchFilters(searchTerm, null, status, policy) },
    status && { label: `status "${status}"`, remove: () => setSearchFilters(searchTerm, agencyId, null, policy) },
    policy && { label: 'the policy-deviation filter', remove: () => setSearchFilters(searchTerm, agencyId, status, null) },
    activePortalKey && { label: `the ${activePortalKey} source only`, remove: () => setActivePortal(null) },
  ].filter((item): item is { label: string; remove: () => void } => Boolean(item));
  return (
    <div className="grid h-full min-h-72 place-items-center p-8 text-center">
      <div className="max-w-sm">
        <div className="mx-auto grid h-9 w-9 place-items-center rounded-lg border border-line bg-canvas text-muted">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" className="h-4 w-4" aria-hidden="true"><path strokeLinecap="round" strokeLinejoin="round" d="M3 7.5h18M5.25 3h13.5A2.25 2.25 0 0 1 21 5.25v13.5A2.25 2.25 0 0 1 18.75 21H5.25A2.25 2.25 0 0 1 3 18.75V5.25A2.25 2.25 0 0 1 5.25 3Z" /></svg>
        </div>
        <h2 className="mt-3 text-label leading-4 font-semibold text-muted">{error ? 'Search unavailable' : 'No matching evidence'}</h2>
        {error ? (
          <p className="mt-1 text-label leading-relaxed text-muted">
            The query did not complete, so there is no result set to interpret. Retry before drawing a conclusion from these filters.
          </p>
        ) : (
          <>
            <p className="mt-1 text-label leading-relaxed text-muted">
              {sourceCount === 0
                ? 'No sources are added yet, so there is nothing to search.'
                : `Searched ${activePortalKey ? '1 source' : `${sourceCount} source${sourceCount === 1 ? '' : 's'}`}: request titles and descriptions, and extracted text of documents already ingested. Nothing here shows that no such record exists. It may not have been collected, ingested yet, or readable as text.`}
            </p>
            {narrowing.length > 0 && (
              <div className="mt-3 flex flex-wrap justify-center gap-1.5" role="group" aria-label="Narrowing the search">
                {narrowing.map((item) => (
                  <button key={item.label} type="button" onClick={item.remove} className="focus-ring rounded-full border border-control px-2.5 py-1 text-caption font-medium text-fg-secondary hover:border-accent/60">
                    Remove {item.label}
                  </button>
                ))}
              </div>
            )}
          </>
        )}
      </div>
    </div>
  );
}

function StatusBadge({ status }: { status: string }) {
  // Portal status text is free-form; only the closed and past-due wordings get their own tone.
  const normalized = status.toLowerCase();
  const tone = normalized === 'closed'
    ? 'border-line-strong bg-raised/70 text-muted'
    : ['overdue', 'past due', 'late'].some((wording) => normalized.includes(wording))
      ? 'border-warning/30 bg-warning/10 text-warning'
      : 'border-accent/30 bg-accent/10 text-accent';
  return <span className={cn('rounded-full border px-1.5 py-0.5 text-caption font-semibold uppercase tracking-wider', tone)}>{status}</span>;
}

function PolicyBadge({ flag }: { flag: string }) {
  return <span className="rounded border border-warning/30 bg-warning/10 px-1.5 py-0.5 text-caption font-medium text-warning">{flag.split('_').join(' ')}</span>;
}

function CaseField({ label, value }: { label: string; value: string }) {
  return <div><dt className="text-caption font-semibold uppercase tracking-wider text-muted">{label}</dt><dd className="mt-1 font-mono text-muted">{value}</dd></div>;
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
  if (document.extractionDeferredReason?.startsWith('REFERENCE_ONLY:')) return false;
  return document.contentType?.toLocaleLowerCase() === 'application/pdf'
    || document.fileName.toLocaleLowerCase().endsWith('.pdf');
}

function formatExtractionState(document: RequestDocumentSummary) {
  if (document.extractionDeferredReason?.startsWith('REFERENCE_ONLY:')) return 'Searchable text · original referenced';
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
