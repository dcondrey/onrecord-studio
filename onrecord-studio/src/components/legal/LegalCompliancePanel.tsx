import { useEffect, useMemo, useState } from 'react';
import { invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import { useAppStore } from '../../store/useAppStore';
import type {
  LegalComplianceFinding,
  LegalEvaluationResult,
  LegalFindingReviewStatus,
  RequestSearchResult,
} from '../../types';
import { cn } from '../../utils/cn';

interface LegalCompliancePanelProps {
  record: RequestSearchResult;
  onOpenLibrary: () => void;
}

export default function LegalCompliancePanel({
  record,
  onOpenLibrary,
}: LegalCompliancePanelProps) {
  const findings = useAppStore((state) => state.activeLegalFindings);
  const authorities = useAppStore((state) => state.legalAuthorities);
  const setFindings = useAppStore((state) => state.setActiveLegalFindings);
  const searchTerm = useAppStore((state) => state.searchTerm);
  const selectedAgencyId = useAppStore((state) => state.selectedAgencyId);
  const selectedStatus = useAppStore((state) => state.selectedStatus);
  const setSearchFilters = useAppStore((state) => state.setSearchFilters);
  const [isLoading, setIsLoading] = useState(true);
  const [isEvaluating, setIsEvaluating] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [evaluation, setEvaluation] = useState<LegalEvaluationResult | null>(null);
  const [pendingReview, setPendingReview] = useState<{
    violationId: string;
    status: LegalFindingReviewStatus;
  } | null>(null);
  const [reviewerNotes, setReviewerNotes] = useState('');
  const [isReviewing, setIsReviewing] = useState(false);
  const [selectedFindingId, setSelectedFindingId] = useState<string | null>(null);
  // Loading, evaluating, and reviewing all publish to the same global findings collection. One
  // gate makes the newest of those operations authoritative, including across record changes.
  const findingsGate = useLatestRequestGate();

  useEffect(() => {
    const request = findingsGate.begin();
    setIsLoading(true);
    // Reset with the rest: otherwise the button stays on "Evaluating…" for the new record until
    // the previous record's evaluation returns.
    setIsEvaluating(false);
    setError(null);
    setEvaluation(null);
    // Findings live in the global workspace store. Clear the previous record immediately so a
    // failed load cannot render record A's legal findings under record B's heading.
    setFindings([]);
    setPendingReview(null);
    setReviewerNotes('');
    setSelectedFindingId(null);
    void invokeCommand('list_request_legal_findings', {
      requestId: record.requestId,
      portalKey: record.portalKey,
    })
      .then((value) => {
        if (findingsGate.isCurrent(request)) setFindings(value);
      })
      .catch((reason) => {
        if (findingsGate.isCurrent(request)) setError(String(reason));
      })
      .finally(() => {
        if (findingsGate.isCurrent(request)) setIsLoading(false);
      });
    return findingsGate.invalidate;
  }, [findingsGate, record.portalKey, record.requestId, setFindings]);

  const activeFindings = useMemo(
    () => findings.filter((finding) => ['OPEN', 'CONFIRMED'].includes(finding.reviewStatus)),
    [findings],
  );
  const selectedFinding = activeFindings.find((finding) => finding.violationId === selectedFindingId) ?? null;
  const selectedAuthority = selectedFinding ? authorities.find((authority) => authority.statuteId === selectedFinding.matchedStatuteId) ?? null : null;

  const evaluate = async () => {
    const request = findingsGate.begin();
    setIsLoading(false);
    setIsReviewing(false);
    setIsEvaluating(true);
    setError(null);
    try {
      const result = await invokeCommand('evaluate_request_legal_compliance', {
        requestId: record.requestId,
        portalKey: record.portalKey,
      });
      if (!findingsGate.isCurrent(request)) return;
      setEvaluation(result);
      setFindings(result.findings);
    } catch (reason) {
      if (!findingsGate.isCurrent(request)) return;
      setError(String(reason));
    } finally {
      if (findingsGate.isCurrent(request)) setIsEvaluating(false);
    }
  };

  const filterByFinding = (finding: LegalComplianceFinding) => {
    setSearchFilters(searchTerm, selectedAgencyId, selectedStatus, finding.classification);
  };

  const beginReview = (finding: LegalComplianceFinding, status: LegalFindingReviewStatus) => {
    setPendingReview({ violationId: finding.violationId, status });
    setReviewerNotes('');
  };

  const submitReview = async () => {
    if (!pendingReview) return;
    const request = findingsGate.begin();
    setIsLoading(false);
    setIsEvaluating(false);
    setIsReviewing(true);
    setError(null);
    try {
      const result = await invokeCommand('review_legal_finding', {
        violationId: pendingReview.violationId,
        requestId: record.requestId,
        portalKey: record.portalKey,
        reviewStatus: pendingReview.status,
        reviewerNotes: reviewerNotes.trim() || null,
      });
      if (!findingsGate.isCurrent(request)) return;
      setFindings(result);
      setPendingReview(null);
      setReviewerNotes('');
    } catch (reason) {
      if (!findingsGate.isCurrent(request)) return;
      setError(String(reason));
    } finally {
      if (findingsGate.isCurrent(request)) setIsReviewing(false);
    }
  };

  return (
    <section className="mt-3 overflow-hidden rounded-lg border border-slate-800 bg-slate-950/55">
      <div className="flex items-start justify-between gap-3 border-b border-slate-800 px-3 py-2.5">
        <div>
          <div className="text-[8px] font-bold uppercase tracking-[0.16em] text-slate-400">
            Legal compliance
          </div>
          <p className="mt-1 text-[9px] leading-relaxed text-slate-400">
            Deterministic deadline checks and versioned policy evidence rules
          </p>
        </div>
        <div className="flex shrink-0 gap-1.5">
          <button
            type="button"
            onClick={onOpenLibrary}
            className="rounded-md border border-slate-700 px-2 py-1 text-[8px] font-semibold text-slate-400 transition hover:border-slate-500 hover:text-slate-200"
          >
            Authority library
          </button>
          <button
            type="button"
            onClick={() => void evaluate()}
            disabled={isEvaluating}
            className="rounded-md border border-indigo-400/25 bg-indigo-500/10 px-2 py-1 text-[8px] font-semibold text-indigo-200 transition hover:bg-indigo-500/15 disabled:cursor-wait disabled:opacity-60"
          >
            {isEvaluating ? 'Evaluating…' : 'Evaluate case'}
          </button>
        </div>
      </div>

      {isLoading ? (
        <div className="px-3 py-4 text-[9px] text-slate-400">Loading legal findings…</div>
      ) : error && activeFindings.length === 0 ? (
        <div className="px-3 py-4">
          <div className="text-[10px] font-medium text-rose-200">Legal findings unavailable</div>
          <p className="mt-1 text-[9px] leading-relaxed text-slate-400">
            The findings query did not complete, so this panel cannot report whether active findings exist.
          </p>
        </div>
      ) : activeFindings.length === 0 ? (
        <div className="px-3 py-4">
          <div className="text-[10px] font-medium text-slate-400">No active findings recorded</div>
          <p className="mt-1 text-[9px] leading-relaxed text-slate-400">
            Run an evaluation after public documents are indexed. Incomplete evidence sets are
            deferred rather than classified.
          </p>
        </div>
      ) : (
        <div className="divide-y divide-slate-800/80">
          {activeFindings.map((finding) => (
            <article
              key={finding.violationId}
              className="px-3 py-3"
            >
              <div className="flex items-center justify-between gap-3">
                <span className="truncate text-[9px] font-semibold text-amber-200">
                  {humanize(finding.classification)}
                </span>
                <span
                  className={cn(
                    'rounded-full border px-1.5 py-0.5 text-[7px] font-bold tracking-wider',
                    finding.reviewStatus === 'CONFIRMED'
                      ? 'border-rose-400/30 bg-rose-500/10 text-rose-200'
                      : 'border-amber-400/25 bg-amber-500/10 text-amber-200',
                  )}
                >
                  {finding.reviewStatus}
                </span>
              </div>
              <div className="mt-1 text-[9px] text-slate-400">{finding.statuteCitation}</div>
              <p className="mt-2 text-[9px] leading-relaxed text-slate-400">
                {finding.evidenceSummary}
              </p>
              <div className="mt-2 flex flex-wrap items-center gap-x-3 gap-y-1 font-mono text-[7px] text-slate-400">
                <span>{(finding.confidenceBasisPoints / 100).toFixed(2)}% rule confidence</span>
                {finding.varianceDays !== null && <span>{finding.varianceDays} days overdue</span>}
                {finding.supportingDocumentIds.length > 0 && (
                  <span>{finding.supportingDocumentIds.length} source documents</span>
                )}
              </div>
              {finding.missingEvidencePhrases.length > 0 && (
                <div className="mt-2 flex flex-wrap gap-1">
                  {finding.missingEvidencePhrases.slice(0, 4).map((phrase) => (
                    <span
                      key={phrase}
                      className="rounded border border-slate-800 bg-slate-900 px-1.5 py-0.5 text-[7px] text-slate-400"
                    >
                      Missing: {phrase}
                    </span>
                  ))}
                </div>
              )}
              <div className="mt-3 flex items-center gap-1.5">
                <button type="button" onClick={() => setSelectedFindingId(finding.violationId)} className="rounded border border-indigo-400/25 bg-indigo-500/[0.06] px-2 py-1 text-[7px] font-semibold text-indigo-200 transition hover:bg-indigo-500/10">
                  Compare authority & evidence
                </button>
                <button type="button" onClick={() => filterByFinding(finding)} className="rounded border border-slate-700 px-2 py-1 text-[7px] font-semibold text-slate-400 transition hover:border-slate-500 hover:text-slate-300">
                  Filter workbench
                </button>
                {finding.reviewStatus === 'OPEN' ? (
                  <>
                    <button type="button" onClick={() => beginReview(finding, 'CONFIRMED')} className="rounded border border-emerald-500/25 bg-emerald-500/[0.06] px-2 py-1 text-[7px] font-semibold text-emerald-300 transition hover:bg-emerald-500/10">
                      Confirm finding
                    </button>
                    <button type="button" onClick={() => beginReview(finding, 'DISMISSED')} className="rounded border border-slate-700 px-2 py-1 text-[7px] font-semibold text-slate-400 transition hover:border-rose-400/30 hover:text-rose-300">
                      Dismiss
                    </button>
                  </>
                ) : (
                  <button type="button" onClick={() => beginReview(finding, 'OPEN')} className="rounded border border-slate-700 px-2 py-1 text-[7px] font-semibold text-slate-400 transition hover:border-slate-500 hover:text-slate-300">
                    Reopen review
                  </button>
                )}
              </div>
              {pendingReview?.violationId === finding.violationId && (
                <div className="mt-2 rounded-md border border-slate-800 bg-slate-900/60 p-2">
                  <label className="block text-[7px] font-semibold uppercase tracking-wider text-slate-400">
                    Reviewer note
                    <textarea value={reviewerNotes} onChange={(event) => setReviewerNotes(event.target.value)} maxLength={8192} rows={2} className="mt-1.5 w-full resize-y rounded border border-slate-800 bg-slate-950 px-2 py-1.5 text-[8px] normal-case tracking-normal text-slate-300 outline-none focus:border-indigo-400/35" />
                  </label>
                  <div className="mt-2 flex justify-end gap-1.5">
                    <button type="button" onClick={() => setPendingReview(null)} disabled={isReviewing} className="rounded px-2 py-1 text-[7px] font-semibold text-slate-400 hover:text-slate-300">Cancel</button>
                    <button type="button" onClick={() => void submitReview()} disabled={isReviewing} className="rounded border border-indigo-400/25 bg-indigo-500/10 px-2 py-1 text-[7px] font-semibold text-indigo-200 disabled:cursor-wait disabled:opacity-60">
                      {isReviewing ? 'Saving…' : `Save as ${humanize(pendingReview.status)}`}
                    </button>
                  </div>
                </div>
              )}
            </article>
          ))}
        </div>
      )}

      {selectedFinding && <div className="grid gap-px border-t border-slate-800 bg-slate-800 md:grid-cols-2"><section className="bg-slate-950 p-3" aria-label="Authority citation"><div className="text-[8px] font-bold uppercase tracking-wider text-indigo-300/70">Authority citation</div><div className="mt-2 text-[10px] font-semibold text-slate-200">{selectedFinding.statuteCitation}</div><p className="mt-1 text-[9px] text-slate-400">{selectedFinding.authorityTitle}</p><dl className="mt-2 space-y-1 font-mono text-[7px] text-slate-400"><div><dt className="inline">Version </dt><dd className="inline text-slate-400">{selectedFinding.matchedVersionId}</dd></div>{selectedFinding.mandateTitle && <div><dt className="inline">Rule </dt><dd className="inline text-slate-400">{selectedFinding.mandateTitle}</dd></div>}</dl>{selectedAuthority?.canonicalSourceUri && <a href={selectedAuthority.canonicalSourceUri} target="_blank" rel="noreferrer" className="mt-2 block break-all text-[8px] text-cyan-300 underline">Open official source</a>}</section><section className="bg-slate-950 p-3" aria-label="Finding evidence excerpt"><div className="flex items-center justify-between"><div className="text-[8px] font-bold uppercase tracking-wider text-amber-300/70">Case evidence</div><button type="button" onClick={() => setSelectedFindingId(null)} className="text-[8px] text-slate-400 hover:text-slate-300">Close</button></div><p className="mt-2 whitespace-pre-wrap text-[9px] leading-relaxed text-slate-300">{selectedFinding.evidenceSummary}</p><div className="mt-2 font-mono text-[7px] text-slate-400">Supporting documents: {selectedFinding.supportingDocumentIds.length ? selectedFinding.supportingDocumentIds.join(', ') : 'none recorded'}</div>{selectedFinding.missingEvidencePhrases.length > 0 && <div className="mt-2 text-[8px] text-amber-200/70">Missing required evidence: {selectedFinding.missingEvidencePhrases.join('; ')}</div>}</section></div>}

      {evaluation && (
        <div
          className={cn(
            'border-t px-3 py-2 text-[8px] leading-relaxed',
            evaluation.evaluationComplete
              ? 'border-emerald-500/20 bg-emerald-500/[0.05] text-emerald-300/80'
              : 'border-amber-500/20 bg-amber-500/[0.05] text-amber-200/80',
          )}
        >
          {evaluation.evaluationComplete
            ? `${evaluation.evaluatedMandateCount} rules evaluated across ${evaluation.corpusCharacterCount.toLocaleString()} searchable characters; ${evaluation.applicableMandateCount} rules applied.`
            : evaluation.warnings.join(' ')}
        </div>
      )}
      {error && (
        <div role="alert" className="border-t border-rose-500/20 bg-rose-500/[0.05] px-3 py-2 text-[8px] leading-relaxed text-rose-300">
          {error}
        </div>
      )}
    </section>
  );
}

function humanize(value: string) {
  return value.split('_').filter(Boolean).map((word) =>
    `${word.charAt(0)}${word.slice(1).toLocaleLowerCase()}`,
  ).join(' ');
}
