import { useCallback, useEffect, useMemo, useState } from 'react';
import { invokeCommand } from '../../ipc';
import { useAppStore } from '../../store/useAppStore';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import type { AgencyPerformanceProfile } from '../../types';

/**
 * A compact strip above the findings inbox. Two numbers only: how many findings arrived since the
 * user last looked, and how many statutory deadline violations the evaluator has counted,
 * scoped to the active portal. Clicking either scrolls the inbox to a relevant filter without
 * changing the workspace view.
 *
 * The numbers are honest counts of what has already been detected. The violation figure
 * reads from the cached `agencyProfiles`; if that cache is empty (analytics not refreshed yet)
 * the strip reports the finding count only, rather than fabricating a zero for the second bucket.
 */
export default function ChangeFeed() {
  const findingsRefreshRequests = useAppStore((state) => state.findingsRefreshRequests);
  const agencyProfiles = useAppStore((state) => state.agencyProfiles);
  const activePortalKey = useAppStore((state) => state.activePortalKey);
  const setSearchFilters = useAppStore((state) => state.setSearchFilters);
  const setActivePortal = useAppStore((state) => state.setActivePortal);
  // Both numbers come from one load so they describe the same moment. `null` until the first
  // count resolves: the strip has nothing honest to say before then.
  const [counts, setCounts] = useState<{ total: number; new: number } | null>(null);
  const gate = useLatestRequestGate();

  const load = useCallback(() => {
    const request = gate.begin();
    // The watermark as it stood when this load began. FindingsInbox bumps `lastFindingsSeenAt`
    // after its own load of the same refresh settles, so reading the live value would zero the
    // count as soon as the inbox finished; the honest cutoff is the previous time the inbox loaded.
    const seenAtStart = useAppStore.getState().lastFindingsSeenAt;
    const cutoff = seenAtStart !== null && Number.isFinite(Date.parse(seenAtStart)) ? seenAtStart : null;
    Promise.all([
      invokeCommand('count_findings', { filter: null }),
      cutoff === null ? null : invokeCommand('count_findings', { filter: { discoveredAfter: cutoff } }),
    ])
      .then(([total, since]) => {
        if (!gate.isCurrent(request)) return;
        setCounts({ total, new: since ?? total });
      })
      .catch(() => {
        // A ChangeFeed that can't count is a ChangeFeed that shouldn't invent a number. Leave the
        // previous value in place until the next successful load; FindingsInbox surfaces the
        // error itself.
      });
  }, [gate]);

  useEffect(() => {
    load();
    return gate.invalidate;
  }, [findingsRefreshRequests, gate, load]);

  const newFindingsCount = counts?.new ?? 0;

  const statutoryCount = useMemo(() => countStatutoryViolations(agencyProfiles, activePortalKey), [
    agencyProfiles,
    activePortalKey,
  ]);

  const focusInboxNew = () => {
    setActivePortal(null);
    setSearchFilters('', null, null, null);
    const anchor = document.getElementById('findings-inbox');
    if (anchor) anchor.scrollIntoView({ behavior: 'smooth', block: 'start' });
  };
  const focusInboxStatutory = () => {
    if (activePortalKey) setActivePortal(activePortalKey);
    setSearchFilters('', null, null, 'STATUTORY_LATENCY_VIOLATION');
    const anchor = document.getElementById('findings-inbox');
    if (anchor) anchor.scrollIntoView({ behavior: 'smooth', block: 'start' });
  };

  if ((counts === null || counts.total === 0) && statutoryCount === null) return null;

  return (
    <section
      aria-label="What changed since you last looked"
      className="flex flex-wrap items-center gap-2 rounded-xl border border-slate-800/70 bg-slate-950/30 px-3 py-2 text-label text-slate-400"
    >
      <button
        type="button"
        onClick={focusInboxNew}
        className="focus-ring inline-flex items-center gap-1.5 rounded-md border border-transparent px-2 py-1 font-semibold text-slate-200 transition hover:border-cyan-400/25 hover:text-cyan-200"
      >
        <span
          aria-hidden="true"
          className="inline-block h-1.5 w-1.5 rounded-full bg-cyan-300"
        />
        {newFindingsCount === 0
          ? 'No new findings since you last opened.'
          : `${newFindingsCount} new ${newFindingsCount === 1 ? 'finding' : 'findings'} since you last opened.`}
      </button>
      {statutoryCount !== null && statutoryCount > 0 && (
        <button
          type="button"
          onClick={focusInboxStatutory}
          className="focus-ring inline-flex items-center gap-1.5 rounded-md border border-transparent px-2 py-1 font-semibold text-amber-300 transition hover:border-amber-400/25 hover:text-amber-200"
        >
          <span
            aria-hidden="true"
            className="inline-block h-1.5 w-1.5 rounded-full bg-amber-300"
          />
          {statutoryCount} statutory deadline {statutoryCount === 1 ? 'violation' : 'violations'} counted.
        </button>
      )}
    </section>
  );
}

// `null` means analytics hasn't produced profiles yet, so ChangeFeed can tell "no counted violations"
// (0) from "not known yet" (null). The figure is the evaluator's counted statutory violations
// (`statutoryViolationCount`), which are deadlines already exceeded. No profile field says a
// deadline is about to lapse, so this strip does not claim one.
function countStatutoryViolations(profiles: AgencyPerformanceProfile[], portalKey: string | null): number | null {
  if (profiles.length === 0) return null;
  const scoped = portalKey
    ? profiles.filter((profile) => profile.portalSubdomain === portalKey)
    : profiles;
  if (scoped.length === 0) return null;
  return scoped.reduce((total, profile) => total + profile.statutoryViolationCount, 0);
}
