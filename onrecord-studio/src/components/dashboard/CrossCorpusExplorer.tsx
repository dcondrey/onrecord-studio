import { useCallback, useMemo } from 'react';
import { useFindings } from '../../hooks/useFindings';
import { useAppStore } from '../../store/useAppStore';
import Badge from '../ui/Badge';
import InlineError from '../ui/InlineError';
import Skeleton from '../ui/Skeleton';
import SeverityGlyph from './SeverityGlyph';
import { confidenceLabel } from '../objects/labels';
import type { Finding } from '../../types';

/**
 * Cross-corpus findings panel: the four detector patterns that span jurisdictions, agencies,
 * and time. Displays as its own section below the ranked FindingsInbox because their subject
 * anchors (jurisdiction pairs, change-points) don't fit the inbox's severity-first ranking as
 * cleanly as a per-request finding does.
 *
 * Rendered as a count per detector above the ranked list.
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

export default function CrossCorpusExplorer() {
  const load = useFindings(null, 200);
  const findings = useMemo(() => load.findings.filter(isCrossCorpusFinding), [load.findings]);
  const error = load.status === 'failed' ? load.message : null;
  const isLoading = load.status === 'loading';
  const setActivePortal = useAppStore((state) => state.setActivePortal);
  const setSearchFilters = useAppStore((state) => state.setSearchFilters);
  const setWorkspaceView = useAppStore((state) => state.setWorkspaceView);

  const counts = useMemo(
    () => CROSS_CORPUS_KINDS
      .map((kind) => ({ kind, count: findings.filter((finding) => finding.kind === kind).length }))
      .filter(({ count }) => count > 0),
    [findings],
  );

  const inspect = useCallback(
    (finding: Finding) => {
      if (finding.subject.portalKey) setActivePortal(finding.subject.portalKey);
      setSearchFilters('', finding.subject.agencyId ?? null, null, null);
      setWorkspaceView('evidence');
    },
    [setActivePortal, setSearchFilters, setWorkspaceView],
  );

  return (
    <section className="flex flex-col gap-3" aria-label="Cross-corpus findings">
      <p className="max-w-prose text-caption text-muted">
        Coordinated language, behavioral change-points, orphan topics and reused denial grounds across agencies and time.
      </p>

      {error && <InlineError summary="Cross-corpus findings could not be read." detail={error} />}

      {findings.length > 0 ? (
        <>
          <ul className="flex flex-wrap gap-1.5" aria-label="Findings by detector">
            {counts.map(({ kind, count }) => (
              <li key={kind}><Badge tone="neutral" className="tabular-nums">{KIND_LABEL[kind]}: {count}</Badge></li>
            ))}
          </ul>
          <ul className="flex flex-col gap-2" aria-label="Cross-corpus findings list">
            {findings.map((finding) => (
              <CrossCorpusRow key={finding.id} finding={finding} onInspect={() => inspect(finding)} />
            ))}
          </ul>
        </>
      ) : isLoading ? (
        <Skeleton variant="lines" lines={3} label="Loading cross-corpus findings" />
      ) : error ? null : (
        <div className="rounded-lg border border-dashed border-line p-4 text-center">
          <p className="text-heading text-fg-secondary">No cross-corpus patterns surfaced yet</p>
          <p className="mt-1 text-caption text-muted">
            These detectors compare across agencies, time windows and jurisdictions, so they need a broader collection to fire.
          </p>
        </div>
      )}
    </section>
  );
}

function CrossCorpusRow({ finding, onInspect }: { finding: CrossCorpusFinding; onInspect: () => void }) {
  return (
    <li>
      <button
        type="button"
        onClick={onInspect}
        aria-label={`Inspect: ${finding.headline}`}
        className="flex w-full items-start gap-3 rounded-lg border border-line p-3 text-left transition-colors hover:border-line-strong hover:bg-raised/40"
      >
        <SeverityGlyph severity={finding.severity} className="mt-1" />
        <span className="min-w-0 flex-1">
          <span className="block text-control text-fg">{finding.headline}</span>
          <span className="mt-1 block text-body text-muted">{finding.explanation}</span>
          <span className="mt-1 block text-caption text-muted">
            {KIND_LABEL[finding.kind]} · {confidenceLabel(finding.confidence)}
          </span>
        </span>
      </button>
    </li>
  );
}
