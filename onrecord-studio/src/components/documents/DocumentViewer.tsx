import { useEffect, useState } from 'react';
import { useC2PA } from '../../hooks/useC2PA';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import { invokeCommand } from '../../ipc';
import type { ProvenanceConfidence, ProvenanceVerificationRecord, RecoveredVersionProfile } from '../../types';
import UnredactorPanel from './UnredactorPanel';

interface DocumentViewerProps {
  documentId: number;
  portalKey: string;
  fileName: string;
  textContent: string;
  fileSizeFormatted: string;
  textPositionLabel?: string;
  knownMaskedFindingCount?: number;
  onNextChunk?: () => void;
}

export default function DocumentViewer({ documentId, portalKey, fileName, textContent, fileSizeFormatted, textPositionLabel, knownMaskedFindingCount = 0, onNextChunk }: DocumentViewerProps) {
  // FEATURE(documents): Add in-document search, match highlighting, and a
  // shareable character-offset link for evidence review.
  const { verifyAssetProvenance, activeManifest, isValidating, error: provenanceError } = useC2PA();
  const [recoveryProfile, setRecoveryProfile] = useState<RecoveredVersionProfile | null>(null);
  const [isRecovering, setIsRecovering] = useState(false);
  const [recoveryError, setRecoveryError] = useState<string | null>(null);
  const [showRedactionAudit, setShowRedactionAudit] = useState(false);
  const [showVerificationHistory, setShowVerificationHistory] = useState(false);
  const [verificationHistory, setVerificationHistory] = useState<ProvenanceVerificationRecord[] | null>(null);
  const [isLoadingHistory, setIsLoadingHistory] = useState(false);
  const [historyError, setHistoryError] = useState<string | null>(null);
  const historyGate = useLatestRequestGate();
  const recoveryGate = useLatestRequestGate();

  useEffect(() => {
    void verifyAssetProvenance(documentId, portalKey);
  }, [documentId, portalKey, verifyAssetProvenance]);

  useEffect(() => {
    historyGate.invalidate();
    recoveryGate.invalidate();
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
  }, [documentId, historyGate, portalKey, recoveryGate]);

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
    // FEATURE(documents): Add container-specific archaeology actions only after the result is
    // represented as typed evidence with source offsets and confidence labels.
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

  return (
    <section className="flex h-full flex-col overflow-hidden rounded-xl border border-slate-800 bg-slate-950 shadow-md">
      <div className="flex items-center justify-between gap-4 border-b border-slate-800 bg-slate-900/40 px-4 py-3">
        <div className="min-w-0">
          <div className="truncate text-xs font-semibold text-slate-200">{fileName}</div>
          <div className="mt-1 font-mono text-[9px] text-slate-600">{portalKey} / document {documentId} · {fileSizeFormatted}</div>
        </div>
        <div className="flex items-center gap-2">
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
      <div className="flex items-center justify-between border-b border-slate-900 px-4 py-2 text-[9px] text-slate-600">
        <span>Local extracted-text view</span>
        <div className="flex items-center gap-3">
          {textPositionLabel && <span>{textPositionLabel}</span>}
          <span>{knownMaskedFindingCount > 0 ? `${knownMaskedFindingCount} sensitive value${knownMaskedFindingCount === 1 ? '' : 's'} masked` : 'No structured PII detected'}</span>
          {onNextChunk && <button type="button" onClick={onNextChunk} className="rounded border border-slate-800 px-2 py-1 text-slate-400 transition hover:border-indigo-400/30 hover:text-indigo-300">Next chunk</button>}
        </div>
      </div>
      {showVerificationHistory && <VerificationHistoryPanel records={verificationHistory} isLoading={isLoadingHistory} error={historyError} />}
      {(recoveryProfile || recoveryError) && <RevisionRecoveryPanel profile={recoveryProfile} error={recoveryError} />}
      {showRedactionAudit && <div className="border-b border-slate-800 p-3"><UnredactorPanel documentId={documentId} portalKey={portalKey} /></div>}
      {/* PERF(documents): Virtualize or paginate very large extracted text
          instead of mounting the entire chunk as one DOM text node. */}
      <div className="min-h-0 flex-1 overflow-y-auto whitespace-pre-wrap p-5 font-mono text-xs leading-relaxed text-slate-300 selection:bg-indigo-500/30">
        {textContent || 'No extracted text is available for this chunk.'}
      </div>
    </section>
  );
}

