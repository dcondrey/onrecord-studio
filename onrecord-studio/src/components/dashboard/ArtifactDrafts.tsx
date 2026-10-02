import { useCallback, useEffect, useState } from 'react';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import type { DraftAvailability, DraftedArtifact } from '../../types';
import DraftedArtifactView from './DraftedArtifactView';

type DraftKind = 'followup' | 'appeal';

const BUTTON =
  'focus-ring rounded-md border border-line-strong bg-surface/70 px-3 py-1 text-label font-semibold text-fg transition disabled:opacity-60';

/**
 * Offers the drafts that apply to one finding and shows the result. Which buttons appear comes
 * from the backend (`draft_availability_for_finding`), so a pattern finding is never offered an
 * appeal and the frontend holds no copy of that rule.
 *
 * Every specific claim in a draft traces back to the finding; the generator never fabricates
 * facts. The draft is always a draft: the user edits and sends it, the app sends nothing.
 */
export default function ArtifactDrafts({ findingId }: { findingId: string }) {
  const [availability, setAvailability] = useState<DraftAvailability | null>(null);
  const [draft, setDraft] = useState<DraftedArtifact | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<DraftKind | null>(null);
  const availabilityGate = useLatestRequestGate();
  const draftGate = useLatestRequestGate();

  // A result requested for another finding must not describe this one: reset and invalidate on
  // every identity change before fetching.
  useEffect(() => {
    draftGate.invalidate();
    setDraft(null);
    setError(null);
    setBusy(null);
    setAvailability(null);
    const request = availabilityGate.begin();
    invokeCommand('draft_availability_for_finding', { findingId })
      .then((result) => {
        if (availabilityGate.isCurrent(request)) setAvailability(result);
      })
      .catch((reason) => {
        if (availabilityGate.isCurrent(request)) setError(formatNativeError(reason));
      });
  }, [findingId, availabilityGate, draftGate]);

  const runDraft = useCallback(
    (kind: DraftKind) => {
      const request = draftGate.begin();
      setBusy(kind);
      setError(null);
      const command =
        kind === 'followup' ? 'draft_followup_from_finding' : 'draft_appeal_from_finding';
      invokeCommand(command, { findingId })
        .then((result) => {
          if (draftGate.isCurrent(request)) setDraft(result);
        })
        .catch((reason) => {
          if (draftGate.isCurrent(request)) setError(formatNativeError(reason));
        })
        .finally(() => {
          if (draftGate.isCurrent(request)) setBusy(null);
        });
    },
    [findingId, draftGate],
  );

  return (
    <div
      role="region"
      className="mt-2 rounded-lg border border-line bg-canvas/40 p-3"
      aria-label="Drafted artifacts"
    >
      <div className="flex flex-wrap items-center gap-2">
        {availability?.followup && (
          <button
            type="button"
            onClick={() => runDraft('followup')}
            disabled={busy === 'followup'}
            className={`${BUTTON} hover:border-accent/30`}
          >
            {busy === 'followup' ? 'Drafting…' : 'Draft records request'}
          </button>
        )}
        {availability?.appeal && (
          <button
            type="button"
            onClick={() => runDraft('appeal')}
            disabled={busy === 'appeal'}
            className={`${BUTTON} hover:border-warning/30`}
          >
            {busy === 'appeal' ? 'Drafting…' : 'Draft appeal or complaint'}
          </button>
        )}
        {availability && !availability.followup && !availability.appeal && (
          <span className="text-caption text-muted">
            This finding names no specific gap to ask about, so no draft applies.
          </span>
        )}
      </div>

      {error && (
        <div role="alert" className="mt-2 rounded-md border border-danger/30 bg-danger/5 px-2 py-1.5 text-caption text-danger">
          {error}
        </div>
      )}

      {draft && <DraftedArtifactView draft={draft} />}
    </div>
  );
}
