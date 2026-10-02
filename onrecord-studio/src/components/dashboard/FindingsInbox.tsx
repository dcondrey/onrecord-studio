import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { isTauri } from '@tauri-apps/api/core';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import { ShapeGlyph, type GlyphShape } from '../ui/shapeGlyph';
import { useAppStore } from '../../store/useAppStore';
import { useInspectFinding } from '../../hooks/useInspectFinding';
import type { Finding, FindingCitation, FindingSeverity } from '../../types';
import { FindingEvidence } from '../ui/EvidenceBasis';
import ArtifactDrafts from './ArtifactDrafts';
import EvidenceGaps from './EvidenceGaps';
import LeadDossier from './LeadDossier';
import EvidencePacketExport from './EvidencePacketExport';

import ShareableBundleExport from './ShareableBundleExport';
import VerifiableReportExport from './VerifiableReportExport';
import { useProfileLensStore } from '../../store/useProfileLensStore';
import { touchesOwnOrganization, useAffiliationDisclosure } from '../account/useProfile';

/** A failed user action. Kept apart from a failed load: the list is still valid when one of these is set. */
const PAGE_SIZE = 50;
const SEVERITIES: FindingSeverity[] = ['CRITICAL', 'SERIOUS', 'NOTABLE', 'INFO'];
type FindingSort = 'ranked' | 'newest';

interface ActionError {
  action: 'refresh' | 'dismiss';
  message: string;
}

/**
 * The autonomous findings inbox: the first thing on the overview screen.
 *
 * Renders one ranked list of everything the detectors have surfaced across the corpus, sorted by
 * severity, then confidence, then freshness. Clicking a row navigates into the app's existing
 * evidence surface with the finding's subject filters pre-applied. No drawers, no toggles: this
 * is what a user sees on first launch, and every finding here is one someone should look at
 * without being asked to.
 */