function RevisionRecoveryPanel({ profile, error }: { profile: RecoveredVersionProfile | null; error: string | null }) {
  if (error) {
    return <div role="alert" className="border-b border-rose-500/20 bg-rose-500/[0.06] px-4 py-2 text-[9px] leading-relaxed text-rose-300">Revision inspection unavailable: {error}</div>;
  }
  if (!profile) return null;
  return (
    <div className="max-h-60 overflow-y-auto border-b border-indigo-400/15 bg-indigo-500/[0.04] px-4 py-3">
      <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-[9px] text-slate-500">
        <span className="font-semibold text-indigo-300">Incremental revision inspection</span>
        <span>{profile.historicalVersionsFound} prior revision{profile.historicalVersionsFound === 1 ? '' : 's'} detected</span>
        <span>{profile.recoveredTextSegments.length} recoverable text layer{profile.recoveredTextSegments.length === 1 ? '' : 's'}</span>
        <span className="font-mono">SHA-256 {profile.sourceSha256.slice(0, 12)}…</span>
      </div>
      {profile.historicalVersionsFound === 0 && <p className="mt-2 text-[9px] text-slate-500">No earlier incremental-save boundary was present. This does not rule out edits made before the current file was created.</p>}
      {profile.analysisTruncated && <p className="mt-2 text-[9px] text-amber-300">The bounded inspection reached a safety limit; the results are incomplete.</p>}
      {profile.warnings.length > 0 && <ul className="mt-2 space-y-1 text-[9px] text-amber-200/80">{profile.warnings.map((warning) => <li key={warning}>{warning}</li>)}</ul>}
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
  if (isValidating) return <span className="rounded-full border border-slate-700 px-2.5 py-1 text-[9px] text-slate-500">Verifying…</span>;
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
    return <div className="border-b border-slate-800 bg-slate-900/30 px-4 py-2 text-[9px] text-slate-500">Reading verification log…</div>;
  }
  if (records.length === 0) {
    return <div className="border-b border-slate-800 bg-slate-900/30 px-4 py-2 text-[9px] text-slate-500">No verification has been recorded for this document. The absence of a log entry is not a verification result.</div>;
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
                <time dateTime={record.verifiedAt} title={record.verifiedAt} className="font-mono text-[9px] text-slate-500">{formatVerifiedAt(record.verifiedAt)}</time>
                <span title={badge.detail} className={`rounded-full border px-2 py-0.5 text-[9px] ${badge.className}`}>{badge.label}</span>
                {record.signatureVerification === 'INVALID' && <span className="rounded-full border border-rose-500/40 bg-rose-500/10 px-2 py-0.5 text-[9px] font-semibold text-rose-300">Signature invalid</span>}
                {record.signatureVerification === 'VALID' && <span className="rounded-full border border-emerald-400/25 bg-emerald-500/[0.07] px-2 py-0.5 text-[9px] font-medium text-emerald-200">Signature valid</span>}
                {record.textProjectionMatches === false && <span className="rounded-full border border-rose-500/40 bg-rose-500/10 px-2 py-0.5 text-[9px] font-semibold text-rose-300">Quoted text drifted</span>}
                {record.evidenceFilePresent === false && <span className="rounded-full border border-amber-400/25 bg-amber-500/10 px-2 py-0.5 text-[9px] font-medium text-amber-300">No evidence file</span>}
              </div>
              <div className="mt-1 flex flex-wrap gap-x-3 gap-y-1 font-mono text-[9px] text-slate-600">
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
