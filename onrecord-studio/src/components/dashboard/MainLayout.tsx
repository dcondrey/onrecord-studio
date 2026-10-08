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
import { formatNativeError, invokeCommand } from '../../ipc';
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
import Dialog, { DialogBody, DialogFooter, DialogHeader } from '../ui/Dialog';
import AnalysisWorkbench from './AnalysisWorkbench';
import CaseTimeline from './CaseTimeline';
import { formatMatchCount, MATCH_BASIS_LABELS, matchBasis, SEARCH_PAGE_SIZE } from '../../utils/searchResults';
import { formatShortDate } from '../objects/labels';
import VirtualList from '../ui/VirtualList';
import PageHeader from '../ui/PageHeader';
import Button from '../ui/Button';
import Badge from '../ui/Badge';
import InlineError from '../ui/InlineError';
import Skeleton from '../ui/Skeleton';

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
  const [documentsStatus, setDocumentsStatus] = useState<'idle' | 'pending' | 'ready' | 'failed'>('idle');
  const [documentListRetries, setDocumentListRetries] = useState(0);
  const [openingDocumentId, setOpeningDocumentId] = useState<number | null>(null);
  const [documentError, setDocumentError] = useState<string | null>(null);
  const syncLogCount = useAppStore((state) => state.syncLogs.length);
  const detailsRef = useRef<HTMLElement>(null);
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
      if (stillCurrent()) store.setSearchPageState(generation, { loadingMore: false, moreError: formatNativeError(reason) });
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
      setOpeningDocumentId(null);
      setActiveDocument(null);
      setPendingDocumentPurge(null);
    }
    documentOwnerRef.current = target;
    if (!target) {
      setDocumentsStatus('idle');
      return documentListGate.invalidate;
    }
    setDocumentsStatus((current) => (current === 'ready' && owner?.requestId === target.requestId && owner?.portalKey === target.portalKey ? current : 'pending'));
    void invokeCommand('list_request_documents', {
      requestId: target.requestId,
      portalKey: target.portalKey,
    }).then((values) => {
      if (documentListGate.isCurrent(request)) { setDocuments(values); setDocumentError(null); setDocumentsStatus('ready'); }
    }).catch((reason) => {
      if (documentListGate.isCurrent(request)) { setDocumentError(formatNativeError(reason)); setDocumentsStatus('failed'); }
    });
    return documentListGate.invalidate;
  }, [documentListGate, documentOpenGate, documentListRetries, evidenceRefreshRequests, selectedRequestPortalKey, selectedRequestId]);

  // Below the two-column breakpoint the details sit under the list, so a selection would change
  // nothing visible without bringing them into view.
  useEffect(() => {
    if (selectedRequestId === null || !detailsRef.current) return;
    if (window.matchMedia?.('(max-width: 1023px)').matches) detailsRef.current.scrollIntoView?.({ block: 'start' });
  }, [selectedRequestId, selectedRequestPortalKey]);

  const purgeDocument = async () => {
    const summary = pendingDocumentPurge;
    // The request this purge was started under, compared again on arrival before anything is
    // written. See the note on `documentOwnerRef`.
    const owner = documentOwnerRef.current;
    const stillOwner = () => documentOwnerRef.current?.requestId === owner?.requestId && documentOwnerRef.current?.portalKey === owner?.portalKey;
    if (!summary) return;
    setDocumentError(null);
    setPurgingDocument(true);
    try {
      const purge = await invokeCommand('purge_document', {
        sourceKey: summary.portalKey,
        documentId: summary.documentId,
      });
      if (!stillOwner()) return;
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
      if (!stillOwner()) return;
      setDocumentError(formatNativeError(reason));
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
    setOpeningDocumentId(summary.documentId);
    try {
      const chunk = await invokeCommand('get_document_text_chunk', {
        documentId: summary.documentId,
        portalKey: summary.portalKey,
        characterOffset,
        characterLimit: 100000,
      });
      if (!documentOpenGate.isCurrent(request)) return;
      setOpeningDocumentId(null);
      setActiveDocument({ summary, chunk });
    } catch (reason) {
      if (!documentOpenGate.isCurrent(request)) return;
      setOpeningDocumentId(null);
      setDocumentError(formatNativeError(reason));
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
        setExportState({ key, isExporting: false, result: null, error: formatNativeError(reason) });
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

  const selectedKey = selected ? `${selected.portalKey}-${selected.requestId}` : null;
  const savedToBriefing = selected ? briefingEvidence.some((item) => item.id === `request:${selected.portalKey}:${selected.requestId}`) : false;
  const portalName = (portalKey: string) => portals.find(({ registration }) => registration.portalKey === portalKey)?.registration.displayName ?? portalKey;

  return (
    <main className="flex min-h-0 flex-1 flex-col overflow-hidden">
      <div className="shrink-0 px-5 pt-6 lg:px-6">
        <PageHeader
          title="Evidence"
          summary={<span className="inline-flex items-center gap-1.5"><Icon name="shield" size="xs" className="text-success" /> Sensitive fields are masked locally before they reach this view.</span>}
          actions={(
            <div role="group" aria-label="Evidence view" className="inline-flex h-8 items-center gap-0.5 rounded-lg bg-raised p-0.5">
              {(['records', 'timeline'] as const).map((mode) => (
                <button
                  key={mode}
                  type="button"
                  aria-pressed={evidenceMode === mode}
                  onClick={() => setEvidenceMode(mode)}
                  className={cn(
                    'focus-ring inline-flex h-full items-center rounded-md px-3 text-control transition-colors duration-fast',
                    evidenceMode === mode ? 'bg-canvas text-fg shadow-panel' : 'text-fg-secondary hover:text-fg',
                  )}
                >
                  {mode === 'records' ? 'Records' : 'Timeline'}
                </button>
              ))}
            </div>
          )}
        />
      </div>
      {evidenceMode === 'timeline' ? <CaseTimeline /> : <>
      <CaseRequests />
      <SearchFilter />
      <AnnotationSearch onOpen={setPendingEvidenceLink} />
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="mx-auto grid w-full max-w-content grid-cols-1 items-start gap-4 px-5 pb-6 lg:grid-cols-[minmax(0,1fr)_minmax(20rem,24rem)] lg:px-6">
          <section aria-labelledby="evidence-requests-title" className="app-panel flex min-h-[520px] min-w-0 flex-col overflow-hidden">
            <div className="flex h-12 items-center border-b border-line px-4">
              <h3 id="evidence-requests-title" className="text-heading text-fg">Requests</h3>
            </div>
            {resultsTotal > results.length && (
              <p role="status" className="border-b border-line px-4 py-2 text-caption tabular-nums text-muted">
                Showing {formatMatchCount(results.length)} of {formatMatchCount(resultsTotal)} matches. More load as you scroll.
              </p>
            )}
            {searchPaging?.nextCursor === null && results.length !== resultsTotal && (
              <p role="status" className="border-b border-warning/30 bg-warning/10 px-4 py-2 text-caption text-warning">
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
                    <div className="flex flex-wrap items-center gap-2 px-4 py-3 text-caption text-muted">
                      {searchPaging.moreError !== null && (
                        <span role="alert" className="min-w-0 break-words text-danger">The next page could not be read: {searchPaging.moreError}</span>
                      )}
                      <Button size="sm" disabled={searchPaging.loadingMore} onClick={() => void loadMoreResults(true)}>
                        {searchPaging.loadingMore ? 'Loading…' : searchPaging.moreError !== null ? 'Try again' : 'Load more'}
                      </Button>
                    </div>
                  ) : undefined}
                  renderRow={(result, { tabIndex }) => {
                    const isSelected = selectedRequestId === result.requestId && selectedRequestPortalKey === result.portalKey;
                    const basis = matchBasis(result, resultsTerm);
                    return (
                      <button
                        type="button"
                        tabIndex={tabIndex}
                        aria-current={isSelected ? 'true' : undefined}
                        onClick={() => setSelectedRequest(result.requestId, result.portalKey)}
                        className={cn(
                          'focus-ring grid h-full w-full grid-cols-[minmax(0,1fr)_104px] gap-4 overflow-hidden border-b border-line px-4 py-3 text-left transition-colors duration-fast hover:bg-raised/60 focus-visible:ring-inset',
                          isSelected && 'bg-accent/5 shadow-[inset_2px_0_0_rgb(var(--accent))]',
                        )}
                      >
                        <span className="block min-w-0">
                          <span className="flex min-w-0 items-center gap-1.5 text-caption text-muted">
                            <StatusBadge status={result.status} />
                            <span className="min-w-0 truncate">
                              {portalName(result.portalKey)} · {result.agencyName}
                              {basis && ` · ${MATCH_BASIS_LABELS[basis].toLowerCase()}`}
                            </span>
                          </span>
                          <span className="mt-1 block truncate text-body font-medium text-fg">{result.title || 'Untitled public-records request'}</span>
                          <span className="mt-0.5 line-clamp-2 text-body text-muted">{result.descriptionPreview || 'No public description supplied.'}</span>
                          {result.integrityFlags.length > 0 && (
                            <span className="mt-2 flex h-5 flex-wrap gap-1 overflow-hidden">
                              {result.integrityFlags.slice(0, 4).map((flag) => <PolicyBadge key={flag} flag={flag} />)}
                            </span>
                          )}
                        </span>
                        <span className="block text-right text-caption tabular-nums text-muted">
                          <span className="block">{formatShortDate(result.createdAt)}</span>
                          <span className="mt-1 block">{result.documentCount} document{result.documentCount === 1 ? '' : 's'}</span>
                        </span>
                      </button>
                    );
                  }}
                />
              )}
            </div>
          </section>

          <div className="grid min-w-0 gap-4 lg:sticky lg:top-0 lg:max-h-[calc(100vh-4rem)] lg:overflow-y-auto">
            <section ref={detailsRef} aria-labelledby="request-details-title" className="app-panel min-w-0 scroll-mt-4 p-4">
              <h3 id="request-details-title" className="text-overline uppercase text-muted">Request details</h3>
              {selected ? (
                <div className="mt-3">
                  <div className="flex items-center justify-between gap-3">
                    <StatusBadge status={selected.status} />
                    <Button
                      size="sm"
                      disabled={savedToBriefing}
                      onClick={() => addBriefingEvidence(requestBriefingEvidence(selected, selectedSourceUrl))}
                    >
                      {savedToBriefing ? 'Saved to briefing' : 'Save to briefing'}
                    </Button>
                  </div>
                  <h4 className="mt-3 break-words text-heading text-fg">{selected.title || 'Untitled request'}</h4>
                  <p className="mt-1 text-caption text-muted">
                    {selected.agencyId === null
                      ? selected.agencyName
                      : <ObjectLink target={{ kind: 'agency', portalKey: selected.portalKey, agencyId: selected.agencyId, label: selected.agencyName }}>{selected.agencyName}</ObjectLink>}
                    {' · '}<ObjectLink target={{ kind: 'request', portalKey: selected.portalKey, requestId: selected.requestId, label: selected.title || `Request ${selected.requestId}` }}>Open in inspector</ObjectLink>
                  </p>
                  <dl className="mt-4 grid grid-cols-2 gap-x-4 gap-y-3 border-t border-line pt-3">
                    <CaseField label="Request ID" value={String(selected.requestId)} code />
                    <CaseField label="Documents" value={String(selected.documentCount)} />
                    <CaseField label="Created" value={formatShortDate(selected.createdAt)} />
                    <CaseField label="Closed" value={selected.closedAt ? formatShortDate(selected.closedAt) : 'Not recorded'} absent={!selected.closedAt} />
                  </dl>
                  <p className="mt-4 rounded-lg border border-line bg-canvas p-3 text-body text-fg-secondary">
                    {selected.descriptionPreview || 'No public description supplied.'}
                  </p>
                  <LegalCompliancePanel record={selected} onOpenLibrary={() => setLegalLibraryOpen(true)} />
                  <section aria-labelledby="request-documents-title" className="mt-4">
                    <h4 id="request-documents-title" className="text-body font-medium text-fg">Public documents</h4>
                    {documentsStatus === 'pending' && <Skeleton variant="lines" lines={3} label="Loading documents" className="mt-2" />}
                    {documentsStatus === 'failed' && (
                      <InlineError className="mt-2" summary="The document list couldn’t be read." detail={documentError ?? undefined} onRetry={() => setDocumentListRetries((count) => count + 1)} />
                    )}
                    {documentsStatus === 'ready' && documents.length === 0 && (
                      <p className="mt-1 text-caption text-muted">No document is stored for this request.</p>
                    )}
                    {documents.length > 0 && (
                      <ul className="mt-2 max-h-64 space-y-1.5 overflow-y-auto">
                        {documents.map((document) => {
                          const canOpen = document.extractionStatus === 'SEARCHABLE'
                            || document.extractionStatus === 'SALVAGED';
                          const stateLabel = formatExtractionState(document);
                          const opening = openingDocumentId === document.documentId;
                          return (
                            <li key={document.documentId} className="flex items-stretch gap-1">
                              <button
                                type="button"
                                disabled={!canOpen || (openingDocumentId !== null && !opening)}
                                aria-busy={opening || undefined}
                                title={document.extractionDeferredReason ?? stateLabel}
                                onClick={() => void openDocument(document)}
                                className="focus-ring flex min-w-0 flex-1 items-center justify-between gap-3 rounded-lg border border-line bg-canvas px-3 py-2 text-left transition-colors duration-fast hover:border-line-strong hover:bg-raised/60 disabled:cursor-default disabled:opacity-60 disabled:hover:border-line disabled:hover:bg-canvas"
                              >
                                <span className="min-w-0">
                                  <span className="block truncate font-mono text-code text-fg">{document.fileName}</span>
                                  <span className="mt-0.5 block truncate text-caption text-muted">{opening ? 'Opening…' : stateLabel}</span>
                                </span>
                                <span className="shrink-0 text-caption tabular-nums text-muted">{formatFileSize(document.fileSize)}</span>
                              </button>
                              {/* Its own control, never a modifier on the open action, and never a
                                  swipe or a long press. Destroying a piece of evidence is not a
                                  variation on reading it.

                                  Refused while the source is synchronizing. Purging one document
                                  stops nothing: the crawl that produced it is still running, holds
                                  no record that the document was deleted, and would re-download it
                                  within the same run. A delete that silently undoes itself is a
                                  worse claim about evidence than one that declines. */}
                              <Button
                                variant="ghost"
                                iconOnly
                                icon="trash"
                                disabled={syncingPortalKeys.includes(document.portalKey)}
                                title={syncingPortalKeys.includes(document.portalKey)
                                  ? 'Pause this source before deleting a document; a running crawl would download it again'
                                  : `Delete ${document.fileName} and everything derived from it`}
                                aria-label={`Delete document ${document.fileName}`}
                                onClick={() => setPendingDocumentPurge(document)}
                                className="h-auto self-stretch hover:text-danger"
                              />
                            </li>
                          );
                        })}
                      </ul>
                    )}
                    {documentError && documentsStatus !== 'failed' && (
                      <InlineError className="mt-2" summary="The document couldn’t be opened." detail={documentError} />
                    )}
                  </section>
                  {hasReportableFinding(selected) && (
                    <div className="mt-4 border-t border-line pt-4">
                      {isTauri() && selected.integrityFlags.includes('STATUTORY_LATENCY_VIOLATION') ? (
                        <StatutoryDeadlineAppeal portalKey={selected.portalKey} requestId={selected.requestId} />
                      ) : (
                        <>
                          <Button
                            variant="primary"
                            disabled={exportState?.key === selectedKey && exportState.isExporting}
                            onClick={() => void exportBrief(selected)}
                          >
                            {exportState?.key === selectedKey && exportState.isExporting ? 'Drafting…' : 'Draft appeal'}
                          </Button>
                          {exportState?.key === selectedKey && exportState.result && (
                            <div className="mt-2 space-y-0.5 break-all text-caption text-success">
                              <p>Saved report: <span className="font-mono">{exportState.result.outputPath}</span></p>
                              <p>Signed evidence package: <span className="font-mono">{exportState.result.packagePath}</span></p>
                              <p>Signer {exportState.result.signatureAlgorithm}: <span className="font-mono">{exportState.result.signerKeyId}</span></p>
                            </div>
                          )}
                          {exportState?.key === selectedKey && exportState.error && (
                            <InlineError className="mt-2" summary="The appeal draft couldn’t be written." detail={exportState.error} />
                          )}
                        </>
                      )}
                    </div>
                  )}
                </div>
              ) : (
                <p className="mt-3 rounded-lg border border-dashed border-line px-4 py-6 text-center text-body text-muted">Select a request to see its documents, deadlines and findings.</p>
              )}
            </section>

            {syncLogCount > 0 && <div className="min-h-[220px]"><ExtractionTerminal /></div>}
          </div>
        </div>
      </div>
      </>}
      {pendingDocumentPurge && (
        <Dialog size="sm" onClose={() => setPendingDocumentPurge(null)}>
          <DialogHeader title={<span className="break-all">Delete {pendingDocumentPurge.fileName}?</span>} />
          <DialogBody className="space-y-2 text-body text-muted">
            <p className="text-fg-secondary">Destroys the {formatFileSize(pendingDocumentPurge.fileSize)} file on disk, its extracted text, and every search, provenance, and integrity record derived from it.</p>
            <p>The request this document belongs to is kept, and so is every other document under it. This cannot be undone from inside the application.</p>
            {/* No tombstone survives the delete: the crawl frontier is rebuilt per sync run and
                the only record that this document was ever ingested is the row being destroyed.
                Saying so is the difference between a deletion and a deletion the operator thinks
                is permanent. */}
            <p>Synchronizing this source again can download the same file back. Remove the source to stop that.</p>
          </DialogBody>
          <DialogFooter>
            <Button variant="ghost" onClick={() => setPendingDocumentPurge(null)}>Cancel</Button>
            <Button variant="destructive" disabled={isPurgingDocument} onClick={() => void purgeDocument()}>{isPurgingDocument ? 'Deleting…' : 'Delete this document'}</Button>
          </DialogFooter>
        </Dialog>
      )}
      {activeDocument && (
        <Dialog
          size="xl"
          onClose={() => setActiveDocument(null)}
          className={cn('h-[min(760px,88vh)]', isPdfDocument(activeDocument.summary) && 'max-w-[min(1440px,96vw)]')}
        >
          <DialogHeader
            title={<span className="block truncate" title={activeDocument.summary.fileName}>{activeDocument.summary.fileName}</span>}
            description={<span className="font-mono text-code">{activeDocument.summary.portalKey} / document {activeDocument.summary.documentId}</span>}
          />
          <DialogBody className="grid grid-cols-[minmax(0,1fr)_360px] gap-4 overflow-hidden">
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
            <aside className="h-full min-h-0 space-y-4 overflow-y-auto rounded-xl border border-line bg-panel p-3" aria-label="Document evidence details">
              {activeDocument.summary.originalSourceUri && (
                <a className="focus-ring inline-flex items-center gap-1 rounded text-body text-accent underline-offset-2 hover:underline" href={activeDocument.summary.originalSourceUri} target="_blank" rel="noreferrer">
                  View the publisher’s original file
                  <span className="sr-only">(opens in browser)</span>
                </a>
              )}
              {activeDocument.summary.extractionDeferredReason?.startsWith('REFERENCE_ONLY:') && <p className="rounded-lg border border-warning/30 bg-warning/10 p-3 text-body text-warning">{activeDocument.summary.extractionDeferredReason.slice('REFERENCE_ONLY:'.length)}</p>}
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
          </DialogBody>
        </Dialog>
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
    <div className="mx-5 mt-4 space-y-2 lg:mx-6">
      <CaseScopeBar state={caseScope} noun="requests" listOnly />
      {caseScope.status === 'ready' && (
        caseScope.value.scope.requests.length === 0 ? (
          <p className="text-body text-muted">None of this case's sources is a stored document, so no request is linked to it.</p>
        ) : (
          <ul aria-label="Requests in the active case" className="motion-stagger flex flex-wrap gap-2">
            {caseScope.value.scope.requests.map((request) => (
              <li key={`${request.portalKey}:${request.requestId}`} className="rounded-lg border border-line bg-panel px-2.5 py-1 text-body">
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
  const activePortalName = useAppStore((state) => state.portals.find(({ registration }) => registration.portalKey === state.activePortalKey)?.registration.displayName ?? state.activePortalKey);
  const setSearchFilters = useAppStore((state) => state.setSearchFilters);
  const setActivePortal = useAppStore((state) => state.setActivePortal);
  const requestEvidenceRefresh = useAppStore((state) => state.requestEvidenceRefresh);
  const narrowing = [
    searchTerm.trim() && { label: `the text "${searchTerm.trim()}"`, remove: () => setSearchFilters('', agencyId, status, policy) },
    agencyId !== null && { label: 'the department filter', remove: () => setSearchFilters(searchTerm, null, status, policy) },
    status && { label: `status "${status}"`, remove: () => setSearchFilters(searchTerm, agencyId, null, policy) },
    policy && { label: 'the policy-deviation filter', remove: () => setSearchFilters(searchTerm, agencyId, status, null) },
    activePortalKey && { label: `the ${activePortalName} filter`, remove: () => setActivePortal(null) },
  ].filter((item): item is { label: string; remove: () => void } => Boolean(item));
  return (
    <div className="grid h-full min-h-72 place-items-center p-8 text-center">
      <div className="max-w-sm">
        <span className="mx-auto grid h-10 w-10 place-items-center rounded-xl bg-raised text-fg-secondary"><Icon name="search" size="lg" /></span>
        <h3 className="mt-3 text-heading text-fg">{error ? 'Search unavailable' : 'No matching evidence'}</h3>
        {error ? (
          <>
            <p className="mt-1 text-body text-muted">
              The query didn’t complete, so there is no result set to interpret. Try again before drawing a conclusion from these filters.
            </p>
            <Button variant="primary" className="mt-4" onClick={requestEvidenceRefresh}>Try again</Button>
          </>
        ) : (
          <>
            <p className="mt-1 text-body text-muted">
              {sourceCount === 0
                ? 'No sources are added yet, so there is nothing to search.'
                : `Searched ${activePortalKey ? '1 source' : `${sourceCount} source${sourceCount === 1 ? '' : 's'}`}: request titles and descriptions, and the text of documents already ingested. This doesn’t show that no such record exists; it may not be collected or readable yet.`}
            </p>
            {narrowing.length > 0 && (
              <div className="mt-4 flex flex-wrap justify-center gap-2" role="group" aria-label="Narrowing the search">
                {narrowing.map((item) => (
                  <Button key={item.label} size="sm" onClick={item.remove}>Remove {item.label}</Button>
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
  // Portal status text is free-form; only past-due wordings get a tone of their own.
  const normalized = status.toLowerCase();
  const overdue = ['overdue', 'past due', 'late'].some((wording) => normalized.includes(wording));
  const label = status === status.toUpperCase() ? status.charAt(0) + status.slice(1).toLowerCase() : status;
  return <Badge tone={overdue ? 'warning' : 'neutral'} className="shrink-0">{label}</Badge>;
}

/** Counted statutory flags say so; a heuristic flag is worded as a pattern and kept neutral. */
const POLICY_LABELS: Record<string, { label: string; tone: 'danger' | 'neutral' }> = {
  STATUTORY_LATENCY_VIOLATION: { label: 'Statutory deadline exceeded', tone: 'danger' },
  STATUTORY_EXTENSION_DEFECTIVE: { label: 'Defective extension notice', tone: 'danger' },
  SUSPICIOUSLY_FAST_CLOSURE: { label: 'Unusually fast closure (pattern)', tone: 'neutral' },
};

function PolicyBadge({ flag }: { flag: string }) {
  const known = POLICY_LABELS[flag];
  const label = known?.label ?? flag.charAt(0) + flag.slice(1).toLowerCase().split('_').join(' ');
  return <Badge tone={known?.tone ?? 'neutral'} dashed={known?.tone !== 'danger'}>{label}</Badge>;
}

function CaseField({ label, value, code = false, absent = false }: { label: string; value: string; code?: boolean; absent?: boolean }) {
  return (
    <div className="min-w-0">
      <dt className="text-caption text-muted">{label}</dt>
      <dd className={cn('mt-0.5 truncate', code ? 'font-mono text-code text-fg' : 'text-body tabular-nums text-fg', absent && 'text-muted')}>{value}</dd>
    </div>
  );
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

// Only a counted statutory flag grounds an appeal. SUSPICIOUSLY_FAST_CLOSURE is a duration
// heuristic; offering an appeal on it would present a statistical flag as a legal ground.
function hasReportableFinding(record: RequestSearchResult) {
  return record.integrityFlags.some((flag) => [
    'STATUTORY_LATENCY_VIOLATION',
    'STATUTORY_EXTENSION_DEFECTIVE',
  ].includes(flag));
}
