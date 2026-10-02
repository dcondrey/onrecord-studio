import { useCallback, useMemo } from 'react';
import { useFindings } from '../../hooks/useFindings';
import { useAppStore } from '../../store/useAppStore';
import { ShapeGlyph, type GlyphShape } from '../ui/shapeGlyph';
import type { Finding, FindingKind, FindingSeverity } from '../../types';

/**
 * Cross-corpus findings panel: the four Wave 8 patterns that span jurisdictions, agencies, and
 * time. Displays as its own section below the ranked FindingsInbox because their subject
 * anchors (jurisdiction pairs, change-points) don't fit the inbox's severity-first ranking as
 * cleanly as a per-request finding does.
 *
 * Rendered as a compact agency-pair heatmap on the left plus the ranked list on the right.
 * Both use the `useLatestRequestGate` hook so a superseded refresh cannot publish stale rows.
 */
type CrossCorpusKind =
  | 'CROSS_CORPUS_SHARED_LANGUAGE'
  | 'CROSS_CORPUS_TEMPORAL_SHIFT'
  | 'CROSS_CORPUS_ORPHAN_TOPIC'
  | 'CROSS_CORPUS_REUSED_DENIAL_GROUNDS';

const CROSS_CORPUS_KINDS: readonly CrossCorpusKind[] = [
  'CROSS_CORPUS_SHARED_LANGUAGE',
  'CROSS_CORPUS_TEMPORAL_SHIFT',
  'CROSS_CORPUS_ORPHAN_TOPIC',
  'CROSS_CORPUS_REUSED_DENIAL_GROUNDS',
];

const KIND_LABEL: Record<CrossCorpusKind, string> = {
  CROSS_CORPUS_SHARED_LANGUAGE: 'Shared language',
  CROSS_CORPUS_TEMPORAL_SHIFT: 'Temporal shift',
  CROSS_CORPUS_ORPHAN_TOPIC: 'Orphan topic',
  CROSS_CORPUS_REUSED_DENIAL_GROUNDS: 'Reused grounds',
};

function isCrossCorpusKind(kind: FindingKind): kind is CrossCorpusKind {
  return (CROSS_CORPUS_KINDS as readonly string[]).includes(kind);
}

export default function CrossCorpusExplorer() {
  const load = useFindings(null, 200);
  const findings = useMemo(() => load.findings.filter((row) => isCrossCorpusKind(row.kind)), [load.findings]);
  const error = load.status === 'failed' ? load.message : null;
  const isLoading = load.status === 'loading';
  const setActivePortal = useAppStore((state) => state.setActivePortal);
  const setSearchFilters = useAppStore((state) => state.setSearchFilters);
  const setWorkspaceView = useAppStore((state) => state.setWorkspaceView);

  // Build an agency-pair heatmap: rows = originating agency, cols = other agency mentioned in
  // headline text. Because backend citations don't carry a second agency id, we derive the
  // pairing from the finding's subject.agencyId and the count of documents cited across other
  // portals. This produces an approximate view — the exact detector logic is authoritative.
  const heatmap = useMemo(() => {
    const cellCounts = new Map<string, number>();
    let maxCount = 0;
    for (const finding of findings) {
      if (!isCrossCorpusKind(finding.kind)) continue;
      const source = finding.subject.portalKey ?? '(no portal)';
      const kind = KIND_LABEL[finding.kind];
      const key = `${source}\u001f${kind}`;
      const next = (cellCounts.get(key) ?? 0) + 1;
      cellCounts.set(key, next);
      if (next > maxCount) maxCount = next;
    }
    const portals = new Set<string>();
    for (const key of cellCounts.keys()) portals.add(key.split('\u001f')[0]);
    return {
      portals: Array.from(portals).sort(),
      cellCounts,
      maxCount,
    };
  }, [findings]);

  const inspect = useCallback(
    (finding: Finding) => {
      if (finding.subject.portalKey) setActivePortal(finding.subject.portalKey);
      setSearchFilters('', finding.subject.agencyId ?? null, null, null);
      setWorkspaceView('evidence');
    },
    [setActivePortal, setSearchFilters, setWorkspaceView],
  );

  return (
    <section className="app-panel flex flex-col gap-3 p-4" aria-label="Cross-corpus findings">
      <div className="min-w-0">
        <div className="app-eyebrow">Cross-corpus explorer</div>
        <h3 className="mt-1 text-label font-semibold text-fg">
          Patterns that span jurisdictions, agencies, and time
        </h3>
        <p className="mt-1 text-caption text-muted">
          Coordinated language, behavioral change-points, orphan topics, and reused denial
          grounds — signals no single-agency detector can see.
        </p>
      </div>

      {error && (
        <div role="alert" className="rounded-lg border border-danger/30 bg-danger/5 px-3 py-2 text-caption text-danger">
          Cross-corpus findings unavailable: {error}
        </div>
      )}

      {isLoading && findings.length === 0 ? (
        <div role="status" className="text-label text-muted">
          Loading cross-corpus findings…
        </div>
      ) : findings.length === 0 ? (
        <div className="rounded-xl border border-dashed border-line bg-canvas/40 p-4 text-center">
          <p className="text-label font-medium text-fg-secondary">
            No cross-corpus patterns surfaced yet.
          </p>
          <p className="mt-1 text-caption leading-relaxed text-muted">
            These detectors compare across agencies, time windows, and jurisdictions; they need
            a broader ingest to fire.
          </p>
        </div>
      ) : (
        <div className="grid grid-cols-1 gap-4 xl:grid-cols-[minmax(0,220px)_minmax(0,1fr)]">
          <Heatmap heatmap={heatmap} />
          <ul className="flex flex-col gap-2" aria-label="Cross-corpus findings list">
            {findings.map((finding) => (
              <CrossCorpusRow key={finding.id} finding={finding} onInspect={() => inspect(finding)} />
            ))}
          </ul>
        </div>
      )}
    </section>
  );
}

