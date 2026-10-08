import { lazy, Suspense, useEffect, useMemo, useRef, useState } from 'react';
import { useC2PA } from '../../hooks/useC2PA';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import { invokeCommand } from '../../ipc';
import type { DocumentClaimChecks, DocumentInternalMetadata, DocumentOriginReport, EvidenceBundle, ForensicFinding, ProvenanceConfidence, ProvenanceVerificationRecord, RecoveredVersionProfile } from '../../types';
import DocumentOriginPanel from './DocumentOriginPanel';
import DocumentAnnotations, { type AnnotationAnchor, type PassageTarget } from './DocumentAnnotations';
import { findPassageInText } from './annotationLogic';
import { ANNOTATION_LIMITS } from '../../types';
import UnredactorPanel from './UnredactorPanel';
import ReconstructionPanel from './ReconstructionPanel';
import { formatInstant } from '../objects/labels';
import { buttonClass } from '../ui/Button';
import { cn } from '../../utils/cn';
import { SearchField } from '../ui/Field';
import RedactionInferencePanel from './RedactionInferencePanel';
import { displayError } from '../../utils/errorMessage';
import { MinimapGutter, MinimapKey, minimapHeadline, type MinimapMark } from '../charts/figures/DocumentMinimap';

// pdf.js loads only when a reader opens the page view.
const PdfPageView = lazy(() => import('./PdfPageView'));

interface DocumentViewerProps {
  documentId: number;
  requestId?: number;
  portalKey: string;
  fileName: string;
  originalAvailable?: boolean;
  textContent: string;
  fileSizeFormatted: string;
  textPositionLabel?: string;
  characterOffset?: number;
  knownMaskedFindingCount?: number;
  /** Characters in the whole stored text, for the minimap; omitted when the caller has no chunk. */
  totalCharacterCount?: number;
  onNextChunk?: () => void;
  onPreviousChunk?: () => void;
  /** Loads the chunk starting at this source character offset; lets the minimap open a mark outside the loaded text. */
  onOpenAtOffset?: (characterOffset: number) => void;
  onSaveEvidence?: (selection: { excerpt: string; characterOffset: number }) => void;
  /** False when a surrounding dialog header already names the file. */
  titled?: boolean;
}

const TEXT_PAGE_CHARACTERS = 12_000;
/** Characters of context loaded before an off-chunk mark, so it does not sit at the very top. */
const OFF_CHUNK_LEAD = 2_000;
const TOOL_CLASS = buttonClass({ size: 'sm', className: 'focus-ring' });
const GHOST_CLASS = buttonClass({ variant: 'ghost', size: 'sm', className: 'focus-ring' });