export default function FindingsInbox() {
  const [findings, setFindings] = useState<Finding[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<ActionError | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [isRefreshing, setIsRefreshing] = useState(false);
  const [limit, setLimit] = useState(PAGE_SIZE);
  const [hasMore, setHasMore] = useState(false);
  const [severityFilter, setSeverityFilter] = useState<FindingSeverity | null>(null);
  const [sort, setSort] = useState<FindingSort>('ranked');
  const [textFilter, setTextFilter] = useState('');
  // Which finding's row is expanded to show ArtifactDrafts. At most one at a time — the drafted
  // text can be long, and stacking several open drafts hides the ranked list.
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const listGate = useLatestRequestGate();
  // Ids whose `dismiss_finding` has not settled. A list load that resolves while one is in flight
  // still carries the row (the backend has not committed the dismissal yet), and must not re-show
  // a finding the user has already removed.
  const pendingDismissals = useRef(new Set<string>());
  // Ids whose dismissal committed while a list load was in flight. That load was answered from
  // the pre-dismissal state, so its rows are filtered too; a load begun after the commit clears
  // the id, since the backend answered it after the row was gone.
  const settledDismissals = useRef(new Set<string>());
  const lensEnabled = useProfileLensStore((state) => state.lensEnabled);
  const setLensEnabled = useProfileLensStore((state) => state.setLensEnabled);
  const setSelfReviewOpen = useProfileLensStore((state) => state.setSelfReviewOpen);
  const { disclosure, status: disclosureStatus } = useAffiliationDisclosure();
  const setWorkspaceView = useAppStore((state) => state.setWorkspaceView);
  const openLeadDossier = useAppStore((state) => state.openLeadDossier);
  const setActivePortal = useAppStore((state) => state.setActivePortal);
  const findingsRefreshRequests = useAppStore((state) => state.findingsRefreshRequests);
  const requestFindingsRefresh = useAppStore((state) => state.requestFindingsRefresh);
  const markFindingsSeen = useAppStore((state) => state.markFindingsSeen);

  const loadFindings = useCallback(() => {
    const request = listGate.begin();
    const settledBeforeLoad = [...settledDismissals.current];
    setIsLoading(true);
    setLoadError(null);
    // One extra row distinguishes "exactly `limit` findings" from "more exist beyond the cap".
    invokeCommand('list_findings', { filter: null, limit: limit + 1 })
      .then((fetched) => {
        if (!listGate.isCurrent(request)) return;
        setHasMore(fetched.length > limit);
        const rows = fetched.slice(0, limit);
        setFindings(
          rows.filter(
            (row) => !pendingDismissals.current.has(row.id) && !settledDismissals.current.has(row.id),
          ),
        );
        for (const id of settledBeforeLoad) settledDismissals.current.delete(id);
        // The user is looking at the inbox. Bump the watermark so ChangeFeed knows the difference
        // between findings that arrived while they weren't looking and findings they've already
        // been shown. Only on a successful load — a failed fetch is not "seen."
        markFindingsSeen();
      })
      .catch((reason) => {
        if (!listGate.isCurrent(request)) return;
        setLoadError(formatNativeError(reason));
      })
      .finally(() => {
        if (listGate.isCurrent(request)) setIsLoading(false);
      });
  }, [limit, listGate, markFindingsSeen]);

  // The store bumps `findingsRefreshRequests` after each ingest (via `refreshFindingsInbox`) and
  // whenever the user presses Refresh. Loading is keyed off that counter so we always pick up the
  // latest sweep without a manual mount-time fetch that could race the ingest hook.
  useEffect(() => {
    loadFindings();
    return listGate.invalidate;
  }, [findingsRefreshRequests, listGate, loadFindings]);

  const refresh = useCallback(async () => {
    setIsRefreshing(true);
    setActionError(null);
    try {
      await invokeCommand('refresh_findings', {});
    } catch (reason) {
      // The list reload below clears only `loadError`, so a failed sweep stays on screen next to
      // the rows the previous sweep produced.
      setActionError({ action: 'refresh', message: formatNativeError(reason) });
    } finally {
      setIsRefreshing(false);
      requestFindingsRefresh();
    }
  }, [requestFindingsRefresh]);

  const dismiss = useCallback(
    async (id: string) => {
      const index = findings.findIndex((row) => row.id === id);
      if (index === -1) return;
      const removed = findings[index];
      pendingDismissals.current.add(id);
      setActionError(null);
      setFindings((rows) => rows.filter((row) => row.id !== id));
      try {
        await invokeCommand('dismiss_finding', { id });
        settledDismissals.current.add(id);
      } catch (reason) {
        // Roll back by re-inserting this one row into whatever list is current. Restoring the
        // snapshot captured at click time would resurrect siblings dismissed since, and would
        // overwrite a list that a reload has replaced in the meantime.
        setFindings((rows) => {
          if (rows.some((row) => row.id === id)) return rows;
          const at = Math.min(index, rows.length);
          return [...rows.slice(0, at), removed, ...rows.slice(at)];
        });
        setActionError({ action: 'dismiss', message: formatNativeError(reason) });
      } finally {
        pendingDismissals.current.delete(id);
      }
    },
    [findings],
  );

  const inspect = useInspectFinding();

  // Evidence view is the deepest surface a citation can reach without a document deep link.
  const openSource = useCallback(
    (citation: FindingCitation) => {
      setActivePortal(citation.portalKey);
      setWorkspaceView('evidence');
    },
    [setActivePortal, setWorkspaceView],
  );

  const hasConfirmedSubject =
    disclosure !== null && (disclosure.ownPortalKeys.length > 0 ||
      disclosure.ownAgencies.length > 0 ||
      disclosure.ownEntityIds.length > 0);
  const lensedFindings = useMemo(
    () => (lensEnabled ? findings.filter((finding) => touchesOwnOrganization(finding, disclosure)) : findings),
    [lensEnabled, findings, disclosure],
  );
  const summary = useMemo(() => summariseSeverity(lensedFindings), [lensedFindings]);
  const visibleFindings = useMemo(() => {
    const needle = textFilter.trim().toLowerCase();
    const matching = lensedFindings.filter(
      (finding) =>
        (severityFilter === null || finding.severity === severityFilter) &&
        (needle === '' || `${finding.headline} ${finding.explanation}`.toLowerCase().includes(needle)),
    );
    return sort === 'newest'
      ? [...matching].sort((a, b) => Date.parse(b.lastSeenAt) - Date.parse(a.lastSeenAt))
      : matching;
  }, [lensedFindings, severityFilter, sort, textFilter]);
  const isNarrowed = severityFilter !== null || textFilter.trim() !== '';

  return (
    <section id="findings-inbox" className="app-panel flex flex-col gap-3 p-4" aria-label="Findings inbox">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-[14rem] flex-1">
          <div className="app-eyebrow">Findings inbox</div>
          <h3 className="mt-1 text-label font-semibold text-slate-200">
            What the detectors are surfacing
          </h3>
          {summary && (
            <p className="mt-1 text-caption text-slate-400">
              {summary} — ranked by severity, then confidence, then freshness.
            </p>
          )}
        </div>
        <div className="flex shrink-0 flex-wrap items-center gap-2">
        <button
          type="button"
          aria-pressed={lensEnabled}
          onClick={() => setLensEnabled(!lensEnabled)}
          title="Show only findings about your confirmed organization. Ranking and confidence are unchanged."
          className="secondary-action focus-ring whitespace-nowrap rounded-lg border px-3 py-1.5 text-label font-semibold text-slate-300 transition"
        >
          {lensEnabled ? 'Showing: my organization' : 'My organization'}
        </button>
        <button
          type="button"
          onClick={refresh}
          disabled={isRefreshing}
          className="secondary-action focus-ring shrink-0 whitespace-nowrap rounded-lg border px-3 py-1.5 text-label font-semibold text-slate-300 transition disabled:opacity-60"
        >
          {isRefreshing ? 'Analyzing…' : 'Refresh findings'}
        </button>
        </div>
      </div>

      {isTauri() && (
        <details className="text-label text-slate-300">
          <summary className="focus-ring cursor-pointer font-semibold">Export a verifiable report of these findings</summary>
          <div className="mt-2">
            <VerifiableReportExport />
            <div className="mt-2">
              <EvidencePacketExport />
            </div>
          </div>
        </details>
      )}

      {isTauri() && (
        <details className="text-label text-slate-300">
          <summary className="focus-ring cursor-pointer font-semibold">Share these findings as a read-only file</summary>
          <div className="mt-2">
            <ShareableBundleExport findings={lensedFindings} />
          </div>
        </details>
      )}

      {findings.length > 0 && (
        <div className="flex flex-wrap items-center gap-2 text-caption text-slate-400">
          <div role="group" aria-label="Filter by severity" className="flex gap-1">
            {SEVERITIES.map((level) => (
              <button
                key={level}
                type="button"
                aria-pressed={severityFilter === level}
                onClick={() => setSeverityFilter((current) => (current === level ? null : level))}
                className={`focus-ring rounded-md border px-2 py-1 font-semibold ${severityFilter === level ? 'border-cyan-400/50 text-cyan-200' : 'border-slate-800 text-slate-400'}`}
              >
                {severityLabel(level)}
              </button>
            ))}
          </div>
          <label className="sr-only" htmlFor="findings-text-filter">Filter findings by text</label>
          <input
            id="findings-text-filter"
            type="search"
            value={textFilter}
            onChange={(event) => setTextFilter(event.target.value)}
            placeholder="Filter findings"
            className="focus-ring min-w-[10rem] flex-1 rounded-md border border-slate-800 bg-slate-950 px-2 py-1 text-label text-slate-200"
          />
          <label htmlFor="findings-sort" className="sr-only">Sort findings</label>
          <select
            id="findings-sort"
            value={sort}
            onChange={(event) => setSort(event.target.value as FindingSort)}
            className="focus-ring rounded-md border border-slate-800 bg-slate-950 px-2 py-1 text-label text-slate-200"
          >
            <option value="ranked">Ranked</option>
            <option value="newest">Newest first</option>
          </select>
        </div>
      )}

      {lensEnabled && !hasConfirmedSubject && (
        <div role="status" className="rounded-lg border border-slate-800 px-3 py-2 text-caption text-slate-300">
          {disclosureStatus === 'loading'
            ? 'Reading the local profile.'
            : disclosureStatus === 'failed'
              ? 'The local profile could not be read, so the organization lens has nothing to filter on.'
              : disclosure?.status === 'DECLARED'
                ? 'No match to your declared organization is confirmed yet, so nothing can be filtered. This is not a statement that the record is clean.'
                : 'No local profile is set, so the organization lens has nothing to filter on.'}
        </div>
      )}
      {lensEnabled && hasConfirmedSubject && (
        <div className="flex items-center gap-2 text-caption text-slate-400">
          <span>{visibleFindings.length} of {findings.length} findings touch your confirmed organization.</span>
          <button type="button" onClick={() => setSelfReviewOpen(true)} className="focus-ring underline">
            Open records self-review
          </button>
        </div>
      )}

      {loadError && (
        <div role="alert" className="rounded-lg border border-rose-500/20 bg-rose-500/[0.04] px-3 py-2 text-caption text-rose-200">
          Findings unavailable: {loadError}
        </div>
      )}
      {actionError && (
        <div role="alert" className="rounded-lg border border-rose-500/20 bg-rose-500/[0.04] px-3 py-2 text-caption text-rose-200">
          {actionError.action === 'refresh'
            ? 'Analysis did not run; the list below is from the previous sweep: '
            : 'The finding could not be dismissed and has been restored: '}
          {actionError.message}
        </div>
      )}

      {isLoading && findings.length === 0 ? (
        <div role="status" className="text-label text-slate-400">
          Loading findings…
        </div>
      ) : lensEnabled && !hasConfirmedSubject ? null : findings.length === 0 && loadError ? null : findings.length === 0 ? (
        <div className="rounded-xl border border-dashed border-slate-800 bg-slate-950/25 p-4 text-center">
          <p className="text-label font-medium text-slate-300">
            No findings surfaced yet.
          </p>
          <p className="mt-1 text-caption leading-relaxed text-slate-400">
            Add a data source or press Refresh — analysis runs automatically after ingest.
          </p>
        </div>
      ) : (
        <ul className="flex flex-col gap-2" aria-label="Ranked findings">
          {visibleFindings.map((finding) => (
            <FindingRow
              key={finding.id}
              own={touchesOwnOrganization(finding, disclosure)}
              finding={finding}
              expanded={expandedId === finding.id}
              onToggle={() => setExpandedId((current) => (current === finding.id ? null : finding.id))}
              onInspect={() => inspect(finding)}
              onDismiss={() => dismiss(finding.id)}
              onAssemble={() => openLeadDossier(finding.id)}
              onOpenSource={openSource}
            />
          ))}
        </ul>
      )}
      {isNarrowed && visibleFindings.length === 0 && findings.length > 0 && (
        <p role="status" className="text-label text-slate-400">No loaded findings match this filter.</p>
      )}
      {hasMore && (
        <div className="flex items-center gap-2 text-caption text-slate-400">
          <span>Showing the top {findings.length} findings; more exist. Filters apply only to what is loaded.</span>
          <button
            type="button"
            onClick={() => setLimit((current) => current + PAGE_SIZE)}
            disabled={isLoading}
            className="secondary-action focus-ring rounded-lg border px-3 py-1 text-label font-semibold text-slate-300 disabled:opacity-60"
          >
            Load {PAGE_SIZE} more
          </button>
        </div>
      )}
      <LeadDossier />
      <EvidenceGaps />
    </section>
  );
}

function FindingRow({
  finding,
  own,
  expanded,
  onToggle,
  onInspect,
  onDismiss,
  onAssemble,
  onOpenSource,
}: {
  finding: Finding;
  own: boolean;
  expanded: boolean;
  onToggle: () => void;
  onInspect: () => void;
  onDismiss: () => void;
  onAssemble: () => void;
  onOpenSource: (citation: FindingCitation) => void;
}) {
  const [shape, tone] = severityGlyph(finding.severity);
  return (
    <li className="focus-within:ring-1 focus-within:ring-cyan-400/40 group rounded-xl border border-slate-800/70 bg-slate-950/30 p-3 transition hover:border-cyan-400/20 hover:bg-cyan-400/[0.03]" data-own-organization={own ? 'true' : undefined}>
      {own && <p className="mb-1 text-caption uppercase tracking-wider text-cyan-300">Your organization (self-declared)</p>}
      <div className="flex flex-wrap items-start gap-3 sm:flex-nowrap">
        <span
          className={`mt-0.5 grid h-7 w-7 shrink-0 place-items-center rounded-lg bg-slate-900/70 ${tone}`}
          aria-hidden="true"
        >
          <svg viewBox="-6 -6 12 12" className="h-3 w-3">
            <ShapeGlyph shape={shape} cx={0} cy={0} radius={4.5} fill="currentColor" />
          </svg>
        </span>
        <div className="flex min-w-0 flex-1 flex-col gap-1.5">
        <button
          type="button"
          onClick={onInspect}
          className="min-w-0 text-left"
          aria-label={finding.headline}
          aria-describedby={`finding-${finding.id}-explanation`}
        >
          <p className="text-label font-semibold text-slate-100">{finding.headline}</p>
          <p id={`finding-${finding.id}-explanation`} className="mt-1 text-label leading-relaxed text-slate-400">{finding.explanation}</p>
          <p className="mt-1 text-caption uppercase tracking-wider text-slate-400">
            {severityLabel(finding.severity)} · {formatRelativeTime(finding.lastSeenAt)}
          </p>
        </button>
        <FindingEvidence finding={finding} onOpenSource={onOpenSource} />
        </div>
        <div className="flex w-full shrink-0 flex-row items-end gap-1 sm:w-auto sm:flex-col">
          <button
            type="button"
            onClick={onToggle}
            aria-expanded={expanded}
            aria-controls={`finding-drafts-${finding.id}`}
            className="focus-ring rounded-md border border-slate-800 px-2 py-1 text-caption font-medium text-slate-400 transition hover:border-cyan-400/40 hover:text-slate-200"
          >
            {expanded ? 'Hide drafts' : 'Draft…'}
          </button>
          <button
            type="button"
            onClick={onAssemble}
            className="focus-ring rounded-md border border-slate-800 px-2 py-1 text-caption font-medium text-slate-400 transition hover:border-cyan-400/40 hover:text-slate-200"
            aria-label={`Assemble dossier: ${finding.headline}`}
          >
            Assemble dossier
          </button>
          <button
            type="button"
            onClick={onDismiss}
            className="focus-ring rounded-md border border-slate-800 px-2 py-1 text-caption font-medium text-slate-400 transition hover:border-slate-600 hover:text-slate-200"
            aria-label={`Dismiss finding: ${finding.headline}`}
          >
            Dismiss
          </button>
        </div>
      </div>
      {expanded && (
        <div id={`finding-drafts-${finding.id}`}>
          <ArtifactDrafts findingId={finding.id} />
          {isTauri() && <EvidencePacketExport findingIds={[finding.id]} />}
        </div>
      )}
    </li>
  );
}

function severityGlyph(severity: FindingSeverity): [GlyphShape, string] {
  switch (severity) {
    case 'CRITICAL':
      return ['diamond', 'text-rose-300'];
    case 'SERIOUS':
      return ['triangle', 'text-amber-300'];
    case 'NOTABLE':
      return ['circle', 'text-cyan-300'];
    case 'INFO':
      return ['ring', 'text-slate-400'];
  }
}

function severityLabel(severity: FindingSeverity): string {
  return severity.charAt(0) + severity.slice(1).toLowerCase();
}

function summariseSeverity(findings: Finding[]): string | null {
  if (findings.length === 0) return null;
  const counts = { CRITICAL: 0, SERIOUS: 0, NOTABLE: 0, INFO: 0 } as Record<FindingSeverity, number>;
  for (const finding of findings) counts[finding.severity] += 1;
  const parts: string[] = [];
  if (counts.CRITICAL) parts.push(`${counts.CRITICAL} critical`);
  if (counts.SERIOUS) parts.push(`${counts.SERIOUS} serious`);
  if (counts.NOTABLE) parts.push(`${counts.NOTABLE} notable`);
  if (counts.INFO) parts.push(`${counts.INFO} info`);
  return parts.join(' · ');
}

function formatRelativeTime(value: string) {
  const date = new Date(value);
  if (Number.isNaN(date.valueOf())) return 'recently';
  const seconds = Math.max(0, Math.round((Date.now() - date.valueOf()) / 1000));
  if (seconds < 60) return 'just now';
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m ago`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)}h ago`;
  return `${Math.floor(seconds / 86400)}d ago`;
}
