import { useCallback, useEffect, useMemo, useState } from 'react';
import { invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import { ShapeGlyph, type GlyphShape } from '../ui/shapeGlyph';
import { useAppStore } from '../../store/useAppStore';
import type { Finding, FindingSeverity } from '../../types';
import ArtifactDrafts from './ArtifactDrafts';

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
  const [error, setError] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [isRefreshing, setIsRefreshing] = useState(false);
  // Which finding's row is expanded to show ArtifactDrafts. At most one at a time — the drafted
  // text can be long, and stacking several open drafts hides the ranked list.
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const listGate = useLatestRequestGate();
  const setWorkspaceView = useAppStore((state) => state.setWorkspaceView);
  const setActivePortal = useAppStore((state) => state.setActivePortal);
  const setSearchFilters = useAppStore((state) => state.setSearchFilters);
  const findingsRefreshRequests = useAppStore((state) => state.findingsRefreshRequests);
  const requestFindingsRefresh = useAppStore((state) => state.requestFindingsRefresh);
  const markFindingsSeen = useAppStore((state) => state.markFindingsSeen);

  const loadFindings = useCallback(() => {
    const request = listGate.begin();
    setIsLoading(true);
    setError(null);
    invokeCommand('list_findings', { filter: null, limit: 50 })
      .then((rows) => {
        if (!listGate.isCurrent(request)) return;
        setFindings(rows);
        // The user is looking at the inbox. Bump the watermark so ChangeFeed knows the difference
        // between findings that arrived while they weren't looking and findings they've already
        // been shown. Only on a successful load — a failed fetch is not "seen."
        markFindingsSeen();
      })
      .catch((reason) => {
        if (!listGate.isCurrent(request)) return;
        setError(String(reason));
      })
      .finally(() => {
        if (listGate.isCurrent(request)) setIsLoading(false);
      });
  }, [listGate, markFindingsSeen]);

  // The store bumps `findingsRefreshRequests` after each ingest (via `refreshFindingsInbox`) and
  // whenever the user presses Refresh. Loading is keyed off that counter so we always pick up the
  // latest sweep without a manual mount-time fetch that could race the ingest hook.
  useEffect(() => {
    loadFindings();
    return listGate.invalidate;
  }, [findingsRefreshRequests, listGate, loadFindings]);

  const refresh = useCallback(async () => {
    setIsRefreshing(true);
    try {
      await invokeCommand('refresh_findings', {});
    } catch (reason) {
      setError(String(reason));
    } finally {
      setIsRefreshing(false);
      requestFindingsRefresh();
    }
  }, [requestFindingsRefresh]);

  const dismiss = useCallback(
    async (id: string) => {
      const previous = findings;
      setFindings((rows) => rows.filter((row) => row.id !== id));
      try {
        await invokeCommand('dismiss_finding', { id });
      } catch (reason) {
        setFindings(previous);
        setError(String(reason));
      }
    },
    [findings],
  );

  const inspect = useCallback(
    (finding: Finding) => {
      const { portalKey, agencyId } = finding.subject;
      if (portalKey) setActivePortal(portalKey);
      setSearchFilters('', agencyId ?? null, null, null);
      setWorkspaceView('evidence');
    },
    [setActivePortal, setSearchFilters, setWorkspaceView],
  );

  const summary = useMemo(() => summariseSeverity(findings), [findings]);

  return (
    <section id="findings-inbox" className="app-panel flex flex-col gap-3 p-4" aria-label="Findings inbox">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="app-eyebrow">Findings inbox</div>
          <h3 className="mt-1 text-[13px] font-semibold text-slate-200">
            Everything the app is surfacing right now
          </h3>
          {summary && (
            <p className="mt-1 text-[9px] text-slate-400">
              {summary} — ranked by severity, then confidence, then freshness.
            </p>
          )}
        </div>
        <button
          type="button"
          onClick={refresh}
          disabled={isRefreshing}
          className="secondary-action focus-ring shrink-0 whitespace-nowrap rounded-lg border px-3 py-1.5 text-[10px] font-semibold text-slate-300 transition disabled:opacity-60"
        >
          {isRefreshing ? 'Analyzing…' : 'Refresh findings'}
        </button>
      </div>

      {error && (
        <div role="alert" className="rounded-lg border border-rose-500/20 bg-rose-500/[0.04] px-3 py-2 text-[9px] text-rose-200">
          Findings unavailable: {error}
        </div>
      )}

      {isLoading && findings.length === 0 ? (
        <div role="status" className="text-[10px] text-slate-400">
          Loading findings…
        </div>
      ) : findings.length === 0 ? (
        <div className="rounded-xl border border-dashed border-slate-800 bg-slate-950/25 p-4 text-center">
          <p className="text-[10px] font-medium text-slate-300">
            No findings surfaced yet.
          </p>
          <p className="mt-1 text-[9px] leading-relaxed text-slate-400">
            Add a data source or press Refresh — analysis runs automatically after ingest.
          </p>
        </div>
      ) : (
        <ul className="flex flex-col gap-2" aria-label="Ranked findings">
          {findings.map((finding) => (
            <FindingRow
              key={finding.id}
              finding={finding}
              expanded={expandedId === finding.id}
              onToggle={() => setExpandedId((current) => (current === finding.id ? null : finding.id))}
              onInspect={() => inspect(finding)}
              onDismiss={() => dismiss(finding.id)}
            />
          ))}
        </ul>
      )}
    </section>
  );
}