export default function DocumentViewer({ documentId, requestId, portalKey, fileName, originalAvailable = true, textContent, fileSizeFormatted, textPositionLabel, characterOffset = 0, knownMaskedFindingCount = 0, totalCharacterCount, onNextChunk, onPreviousChunk, onOpenAtOffset, onSaveEvidence, titled = true }: DocumentViewerProps) {
  const { verifyAssetProvenance, activeManifest, isValidating, error: provenanceError } = useC2PA();
  const [recoveryProfile, setRecoveryProfile] = useState<RecoveredVersionProfile | null>(null);
  const [isRecovering, setIsRecovering] = useState(false);
  const [recoveryError, setRecoveryError] = useState<string | null>(null);
  const [showRedactionAudit, setShowRedactionAudit] = useState(false);
  const [showVerificationHistory, setShowVerificationHistory] = useState(false);
  const [verificationHistory, setVerificationHistory] = useState<ProvenanceVerificationRecord[] | null>(null);
  const [isLoadingHistory, setIsLoadingHistory] = useState(false);
  const [historyError, setHistoryError] = useState<string | null>(null);
  const [residualAudit, setResidualAudit] = useState<EvidenceBundle | null>(null);
  const [residualAuditError, setResidualAuditError] = useState<string | null>(null);
  const [isAuditingResiduals, setIsAuditingResiduals] = useState(false);
  const [documentSearch, setDocumentSearch] = useState('');
  const [activeMatch, setActiveMatch] = useState(0);
  const [textPage, setTextPage] = useState(0);
  const [shareStatus, setShareStatus] = useState<string | null>(null);
  const [briefingStatus, setBriefingStatus] = useState<string | null>(null);
  const [inferenceSpan, setInferenceSpan] = useState<{ start: number; end: number } | null>(null);
  const [claimChecks, setClaimChecks] = useState<DocumentClaimChecks | null>(null);
  const [claimChecksState, setClaimChecksState] = useState<'idle' | 'pending' | 'failed'>('idle');
  const [claimChecksError, setClaimChecksError] = useState<string | null>(null);
  const [showAnnotations, setShowAnnotations] = useState(false);
  const [showPageView, setShowPageView] = useState(false);
  const canViewPages = originalAvailable && fileName.toLocaleLowerCase().endsWith('.pdf');
  const textPaneRef = useRef<HTMLDivElement>(null);
  const historyGate = useLatestRequestGate();
  const recoveryGate = useLatestRequestGate();
  const residualAuditGate = useLatestRequestGate();
  const truncationGate = useLatestRequestGate();
  const shareGate = useLatestRequestGate();
  const claimGate = useLatestRequestGate();
  const packGate = useLatestRequestGate();
  const originGate = useLatestRequestGate();
  const minimapGate = useLatestRequestGate();
  // The minimap's stored marks for this document: finding citations and annotations. Cleared on
  // every document switch and written only by the latest read (minimapGate).
  const [storedMarks, setStoredMarks] = useState<MinimapMark[] | null>(null);
  const [activeMark, setActiveMark] = useState<string | null>(null);
  // A mark waiting on its chunk, tied to the document it was chosen in so a switch never applies it elsewhere.
  const pendingMarkRef = useRef<{ documentId: number; portalKey: string; mark: MinimapMark } | null>(null);
  const [originReport, setOriginReport] = useState<DocumentOriginReport | null>(null);
  const [originFailed, setOriginFailed] = useState(false);
  const [internalMetadata, setInternalMetadata] = useState<DocumentInternalMetadata | null>(null);
  const [internalMetadataFailed, setInternalMetadataFailed] = useState(false);
  const [packState, setPackState] = useState<'idle' | 'pending' | 'failed'>('idle');
  const [packError, setPackError] = useState<string | null>(null);
  const [truncatedEntityKinds, setTruncatedEntityKinds] = useState<string[]>([]);

  useEffect(() => {
    if (originalAvailable) void verifyAssetProvenance(documentId, portalKey);
  }, [documentId, portalKey, originalAvailable, verifyAssetProvenance]);

  // useC2PA's own state is not remounted or reset when documentId changes -- this viewer is
  // reused across documents (see MainLayout, which renders it without a `key`) -- so
  // activeManifest can still hold the previous document's result: while the new document's own
  // check is in flight, or indefinitely once originalAvailable is false and no new check is ever
  // issued. The backend echoes documentId/portalKey back on every result, so that is checked here
  // rather than trusting that a result in hand belongs to what is currently on screen.
  const manifestForActiveDocument = useMemo(
    () => (activeManifest && activeManifest.documentId === documentId && activeManifest.portalKey === portalKey
      ? activeManifest
      : null),
    [activeManifest, documentId, portalKey],
  );

  useEffect(() => {
    historyGate.invalidate();
    recoveryGate.invalidate();
    residualAuditGate.invalidate();
    truncationGate.invalidate();
    shareGate.invalidate();
    setTruncatedEntityKinds([]);
    setRecoveryProfile(null);
    setRecoveryError(null);
    setIsRecovering(false);
    setShowRedactionAudit(false);
    // The log is per-document. Without this reset document A's history stays mounted under B,
    // which is a false attribution of evidence, not a stale render.
    setShowVerificationHistory(false);
    setVerificationHistory(null);
    setHistoryError(null);
    setIsLoadingHistory(false);
    setResidualAudit(null);
    setResidualAuditError(null);
    setIsAuditingResiduals(false);
    setDocumentSearch('');
    setActiveMatch(0);
    setTextPage(0);
    setShareStatus(null);
    setBriefingStatus(null);
    claimGate.invalidate();
    packGate.invalidate();
    setPackState('idle');
    setPackError(null);
    setClaimChecks(null);
    setClaimChecksState('idle');
    setClaimChecksError(null);
  }, [claimGate, documentId, historyGate, packGate, portalKey, recoveryGate, residualAuditGate, shareGate, truncationGate]);

  const downloadQueryPack = async () => {
    const request = packGate.begin();
    setPackState('pending');
    setPackError(null);
    try {
      const pack = await invokeCommand('export_claim_query_pack', { documentId, portalKey });
      // A pack built for one document must not be saved under the name of the document the
      // viewer moved to while it was built.
      if (!packGate.isCurrent(request)) return;
      const url = URL.createObjectURL(new Blob([JSON.stringify(pack, null, 2)], { type: 'application/json' }));
      const anchor = document.createElement('a');
      anchor.href = url;
      anchor.download = `claim-query-pack-${portalKey}-${documentId}.json`;
      anchor.click();
      URL.revokeObjectURL(url);
      setPackState('idle');
    } catch (reason) {
      if (!packGate.isCurrent(request)) return;
      setPackError(displayError(reason).message);
      setPackState('failed');
    }
  };

  const checkFigures = async () => {
    const request = claimGate.begin();
    setClaimChecksState('pending');
    setClaimChecksError(null);
    try {
      const result = await invokeCommand('list_document_claim_checks', { documentId, portalKey });
      // A check requested for one document must not be shown under the document the viewer
      // moved to while it ran.
      if (!claimGate.isCurrent(request)) return;
      setClaimChecks(result);
      setClaimChecksState('idle');
    } catch (reason) {
      if (!claimGate.isCurrent(request)) return;
      setClaimChecksError(displayError(reason).message);
      setClaimChecksState('failed');
    }
  };

  useEffect(() => {
    const token = truncationGate.begin();
    invokeCommand('get_document_entity_truncations', { documentId, portalKey })
      .then((kinds) => {
        if (truncationGate.isCurrent(token)) setTruncatedEntityKinds(kinds);
      })
      .catch(() => {
        // No answer leaves the notice hidden; only a recorded truncation is worth stating.
      });
  }, [documentId, portalKey, truncationGate]);

  // Whether a file was added by hand is a statement about that file. A report for document A
  // arriving under document B would label B with A's origin and A's matches.
  useEffect(() => {
    const request = originGate.begin();
    setOriginReport(null);
    setOriginFailed(false);
    setInternalMetadata(null);
    setInternalMetadataFailed(false);
    void (async () => {
      let report: DocumentOriginReport;
      try {
        report = await invokeCommand('describe_document_origin', { documentId, portalKey });
      } catch {
        if (originGate.isCurrent(request)) setOriginFailed(true);
        return;
      }
      if (!originGate.isCurrent(request)) return;
      setOriginReport(report);
      if (report.origin !== 'ADDED_BY_USER') return;
      try {
        const metadata = await invokeCommand('read_document_internal_metadata', { documentId, portalKey });
        if (originGate.isCurrent(request)) setInternalMetadata(metadata);
      } catch {
        if (originGate.isCurrent(request)) setInternalMetadataFailed(true);
      }
    })();
  }, [documentId, originGate, portalKey]);

  useEffect(() => { setTextPage(0); setActiveMatch(0); }, [characterOffset, textContent]);

  useEffect(() => {
    const request = minimapGate.begin();
    setStoredMarks(null);
    void Promise.allSettled([
      invokeCommand('list_findings', { filter: { portalKey, documentId }, limit: MINIMAP_FINDING_LIMIT }),
      invokeCommand('list_document_annotations', { portalKey, documentId }),
      invokeCommand('get_casework_state', {}),
    ]).then(([findings, annotations, casework]) => {
      if (!minimapGate.isCurrent(request)) return;
      const marks: MinimapMark[] = [];
      if (findings.status === 'fulfilled' && Array.isArray(findings.value)) {
        for (const finding of findings.value) {
          finding.citations.forEach((citation, index) => {
            if (citation.documentId !== documentId || citation.portalKey !== portalKey) return;
            marks.push({ key: `finding:${finding.id}:${index}`, kind: 'finding', start: citation.characterStart, end: citation.characterEnd, label: `Finding: ${finding.headline}`, quote: citation.quote });
          });
        }
      }
      if (annotations.status === 'fulfilled' && Array.isArray(annotations.value?.annotations)) {
        for (const annotation of annotations.value.annotations) {
          marks.push({ key: `annotation:${annotation.id}`, kind: 'annotation', start: annotation.characterOffset, end: annotation.characterOffset === null ? null : annotation.characterOffset + Array.from(annotation.quote).length, label: `Annotation: ${annotation.note || annotation.quote.slice(0, 60)}`, quote: annotation.quote });
        }
      }
      if (casework.status === 'fulfilled' && Array.isArray(casework.value?.cases)) {
        for (const found of casework.value.cases) {
          for (const citation of found.evidence) {
            if (citation.documentId !== documentId || citation.portalKey !== portalKey) continue;
            marks.push({ key: `case:${found.id}:${citation.id}`, kind: 'annotation', start: citation.characterOffset, end: citation.characterOffset === null ? null : citation.characterOffset + Array.from(citation.quote).length, label: `Case citation (${found.title || 'untitled case'}): ${citation.label}`, quote: citation.quote });
          }
        }
      }
      setStoredMarks(marks);
    });
    return minimapGate.invalidate;
  }, [documentId, minimapGate, portalKey]);

  const searchMatches = useMemo(() => findMatches(textContent, documentSearch), [documentSearch, textContent]);
  const pageCount = Math.max(1, Math.ceil(textContent.length / TEXT_PAGE_CHARACTERS));
  const selectedMatch = searchMatches[activeMatch] ?? null;

  useEffect(() => {
    setActiveMatch(0);
    if (searchMatches[0] !== undefined) {
      setTextPage(Math.floor(searchMatches[0] / TEXT_PAGE_CHARACTERS));
    }
  }, [documentSearch, searchMatches]);

  const chunkMarks = useMemo<MinimapMark[]>(() => {
    const matchOffsets = sourceOffsets(textContent, searchMatches, characterOffset);
    const needle = Array.from(documentSearch.trim()).length;
    const redactions = scanRedactionMarkers(textContent);
    const redactionStarts = sourceOffsets(textContent, redactions.map((span) => span.start), characterOffset);
    const redactionEnds = sourceOffsets(textContent, redactions.map((span) => span.end), characterOffset);
    return [
      ...matchOffsets.map((start, index) => ({ key: `match:${index}`, kind: 'match' as const, start, end: start + needle, label: `Match ${index + 1} of ${searchMatches.length}` })),
      ...redactionStarts.map((start, index) => ({ key: `redaction:${index}`, kind: 'redaction' as const, start, end: redactionEnds[index], label: 'Redaction marker' })),
    ];
  }, [characterOffset, documentSearch, searchMatches, textContent]);
  const minimapMarks = useMemo(
    () => [...(storedMarks ?? []), ...chunkMarks],
    [chunkMarks, storedMarks],
  );
  const loadedEnd = characterOffset + Array.from(textContent).length;
  // The stored text's own count; the masked chunk can differ in length from what it masks.
  const minimapTotal = totalCharacterCount ?? loadedEnd;

  const activateMark = (mark: MinimapMark) => {
    setActiveMark(mark.key);
    if (mark.kind === 'match') {
      const index = Number(mark.key.slice('match:'.length));
      setActiveMatch(index);
      setTextPage(Math.floor(searchMatches[index] / TEXT_PAGE_CHARACTERS));
      return;
    }
    if (mark.start === null) return;
    if (mark.start < characterOffset || mark.start > loadedEnd) {
      // Outside the loaded text: load the chunk around it, then select it once that chunk is shown.
      if (!onOpenAtOffset) return;
      pendingMarkRef.current = { documentId, portalKey, mark };
      onOpenAtOffset(Math.max(0, mark.start - OFF_CHUNK_LEAD));
      return;
    }
    const local = utf16Index(textContent, mark.start - characterOffset);
    setTextPage(Math.floor(local / TEXT_PAGE_CHARACTERS));
    const pane = textPaneRef.current;
    if (pane) {
      const within = (local % TEXT_PAGE_CHARACTERS) / Math.max(1, Math.min(TEXT_PAGE_CHARACTERS, textContent.length - Math.floor(local / TEXT_PAGE_CHARACTERS) * TEXT_PAGE_CHARACTERS));
      requestAnimationFrame(() => { pane.scrollTop = Math.max(0, within * pane.scrollHeight - pane.clientHeight / 3); });
    }
  };

  // Selects a mark chosen outside the loaded text once its chunk arrives, if the document is still the same one.
  useEffect(() => {
    const pending = pendingMarkRef.current;
    if (!pending) return;
    if (pending.documentId !== documentId || pending.portalKey !== portalKey) {
      pendingMarkRef.current = null;
      return;
    }
    const start = pending.mark.start;
    if (start === null || start < characterOffset || start > loadedEnd) return;
    pendingMarkRef.current = null;
    activateMark(pending.mark);
  });

  const moveToMatch = (direction: -1 | 1) => {
    if (searchMatches.length === 0) return;
    const next = (activeMatch + direction + searchMatches.length) % searchMatches.length;
    setActiveMatch(next);
    setActiveMark(`match:${next}`);
    setTextPage(Math.floor(searchMatches[next] / TEXT_PAGE_CHARACTERS));
  };

  const shareEvidenceLocation = async () => {
    const request = shareGate.begin();
    const localIndex = selectedMatch ?? textPage * TEXT_PAGE_CHARACTERS;
    const sourceCharacterOffset = characterOffset + Array.from(textContent.slice(0, localIndex)).length;
    const fragment = new URLSearchParams({
      portal: portalKey,
      ...(requestId === undefined ? {} : { request: String(requestId) }),
      document: String(documentId),
      offset: String(sourceCharacterOffset),
    }).toString();
    const url = `${window.location.href.split('#')[0]}#${fragment}`;
    window.history.replaceState(null, '', `#${fragment}`);
    try {
      await navigator.clipboard.writeText(url);
      // The clipboard write is async and cannot be cancelled. Without this check, a write started
      // for this document can resolve after the viewer has moved to another one and confirm an
      // offset that is no longer what is on screen.
      if (!shareGate.isCurrent(request)) return;
      setShareStatus(`Copied offset ${sourceCharacterOffset.toLocaleString()}`);
    } catch {
      if (!shareGate.isCurrent(request)) return;
      setShareStatus(`Link set at offset ${sourceCharacterOffset.toLocaleString()}`);
    }
  };

  const readSelectionAnchor = (): AnnotationAnchor | null => {
    const pane = textPaneRef.current;
    const selection = window.getSelection();
    if (!pane || !selection || selection.rangeCount === 0 || selection.isCollapsed) return null;
    const range = selection.getRangeAt(0);
    if (!pane.contains(range.commonAncestorContainer)) return null;
    const before = document.createRange();
    before.selectNodeContents(pane);
    before.setEnd(range.startContainer, range.startOffset);
    const selected = range.toString();
    const quote = selected.trim();
    if (quote === '') return null;
    const start = textPage * TEXT_PAGE_CHARACTERS + before.toString().length + (selected.length - selected.trimStart().length);
    return {
      quote,
      prefix: textContent.slice(Math.max(0, start - ANNOTATION_LIMITS.contextCharacters), start),
      suffix: textContent.slice(start + quote.length, start + quote.length + ANNOTATION_LIMITS.contextCharacters),
    };
  };

  // The text on screen is one masked chunk, so a stored position in the stored text is only a hint
  // for which of several equal passages in the chunk is meant.
  const jumpToPassage = (passage: PassageTarget): boolean => {
    const expected = passage.sourceOffset === null ? 0 : passage.sourceOffset - characterOffset;
    const index = findPassageInText(textContent, passage, expected);
    if (index === null) return false;
    setTextPage(Math.floor(index / TEXT_PAGE_CHARACTERS));
    return true;
  };

  const saveEvidenceToBriefing = () => {
    if (!onSaveEvidence) return;
    const anchor = selectedMatch ?? textPage * TEXT_PAGE_CHARACTERS;
    const excerptStart = Math.max(0, anchor - (selectedMatch === null ? 0 : 180));
    const excerptEnd = Math.min(
      textContent.length,
      anchor + (selectedMatch === null ? 520 : Math.max(documentSearch.trim().length, 1) + 340),
    );
    const sourceCharacterOffset = characterOffset
      + Array.from(textContent.slice(0, excerptStart)).length;
    onSaveEvidence({
      excerpt: textContent.slice(excerptStart, excerptEnd),
      characterOffset: sourceCharacterOffset,
    });
    setBriefingStatus(`Saved excerpt at character ${sourceCharacterOffset.toLocaleString()}`);
  };

  const toggleVerificationHistory = async () => {
    if (showVerificationHistory) {
      setShowVerificationHistory(false);
      return;
    }
    setShowVerificationHistory(true);
    if (verificationHistory || isLoadingHistory) return;
    const request = historyGate.begin();
    setIsLoadingHistory(true);
    setHistoryError(null);
    try {
      // No limit is passed: the backend rejects out-of-range values and its default is the
      // contract this view wants.
      const records = await invokeCommand('list_document_provenance_verifications', {
        documentId,
        portalKey,
      });
      if (!historyGate.isCurrent(request)) return;
      setVerificationHistory(records);
    } catch (reason) {
      if (!historyGate.isCurrent(request)) return;
      setVerificationHistory(null);
      setHistoryError(displayError(reason).message);
    } finally {
      if (historyGate.isCurrent(request)) setIsLoadingHistory(false);
    }
  };

  const inspectRevisionHistory = async () => {
    // PDF revision carving returns a typed recovery profile; the adjacent residual-evidence audit
    // uses the shared finding contract with source locators and confidence labels for every
    // supported container.
    const request = recoveryGate.begin();
    setIsRecovering(true);
    setRecoveryError(null);
    // The scan is bounded but not instant, and it cannot be cancelled. Sequencing is what keeps
    // one document's recovered revisions from being rendered as another document's.
    try {
      const profile = await invokeCommand('carve_document_version_history', {
        documentId,
        portalKey,
      });
      if (!recoveryGate.isCurrent(request)) return;
      setRecoveryProfile(profile);
    } catch (reason) {
      if (!recoveryGate.isCurrent(request)) return;
      setRecoveryProfile(null);
      setRecoveryError(displayError(reason).message);
    } finally {
      if (recoveryGate.isCurrent(request)) setIsRecovering(false);
    }
  };

  const inspectResidualEvidence = async () => {
    const request = residualAuditGate.begin();
    setIsAuditingResiduals(true);
    setResidualAuditError(null);
    try {
      const bundle = await invokeCommand('audit_document_residual_evidence', {
        documentId,
        portalKey,
        includeHypotheses: false,
      });
      if (residualAuditGate.isCurrent(request)) setResidualAudit(bundle);
    } catch (reason) {
      if (!residualAuditGate.isCurrent(request)) return;
      setResidualAudit(null);
      setResidualAuditError(displayError(reason).message);
    } finally {
      if (residualAuditGate.isCurrent(request)) setIsAuditingResiduals(false);
    }
  };

  return (
    <section className="flex h-full min-h-0 flex-1 flex-col overflow-hidden rounded-xl border border-line bg-canvas">
      <div className={cn('flex justify-between gap-4 border-b border-line px-4 py-3', titled ? 'items-start' : 'items-center')}>
        <div className="min-w-0">
          {titled && <h2 className="truncate text-heading text-fg" title={fileName}>{fileName}</h2>}
          <div className={cn('flex flex-wrap items-center gap-x-1.5 text-caption text-muted', titled && 'mt-1')}>
            <span className="font-mono text-code">{portalKey} / {documentId}</span>
            <span aria-hidden="true">·</span>
            <span className="whitespace-nowrap tabular-nums">{fileSizeFormatted}</span>
          </div>
        </div>
        <div className="shrink-0">
          {originalAvailable ? <ProvenanceShield isValidating={isValidating} result={manifestForActiveDocument} error={provenanceError} /> : <span className="text-caption text-warning">Original referenced · file verification unavailable</span>}
        </div>
      </div>
      <div role="toolbar" aria-label="Inspect this document" className="flex flex-wrap items-center gap-1.5 border-b border-line px-4 py-2">
        <span className="mr-1 text-overline uppercase text-muted">Inspect</span>
          <button type="button" onClick={() => void inspectResidualEvidence()} disabled={!originalAvailable || isAuditingResiduals} className={TOOL_CLASS}>
            {isAuditingResiduals ? 'Auditing residuals…' : residualAudit ? 'Reaudit residuals' : 'Audit residual evidence'}
          </button>
          <button type="button" onClick={() => setShowRedactionAudit((visible) => !visible)} aria-expanded={showRedactionAudit} className={TOOL_CLASS}>
            {showRedactionAudit ? 'Hide redaction audit' : 'Audit redactions'}
          </button>
          {fileName.toLocaleLowerCase().endsWith('.pdf') && (
            <button type="button" onClick={() => void inspectRevisionHistory()} disabled={!originalAvailable || isRecovering} className={TOOL_CLASS}>
              {isRecovering ? 'Inspecting revisions…' : recoveryProfile ? 'Reinspect revisions' : 'Inspect revision history'}
            </button>
          )}
          <button type="button" onClick={() => void checkFigures()} disabled={claimChecksState === 'pending'} className={TOOL_CLASS}>
            {claimChecksState === 'pending' ? 'Checking figures…' : claimChecks ? 'Recheck figures' : 'Check figures'}
          </button>
          {claimChecks && claimChecks.checks.some((check) => check.result.outcome !== 'unreconciled') && (
            <button type="button" onClick={() => void downloadQueryPack()} disabled={packState === 'pending'} className={TOOL_CLASS}>
              {packState === 'pending' ? 'Building query pack…' : 'Download query pack'}
            </button>
          )}
          {packError && <span role="alert" className="text-caption text-danger">Query pack failed: {packError}</span>}
          <button type="button" onClick={() => void toggleVerificationHistory()} aria-expanded={showVerificationHistory} className={TOOL_CLASS}>
            {showVerificationHistory ? 'Hide verification log' : 'Verification log'}
          </button>
      </div>
      <div className="flex items-center justify-between border-b border-line px-4 py-2 text-caption text-muted">
        <span>Local extracted-text view</span>
        <div className="flex items-center gap-3">
          {textPositionLabel && <span>{textPositionLabel}</span>}
          <span>{knownMaskedFindingCount > 0 ? `${knownMaskedFindingCount} sensitive value${knownMaskedFindingCount === 1 ? '' : 's'} masked` : 'No structured PII detected'}</span>
          {onPreviousChunk && <button type="button" onClick={onPreviousChunk} className={GHOST_CLASS}>Previous chunk</button>}
          {onNextChunk && <button type="button" onClick={onNextChunk} className={GHOST_CLASS}>Next chunk</button>}
        </div>
      </div>
      <div className="flex flex-wrap items-center gap-2 border-b border-line px-4 py-2 text-caption">
        <SearchField
          size="sm"
          aria-label="Search within this extracted-text chunk"
          value={documentSearch}
          onChange={(event) => setDocumentSearch(event.target.value)}
          placeholder="Find in extracted text"
          wrapperClassName="min-w-[12rem] flex-1"
          trailing={documentSearch.trim() ? <span className="whitespace-nowrap text-caption tabular-nums text-muted">{searchMatches.length === 0 ? 'No matches' : `${activeMatch + 1} of ${searchMatches.length}`}</span> : null}
        />
        {documentSearch.trim() && (
          <>
            <button type="button" onClick={() => moveToMatch(-1)} disabled={searchMatches.length === 0} className={GHOST_CLASS}>Previous</button>
            <button type="button" onClick={() => moveToMatch(1)} disabled={searchMatches.length === 0} className={GHOST_CLASS}>Next</button>
          </>
        )}
        <button type="button" onClick={() => setShowAnnotations((visible) => !visible)} aria-expanded={showAnnotations} className={TOOL_CLASS}>{showAnnotations ? 'Hide annotations' : 'Annotate'}</button>
        {canViewPages && <button type="button" onClick={() => { setShowPageView((visible) => !visible); setShowAnnotations(true); }} aria-pressed={showPageView} className={TOOL_CLASS}>Page view</button>}
        <button type="button" onClick={() => void shareEvidenceLocation()} className={TOOL_CLASS}>Copy evidence link</button>
        {onSaveEvidence && <button type="button" onClick={saveEvidenceToBriefing} className={buttonClass({ variant: 'primary', size: 'sm', className: 'focus-ring' })}>Save excerpt to briefing</button>}
        {shareStatus && <span role="status" className="text-success">{shareStatus}</span>}
        {briefingStatus && <span role="status" className="text-success">{briefingStatus}</span>}
      </div>
      {originFailed && (
        <p role="alert" className="border-b border-line px-4 py-2 text-caption text-warning">
          Could not determine whether this file was collected from a source or added by hand.
        </p>
      )}
      {originReport?.origin === 'ADDED_BY_USER' && (
        <DocumentOriginPanel report={originReport} metadata={internalMetadata} metadataFailed={internalMetadataFailed} />
      )}
      {manifestForActiveDocument && manifestForActiveDocument.claimGraph.claims.length > 0 && (
        <ProvenanceGraphPanel graph={manifestForActiveDocument.claimGraph} />
      )}
      {showVerificationHistory && <VerificationHistoryPanel records={verificationHistory} isLoading={isLoadingHistory} error={historyError} />}
      {(recoveryProfile || recoveryError) && <RevisionRecoveryPanel profile={recoveryProfile} error={recoveryError} />}
      {truncatedEntityKinds.length > 0 && (
        <p role="status" className="text-label leading-4 text-warning">
          Entity extraction stopped early for {truncatedEntityKinds.join(', ').toLowerCase().replace(/_/g, ' ')}. Entities listed for this document are incomplete.
        </p>
      )}
      {(claimChecksState !== 'idle' || claimChecks) && <ClaimChecksPanel checks={claimChecks} state={claimChecksState} error={claimChecksError} />}
      {(residualAudit || residualAuditError) && <ResidualEvidencePanel bundle={residualAudit} error={residualAuditError} />}
      {showRedactionAudit && (
        <div className="max-h-[70vh] space-y-3 overflow-y-auto border-b border-line p-3">
          <ReconstructionPanel documentId={documentId} portalKey={portalKey} />
          {originalAvailable && fileName.toLocaleLowerCase().endsWith('.pdf') && (
            <UnredactorPanel documentId={documentId} portalKey={portalKey} />
          )}
          <InferenceLauncher
            textContent={textContent}
            activeSpan={inferenceSpan}
            onSelectSpan={setInferenceSpan}
          />
          {inferenceSpan && (
            <RedactionInferencePanel
              documentId={documentId}
              portalKey={portalKey}
              spanStart={characterOffset + Array.from(textContent.slice(0, inferenceSpan.start)).length}
              spanEnd={characterOffset + Array.from(textContent.slice(0, inferenceSpan.end)).length}
              onClose={() => setInferenceSpan(null)}
            />
          )}
        </div>
      )}
      <div className={showPageView && canViewPages ? 'flex min-h-0 flex-1' : 'contents'}>
      {showAnnotations && (
        <DocumentAnnotations
          key={`${portalKey}:${documentId}`}
          portalKey={portalKey}
          documentId={documentId}
          fileName={fileName}
          readSelection={readSelectionAnchor}
          onJump={jumpToPassage}
        />
      )}
      {showPageView && canViewPages && (
        <Suspense fallback={<p role="status" className="p-3 text-label text-muted">Loading the page viewer…</p>}>
          <PdfPageView key={`${portalKey}:${documentId}`} portalKey={portalKey} documentId={documentId} fileName={fileName} />
        </Suspense>
      )}
      </div>
      <div className="border-b border-line px-4 py-1.5">
        <p className="text-caption text-muted" data-testid="minimap-headline">{minimapHeadline(minimapMarks, minimapTotal)}</p>
        <MinimapKey marks={minimapMarks} />
      </div>
      <div hidden={showPageView && canViewPages} className="flex min-h-0 flex-1">
        <div ref={textPaneRef} className="min-h-0 flex-1 overflow-y-auto p-5 font-mono text-label leading-relaxed text-fg-secondary selection:bg-highlight/40">
          <HighlightedTextPage text={textContent} query={documentSearch} page={textPage} activeMatchIndex={selectedMatch} />
        </div>
        <MinimapGutter marks={minimapMarks} total={minimapTotal} loaded={{ start: characterOffset, end: Math.min(loadedEnd, minimapTotal) }} activeKey={activeMark} onActivate={activateMark} />
      </div>
      {pageCount > 1 && <div className="flex items-center justify-center gap-3 border-t border-line px-4 py-2 text-caption text-muted"><button type="button" disabled={textPage === 0} onClick={() => setTextPage((page) => Math.max(0, page - 1))} className="rounded border border-control px-2 py-1 disabled:opacity-40">Previous text page</button><span className="font-mono">{textPage + 1}/{pageCount}</span><button type="button" disabled={textPage + 1 >= pageCount} onClick={() => setTextPage((page) => Math.min(pageCount - 1, page + 1))} className="rounded border border-control px-2 py-1 disabled:opacity-40">Next text page</button></div>}
    </section>
  );
}

