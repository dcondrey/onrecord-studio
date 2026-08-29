import { useEffect, useMemo, useState } from 'react';
import { useC2PA } from '../../hooks/useC2PA';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import { invokeCommand } from '../../ipc';
import type { EvidenceBundle, ForensicFinding, ProvenanceConfidence, ProvenanceVerificationRecord, RecoveredVersionProfile } from '../../types';
import UnredactorPanel from './UnredactorPanel';

interface DocumentViewerProps {
  documentId: number;
  requestId?: number;
  portalKey: string;
  fileName: string;
  textContent: string;
  fileSizeFormatted: string;
  textPositionLabel?: string;
  characterOffset?: number;
  knownMaskedFindingCount?: number;
  onNextChunk?: () => void;
}

const TEXT_PAGE_CHARACTERS = 12_000;

export default function DocumentViewer({ documentId, requestId, portalKey, fileName, textContent, fileSizeFormatted, textPositionLabel, characterOffset = 0, knownMaskedFindingCount = 0, onNextChunk }: DocumentViewerProps) {
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
  const historyGate = useLatestRequestGate();
  const recoveryGate = useLatestRequestGate();
  const residualAuditGate = useLatestRequestGate();

  useEffect(() => {
    void verifyAssetProvenance(documentId, portalKey);
  }, [documentId, portalKey, verifyAssetProvenance]);

  useEffect(() => {
    historyGate.invalidate();
    recoveryGate.invalidate();
    residualAuditGate.invalidate();
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
  }, [documentId, historyGate, portalKey, recoveryGate, residualAuditGate]);

  const searchMatches = useMemo(() => findMatches(textContent, documentSearch), [documentSearch, textContent]);
  const pageCount = Math.max(1, Math.ceil(textContent.length / TEXT_PAGE_CHARACTERS));
  const selectedMatch = searchMatches[activeMatch] ?? null;

  useEffect(() => {
    setActiveMatch(0);
    if (searchMatches[0] !== undefined) {
      setTextPage(Math.floor(searchMatches[0] / TEXT_PAGE_CHARACTERS));
    }
  }, [documentSearch, searchMatches]);

  const moveToMatch = (direction: -1 | 1) => {
    if (searchMatches.length === 0) return;
    const next = (activeMatch + direction + searchMatches.length) % searchMatches.length;
    setActiveMatch(next);
    setTextPage(Math.floor(searchMatches[next] / TEXT_PAGE_CHARACTERS));
  };

  const shareEvidenceLocation = async () => {
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
      setShareStatus(`Copied offset ${sourceCharacterOffset.toLocaleString()}`);
    } catch {
      setShareStatus(`Link set at offset ${sourceCharacterOffset.toLocaleString()}`);
    }
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
      setHistoryError(String(reason));
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
      setRecoveryError(String(reason));
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
      setResidualAuditError(String(reason));
    } finally {
      if (residualAuditGate.isCurrent(request)) setIsAuditingResiduals(false);
    }
  };

  return (
    <section className="flex h-full flex-col overflow-hidden rounded-xl border border-slate-800 bg-slate-950 shadow-md">
      <div className="flex items-center justify-between gap-4 border-b border-slate-800 bg-slate-900/40 px-4 py-3">
        <div className="min-w-0">
          <div className="truncate text-xs font-semibold text-slate-200">{fileName}</div>
          <div className="mt-1 font-mono text-[9px] text-slate-400">{portalKey} / document {documentId} · {fileSizeFormatted}</div>
        </div>
        <div className="flex items-center gap-2">
          <button type="button" onClick={() => void inspectResidualEvidence()} disabled={isAuditingResiduals} className="rounded-full border border-cyan-400/25 bg-cyan-500/10 px-2.5 py-1 text-[9px] font-medium text-cyan-300 transition hover:border-cyan-300/40 disabled:cursor-wait disabled:opacity-60">
            {isAuditingResiduals ? 'Auditing residuals…' : residualAudit ? 'Reaudit residuals' : 'Audit residual evidence'}
          </button>
          {fileName.toLocaleLowerCase().endsWith('.pdf') && (
            <>
              <button type="button" onClick={() => setShowRedactionAudit((visible) => !visible)} aria-expanded={showRedactionAudit} className="rounded-full border border-indigo-400/25 bg-indigo-500/10 px-2.5 py-1 text-[9px] font-medium text-indigo-300 transition hover:border-indigo-300/40">
                {showRedactionAudit ? 'Hide redaction audit' : 'Audit redactions'}
              </button>
              <button type="button" onClick={() => void inspectRevisionHistory()} disabled={isRecovering} className="rounded-full border border-indigo-400/25 bg-indigo-500/10 px-2.5 py-1 text-[9px] font-medium text-indigo-300 transition hover:border-indigo-300/40 disabled:cursor-wait disabled:opacity-60">
                {isRecovering ? 'Inspecting revisions…' : recoveryProfile ? 'Reinspect revisions' : 'Inspect revision history'}
              </button>
            </>
          )}
          <button type="button" onClick={() => void toggleVerificationHistory()} aria-expanded={showVerificationHistory} className="rounded-full border border-slate-700 px-2.5 py-1 text-[9px] font-medium text-slate-400 transition hover:border-indigo-400/30 hover:text-indigo-300">
            {showVerificationHistory ? 'Hide verification log' : 'Verification log'}
          </button>
          <ProvenanceShield isValidating={isValidating} result={activeManifest} error={provenanceError} />
        </div>
      </div>
      <div className="flex items-center justify-between border-b border-slate-900 px-4 py-2 text-[9px] text-slate-400">
        <span>Local extracted-text view</span>
        <div className="flex items-center gap-3">
          {textPositionLabel && <span>{textPositionLabel}</span>}
          <span>{knownMaskedFindingCount > 0 ? `${knownMaskedFindingCount} sensitive value${knownMaskedFindingCount === 1 ? '' : 's'} masked` : 'No structured PII detected'}</span>
          {onNextChunk && <button type="button" onClick={onNextChunk} className="rounded border border-slate-800 px-2 py-1 text-slate-400 transition hover:border-indigo-400/30 hover:text-indigo-300">Next chunk</button>}
        </div>
      </div>
      <div className="flex flex-wrap items-center gap-2 border-b border-slate-900 px-4 py-2 text-[9px]">
        <label className="relative min-w-[220px] flex-1">
          <span className="sr-only">Search within this extracted-text chunk</span>
          <input value={documentSearch} onChange={(event) => setDocumentSearch(event.target.value)} placeholder="Find in extracted text" className="focus-ring h-8 w-full rounded-lg border border-slate-800 bg-slate-950 px-3 text-[10px] text-slate-200 placeholder:text-slate-400" />
        </label>
        <span className="min-w-16 text-center font-mono text-slate-400">{searchMatches.length === 0 ? '0 matches' : `${activeMatch + 1}/${searchMatches.length}`}</span>
        <button type="button" onClick={() => moveToMatch(-1)} disabled={searchMatches.length === 0} className="rounded border border-slate-800 px-2 py-1 text-slate-400 disabled:opacity-40">Previous</button>
        <button type="button" onClick={() => moveToMatch(1)} disabled={searchMatches.length === 0} className="rounded border border-slate-800 px-2 py-1 text-slate-400 disabled:opacity-40">Next</button>
        <button type="button" onClick={() => void shareEvidenceLocation()} className="rounded border border-indigo-400/25 bg-indigo-500/10 px-2 py-1 text-indigo-300">Copy evidence link</button>
        {shareStatus && <span role="status" className="text-emerald-300">{shareStatus}</span>}
      </div>
      {activeManifest && activeManifest.claimGraph.claims.length > 0 && (
        <ProvenanceGraphPanel graph={activeManifest.claimGraph} />
      )}
      {showVerificationHistory && <VerificationHistoryPanel records={verificationHistory} isLoading={isLoadingHistory} error={historyError} />}
      {(recoveryProfile || recoveryError) && <RevisionRecoveryPanel profile={recoveryProfile} error={recoveryError} />}
      {(residualAudit || residualAuditError) && <ResidualEvidencePanel bundle={residualAudit} error={residualAuditError} />}
      {showRedactionAudit && <div className="border-b border-slate-800 p-3"><UnredactorPanel documentId={documentId} portalKey={portalKey} /></div>}
      <div className="min-h-0 flex-1 overflow-y-auto p-5 font-mono text-xs leading-relaxed text-slate-300 selection:bg-indigo-500/30">
        <HighlightedTextPage text={textContent} query={documentSearch} page={textPage} activeMatchIndex={selectedMatch} />
      </div>
      {pageCount > 1 && <div className="flex items-center justify-center gap-3 border-t border-slate-900 px-4 py-2 text-[9px] text-slate-400"><button type="button" disabled={textPage === 0} onClick={() => setTextPage((page) => Math.max(0, page - 1))} className="rounded border border-slate-800 px-2 py-1 disabled:opacity-40">Previous text page</button><span className="font-mono">{textPage + 1}/{pageCount}</span><button type="button" disabled={textPage + 1 >= pageCount} onClick={() => setTextPage((page) => Math.min(pageCount - 1, page + 1))} className="rounded border border-slate-800 px-2 py-1 disabled:opacity-40">Next text page</button></div>}
    </section>
  );
}

