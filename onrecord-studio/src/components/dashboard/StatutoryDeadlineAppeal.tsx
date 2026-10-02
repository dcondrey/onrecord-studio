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
      className="rounded-lg border border-line bg-canvas/40 p-3"
      aria-label="Statutory deadline appeal"
    >
      <button
        type="button"
        onClick={run}
        disabled={busy}
        className="focus-ring rounded-md border border-control bg-surface/70 px-3 py-1 text-label font-semibold text-fg transition hover:border-warning/30 disabled:opacity-60"
      >
        {busy ? 'Drafting…' : 'Draft statutory deadline appeal'}
      </button>
      {error && (
        <div role="alert" className="mt-2 rounded-md border border-danger/30 bg-danger/5 px-2 py-1.5 text-caption text-danger">
          {error}
        </div>
      )}
      {draft && <DraftedArtifactView draft={draft} />}
    </div>
  );
}