/**
 * Small launcher that lists the redaction markers present in the extracted text and lets the
 * user pick one to run inference against. It uses a light scan (`[REDACTED]`, `[WITHHELD]`,
 * `(b)(6)`, block/underscore runs) that matches what the Rust engine's `scan_redaction_spans`
 * looks for, so what's clickable here maps to what the backend can reason about.
 */
function InferenceLauncher({
  textContent,
  activeSpan,
  onSelectSpan,
}: {
  textContent: string;
  activeSpan: { start: number; end: number } | null;
  onSelectSpan: (span: { start: number; end: number } | null) => void;
}) {
  const spans = useMemo(() => scanRedactionMarkers(textContent).slice(0, 20), [textContent]);
  if (spans.length === 0) {
    return (
      <p className="text-label text-muted">
        No `[REDACTED]`, exemption tag, or block-run markers found in the extracted text of this document.
      </p>
    );
  }
  return (
    <div className="rounded border border-line bg-surface/40 p-2">
      <div className="text-caption font-semibold uppercase tracking-wide text-muted">
        Infer identity for a redaction
      </div>
      <ul className="mt-1 flex flex-wrap gap-1">
        {spans.map((span) => {
          const isActive = activeSpan?.start === span.start && activeSpan.end === span.end;
          const label = textContent.slice(span.start, span.end).trim().slice(0, 24);
          return (
            <li key={`${span.start}-${span.end}`}>
              <button
                type="button"
                onClick={() => onSelectSpan(isActive ? null : span)}
                className={`rounded border px-2 py-1 text-label ${
                  isActive
                    ? 'border-highlight bg-highlight/20 text-highlight'
                    : 'border-control text-fg-secondary'
                }`}
                aria-pressed={isActive}
              >
                {label || 'redaction'}
                <span className="ml-1 text-muted">·{span.start}</span>
              </button>
            </li>
          );
        })}
      </ul>
    </div>
  );
}