function ProvenanceGraphPanel({ graph }: { graph: import('../../types').ProvenanceClaimGraph }) {
  return (
    <details className="border-b border-slate-800 bg-slate-900/20 px-4 py-2">
      <summary className="cursor-pointer text-[9px] font-semibold text-slate-300">
        C2PA claim graph · {graph.claims.length} claim{graph.claims.length === 1 ? '' : 's'} · {graph.ingredients.length} ingredient relationship{graph.ingredients.length === 1 ? '' : 's'}
      </summary>
      <div className="mt-2 grid gap-3 border-t border-slate-800 pt-2 lg:grid-cols-2">
        <div>
          <div className="text-[9px] font-semibold text-slate-400">Claims</div>
          <ul className="mt-1 space-y-1 text-[9px] text-slate-400">
            {graph.claims.map((claim) => (
              <li key={claim.manifestLabel} className="rounded border border-slate-800 bg-slate-950/60 px-2 py-1.5">
                <div className="font-mono text-slate-300">{claim.manifestLabel}{claim.isActive ? ' · active' : ''}</div>
                <div>{claim.title ?? 'Untitled asset'}{claim.format ? ` · ${claim.format}` : ''}{claim.claimGenerator ? ` · ${claim.claimGenerator}` : ''}</div>
                <div>{claim.assertionLabels.length} signed assertion reference{claim.assertionLabels.length === 1 ? '' : 's'}</div>
              </li>
            ))}
          </ul>
        </div>
        <div>
          <div className="text-[9px] font-semibold text-slate-400">Ingredient relationships</div>
          {graph.ingredients.length === 0 ? (
            <p className="mt-1 text-[9px] text-slate-400">The manifest declares no ingredient relationships.</p>
          ) : (
            <ul className="mt-1 space-y-1 text-[9px] text-slate-400">
              {graph.ingredients.map((ingredient, index) => (
                <li key={`${ingredient.sourceManifestLabel}-${ingredient.targetManifestLabel ?? ingredient.instanceId ?? index}`} className="rounded border border-slate-800 bg-slate-950/60 px-2 py-1.5">
                  <span className="font-mono text-slate-300">{ingredient.sourceManifestLabel}</span>
                  {' → '}{ingredient.targetManifestLabel ?? ingredient.title ?? ingredient.instanceId ?? 'unidentified ingredient'}
                  <span className="ml-1 text-indigo-300">({ingredient.relationship})</span>
                  {ingredient.validationStatuses.map((status) => <div key={status} className="text-amber-300">{status}</div>)}
                </li>
              ))}
            </ul>
          )}
        </div>
      </div>
      <ul className="mt-2 space-y-0.5 text-[9px] text-slate-400">
        {graph.certificateChainDiagnostics.map((diagnostic) => <li key={diagnostic}>{diagnostic}</li>)}
        {graph.validationStatuses.map((status) => <li key={status} className="text-amber-300">{status}</li>)}
        {graph.truncated && <li className="text-amber-300">The embedded graph exceeded safety bounds; this view is incomplete.</li>}
      </ul>
    </details>
  );
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
    segments.push(<mark key={localIndex} className={isActive ? 'rounded bg-amber-300 px-0.5 text-slate-950 ring-2 ring-amber-200/60' : 'rounded bg-indigo-400/35 px-0.5 text-slate-100'}>{visible.slice(localIndex, localIndex + needle.length)}</mark>);
    copied = localIndex + needle.length;
  }
  segments.push(visible.slice(copied));
  return <span className="whitespace-pre-wrap">{segments}</span>;
}

