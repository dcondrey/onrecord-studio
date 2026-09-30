import { useCallback, useEffect, useState } from 'react';
import { invokeCommand } from '../../ipc';
import { useAppStore } from '../../store/useAppStore';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import { ShapeGlyph, type GlyphShape } from '../ui/shapeGlyph';
import { FindingEvidence } from '../ui/EvidenceBasis';
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
    if (open) setOpenedAt((current) => current ?? Date.now());
    else setOpenedAt(null);
  }, [open]);

  const load = useCallback(() => {
    if (!open) return;
    const request = gate.begin();
    setIsLoading(true);
    setError(null);
    invokeCommand('list_findings', { filter: null, limit: 40 })
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
  }, [gate, open]);

  useEffect(() => {
    load();
    return gate.invalidate;
  }, [findingsRefreshRequests, gate, load, open]);

  if (!open) return null;

  const cutoff = openedAt ?? 0;
  // The brief is opened by App.tsx immediately after refresh_findings, but the detectors may
  // have discovered the ingested rows moments before the overlay itself opened. Widen the window
  // to the last 15 minutes so a ChangeFeed-scale ingest doesn't render "no new findings" when
  // the corpus obviously grew.
  const window = 15 * 60 * 1000;
  const cutoffWithWindow = Math.max(0, cutoff - window);
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
      className="fixed inset-x-0 top-0 z-40 border-b border-cyan-400/20 bg-slate-950/95 shadow-lg backdrop-blur"
    >
      <div className="mx-auto flex w-full max-w-[1800px] flex-col gap-3 p-4 sm:p-5">
        <header className="flex items-start justify-between gap-3">
          <div className="min-w-0">
            <div className="app-eyebrow text-cyan-300/70">Just landed</div>
            <h2 id="post-ingest-brief-title" className="mt-1 text-[14px] font-semibold text-slate-100">
              {notable.length === 0
                ? 'Ingest complete. Nothing notable surfaced in the new data.'
                : `${notable.length} new finding${notable.length === 1 ? '' : 's'} from this ingest`}
            </h2>
            {error && (
              <p role="alert" className="mt-1 text-[9px] text-rose-300">
                Findings load failed: {error}
              </p>
            )}
          </div>
          <button
            type="button"
            onClick={() => close(false)}
            className="focus-ring shrink-0 rounded-lg border border-slate-800 px-3 py-1.5 text-[10px] font-semibold text-slate-300 transition hover:border-slate-600 hover:text-slate-100"
          >
            Dismiss
          </button>
        </header>
        {isLoading && notable.length === 0 ? (
          <p role="status" className="text-[10px] text-slate-400">
            Reading detector output…
          </p>
        ) : notable.length === 0 ? (
          <p className="text-[10px] text-slate-400">
            The findings inbox below carries anything the detectors have surfaced on prior data.
          </p>
        ) : (
          <ul className="grid gap-2 md:grid-cols-2 xl:grid-cols-3" aria-label="New findings from this ingest">
            {notable.map((finding) => (
              <li key={finding.id} className="flex flex-col gap-2 rounded-xl border border-slate-800/70 bg-slate-950/40 p-3">
                <button
                  type="button"
                  onClick={() => inspect(finding)}
                  aria-label={`Open finding: ${finding.headline}`}
                  className="focus-ring group flex w-full items-start gap-3 rounded-lg text-left transition hover:bg-cyan-400/[0.03]"
                >
                  <PostIngestSeverityGlyph severity={finding.severity} />
                  <span className="min-w-0 flex-1">
                    <span className="block text-[11px] font-semibold text-slate-100">
                      {finding.headline}
                    </span>
                    <span className="mt-1 block text-[9px] leading-relaxed text-slate-400">
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
      className={`grid h-6 w-6 shrink-0 place-items-center rounded-lg bg-slate-900/70 ${tone}`}
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
      return ['diamond', 'text-rose-300'];
    case 'SERIOUS':
      return ['triangle', 'text-amber-300'];
    case 'NOTABLE':
      return ['circle', 'text-cyan-300'];
    case 'INFO':
      return ['ring', 'text-slate-400'];
  }
}