function scanRedactionMarkers(text: string): { start: number; end: number }[] {
  const spans: { start: number; end: number }[] = [];
  const patterns = [/\[REDACTED\]/gi, /\[WITHHELD\]/gi, /\[EXEMPT\]/gi, /\(b\)\(\d\)(\([A-Za-z]\))?/g, /█{3,}|▓{3,}|■{3,}/g, /_{5,}/g, /X{5,}/g];
  for (const pattern of patterns) {
    let match: RegExpExecArray | null;
    while ((match = pattern.exec(text)) !== null) {
      spans.push({ start: match.index, end: match.index + match[0].length });
    }
  }
  spans.sort((a, b) => a.start - b.start);
  const deduped: { start: number; end: number }[] = [];
  for (const span of spans) {
    const last = deduped[deduped.length - 1];
    if (last && span.start <= last.end) {
      last.end = Math.max(last.end, span.end);
    } else {
      deduped.push({ ...span });
    }
  }
  return deduped;
}

function ProvenanceGraphPanel({ graph }: { graph: import('../../types').ProvenanceClaimGraph }) {
  return (
    <details className="border-b border-line bg-surface/40 px-4 py-2">
      <summary className="text-caption font-semibold text-fg-secondary">
        C2PA claim graph · {graph.claims.length} claim{graph.claims.length === 1 ? '' : 's'} · {graph.ingredients.length} ingredient relationship{graph.ingredients.length === 1 ? '' : 's'}
      </summary>
      <div className="mt-2 grid gap-3 border-t border-line pt-2 lg:grid-cols-2">
        <div>
          <div className="text-caption font-semibold text-muted">Claims</div>
          <ul className="mt-1 space-y-1 text-caption text-muted">
            {graph.claims.map((claim) => (
              <li key={claim.manifestLabel} className="rounded border border-line bg-canvas/70 px-2 py-1.5">
                <div className="font-mono text-fg-secondary">{claim.manifestLabel}{claim.isActive ? ' · active' : ''}</div>
                <div>{claim.title ?? 'Untitled asset'}{claim.format ? ` · ${claim.format}` : ''}{claim.claimGenerator ? ` · ${claim.claimGenerator}` : ''}</div>
                <div>{claim.assertionLabels.length} signed assertion reference{claim.assertionLabels.length === 1 ? '' : 's'}</div>
              </li>
            ))}
          </ul>
        </div>
        <div>
          <div className="text-caption font-semibold text-muted">Ingredient relationships</div>
          {graph.ingredients.length === 0 ? (
            <p className="mt-1 text-caption text-muted">The manifest declares no ingredient relationships.</p>
          ) : (
            <ul className="mt-1 space-y-1 text-caption text-muted">
              {graph.ingredients.map((ingredient, index) => (
                <li key={`${ingredient.sourceManifestLabel}-${ingredient.targetManifestLabel ?? ingredient.instanceId ?? index}`} className="rounded border border-line bg-canvas/70 px-2 py-1.5">
                  <span className="font-mono text-fg-secondary">{ingredient.sourceManifestLabel}</span>
                  {' → '}{ingredient.targetManifestLabel ?? ingredient.title ?? ingredient.instanceId ?? 'unidentified ingredient'}
                  <span className="ml-1 text-highlight">({ingredient.relationship})</span>
                  {ingredient.validationStatuses.map((status) => <div key={status} className="text-warning">{status}</div>)}
                </li>
              ))}
            </ul>
          )}
        </div>
      </div>
      <ul className="mt-2 space-y-0.5 text-caption text-muted">
        {graph.certificateChainDiagnostics.map((diagnostic) => <li key={diagnostic}>{diagnostic}</li>)}
        {graph.validationStatuses.map((status) => <li key={status} className="text-warning">{status}</li>)}
        {graph.truncated && <li className="text-warning">The embedded graph exceeded safety bounds; this view is incomplete.</li>}
      </ul>
    </details>
  );
}

