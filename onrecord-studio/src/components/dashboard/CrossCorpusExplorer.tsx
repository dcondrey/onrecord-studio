import { useCallback, useMemo } from 'react';
import { useFindings } from '../../hooks/useFindings';
import { useAppStore } from '../../store/useAppStore';
import { ShapeGlyph } from '../ui/shapeGlyph';
import { severityGlyph } from './findingSeverity';
import type { Finding } from '../../types';

/**
 * Cross-corpus findings panel: the four Wave 8 patterns that span jurisdictions, agencies, and
 * time. Displays as its own section below the ranked FindingsInbox because their subject
 * anchors (jurisdiction pairs, change-points) don't fit the inbox's severity-first ranking as
 * cleanly as a per-request finding does.
 *
 * Rendered as a portal × detector heatmap on the left plus the ranked list on the right.
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

type CrossCorpusFinding = Finding & { kind: CrossCorpusKind };

function isCrossCorpusFinding(finding: Finding): finding is CrossCorpusFinding {
  return (CROSS_CORPUS_KINDS as readonly string[]).includes(finding.kind);
}

function heatmapKey(portal: string, kind: CrossCorpusKind) {
  return `${portal}\u001f${kind}`;
}

export default function CrossCorpusExplorer() {
  const load = useFindings(null, 200);
  const findings = useMemo(() => load.findings.filter(isCrossCorpusFinding), [load.findings]);
  const error = load.status === 'failed' ? load.message : null;
  const isLoading = load.status === 'loading';
  const setActivePortal = useAppStore((state) => state.setActivePortal);
  const setSearchFilters = useAppStore((state) => state.setSearchFilters);
  const setWorkspaceView = useAppStore((state) => state.setWorkspaceView);

  // Findings carry one subject portal, not a second jurisdiction, so the heatmap counts findings
  // per subject portal and detector rather than per jurisdiction pair.
  const heatmap = useMemo(() => {
    const cellCounts = new Map<string, number>();
    let maxCount = 0;
    for (const finding of findings) {
      const key = heatmapKey(finding.subject.portalKey ?? '(no portal)', finding.kind);
      const next = (cellCounts.get(key) ?? 0) + 1;
      cellCounts.set(key, next);
      maxCount = Math.max(maxCount, next);
    }
    const portals = new Set<string>();
    for (const key of cellCounts.keys()) portals.add(key.split('\u001f')[0]);
    return { portals: Array.from(portals).sort(), cellCounts, maxCount };
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

      {findings.length > 0 ? (
        <div className="grid grid-cols-1 gap-4 xl:grid-cols-[minmax(0,220px)_minmax(0,1fr)]">
          <Heatmap heatmap={heatmap} />
          <ul className="flex flex-col gap-2" aria-label="Cross-corpus findings list">
            {findings.map((finding) => (
              <CrossCorpusRow key={finding.id} finding={finding} onInspect={() => inspect(finding)} />
            ))}
          </ul>
        </div>
      ) : isLoading ? (
        <div role="status" className="text-label text-muted">
          Loading cross-corpus findings…
        </div>
      ) : error ? null : (
        <div className="rounded-xl border border-dashed border-line bg-canvas/40 p-4 text-center">
          <p className="text-label font-medium text-fg-secondary">
            No cross-corpus patterns surfaced yet.
          </p>
          <p className="mt-1 text-caption leading-relaxed text-muted">
            These detectors compare across agencies, time windows, and jurisdictions; they need
            a broader ingest to fire.
          </p>
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
  return (
    <div className="rounded-lg border border-line bg-canvas/40 p-3">
      <p className="text-caption uppercase tracking-wider text-muted">Jurisdiction × detector</p>
      <div role="region" tabIndex={0} aria-label="Jurisdiction × detector heatmap" className="focus-ring mt-2 overflow-x-auto">
        <table className="w-full text-caption text-fg-secondary">
          <thead>
            <tr>
              <th className="pr-2 text-left font-normal text-muted">Portal</th>
              {CROSS_CORPUS_KINDS.map((kind) => (
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
                {CROSS_CORPUS_KINDS.map((kind) => {
                  const label = KIND_LABEL[kind];
                  const count = heatmap.cellCounts.get(heatmapKey(portal, kind)) ?? 0;
                  return (
                    <td key={kind} className="px-1 py-0.5 text-center">
                      <span
                        className={`inline-block h-3.5 w-full min-w-[1.25rem] rounded-sm ${count === 0 ? 'bg-line/30' : 'bg-accent'}`}
                        style={count === 0 ? undefined : { opacity: 0.15 + (count / heatmap.maxCount) * 0.65 }}
                      >
                        <span className="sr-only">
                          {portal} · {label}: {count}
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

function CrossCorpusRow({ finding, onInspect }: { finding: CrossCorpusFinding; onInspect: () => void }) {
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
              {KIND_LABEL[finding.kind]}
              {finding.confidence.kind === 'MEASURED' &&
                ` · ${(finding.confidence.basisPoints / 100).toFixed(0)}% confidence`}
            </p>
          </div>
        </div>
      </button>
    </li>
  );
}
