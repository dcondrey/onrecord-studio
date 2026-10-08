import { useCallback, useEffect, useRef, useState, type KeyboardEvent } from 'react';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useAppStore } from '../../store/useAppStore';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import { FindingEvidence } from '../ui/EvidenceBasis';
import type { Finding, FindingCitation } from '../../types';
import SeverityGlyph from './SeverityGlyph';
import Button from '../ui/Button';
import InlineError from '../ui/InlineError';
import Skeleton from '../ui/Skeleton';

// The detectors may record an ingest's findings moments before the overlay opens, so "new" reaches
// this far back from the moment it opened.
const RECENT_WINDOW_MS = 15 * 60 * 1000;

/**
 * Shown in the overview after every import with the newest findings, ranked by severity. "New"
 * means discovered within the window before it opened, not since a persistent watermark: it
 * surfaces this import's work, not the backlog, which lives under "Needs attention".
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
  const sectionRef = useRef<HTMLElement>(null);

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

  useEffect(() => {
    if (open) sectionRef.current?.focus({ preventScroll: false });
  }, [open]);

  if (!open) return null;

  const dismiss = () => {
    close(false);
    // The brief unmounts, so focus would fall to the body; the page title is where reading resumes.
    const heading = document.querySelector<HTMLElement>('main h2');
    if (!heading) return;
    if (!heading.hasAttribute('tabindex')) heading.setAttribute('tabindex', '-1');
    heading.focus({ preventScroll: true });
  };

  const onKeyDown = (event: KeyboardEvent<HTMLElement>) => {
    if (event.key !== 'Escape') return;
    event.stopPropagation();
    dismiss();
  };

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

  const title = error !== null && notable.length === 0
    ? 'New findings could not be read.'
    : isLoading && notable.length === 0
      ? 'Checking for new findings…'
      : notable.length === 0
        ? 'Import finished. No new findings.'
        : `${notable.length} new finding${notable.length === 1 ? '' : 's'} from this import`;

  return (
    <section
      ref={sectionRef}
      tabIndex={-1}
      aria-labelledby="post-ingest-brief-title"
      onKeyDown={onKeyDown}
      className="app-panel motion-reveal border-accent/30 p-4 outline-none"
    >
      <header className="flex items-center justify-between gap-3">
        <div className="min-w-0">
          {notable.length > 0 && <p className="text-overline uppercase text-muted">New findings</p>}
          <h3 id="post-ingest-brief-title" role={notable.length === 0 ? 'status' : undefined} className="text-heading text-fg">{title}</h3>
        </div>
        <Button size="sm" variant="ghost" onClick={dismiss}>Dismiss</Button>
      </header>
      {error !== null && (
        <InlineError className="mt-3" summary="Whether this import surfaced anything isn’t known until the findings load." detail={error} onRetry={load} />
      )}
      {isLoading && notable.length === 0 && error === null && (
        <Skeleton variant="lines" lines={2} label="Checking for new findings" className="mt-3" />
      )}
      {notable.length > 0 && (
        <ul className="mt-3 grid gap-2 md:grid-cols-2 xl:grid-cols-3" aria-label="New findings from this import">
          {notable.map((finding) => (
            <li key={finding.id} className="flex flex-col gap-2 rounded-lg border border-line bg-canvas p-3">
              <button
                type="button"
                onClick={() => inspect(finding)}
                aria-label={`Open finding: ${finding.headline}`}
                className="focus-ring group flex w-full items-start gap-3 rounded-md text-left"
              >
                <SeverityGlyph severity={finding.severity} className="h-6 w-6 rounded-md bg-raised" />
                <span className="min-w-0 flex-1">
                  <span className="block text-body font-medium text-fg group-hover:underline">{finding.headline}</span>
                  <span className="mt-1 line-clamp-2 text-caption text-muted">{finding.explanation}</span>
                </span>
              </button>
              <FindingEvidence finding={finding} onOpenSource={openSource} />
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