const MINIMAP_FINDING_LIMIT = 200;

/** Source offsets (code points from `base`) of ascending UTF-16 indices into `text`, in one pass. */
function sourceOffsets(text: string, indices: readonly number[], base: number): number[] {
  const result: number[] = [];
  let unit = 0;
  let points = 0;
  for (const index of indices) {
    while (unit < index && unit < text.length) {
      const code = text.charCodeAt(unit);
      unit += code >= 0xd800 && code <= 0xdbff && unit + 1 < text.length ? 2 : 1;
      points += 1;
    }
    result.push(base + points);
  }
  return result;
}

/** The UTF-16 index of the `points`-th code point of `text`. */
function utf16Index(text: string, points: number): number {
  let unit = 0;
  for (let seen = 0; seen < points && unit < text.length; seen++) {
    const code = text.charCodeAt(unit);
    unit += code >= 0xd800 && code <= 0xdbff && unit + 1 < text.length ? 2 : 1;
  }
  return unit;
}

function findMatches(text: string, query: string) {
  const needle = query.trim().toLocaleLowerCase();
  if (!needle) return [];
  const haystack = text.toLocaleLowerCase();
  const matches: number[] = [];
  let cursor = 0;
  while (matches.length < 1_000) {
    const index = haystack.indexOf(needle, cursor);
    if (index < 0) break;
    matches.push(index);
    cursor = index + Math.max(1, needle.length);
  }
  return matches;
}

