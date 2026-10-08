import { useCallback, useEffect, useState } from 'react';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useAppStore } from '../../store/useAppStore';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import { FindingEvidence } from '../ui/EvidenceBasis';
import type { Finding, FindingCitation } from '../../types';
import SeverityGlyph from './SeverityGlyph';

// The detectors may record an ingest's findings moments before the overlay opens, so "new" reaches
// this far back from the moment it opened.
const RECENT_WINDOW_MS = 15 * 60 * 1000;

/**
 * Opens after every ingest with the newest findings, ranked by severity. "New" means discovered
 * within the window before this overlay opened, not since a persistent watermark: it surfaces this
 * ingest's work, not the backlog, which lives in the findings inbox.
 */
const windowStart = (openedAt: number) => Math.max(0, openedAt - RECENT_WINDOW_MS);

export default function PostIngestBrief() {
  const open = useAppStore((state) => state.postIngestBriefOpen);
  const close = useAppStore((state) => state.setPostIngestBriefOpen);
  const findingsRefreshRequests = useAppStore((state) => state.findingsRefreshRequests);
  const setWorkspaceView = useAppStore((state) => state.setWorkspaceView);
  const setActivePortal = useAppStore((state) => state.setActivePortal);
  const setSearchFilters = useAppStore((state) => state.setSearchFilters);
  const [findings, setFindings] = useState<Finding[]>([]);
  const [openedAt, setOpenedAt] = useState<number | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const gate = useLatestRequestGate();

  useEffect(() => {
    if (open) {
      setOpenedAt((current) => current ?? Date.now());
    } else {
      setOpenedAt(null);
      // The rows belonged to the ingest that just closed. Keeping them would show them again,
      // as this ingest's work, when the next one opens the brief before its own load resolves.
      setFindings([]);
    }
  }, [open]);

  const load = useCallback(() => {
    if (!open || openedAt === null) return;
    const request = gate.begin();
    setIsLoading(true);
    setError(null);
    // Filtered at the source: the backend ranks by severity before it limits, so without the
    // cutoff a new low-severity finding could sit behind forty older ones and never arrive.
    const discoveredAfter = new Date(windowStart(openedAt)).toISOString();
    invokeCommand('list_findings', { filter: { discoveredAfter }, limit: 40 })
      .then((rows) => {
        if (!gate.isCurrent(request)) return;
        setFindings(rows);
      })
      .catch((reason) => {
        if (!gate.isCurrent(request)) return;
        setError(formatNativeError(reason));
      })
      .finally(() => {
        if (gate.isCurrent(request)) setIsLoading(false);
      });
  }, [gate, open, openedAt]);

  useEffect(() => {
    load();
    return gate.invalidate;
  }, [findingsRefreshRequests, gate, load, open]);

  if (!open) return null;

  const cutoff = windowStart(openedAt ?? 0);
  const notable = findings
    .filter((finding) => {
      const discovered = Date.parse(finding.discoveredAt);
      return Number.isFinite(discovered) && discovered >= cutoff;
    })
    .slice(0, 10);

  const inspect = (finding: Finding) => {
    const { portalKey, agencyId } = finding.subject;
    if (portalKey) setActivePortal(portalKey);
    setSearchFilters('', agencyId ?? null, null, null);
    setWorkspaceView('evidence');
    close(false);
  };

  const openSource = (citation: FindingCitation) => {
    setActivePortal(citation.portalKey);
    setWorkspaceView('evidence');
    close(false);
  };

  return (
    <div
      role="dialog"
      aria-modal="false"
      aria-labelledby="post-ingest-brief-title"
      className="fixed inset-x-0 top-0 z-40 border-b border-accent/30 bg-canvas shadow-lg backdrop-blur"
    >
      <div className="mx-auto flex w-full max-w-[1800px] flex-col gap-3 p-4 sm:p-5">
        <header className="flex items-start justify-between gap-3">
          <div className="min-w-0">
            <div className="app-eyebrow text-accent">Just landed</div>
            <h2 id="post-ingest-brief-title" className="mt-1 text-[14px] font-semibold text-fg">
              {error !== null && notable.length === 0
                ? 'Ingest complete. The new findings could not be read.'
                : notable.length === 0
                  ? 'Ingest complete. Nothing notable surfaced in the new data.'
                  : `${notable.length} new finding${notable.length === 1 ? '' : 's'} from this ingest`}
            </h2>
            {error && (
              <p role="alert" className="mt-1 text-caption text-danger">
                Findings load failed: {error}
              </p>
            )}
          </div>
          <button
            type="button"
            onClick={() => close(false)}
            className="focus-ring shrink-0 rounded-lg border border-control px-3 py-1.5 text-label font-semibold text-fg-secondary transition hover:border-control hover:text-fg"
          >
            Dismiss
          </button>
        </header>
        {isLoading && notable.length === 0 ? (
          <p role="status" className="text-label text-muted">
            Reading detector output…
          </p>
        ) : notable.length === 0 ? (
          <p className="text-label text-muted">
            {error !== null
              ? 'Whether this ingest surfaced anything is not known until the findings load.'
              : 'The findings inbox below carries anything the detectors have surfaced on prior data.'}
          </p>
        ) : (
          <ul className="grid gap-2 md:grid-cols-2 xl:grid-cols-3" aria-label="New findings from this ingest">
            {notable.map((finding) => (
              <li key={finding.id} className="flex flex-col gap-2 rounded-xl border border-line bg-canvas/40 p-3">
                <button
                  type="button"
                  onClick={() => inspect(finding)}
                  aria-label={`Open finding: ${finding.headline}`}
                  className="focus-ring group flex w-full items-start gap-3 rounded-lg text-left transition hover:bg-accent/5"
                >
                  <SeverityGlyph severity={finding.severity} className="h-6 w-6 rounded-lg bg-surface/70" />
                  <span className="min-w-0 flex-1">
                    <span className="block text-label font-semibold text-fg">
                      {finding.headline}
                    </span>
                    <span className="mt-1 block text-caption leading-relaxed text-muted">
                      {finding.explanation}
                    </span>
                  </span>
                </button>
                <FindingEvidence finding={finding} onOpenSource={openSource} />
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}
