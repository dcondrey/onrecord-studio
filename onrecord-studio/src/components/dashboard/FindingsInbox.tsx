import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { isTauri } from '@tauri-apps/api/core';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useCaseScope } from '../../hooks/useCaseScope';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import { cn } from '../../utils/cn';
import Badge from '../ui/Badge';
import Button from '../ui/Button';
import InlineError from '../ui/InlineError';
import Skeleton from '../ui/Skeleton';
import SeverityGlyph from './SeverityGlyph';
import { severityLabel } from '../objects/labels';
import { useAppStore } from '../../store/useAppStore';
import { useInspectFinding } from '../../hooks/useInspectFinding';
import type { Finding, FindingCitation, FindingSeverity } from '../../types';
import { FindingEvidence } from '../ui/EvidenceBasis';
import ArtifactDrafts from './ArtifactDrafts';
import EvidenceGaps from './EvidenceGaps';
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
  action: 'refresh' | 'dismiss' | 'restore';
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
const UNDO_WINDOW_MS = 10_000;

export default function FindingsInbox() {
  const [findings, setFindings] = useState<Finding[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<ActionError | null>(null);
  // The most recent dismissal can be undone for a short while; restoring puts the row back where it was.
  const [lastDismissed, setLastDismissed] = useState<{ finding: Finding; index: number } | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [isRefreshing, setIsRefreshing] = useState(false);
  const [limit, setLimit] = useState(PAGE_SIZE);
  const [hasMore, setHasMore] = useState(false);
  const [severityFilter, setSeverityFilter] = useState<FindingSeverity | null>(null);
  const [sort, setSort] = useState<FindingSort>('ranked');
  const [textFilter, setTextFilter] = useState('');
  const [exportOpen, setExportOpen] = useState(false);
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
  const openLeadDossier = useAppStore((state) => state.openLeadDossier);
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
        // The user is looking at the inbox. Bump the watermark so the sidebar badge knows the difference
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
        setLastDismissed({ finding: removed, index });
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

  useEffect(() => {
    if (!lastDismissed) return;
    const timer = window.setTimeout(() => setLastDismissed(null), UNDO_WINDOW_MS);
    return () => window.clearTimeout(timer);
  }, [lastDismissed]);

  const undoDismissal = useCallback(async () => {
    if (!lastDismissed) return;
    const { finding, index } = lastDismissed;
    setLastDismissed(null);
    setActionError(null);
    try {
      await invokeCommand('restore_finding', { id: finding.id });
      settledDismissals.current.delete(finding.id);
      setFindings((rows) => {
        if (rows.some((row) => row.id === finding.id)) return rows;
        const at = Math.min(index, rows.length);
        return [...rows.slice(0, at), { ...finding, dismissedAt: null }, ...rows.slice(at)];
      });
    } catch (reason) {
      setActionError({ action: 'restore', message: formatNativeError(reason) });
    }
  }, [lastDismissed]);

  const inspect = useInspectFinding();

  // A citation opens its passage in the inspector; the citation does not record its request, so the
  // document layer says so rather than guessing one for the viewer.
  const inspectObject = useAppStore((state) => state.inspect);
  const openSource = useCallback(
    (citation: FindingCitation) => {
      inspectObject({
        kind: 'document',
        portalKey: citation.portalKey,
        documentId: citation.documentId,
        requestId: null,
        characterStart: citation.characterStart,
        characterEnd: citation.characterEnd,
        label: citation.label ?? undefined,
      });
    },
    [inspectObject],
  );

  const hasConfirmedSubject =
    disclosure !== null && (disclosure.ownPortalKeys.length > 0 ||
      disclosure.ownAgencies.length > 0 ||
      disclosure.ownEntityIds.length > 0);
  const caseScope = useCaseScope();
  const lensedFindings = useMemo(() => {
    const lensed = lensEnabled ? findings.filter((finding) => touchesOwnOrganization(finding, disclosure)) : findings;
    return caseScope.status === 'ready' ? lensed.filter((finding) => caseScope.value.findingIds.has(finding.id)) : lensed;
  }, [caseScope, lensEnabled, findings, disclosure]);
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
    <section id="findings-inbox" className="app-panel flex flex-col gap-4 p-5" aria-label="Findings inbox">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <p className="min-w-[14rem] flex-1 text-caption tabular-nums text-muted">
          {summary ? `${summary}. Ranked by severity, then confidence, then freshness.` : 'Ranked by severity, then confidence, then freshness.'}
        </p>
        <div className="flex shrink-0 flex-wrap items-center gap-2">
          <Button
            size="sm"
            aria-pressed={lensEnabled}
            onClick={() => setLensEnabled(!lensEnabled)}
            title="Show only findings about your confirmed organization. Ranking and confidence are unchanged."
            className={cn('whitespace-nowrap', lensEnabled && 'border-accent/30 bg-accent/10 text-accent hover:bg-accent/15')}
          >
            Only my organization
          </Button>
          {isTauri() && (
            <Button size="sm" aria-expanded={exportOpen} aria-controls="findings-export" onClick={() => setExportOpen((open) => !open)} className="whitespace-nowrap">
              Export…
            </Button>
          )}
          <Button size="sm" onClick={refresh} disabled={isRefreshing} className="whitespace-nowrap">
            {isRefreshing ? 'Analyzing…' : 'Refresh'}
          </Button>
        </div>
      </div>

      {isTauri() && exportOpen && (
        <div id="findings-export" className="space-y-3 rounded-lg border border-line p-4 text-body text-fg-secondary">
          <VerifiableReportExport />
          <EvidencePacketExport />
          <ShareableBundleExport findings={lensedFindings} />
        </div>
      )}

      {findings.length > 0 && (
        <div className="flex flex-wrap items-center gap-2 text-caption text-muted">
          <div role="group" aria-label="Filter by severity" className="flex gap-1">
            {SEVERITIES.map((level) => (
              <button
                key={level}
                type="button"
                aria-pressed={severityFilter === level}
                onClick={() => setSeverityFilter((current) => (current === level ? null : level))}
                className={cn('focus-ring h-7 rounded-md border px-2.5 text-control', severityFilter === level ? 'border-accent/30 bg-accent/10 text-accent' : 'border-line-strong text-fg-secondary hover:bg-raised')}
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
            className="analysis-input h-7 min-w-[10rem] flex-1 rounded-md"
          />
          <label htmlFor="findings-sort" className="sr-only">Sort findings</label>
          <select
            id="findings-sort"
            value={sort}
            onChange={(event) => setSort(event.target.value as FindingSort)}
            className="analysis-input h-7 w-auto rounded-md"
          >
            <option value="ranked">Ranked</option>
            <option value="newest">Newest first</option>
          </select>
        </div>
      )}

      {lensEnabled && !hasConfirmedSubject && (
        <div role="status" className="rounded-lg border border-line px-3 py-2 text-caption text-fg-secondary">
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
        <div className="flex items-center gap-2 text-caption text-muted">
          <span>{visibleFindings.length} of {findings.length} findings touch your confirmed organization.</span>
          <button type="button" onClick={() => setSelfReviewOpen(true)} className="focus-ring underline">
            Open records self-review
          </button>
        </div>
      )}

      {loadError && (
        <InlineError summary="Findings could not be read." detail={loadError} onRetry={refresh} />
      )}
      {lastDismissed && (
        <div role="status" className="motion-pop flex items-center justify-between gap-3 rounded-lg border border-line-strong bg-raised px-3 py-2 text-label text-fg-secondary">
          <span className="min-w-0 truncate">Dismissed: {lastDismissed.finding.headline}</span>
          <button type="button" onClick={() => void undoDismissal()} className="motion-press focus-ring shrink-0 rounded-md px-2 py-0.5 font-semibold text-accent hover:bg-accent/10">
            Undo
          </button>
        </div>
      )}
      {actionError && (
        <div role="alert" className="rounded-lg border border-danger/30 bg-danger/5 px-3 py-2 text-caption text-danger">
          {actionError.action === 'refresh'
            ? 'Analysis did not run; the list below is from the previous sweep: '
            : actionError.action === 'restore'
              ? 'The finding is still dismissed; restoring it failed: '
              : 'The finding could not be dismissed and has been restored: '}
          {actionError.message}
        </div>
      )}

      {isLoading && findings.length === 0 ? (
        <div className="flex flex-col gap-2">
          <Skeleton variant="block" height={120} label="Loading findings" />
          {[1, 2].map((slot) => <div key={slot} aria-hidden="true"><Skeleton variant="block" height={120} label="" /></div>)}
        </div>
      ) : lensEnabled && !hasConfirmedSubject ? null : findings.length === 0 && loadError ? null : findings.length === 0 ? (
        <div className="rounded-xl border border-dashed border-line bg-canvas/40 p-4 text-center">
          <p className="text-heading text-fg-secondary">No findings surfaced yet</p>
          <p className="mt-1 text-caption text-muted">
            Add a data source or press Refresh. Analysis runs automatically after each collection.
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
        <p role="status" className="text-label text-muted">No loaded findings match this filter.</p>
      )}
      {hasMore && (
        <div className="flex items-center gap-2 text-caption text-muted">
          <span>Showing the top {findings.length} findings; more exist. Filters apply only to what is loaded.</span>
          <button
            type="button"
            onClick={() => setLimit((current) => current + PAGE_SIZE)}
            disabled={isLoading}
            className="secondary-action focus-ring rounded-lg border px-3 py-1 text-label font-semibold text-fg-secondary disabled:opacity-60"
          >
            Load {PAGE_SIZE} more
          </button>
        </div>
      )}
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
  return (
    <li className="flex flex-col gap-3 rounded-lg border border-line p-4 transition-colors hover:border-line-strong" data-own-organization={own ? 'true' : undefined}>
      {own && <Badge tone="neutral">Your organization (self-declared)</Badge>}
      <div className="flex items-start gap-3">
        <SeverityGlyph severity={finding.severity} className="mt-1" />
        <div className="flex min-w-0 flex-1 flex-col gap-2">
          <button
            type="button"
            onClick={onInspect}
            className="focus-ring min-w-0 rounded text-left"
          >
            <p className="text-heading text-fg">{finding.headline}</p>
            <p className="mt-1 whitespace-pre-line break-words text-body text-muted">{finding.explanation}</p>
            <p className="mt-1 text-caption tabular-nums text-muted">
              {severityLabel(finding.severity)} · {formatRelativeTime(finding.lastSeenAt)}
            </p>
          </button>
          <FindingEvidence finding={finding} onOpenSource={onOpenSource} />
        </div>
      </div>
      <div className="flex flex-wrap items-center gap-2 pl-6">
        <Button size="sm" onClick={onAssemble} aria-label={`Open evidence chain: ${finding.headline}`}>Open evidence chain</Button>
        <Button
          size="sm"
          onClick={onToggle}
          aria-expanded={expanded}
          aria-controls={`finding-drafts-${finding.id}`}
          aria-label={expanded ? `Hide drafts: ${finding.headline}` : `Draft request: ${finding.headline}`}
        >
          {expanded ? 'Hide drafts' : 'Draft request'}
        </Button>
        <Button
          size="sm"
          variant="ghost"
          iconOnly
          icon="x"
          onClick={onDismiss}
          aria-label={`Dismiss finding: ${finding.headline}`}
          title="Dismiss"
          className="ml-auto"
        />
      </div>
      <div id={`finding-drafts-${finding.id}`} hidden={!expanded}>
        {expanded && (
          <>
            <ArtifactDrafts findingId={finding.id} />
            {isTauri() && <EvidencePacketExport findingIds={[finding.id]} />}
          </>
        )}
      </div>
    </li>
  );
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
