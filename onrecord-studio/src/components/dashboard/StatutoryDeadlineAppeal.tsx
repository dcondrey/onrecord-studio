import { useCallback, useEffect, useState } from 'react';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import type { DraftedArtifact } from '../../types';
import DraftedArtifactView from './DraftedArtifactView';

/**
 * Drafts an appeal for one request whose statutory deadline the evaluator counts as exceeded.
 * The backend refuses when the determination date is a proxy or the extension record is
 * defective, and the refusal is shown as is. Mount it beside a request's compliance summary.
 */
export default function StatutoryDeadlineAppeal({
  portalKey,
  requestId,
}: {
  portalKey: string;
  requestId: number;
}) {
  const [draft, setDraft] = useState<DraftedArtifact | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const gate = useLatestRequestGate();

  useEffect(() => {
    gate.invalidate();
    setDraft(null);
    setError(null);
    setBusy(false);
  }, [portalKey, requestId, gate]);

  const run = useCallback(() => {
    const request = gate.begin();
    setBusy(true);
    setError(null);
    invokeCommand('draft_appeal_from_statutory_deadline', { portalKey, requestId })
      .then((result) => {
        if (gate.isCurrent(request)) setDraft(result);
      })
      .catch((reason) => {
        if (gate.isCurrent(request)) setError(formatNativeError(reason));
      })
      .finally(() => {
        if (gate.isCurrent(request)) setBusy(false);
      });
  }, [portalKey, requestId, gate]);

  return (
    <div
      className="rounded-lg border border-slate-800/70 bg-slate-950/40 p-3"
      aria-label="Statutory deadline appeal"
    >
      <button
        type="button"
        onClick={run}
        disabled={busy}
        className="focus-ring rounded-md border border-slate-700 bg-slate-900/60 px-3 py-1 text-[10px] font-semibold text-slate-200 transition hover:border-amber-400/40 disabled:opacity-60"
      >
        {busy ? 'Drafting…' : 'Draft statutory deadline appeal'}
      </button>
      {error && (
        <div role="alert" className="mt-2 rounded-md border border-rose-500/20 bg-rose-500/[0.04] px-2 py-1.5 text-[9px] text-rose-200">
          {error}
        </div>
      )}
      {draft && <DraftedArtifactView draft={draft} />}
    </div>
  );
}