function ResidualEvidencePanel({ bundle, error }: { bundle: EvidenceBundle | null; error: string | null }) {
  if (error) {
    return <div role="alert" className="border-b border-rose-500/20 bg-rose-500/[0.06] px-4 py-2 text-[9px] leading-relaxed text-rose-300">Residual-evidence audit unavailable: {error}</div>;
  }
  if (!bundle) return null;
  return (
    <section className="max-h-72 overflow-y-auto border-b border-cyan-400/15 bg-cyan-500/[0.03] px-4 py-3" aria-label="Residual evidence audit result">
      <div className="flex flex-wrap items-center gap-2 text-[9px]">
        <span className="font-semibold text-cyan-300">Residual evidence audit</span>
        <span className={bundle.coverageComplete ? 'text-emerald-300' : 'text-amber-300'}>{bundle.coverageComplete ? 'Supported collector coverage complete' : 'Coverage incomplete'}</span>
        <span className="text-slate-400">{bundle.findings.length.toLocaleString()} finding{bundle.findings.length === 1 ? '' : 's'}</span>
      </div>
      {bundle.warnings.length > 0 && <ul className="mt-2 space-y-1 text-[9px] text-amber-200/80">{bundle.warnings.map((warning) => <li key={warning}>{humanizeEvidenceKind(warning)}</li>)}</ul>}
      {bundle.findings.length === 0 && <p className="mt-2 text-[9px] text-slate-400">No evidence was reported by the collectors that ran. An incomplete audit is not a negative finding.</p>}
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
    <article className="rounded-lg border border-slate-800 bg-slate-950/70 p-2.5">
      <div className="flex flex-wrap items-center gap-2 text-[8px] uppercase tracking-wider">
        <span className="font-semibold text-slate-300">{humanizeEvidenceKind(finding.kind)}</span>
        <span className="rounded border border-slate-700 px-1.5 py-0.5 text-slate-400">{finding.class}</span>
        <span className="rounded border border-slate-700 px-1.5 py-0.5 text-slate-400">{finding.confidence}</span>
      </div>
      {location && <div className="mt-1 break-all font-mono text-[8px] text-slate-400">{location}</div>}
      {finding.maskedPreview && <pre className="mt-2 max-h-24 overflow-auto whitespace-pre-wrap rounded border border-slate-800 p-2 font-mono text-[9px] text-slate-400">{finding.maskedPreview}</pre>}
      <p className="mt-2 text-[9px] leading-relaxed text-slate-400">{finding.explanation}</p>
      {finding.valueSha256 && <div className="mt-1 break-all font-mono text-[8px] text-slate-400">SHA-256 {finding.valueSha256}</div>}
    </article>
  );
}