function HighlightedTextPage({ text, query, page, activeMatchIndex }: { text: string; query: string; page: number; activeMatchIndex: number | null }) {
  if (!text) return <span className="whitespace-pre-wrap">No extracted text is available for this chunk.</span>;
  const start = page * TEXT_PAGE_CHARACTERS;
  const end = Math.min(text.length, start + TEXT_PAGE_CHARACTERS);
  const visible = text.slice(start, end);
  const needle = query.trim();
  if (!needle) return <span className="whitespace-pre-wrap">{visible}</span>;
  const matches = findMatches(visible, needle);
  if (matches.length === 0) return <span className="whitespace-pre-wrap">{visible}</span>;
  const segments: React.ReactNode[] = [];
  let copied = 0;
  for (const localIndex of matches) {
    if (localIndex > copied) segments.push(visible.slice(copied, localIndex));
    const isActive = start + localIndex === activeMatchIndex;
    segments.push(<mark key={localIndex} className={isActive ? 'rounded-sm bg-highlight px-0.5 text-on-accent ring-2 ring-highlight/40' : 'rounded-sm bg-highlight/20 px-0.5 text-fg'}>{visible.slice(localIndex, localIndex + needle.length)}</mark>);
    copied = localIndex + needle.length;
  }
  segments.push(visible.slice(copied));
  return <span className="whitespace-pre-wrap">{segments}</span>;
}

function ResidualEvidencePanel({ bundle, error }: { bundle: EvidenceBundle | null; error: string | null }) {
  if (error) {
    return <div role="alert" className="border-b border-danger/30 bg-danger/5 px-4 py-2 text-caption leading-relaxed text-danger">Residual-evidence audit unavailable: {error}</div>;
  }
  if (!bundle) return null;
  return (
    <section className="max-h-72 overflow-y-auto border-b border-accent/30 bg-accent/5 px-4 py-3" aria-label="Residual evidence audit result">
      <div className="flex flex-wrap items-center gap-2 text-caption">
        <span className="font-semibold text-accent">Residual evidence audit</span>
        <span className={bundle.coverageComplete ? 'text-success' : 'text-warning'}>{bundle.coverageComplete ? 'Supported collector coverage complete' : 'Coverage incomplete'}</span>
        <span className="text-muted">{bundle.findings.length.toLocaleString()} finding{bundle.findings.length === 1 ? '' : 's'}</span>
      </div>
      {bundle.warnings.length > 0 && <ul className="mt-2 space-y-1 text-caption text-warning">{bundle.warnings.map((warning) => <li key={warning}>{humanizeEvidenceKind(warning)}</li>)}</ul>}
      {bundle.findings.length === 0 && <p className="mt-2 text-caption text-muted">No evidence was reported by the collectors that ran. An incomplete audit is not a negative finding.</p>}
      <div className="mt-2 space-y-2">
        {bundle.findings.map((finding, index) => <ResidualFinding key={`${finding.kind}-${finding.valueSha256 ?? index}-${index}`} finding={finding} />)}
      </div>
    </section>
  );
}

function ResidualFinding({ finding }: { finding: ForensicFinding }) {
  const location = [
    finding.locator.pageNumber !== null ? `page ${finding.locator.pageNumber}` : null,
    finding.locator.objectReference,
    finding.locator.containerPart,
    finding.locator.xmlPath,
    finding.locator.byteOffset !== null ? `byte ${finding.locator.byteOffset.toLocaleString()}` : null,
  ].filter(Boolean).join(' · ');
  return (
    <article className="rounded-lg border border-line bg-canvas/70 p-2.5">
      <div className="flex flex-wrap items-center gap-2 text-caption uppercase tracking-wider">
        <span className="font-semibold text-fg-secondary">{humanizeEvidenceKind(finding.kind)}</span>
        <span className="rounded border border-line-strong px-1.5 py-0.5 text-muted">{finding.class}</span>
        <span className="rounded border border-line-strong px-1.5 py-0.5 text-muted">{finding.confidence}</span>
      </div>
      {location && <div className="mt-1 break-all font-mono text-caption text-muted">{location}</div>}
      {finding.maskedPreview && <pre className="mt-2 max-h-24 overflow-auto whitespace-pre-wrap rounded border border-line p-2 font-mono text-caption text-muted">{finding.maskedPreview}</pre>}
      <p className="mt-2 text-caption leading-relaxed text-muted">{finding.explanation}</p>
      {finding.valueSha256 && <div className="mt-1 break-all font-mono text-caption text-muted">SHA-256 {finding.valueSha256}</div>}
    </article>
  );
}

function humanizeEvidenceKind(value: string) {
  return value.split(/[_:]/).filter(Boolean).map((part) => part.toLocaleLowerCase()).join(' ');
}

