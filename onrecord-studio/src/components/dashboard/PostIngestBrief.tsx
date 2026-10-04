import { useCallback, useEffect, useState } from 'react';
import { invokeCommand } from '../../ipc';
import { useAppStore } from '../../store/useAppStore';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import { ShapeGlyph, type GlyphShape } from '../ui/shapeGlyph';
import { FindingEvidence } from '../ui/EvidenceBasis';
import ChecksNotRunNotice from './ChecksNotRunNotice';
import type { Finding, FindingCitation, FindingSeverity } from '../../types';

/**
 * The panel that opens automatically after every ingest. It shows the newest findings that
 * arrived while the user was doing nothing, ranked by severity, so a user who added a source
 * and walked away comes back to "here is what looks wrong in what you just added" instead of
 * a chime and a log line.
 *
 * "New" here means `discoveredAt` after the moment this component's overlay opened, not since
 * some persistent watermark: the intent is to surface *this ingest's* work, not the whole
 * backlog. The full backlog lives in FindingsInbox below.
 */
// The brief is opened by App.tsx immediately after refresh_findings, but the detectors may have
// discovered the ingested rows moments before the overlay itself opened. Widen the window to the
// last 15 minutes so a ChangeFeed-scale ingest doesn't render "no new findings" when the corpus
// obviously grew.
const RECENT_WINDOW_MS = 15 * 60 * 1000;

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
  const refreshOutcome = useAppStore((state) => state.findingsRefreshOutcome);
  const notRun = refreshOutcome?.status === 'completed' ? refreshOutcome.notRun : [];

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
    const discoveredAfter = new Date(Math.max(0, openedAt - RECENT_WINDOW_MS)).toISOString();
    invokeCommand('list_findings', { filter: { discoveredAfter }, limit: 40 })
      .then((rows) => {
        if (!gate.isCurrent(request)) return;
        setFindings(rows);
      })
      .catch((reason) => {
        if (!gate.isCurrent(request)) return;
        setError(String(reason));
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

  const cutoff = openedAt ?? 0;
  const cutoffWithWindow = Math.max(0, cutoff - RECENT_WINDOW_MS);
  const notable = findings
    .filter((finding) => {
      const discovered = Date.parse(finding.discoveredAt);
      return Number.isFinite(discovered) && discovered >= cutoffWithWindow;
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
                : notable.length === 0 && notRun.length > 0
                  ? `Ingest complete. Nothing surfaced from the checks that ran; ${notRun.length} did not run.`
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
        <ChecksNotRunNotice />
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
                  <PostIngestSeverityGlyph severity={finding.severity} />
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

function PostIngestSeverityGlyph({ severity }: { severity: FindingSeverity }) {
  const [shape, tone] = severityGlyph(severity);
  return (
    <span
      aria-hidden="true"
      className={`grid h-6 w-6 shrink-0 place-items-center rounded-lg bg-surface/70 ${tone}`}
    >
      <svg viewBox="-6 -6 12 12" className="h-3 w-3">
        <ShapeGlyph shape={shape} cx={0} cy={0} radius={4.5} fill="currentColor" />
      </svg>
    </span>
  );
}

function severityGlyph(severity: FindingSeverity): [GlyphShape, string] {
  switch (severity) {
    case 'CRITICAL':
      return ['diamond', 'text-danger'];
    case 'SERIOUS':
      return ['triangle', 'text-warning'];
    case 'NOTABLE':
      return ['circle', 'text-accent'];
    case 'INFO':
      return ['ring', 'text-muted'];
  }
}