function humanizeEvidenceKind(value: string) {
  return value.split(/[_:]/).filter(Boolean).map((part) => part.toLocaleLowerCase()).join(' ');
}

function RevisionRecoveryPanel({ profile, error }: { profile: RecoveredVersionProfile | null; error: string | null }) {
  if (error) {
    return <div role="alert" className="border-b border-rose-500/20 bg-rose-500/[0.06] px-4 py-2 text-[9px] leading-relaxed text-rose-300">Revision inspection unavailable: {error}</div>;
  }
  if (!profile) return null;
  return (
    <div className="max-h-60 overflow-y-auto border-b border-indigo-400/15 bg-indigo-500/[0.04] px-4 py-3">
      <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-[9px] text-slate-400">
        <span className="font-semibold text-indigo-300">Incremental revision inspection</span>
        <span>{profile.historicalVersionsFound} prior revision{profile.historicalVersionsFound === 1 ? '' : 's'} detected</span>
        <span>{profile.recoveredTextSegments.length} recoverable text layer{profile.recoveredTextSegments.length === 1 ? '' : 's'}</span>
        <span className="font-mono">SHA-256 {profile.sourceSha256.slice(0, 12)}…</span>
      </div>
      {profile.historicalVersionsFound === 0 && <p className="mt-2 text-[9px] text-slate-400">No earlier incremental-save boundary was present. This does not rule out edits made before the current file was created.</p>}
      {profile.analysisTruncated && <p className="mt-2 text-[9px] text-amber-300">The bounded inspection reached a safety limit; the results are incomplete.</p>}
      {profile.warnings.length > 0 && <ul className="mt-2 space-y-1 text-[9px] text-amber-200/80">{profile.warnings.map((warning) => <li key={warning}>{warning}</li>)}</ul>}
      {profile.revisionDiagnostics.map((diagnostic) => (
        <details key={`diagnostic-${diagnostic.revisionNumber}-${diagnostic.snapshotEndOffset}`} className="mt-2 rounded-lg border border-slate-800 bg-slate-950/70 px-3 py-2">
          <summary className="cursor-pointer text-[9px] font-medium text-slate-300">Revision {diagnostic.revisionNumber} structure · {diagnostic.objectCount.toLocaleString()} objects · {diagnostic.pageCount.toLocaleString()} pages · {diagnostic.pageExtractionFailureCount.toLocaleString()} page errors</summary>
          <dl className="mt-2 grid grid-cols-2 gap-2 border-t border-slate-800 pt-2 text-[9px] text-slate-400">
            <div><dt>Xref</dt><dd className="font-mono text-slate-300">{diagnostic.crossReferenceType ?? 'unparsed'} at {diagnostic.xrefStartOffset?.toLocaleString() ?? 'unknown'}</dd></div>
            <div><dt>Previous xref</dt><dd className="font-mono text-slate-300">{diagnostic.declaredPreviousXrefOffset?.toLocaleString() ?? 'none declared'}</dd></div>
            <div><dt>Nonzero generations</dt><dd className="font-mono text-slate-300">{diagnostic.nonzeroGenerationObjectCount.toLocaleString()}</dd></div>
            <div><dt>Unreferenced objects / streams</dt><dd className="font-mono text-slate-300">{diagnostic.unreferencedObjectCount.toLocaleString()} / {diagnostic.unreferencedStreamCount.toLocaleString()}</dd></div>
          </dl>
          {!diagnostic.objectGraphTraversalComplete && <p className="mt-2 text-[9px] text-amber-300">Object-graph traversal encountered an unresolved reference or nesting limit.</p>}
          {diagnostic.structuralParseError && <p className="mt-2 break-words font-mono text-[9px] text-rose-300">{diagnostic.structuralParseError}</p>}
          {diagnostic.representativePageErrors.length > 0 && <ul className="mt-2 space-y-1 font-mono text-[9px] text-amber-200/80">{diagnostic.representativePageErrors.map((message) => <li key={message}>{message}</li>)}</ul>}
        </details>
      ))}
      {profile.recoveredTextSegments.map((segment) => (
        <details key={`${segment.revisionNumber}-${segment.recoveredTextSha256}`} className="mt-2 rounded-lg border border-slate-800 bg-slate-950/70 px-3 py-2">
          <summary className="cursor-pointer text-[9px] font-medium text-slate-300">Revision {segment.revisionNumber} · {segment.recoveredCharacterCount.toLocaleString()} recovered characters · {segment.maskedFindingCount} sensitive values masked</summary>
          <pre className="mt-2 max-h-32 overflow-auto whitespace-pre-wrap border-t border-slate-800 pt-2 font-mono text-[9px] leading-relaxed text-slate-400">{segment.maskedText}</pre>
        </details>
      ))}
      {profile.postEofMetadataArtifacts.length > 0 && (
        <details className="mt-2 rounded-lg border border-slate-800 bg-slate-950/70 px-3 py-2">
          <summary className="cursor-pointer text-[9px] font-medium text-slate-300">{profile.postEofMetadataArtifacts.length} post-EOF metadata artifact{profile.postEofMetadataArtifacts.length === 1 ? '' : 's'}</summary>
          <ul className="mt-2 space-y-1 border-t border-slate-800 pt-2 font-mono text-[9px] text-slate-400">{profile.postEofMetadataArtifacts.map((artifact) => <li key={`${artifact.byteOffset}-${artifact.valueSha256}`}>{artifact.category.split('_').join(' ')} at byte {artifact.byteOffset.toLocaleString()}: {artifact.maskedValue}</li>)}</ul>
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
  TRUSTED_CHAIN_OF_CUSTODY: { label: 'Chain of custody verified', detail: 'A valid signature chains to a configured trust anchor. This build configures no trust anchors, so it does not produce this result.', className: 'border-emerald-400/30 bg-emerald-500/10 font-semibold text-emerald-300' },
  VERIFIED_SIGNATURE: { label: 'Manifest signature verified', detail: 'The signature verifies over the asset, but the signer chains to no configured trust anchor.', className: 'border-emerald-400/25 bg-emerald-500/[0.07] font-medium text-emerald-200' },
  LOCAL_DIGEST_MATCH: { label: 'Local file hash verified', detail: 'The stored bytes match the digest recorded at ingestion. This is local integrity, not provenance.', className: 'border-cyan-400/25 bg-cyan-500/10 font-medium text-cyan-300' },
  RECORDED_CLAIM_ONLY: { label: 'Recorded claim, unchecked', detail: 'Manifest metadata exists but nothing has been checked against the stored bytes.', className: 'border-amber-400/25 bg-amber-500/10 font-medium text-amber-300' },
  NO_RECORDED_DIGEST: { label: 'No recorded digest', detail: 'No digest was recorded for this document, so nothing about the stored bytes can be confirmed.', className: 'border-amber-400/25 bg-amber-500/10 font-medium text-amber-300' },
  EVIDENCE_FILE_UNAVAILABLE: { label: 'Evidence file missing', detail: 'The check ran and found no readable evidence file: none was recorded, the recorded path no longer resolves, or it failed the evidence-store gate. A custody gap, not an unrun check.', className: 'border-amber-400/25 bg-amber-500/10 font-medium text-amber-300' },
  DIGEST_MISMATCH: { label: 'Digest mismatch', detail: 'The stored bytes do not hash to the recorded digest. The file changed after ingestion, or the wrong file is stored under this identifier.', className: 'border-rose-500/40 bg-rose-500/10 font-semibold text-rose-300' },
};

const SIGNATURE_INVALID_BADGE = { label: 'Manifest signature invalid', detail: 'A provenance claim is attached to this document and it does not hold.', className: 'border-rose-500/40 bg-rose-500/10 font-semibold text-rose-300' };

// Fires only when Rust adds a rung this union has not learned yet. Naming a real rung there would
// assert a result the check never produced, which is the failure this whole badge set exists to
// avoid, so the unknown value is shown verbatim instead.
const UNRECOGNIZED_BADGE = { label: 'Unrecognized confidence state', detail: 'The backend reported a confidence level this build does not know how to present. Treat the result as unclassified.', className: 'border-fuchsia-400/30 bg-fuchsia-500/10 font-semibold text-fuchsia-300' };

function ProvenanceShield({ isValidating, result, error }: { isValidating: boolean; result: ReturnType<typeof useC2PA>['activeManifest']; error: string | null }) {
  if (isValidating) return <span className="rounded-full border border-slate-700 px-2.5 py-1 text-[9px] text-slate-400">Verifying…</span>;
  if (!result) return <span title={error ?? 'No verification has been run for this document.'} className="rounded-full border border-slate-700 bg-slate-800/40 px-2.5 py-1 text-[9px] font-medium text-slate-400">Provenance not checked</span>;
  const badge: { label: string; detail: string; className: string } | undefined = PROVENANCE_BADGES[result.provenanceConfidence];
  // An invalid manifest never lowers what the digest earned, so the backend keeps the rung and
  // records a finding. Show both: the finding must not be absorbed by a cyan badge, and the
  // integrity fact the digest earned must not vanish behind the finding.
  const signatureInvalid = result.signatureVerification === 'INVALID' && result.provenanceConfidence !== 'DIGEST_MISMATCH';
  const shown = badge ?? { ...UNRECOGNIZED_BADGE, detail: `${UNRECOGNIZED_BADGE.detail} Reported: ${String(result.provenanceConfidence)}.` };
  const title = [shown.detail, ...result.verificationErrors].join(' ');
  return (
    <>
      {signatureInvalid && <span title={SIGNATURE_INVALID_BADGE.detail} className={`rounded-full border px-2.5 py-1 text-[9px] ${SIGNATURE_INVALID_BADGE.className}`}>{SIGNATURE_INVALID_BADGE.label}</span>}
      <span title={title} className={`rounded-full border px-2.5 py-1 text-[9px] ${shown.className}`}>{shown.label}</span>
      {result.textProjectionMatches === false && (
        <span title="The extracted text this application indexes and quotes no longer hashes to the digest recorded when the document was extracted. The file itself may still be intact." className="rounded-full border border-rose-500/40 bg-rose-500/10 px-2.5 py-1 text-[9px] font-semibold text-rose-300">Quoted text drifted</span>
      )}
    </>
  );
}

// A logged row carries no confidence level when it was written before the ladder existed. That is
// an absent field, and rendering it through PROVENANCE_BADGES would fall through to
// UNRECOGNIZED_BADGE and claim the backend reported something it never reported.
const PREDATES_CLASSIFICATION_BADGE = { label: 'Logged before classification', detail: 'This check ran before the confidence ladder existed, so no level was recorded. Absent, not low.', className: 'border-slate-600 bg-slate-800/50 font-medium text-slate-400' };
const UNCLASSIFIED_BADGE = { label: 'No confidence recorded', detail: 'This row carries no confidence level and is not marked as predating classification. Treat the level as unknown.', className: 'border-fuchsia-400/30 bg-fuchsia-500/10 font-semibold text-fuchsia-300' };

function badgeForRecord(record: ProvenanceVerificationRecord): { label: string; detail: string; className: string } {
  if (record.provenanceConfidence === null) {
    return record.predatesClassification ? PREDATES_CLASSIFICATION_BADGE : UNCLASSIFIED_BADGE;
  }
  const badge: { label: string; detail: string; className: string } | undefined = PROVENANCE_BADGES[record.provenanceConfidence];
  return badge ?? { ...UNRECOGNIZED_BADGE, detail: `${UNRECOGNIZED_BADGE.detail} Reported: ${String(record.provenanceConfidence)}.` };
}

// When a check ran is part of what the log attests, so the exact instant has to stay recoverable
// while the visible text stays readable. The local-time rendering is what an investigator reads;
// the raw ISO string -- offset and sub-second precision intact -- is kept verbatim on `dateTime`
// and on the tooltip. A value that will not parse is shown as it was recorded rather than as
// "Invalid Date", which would hide a malformed timestamp instead of showing it.
function formatVerifiedAt(value: string): string {
  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) return value;
  return parsed.toLocaleString(undefined, {
    year: 'numeric',
    month: 'short',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
    timeZoneName: 'short',
  });
}

// The append-only log is the artifact an investigator cites: that this document was checked, when,
// and what the check said at that time. It is read on demand rather than on mount because it is
// evidence about the past, not the header's current state.
function VerificationHistoryPanel({ records, isLoading, error }: { records: ProvenanceVerificationRecord[] | null; isLoading: boolean; error: string | null }) {
  if (error) {
    return <div role="alert" className="border-b border-rose-500/20 bg-rose-500/[0.06] px-4 py-2 text-[9px] leading-relaxed text-rose-300">Verification log unavailable: {error}</div>;
  }
  if (isLoading || !records) {
    return <div className="border-b border-slate-800 bg-slate-900/30 px-4 py-2 text-[9px] text-slate-400">Reading verification log…</div>;
  }
  if (records.length === 0) {
    return <div className="border-b border-slate-800 bg-slate-900/30 px-4 py-2 text-[9px] text-slate-400">No verification has been recorded for this document. The absence of a log entry is not a verification result.</div>;
  }
  return (
    <div className="max-h-60 overflow-y-auto border-b border-slate-800 bg-slate-900/30 px-4 py-3">
      <div className="text-[9px] font-semibold text-slate-300">Verification log · {records.length} recorded check{records.length === 1 ? '' : 's'}, newest first</div>
      <ol className="mt-2 space-y-2">
        {records.map((record) => {
          const badge = badgeForRecord(record);
          return (
            <li key={record.verificationId} className="rounded-lg border border-slate-800 bg-slate-950/70 px-3 py-2">
              <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
                <time dateTime={record.verifiedAt} title={record.verifiedAt} className="font-mono text-[9px] text-slate-400">{formatVerifiedAt(record.verifiedAt)}</time>
                <span title={badge.detail} className={`rounded-full border px-2 py-0.5 text-[9px] ${badge.className}`}>{badge.label}</span>
                {record.signatureVerification === 'INVALID' && <span className="rounded-full border border-rose-500/40 bg-rose-500/10 px-2 py-0.5 text-[9px] font-semibold text-rose-300">Signature invalid</span>}
                {record.signatureVerification === 'VALID' && <span className="rounded-full border border-emerald-400/25 bg-emerald-500/[0.07] px-2 py-0.5 text-[9px] font-medium text-emerald-200">Signature valid</span>}
                {record.textProjectionMatches === false && <span className="rounded-full border border-rose-500/40 bg-rose-500/10 px-2 py-0.5 text-[9px] font-semibold text-rose-300">Quoted text drifted</span>}
                {record.evidenceFilePresent === false && <span className="rounded-full border border-amber-400/25 bg-amber-500/10 px-2 py-0.5 text-[9px] font-medium text-amber-300">No evidence file</span>}
              </div>
              <div className="mt-1 flex flex-wrap gap-x-3 gap-y-1 font-mono text-[9px] text-slate-400">
                <span>computed {record.computedSha256 ? `${record.computedSha256.slice(0, 12)}…` : '—'}</span>
                <span>recorded {record.expectedSha256 ? `${record.expectedSha256.slice(0, 12)}…` : '—'}</span>
                {/* 'NONE' printed bare would sit beside two digest facts and read as a truncated value. */}
                {record.recordedDigestSource === 'NONE' && <span>no recorded digest source</span>}
                {record.recordedDigestSource !== null && record.recordedDigestSource !== 'NONE' && <span>compared against {record.recordedDigestSource.split('_').join(' ').toLocaleLowerCase()}</span>}
                {record.invalidAssertionCount > 0 && <span className="text-amber-300">{record.invalidAssertionCount} invalid assertion{record.invalidAssertionCount === 1 ? '' : 's'}</span>}
              </div>
              {record.verificationErrors.length > 0 && (
                <ul className="mt-1 space-y-0.5 text-[9px] leading-relaxed text-amber-200/80">{record.verificationErrors.map((reason) => <li key={reason}>{reason}</li>)}</ul>
              )}
            </li>
          );
        })}
      </ol>
    </div>
  );
}
