import { useCallback, useEffect, useMemo, useState } from 'react';
import { invokeCommand } from '../../ipc';
import { useAppStore } from '../../store/useAppStore';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import type { AgencyPerformanceProfile, Finding } from '../../types';

/**
 * A compact strip above the findings inbox. Two numbers only: how many findings arrived since the
 * user last looked, and how many statutory deadlines are due to lapse in the next seven days
 * scoped to the active portal. Clicking either scrolls the inbox to a relevant filter without
 * changing the workspace view.
 *
 * The numbers are honest counts of what has already been detected. The lapsing-deadlines figure
 * reads from the cached `agencyProfiles`; if that cache is empty (analytics not refreshed yet)
 * the strip reports the finding count only, rather than fabricating a zero for the second bucket.
 */
export default function ChangeFeed() {
  const lastFindingsSeenAt = useAppStore((state) => state.lastFindingsSeenAt);
  const findingsRefreshRequests = useAppStore((state) => state.findingsRefreshRequests);
  const agencyProfiles = useAppStore((state) => state.agencyProfiles);
  const activePortalKey = useAppStore((state) => state.activePortalKey);
  const setSearchFilters = useAppStore((state) => state.setSearchFilters);
  const setActivePortal = useAppStore((state) => state.setActivePortal);
  const [findings, setFindings] = useState<Finding[]>([]);
  const gate = useLatestRequestGate();

  const load = useCallback(() => {
    const request = gate.begin();
    invokeCommand('list_findings', { filter: null, limit: 200 })
      .then((rows) => {
        if (!gate.isCurrent(request)) return;
        setFindings(rows);
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

  const newFindingsCount = useMemo(() => {
    if (findings.length === 0) return 0;
    if (!lastFindingsSeenAt) return findings.length;
    const cutoff = Date.parse(lastFindingsSeenAt);
    if (!Number.isFinite(cutoff)) return findings.length;
    return findings.filter((finding) => {
      const discovered = Date.parse(finding.discoveredAt);
      return Number.isFinite(discovered) && discovered > cutoff;
    }).length;
  }, [findings, lastFindingsSeenAt]);

  const lapsingSoon = useMemo(() => countLapsingDeadlines(agencyProfiles, activePortalKey), [
    agencyProfiles,
    activePortalKey,
  ]);

  const focusInboxNew = () => {
    setActivePortal(null);
    setSearchFilters('', null, null, null);
    const anchor = document.getElementById('findings-inbox');
    if (anchor) anchor.scrollIntoView({ behavior: 'smooth', block: 'start' });
  };
  const focusInboxLapsing = () => {
    if (activePortalKey) setActivePortal(activePortalKey);
    setSearchFilters('', null, null, 'STATUTORY_DEADLINE_EXCEEDED');
    const anchor = document.getElementById('findings-inbox');
    if (anchor) anchor.scrollIntoView({ behavior: 'smooth', block: 'start' });
  };

  if (findings.length === 0 && lapsingSoon === null) return null;

  return (
    <section
      aria-label="What changed since you last looked"
      className="flex flex-wrap items-center gap-2 rounded-xl border border-slate-800/70 bg-slate-950/30 px-3 py-2 text-[10px] text-slate-400"
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
      {lapsingSoon !== null && lapsingSoon > 0 && (
        <button
          type="button"
          onClick={focusInboxLapsing}
          className="focus-ring inline-flex items-center gap-1.5 rounded-md border border-transparent px-2 py-1 font-semibold text-amber-300 transition hover:border-amber-400/25 hover:text-amber-200"
        >
          <span
            aria-hidden="true"
            className="inline-block h-1.5 w-1.5 rounded-full bg-amber-300"
          />
          {lapsingSoon} deadline{lapsingSoon === 1 ? '' : 's'} lapsing within 7 days.
        </button>
      )}
    </section>
  );
}

// The profile carries `statutoryDeadlineWithinDays` on the shape it exposes to the frontend, and
// `null` means analytics hasn't finished computing it. We return `null` from this helper too when
// the cache is empty so ChangeFeed can distinguish "no lapsing deadlines" (0) from "we don't
// know yet" (null) — an application of the "absence and measurement never share a
// representation" invariant to the UI layer.
function countLapsingDeadlines(profiles: AgencyPerformanceProfile[], portalKey: string | null): number | null {
  if (profiles.length === 0) return null;
  const scoped = portalKey
    ? profiles.filter((profile) => profile.portalSubdomain === portalKey)
    : profiles;
  if (scoped.length === 0) return null;
  return scoped.reduce((total, profile) => {
    const value = (profile as unknown as { deadlinesLapsingWithinSevenDays?: number | null })
      .deadlinesLapsingWithinSevenDays;
    if (typeof value === 'number' && value > 0) return total + value;
    return total;
  }, 0);
}