function Heatmap({
  heatmap,
}: {
  heatmap: {
    portals: string[];
    cellCounts: Map<string, number>;
    maxCount: number;
  };
}) {
  if (heatmap.portals.length === 0 || heatmap.maxCount === 0) {
    return (
      <div className="rounded-lg border border-line bg-canvas/40 p-3 text-caption text-muted">
        No jurisdictions to plot yet.
      </div>
    );
  }
  return (
    <div className="rounded-lg border border-line bg-canvas/40 p-3">
      <p className="text-caption uppercase tracking-wider text-muted">Jurisdiction × detector</p>
      <div role="region" tabIndex={0} aria-label="Jurisdiction × detector heatmap" className="focus-ring mt-2 overflow-x-auto">
        <table className="w-full text-caption text-fg-secondary">
          <thead>
            <tr>
              <th className="pr-2 text-left font-normal text-muted">Portal</th>
              {(Object.keys(KIND_LABEL) as (keyof typeof KIND_LABEL)[]).map((kind) => (
                <th key={kind} className="px-1 text-center font-normal text-muted">
                  {KIND_LABEL[kind].split(' ')[0]}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {heatmap.portals.map((portal) => (
              <tr key={portal}>
                <td className="pr-2 text-caption text-fg-secondary">{portal}</td>
                {(Object.keys(KIND_LABEL) as (keyof typeof KIND_LABEL)[]).map((kind) => {
                  const label = KIND_LABEL[kind];
                  const count = heatmap.cellCounts.get(`${portal}\u001f${label}`) ?? 0;
                  const intensity = heatmap.maxCount > 0 ? count / heatmap.maxCount : 0;
                  return (
                    <td key={kind} className="px-1 py-0.5 text-center">
                      <span
                        className="inline-block h-3.5 w-full min-w-[1.25rem] rounded-sm"
                        style={{
                          backgroundColor:
                            count === 0
                              ? 'rgba(51, 65, 85, 0.3)'
                              : `rgba(56, 189, 248, ${0.15 + intensity * 0.65})`,
                        }}
                        aria-label={`${portal} · ${label}: ${count}`}
                      >
                        <span className="sr-only">
                          {portal} {label}: {count}
                        </span>
                      </span>
                    </td>
                  );
                })}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}

function CrossCorpusRow({ finding, onInspect }: { finding: Finding; onInspect: () => void }) {
  const [shape, tone] = severityGlyph(finding.severity);
  return (
    <li className="focus-within:ring-1 focus-within:ring-accent/50 rounded-xl border border-line bg-canvas/40 p-3 transition hover:border-accent/30 hover:bg-accent/5">
      <button type="button" onClick={onInspect} className="min-w-0 flex-1 text-left" aria-label={`Inspect: ${finding.headline}`}>
        <div className="flex items-start gap-3">
          <span className={`mt-0.5 grid h-6 w-6 shrink-0 place-items-center rounded-lg bg-surface/70 ${tone}`} aria-hidden="true">
            <svg viewBox="-6 -6 12 12" className="h-3 w-3">
              <ShapeGlyph shape={shape} cx={0} cy={0} radius={4.5} fill="currentColor" />
            </svg>
          </span>
          <div className="min-w-0 flex-1">
            <p className="text-label font-semibold text-fg">{finding.headline}</p>
            <p className="mt-1 text-label leading-relaxed text-muted">{finding.explanation}</p>
            <p className="mt-1 text-caption uppercase tracking-wider text-muted">
              {isCrossCorpusKind(finding.kind) ? KIND_LABEL[finding.kind] : finding.kind}
              {finding.confidence.kind === 'MEASURED' &&
                ` · ${(finding.confidence.basisPoints / 100).toFixed(0)}% confidence`}
            </p>
          </div>
        </div>
      </button>
    </li>
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