function FindingRow({
  finding,
  expanded,
  onToggle,
  onInspect,
  onDismiss,
}: {
  finding: Finding;
  expanded: boolean;
  onToggle: () => void;
  onInspect: () => void;
  onDismiss: () => void;
}) {
  const [shape, tone] = severityGlyph(finding.severity);
  return (
    <li className="focus-within:ring-1 focus-within:ring-cyan-400/40 group rounded-xl border border-slate-800/70 bg-slate-950/30 p-3 transition hover:border-cyan-400/20 hover:bg-cyan-400/[0.03]">
      <div className="flex items-start gap-3">
        <span
          className={`mt-0.5 grid h-7 w-7 shrink-0 place-items-center rounded-lg bg-slate-900/70 ${tone}`}
          aria-hidden="true"
        >
          <svg viewBox="-6 -6 12 12" className="h-3 w-3">
            <ShapeGlyph shape={shape} cx={0} cy={0} radius={4.5} fill="currentColor" />
          </svg>
        </span>
        <button
          type="button"
          onClick={onInspect}
          className="min-w-0 flex-1 text-left"
          aria-label={`Inspect finding: ${finding.headline}`}
        >
          <p className="text-[11px] font-semibold text-slate-100">{finding.headline}</p>
          <p className="mt-1 text-[10px] leading-relaxed text-slate-400">{finding.explanation}</p>
          <p className="mt-1 text-[8px] uppercase tracking-wider text-slate-400">
            {severityLabel(finding.severity)}
            {finding.confidence.kind === 'MEASURED' &&
              ` · ${(finding.confidence.basisPoints / 100).toFixed(0)}% confidence`}
            {finding.confidence.kind === 'UNASSESSED' && ' · confidence unassessed'}
            {' · '}
            {formatRelativeTime(finding.lastSeenAt)}
          </p>
        </button>
        <div className="flex shrink-0 flex-col items-end gap-1">
          <button
            type="button"
            onClick={onToggle}
            aria-expanded={expanded}
            aria-controls={`finding-drafts-${finding.id}`}
            className="focus-ring rounded-md border border-slate-800 px-2 py-0.5 text-[8px] font-medium text-slate-400 transition hover:border-cyan-400/40 hover:text-slate-200"
          >
            {expanded ? 'Hide drafts' : 'Draft…'}
          </button>
          <button
            type="button"
            onClick={onDismiss}
            className="focus-ring rounded-md border border-slate-800 px-2 py-0.5 text-[8px] font-medium text-slate-400 transition hover:border-slate-600 hover:text-slate-200"
            aria-label={`Dismiss finding: ${finding.headline}`}
          >
            Dismiss
          </button>
        </div>
      </div>
      {expanded && (
        <div id={`finding-drafts-${finding.id}`}>
          <ArtifactDrafts findingId={finding.id} />
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