function RevisionRecoveryPanel({ profile, error }: { profile: RecoveredVersionProfile | null; error: string | null }) {
  if (error) {
    return <div role="alert" className="border-b border-danger/30 bg-danger/5 px-4 py-2 text-caption leading-relaxed text-danger">Revision inspection unavailable: {error}</div>;
  }
  if (!profile) return null;
  return (
    <div className="max-h-60 overflow-y-auto border-b border-highlight/30 bg-highlight/5 px-4 py-3">
      <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-caption text-muted">
        <span className="font-semibold text-highlight">Incremental revision inspection</span>
        <span>{profile.historicalVersionsFound} candidate revision boundar{profile.historicalVersionsFound === 1 ? 'y' : 'ies'}</span>
        <span>{profile.recoveredTextSegments.length} extracted historical text layer{profile.recoveredTextSegments.length === 1 ? '' : 's'}</span>
        <span className="font-mono">SHA-256 {profile.sourceSha256.slice(0, 12)}…</span>
      </div>
      {profile.historicalVersionsFound === 0 && <p className="mt-2 text-caption text-muted">No earlier incremental-save boundary was present. This does not rule out edits made before the current file was created.</p>}
      {profile.recoveredTextSegments.length > 0 && <p className="mt-2 text-caption text-muted">Historical text may duplicate the current document. A revision boundary does not establish removed or concealed content.</p>}
      {profile.analysisTruncated && <p className="mt-2 text-caption text-warning">The bounded inspection reached a safety limit; the results are incomplete.</p>}
      {profile.warnings.length > 0 && <ul className="mt-2 space-y-1 text-caption text-warning">{profile.warnings.map((warning) => <li key={warning}>{warning}</li>)}</ul>}
      {profile.revisionDiagnostics.map((diagnostic) => (
        <details key={`diagnostic-${diagnostic.revisionNumber}-${diagnostic.snapshotEndOffset}`} className="mt-2 rounded-lg border border-line bg-canvas/70 px-3 py-2">
          <summary className="text-caption font-medium text-fg-secondary">Revision {diagnostic.revisionNumber} structure · {diagnostic.objectCount.toLocaleString()} objects · {diagnostic.pageCount.toLocaleString()} pages · {diagnostic.pageExtractionFailureCount.toLocaleString()} page errors</summary>
          <dl className="mt-2 grid grid-cols-2 gap-2 border-t border-line pt-2 text-caption text-muted">
            <div><dt>Xref</dt><dd className="font-mono text-fg-secondary">{diagnostic.crossReferenceType ?? 'unparsed'} at {diagnostic.xrefStartOffset?.toLocaleString() ?? 'unknown'}</dd></div>
            <div><dt>Previous xref</dt><dd className="font-mono text-fg-secondary">{diagnostic.declaredPreviousXrefOffset?.toLocaleString() ?? 'none declared'}</dd></div>
            <div><dt>Nonzero generations</dt><dd className="font-mono text-fg-secondary">{diagnostic.nonzeroGenerationObjectCount.toLocaleString()}</dd></div>
            <div><dt>Unreferenced objects / streams</dt><dd className="font-mono text-fg-secondary">{diagnostic.unreferencedObjectCount.toLocaleString()} / {diagnostic.unreferencedStreamCount.toLocaleString()}</dd></div>
          </dl>
          {!diagnostic.objectGraphTraversalComplete && <p className="mt-2 text-caption text-warning">Object-graph traversal encountered an unresolved reference or nesting limit.</p>}
          {diagnostic.structuralParseError && <p className="mt-2 break-words font-mono text-caption text-danger">{diagnostic.structuralParseError}</p>}
          {diagnostic.representativePageErrors.length > 0 && <ul className="mt-2 space-y-1 font-mono text-caption text-warning">{diagnostic.representativePageErrors.map((message) => <li key={message}>{message}</li>)}</ul>}
        </details>
      ))}
      {profile.recoveredTextSegments.map((segment) => (
        <details key={`${segment.revisionNumber}-${segment.recoveredTextSha256}`} className="mt-2 rounded-lg border border-line bg-canvas/70 px-3 py-2">
          <summary className="text-caption font-medium text-fg-secondary">Revision {segment.revisionNumber} · {segment.recoveredCharacterCount.toLocaleString()} recovered characters · {segment.maskedFindingCount} sensitive values masked</summary>
          <pre className="mt-2 max-h-32 overflow-auto whitespace-pre-wrap border-t border-line pt-2 font-mono text-caption leading-relaxed text-muted">{segment.maskedText}</pre>
        </details>
      ))}
      {profile.postEofMetadataArtifacts.length > 0 && (
        <details className="mt-2 rounded-lg border border-line bg-canvas/70 px-3 py-2">
          <summary className="text-caption font-medium text-fg-secondary">{profile.postEofMetadataArtifacts.length} post-EOF metadata artifact{profile.postEofMetadataArtifacts.length === 1 ? '' : 's'}</summary>
          <ul className="mt-2 space-y-1 border-t border-line pt-2 font-mono text-caption text-muted">{profile.postEofMetadataArtifacts.map((artifact) => <li key={`${artifact.byteOffset}-${artifact.valueSha256}`}>{artifact.category.split('_').join(' ')} at byte {artifact.byteOffset.toLocaleString()}: {artifact.maskedValue}</li>)}</ul>
        </details>
      )}
    </div>
  );
}

// One badge per rung of the confidence ladder. Collapsing rungs is what made a digest mismatch --
// a substitution finding -- render identically to "no digest was ever recorded".
const PROVENANCE_BADGES: Record<ProvenanceConfidence, { label: string; detail: string; className: string }> = {
  // Kept although the current backend cannot report it: the log panel renders rows recorded by
  // any build, and dropping the entry would send a real rung through UNRECOGNIZED_BADGE.
  TRUSTED_CHAIN_OF_CUSTODY: { label: 'Chain of custody verified', detail: 'A valid signature chains to a configured trust anchor. This build configures no trust anchors, so it does not produce this result.', className: 'border-success/30 bg-success/10 font-semibold text-success' },
  VERIFIED_SIGNATURE: { label: 'Manifest signature verified', detail: 'The signature verifies over the asset, but the signer chains to no configured trust anchor.', className: 'border-success/30 bg-success/10 font-medium text-success' },
  LOCAL_DIGEST_MATCH: { label: 'Local file hash verified', detail: 'The stored bytes match the digest recorded at ingestion. This is local integrity, not provenance.', className: 'border-accent/30 bg-accent/10 font-medium text-accent' },
  RECORDED_CLAIM_ONLY: { label: 'Recorded claim, unchecked', detail: 'Manifest metadata exists but nothing has been checked against the stored bytes.', className: 'border-warning/30 bg-warning/10 font-medium text-warning' },
  NO_RECORDED_DIGEST: { label: 'No recorded digest', detail: 'No digest was recorded for this document, so nothing about the stored bytes can be confirmed.', className: 'border-warning/30 bg-warning/10 font-medium text-warning' },
  EVIDENCE_FILE_UNAVAILABLE: { label: 'Evidence file missing', detail: 'The check ran and found no readable evidence file: none was recorded, the recorded path no longer resolves, or it failed the evidence-store gate. A custody gap, not an unrun check.', className: 'border-warning/30 bg-warning/10 font-medium text-warning' },
  DIGEST_MISMATCH: { label: 'Digest mismatch', detail: 'The stored bytes do not hash to the recorded digest. The file changed after ingestion, or the wrong file is stored under this identifier.', className: 'border-danger/30 bg-danger/10 font-semibold text-danger' },
};

const SIGNATURE_INVALID_BADGE = { label: 'Manifest signature invalid', detail: 'A provenance claim is attached to this document and it does not hold.', className: 'border-danger/30 bg-danger/10 font-semibold text-danger' };

// Fires only when Rust adds a rung this union has not learned yet. Naming a real rung there would
// assert a result the check never produced, which is the failure this whole badge set exists to
// avoid, so the unknown value is shown verbatim instead.
const UNRECOGNIZED_BADGE = { label: 'Unrecognized confidence state', detail: 'The backend reported a confidence level this build does not know how to present. Treat the result as unclassified.', className: 'border-highlight/30 bg-highlight/10 font-semibold text-highlight' };

function ProvenanceShield({ isValidating, result, error }: { isValidating: boolean; result: ReturnType<typeof useC2PA>['activeManifest']; error: string | null }) {
  if (isValidating) return <span className="rounded-full border border-line-strong px-2.5 py-1 text-caption text-muted">Verifying…</span>;
  if (!result) return <span title={error ?? 'No verification has been run for this document.'} className="rounded-full border border-line-strong bg-raised/40 px-2.5 py-1 text-caption font-medium text-muted">Provenance not checked</span>;
  const badge: { label: string; detail: string; className: string } | undefined = PROVENANCE_BADGES[result.provenanceConfidence];
  // An invalid manifest never lowers what the digest earned, so the backend keeps the rung and
  // records a finding. Show both: the finding must not be absorbed by a cyan badge, and the
  // integrity fact the digest earned must not vanish behind the finding.
  const signatureInvalid = result.signatureVerification === 'INVALID' && result.provenanceConfidence !== 'DIGEST_MISMATCH';
  const shown = badge ?? { ...UNRECOGNIZED_BADGE, detail: `${UNRECOGNIZED_BADGE.detail} Reported: ${String(result.provenanceConfidence)}.` };
  const title = [shown.detail, ...result.verificationErrors].join(' ');
  return (
    <>
      {signatureInvalid && <span title={SIGNATURE_INVALID_BADGE.detail} className={`rounded-full border px-2.5 py-1 text-caption ${SIGNATURE_INVALID_BADGE.className}`}>{SIGNATURE_INVALID_BADGE.label}</span>}
      <span title={title} className={`rounded-full border px-2.5 py-1 text-caption ${shown.className}`}>{shown.label}</span>
      {result.textProjectionMatches === false && (
        <span title="The extracted text this application indexes and quotes no longer hashes to the digest recorded when the document was extracted. The file itself may still be intact." className="rounded-full border border-danger/30 bg-danger/10 px-2.5 py-1 text-caption font-semibold text-danger">Quoted text drifted</span>
      )}
    </>
  );
}

