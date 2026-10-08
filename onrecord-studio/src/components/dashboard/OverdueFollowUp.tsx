import { useCallback, useEffect, useState } from 'react';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import type { AgencyIdentity, OverdueFollowUpDraft } from '../../types';
import DraftedArtifactView from './DraftedArtifactView';

const key = (agency: AgencyIdentity) => `${agency.portalKey}\u001f${agency.agencyId}`;

/**
 * Follow-up letter for one saved casework request whose computed determination deadline has
 * passed with no recorded response. The backend decides whether a letter exists: before the
 * deadline has passed, or when a date is not recorded, it refuses and the refusal is shown as is.
 * Every result is gated on the case and request it was asked for. Nothing here sends anything.
 */
export default function OverdueFollowUp({
  caseId,
  requestId,
  dueOn,
}: {
  caseId: string;
  requestId: string;
  dueOn: string;
}) {
  const [draft, setDraft] = useState<OverdueFollowUpDraft | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const gate = useLatestRequestGate();

  useEffect(() => {
    gate.invalidate();
    setDraft(null);
    setError(null);
    setBusy(false);
  }, [caseId, requestId, gate]);

  const run = useCallback((agency: AgencyIdentity | null) => {
    const request = gate.begin();
    setBusy(true);
    setError(null);
    invokeCommand('draft_overdue_follow_up', {
      caseId,
      requestId,
      agency: agency && { portalKey: agency.portalKey, agencyId: agency.agencyId },
    })
      .then((result) => {
        if (gate.isCurrent(request)) setDraft(result);
      })
      .catch((reason) => {
        if (!gate.isCurrent(request)) return;
        setDraft(null);
        setError(formatNativeError(reason));
      })
      .finally(() => {
        if (gate.isCurrent(request)) setBusy(false);
      });
  }, [caseId, requestId, gate]);

  const record = draft?.agencyRecord ?? null;
  return (
    <div className="rounded-lg border border-line bg-canvas/40 p-3" aria-label="Overdue follow-up letter">
      <p className="text-body text-muted">
        A follow-up letter exists only after {dueOn} has passed with no recorded response in the last saved revision. It is text for you to review and send yourself; this application sends nothing.
      </p>
      <button type="button" onClick={() => run(null)} disabled={busy} className="analysis-button mt-2">
        {busy ? 'Drafting…' : 'Draft overdue follow-up letter'}
      </button>
      {error && (
        <div role="alert" className="mt-2 rounded-md border border-danger/30 bg-danger/5 px-2 py-1.5 text-caption text-danger">
          {error}
        </div>
      )}
      {draft && (
        <>
          <label className="mt-2 block text-body text-fg-secondary">
            Agency record from your library (optional)
            <select
              className="mt-1 block w-full rounded border border-control bg-surface px-2 py-1 text-label text-fg"
              value={record ? key(record) : ''}
              disabled={busy}
              onChange={(event) => run(draft.agencyOptions.find((option) => key(option) === event.target.value) ?? null)}
            >
              <option value="">None: leave the agency&apos;s record out</option>
              {draft.agencyOptions.map((option) => (
                <option key={key(option)} value={key(option)}>
                  {option.agencyName ?? `Agency ${option.agencyId}`} on {option.portalKey} ({option.closedRequests} closed requests)
                </option>
              ))}
            </select>
          </label>
          {draft.agencyOptions.length === 0 && (
            <p className="mt-1 text-body text-muted">No agency in your library has enough closed requests for a counted paragraph.</p>
          )}
          {record && (
            <p className="mt-1 text-body text-muted">
              {draft.agencyParagraphIncluded
                ? `The letter includes one paragraph counted from ${record.closedRequests} closed requests in your library. Delete it from the text below if you do not want it.`
                : `Only ${record.closedRequests} closed requests are held for this agency, too few to count; no paragraph was added.`}
            </p>
          )}
          <DraftedArtifactView draft={draft.artifact} />
        </>
      )}
    </div>
  );
}