// A logged row carries no confidence level when it was written before the ladder existed. That is
// an absent field, and rendering it through PROVENANCE_BADGES would fall through to
// UNRECOGNIZED_BADGE and claim the backend reported something it never reported.
const PREDATES_CLASSIFICATION_BADGE = { label: 'Logged before classification', detail: 'This check ran before the confidence ladder existed, so no level was recorded. Absent, not low.', className: 'border-line-strong bg-raised/70 font-medium text-muted' };
const UNCLASSIFIED_BADGE = { label: 'No confidence recorded', detail: 'This row carries no confidence level and is not marked as predating classification. Treat the level as unknown.', className: 'border-highlight/30 bg-highlight/10 font-semibold text-highlight' };

function badgeForRecord(record: ProvenanceVerificationRecord): { label: string; detail: string; className: string } {
  if (record.provenanceConfidence === null) {
    return record.predatesClassification ? PREDATES_CLASSIFICATION_BADGE : UNCLASSIFIED_BADGE;
  }
  const badge: { label: string; detail: string; className: string } | undefined = PROVENANCE_BADGES[record.provenanceConfidence];
  return badge ?? { ...UNRECOGNIZED_BADGE, detail: `${UNRECOGNIZED_BADGE.detail} Reported: ${String(record.provenanceConfidence)}.` };
}

// The append-only log is the artifact an investigator cites: that this document was checked, when,
// and what the check said at that time. It is read on demand rather than on mount because it is
// evidence about the past, not the header's current state.
function VerificationHistoryPanel({ records, isLoading, error }: { records: ProvenanceVerificationRecord[] | null; isLoading: boolean; error: string | null }) {
  if (error) {
    return <div role="alert" className="border-b border-danger/30 bg-danger/5 px-4 py-2 text-caption leading-relaxed text-danger">Verification log unavailable: {error}</div>;
  }
  if (isLoading || !records) {
    return <div className="border-b border-line bg-surface/40 px-4 py-2 text-caption text-muted">Reading verification log…</div>;
  }
  if (records.length === 0) {
    return <div className="border-b border-line bg-surface/40 px-4 py-2 text-caption text-muted">No verification has been recorded for this document. The absence of a log entry is not a verification result.</div>;
  }
  return (
    <div className="max-h-60 overflow-y-auto border-b border-line bg-surface/40 px-4 py-3">
      <div className="text-caption font-semibold text-fg-secondary">Verification log · {records.length} recorded check{records.length === 1 ? '' : 's'}, newest first</div>
      <ol className="mt-2 space-y-2">
        {records.map((record) => {
          const badge = badgeForRecord(record);
          return (
            <li key={record.verificationId} className="rounded-lg border border-line bg-canvas/70 px-3 py-2">
              <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
                <time dateTime={record.verifiedAt} title={record.verifiedAt} className="text-caption tabular-nums text-muted">{formatInstant(record.verifiedAt)}</time>
                <span title={badge.detail} className={`rounded-full border px-2 py-0.5 text-caption ${badge.className}`}>{badge.label}</span>
                {record.signatureVerification === 'INVALID' && <span className="rounded-full border border-danger/30 bg-danger/10 px-2 py-0.5 text-caption font-semibold text-danger">Signature invalid</span>}
                {record.signatureVerification === 'VALID' && <span className="rounded-full border border-success/30 bg-success/10 px-2 py-0.5 text-caption font-medium text-success">Signature valid</span>}
                {record.textProjectionMatches === false && <span className="rounded-full border border-danger/30 bg-danger/10 px-2 py-0.5 text-caption font-semibold text-danger">Quoted text drifted</span>}
                {record.evidenceFilePresent === false && <span className="rounded-full border border-warning/30 bg-warning/10 px-2 py-0.5 text-caption font-medium text-warning">No evidence file</span>}
              </div>
              <div className="mt-1 flex flex-wrap gap-x-3 gap-y-1 font-mono text-caption text-muted">
                <span>computed {record.computedSha256 ? `${record.computedSha256.slice(0, 12)}…` : '—'}</span>
                <span>recorded {record.expectedSha256 ? `${record.expectedSha256.slice(0, 12)}…` : '—'}</span>
                {/* 'NONE' printed bare would sit beside two digest facts and read as a truncated value. */}
                {record.recordedDigestSource === 'NONE' && <span>no recorded digest source</span>}
                {record.recordedDigestSource !== null && record.recordedDigestSource !== 'NONE' && <span>compared against {record.recordedDigestSource.split('_').join(' ').toLocaleLowerCase()}</span>}
                {record.invalidAssertionCount > 0 && <span className="text-warning">{record.invalidAssertionCount} invalid assertion{record.invalidAssertionCount === 1 ? '' : 's'}</span>}
              </div>
              {record.verificationErrors.length > 0 && (
                <ul className="mt-1 space-y-0.5 text-caption leading-relaxed text-warning">{record.verificationErrors.map((reason) => <li key={reason}>{reason}</li>)}</ul>
              )}
            </li>
          );
        })}
      </ol>
    </div>
  );
}

function ClaimChecksPanel({ checks, state, error }: { checks: DocumentClaimChecks | null; state: 'idle' | 'pending' | 'failed'; error: string | null }) {
  return (
    <section aria-label="Figures checked against imported tables" className="space-y-2 border-b border-line p-3 text-label">
      <h3 className="text-label font-semibold text-fg">Figures checked against imported tables</h3>
      {state === 'pending' && <p role="status" className="text-muted">Checking the figures in this document…</p>}
      {state === 'failed' && <p role="alert" className="text-danger">The check could not run: {error}</p>}
      {state === 'idle' && checks && checks.checks.length === 0 && (
        <p role="status" className="text-muted">No figures found in this document that name a budget or spending total, a percent change, or a balance.</p>
      )}
      {state === 'idle' && checks && checks.checks.length > 0 && (
        <ul role="list" className="space-y-2">
          {checks.checks.map((check) => (
            <li key={`${check.characterStart}-${check.characterEnd}`} className="rounded-lg border border-line bg-canvas/40 p-2">
              <blockquote className="text-fg-secondary">“{check.claimText}”</blockquote>
              <div className="mt-1 text-caption text-muted">
                {check.kindLabel}
                {check.scope.fiscalYear !== null && ` · FY ${check.scope.fiscalYear}`}
                {check.scope.fund !== null && ` · fund ${check.scope.fund}`}
                {check.scope.department !== null && ` · ${check.scope.department}`}
              </div>
              {check.result.outcome === 'consistent' && (
                <p className="mt-1 text-success">Consistent: the data shows {check.result.observed}, within {check.result.band}. Query id(s) {check.result.queryIds.join(', ')}.</p>
              )}
              {check.result.outcome === 'mismatch' && (
                <p className="mt-1 text-warning">Mismatch: states {check.result.claimed}; the data shows {check.result.observed}, a difference of {check.result.difference}, outside {check.result.band}. Query id(s) {check.result.queryIds.join(', ')}.</p>
              )}
              {check.result.outcome === 'unreconciled' && (
                <p className="mt-1 text-muted">Not checked ({check.result.reason}): {check.result.detail}</p>
              )}
            </li>
          ))}
        </ul>
      )}
      {state === 'idle' && checks?.truncated && (
        <p role="status" className="text-warning">Not every figure was checked: the document exceeded the per-document claim cap or the text window that was read.</p>
      )}
    </section>
  );
}
