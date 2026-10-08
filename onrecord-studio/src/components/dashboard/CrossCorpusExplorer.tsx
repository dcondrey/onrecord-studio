import { useCallback, useMemo, useState } from 'react';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useFindings } from '../../hooks/useFindings';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import { useAppStore } from '../../store/useAppStore';
import Button from '../ui/Button';
import InlineError from '../ui/InlineError';
import Skeleton from '../ui/Skeleton';
import SeverityGlyph from './SeverityGlyph';
import { confidenceLabel } from '../objects/labels';
import type { Finding } from '../../types';
import StateGrid, { type StateGridCell, type StateGridRow } from '../charts/figures/StateGrid';
import { seqStep } from '../charts/marks/HeatCell';

/**
 * Cross-corpus findings panel: the four detector patterns that span jurisdictions, agencies,
 * and time. Displays as its own section below the ranked FindingsInbox because their subject
 * anchors (jurisdiction pairs, change-points) don't fit the inbox's severity-first ranking as
 * cleanly as a per-request finding does.
 *
 * Rendered as a jurisdiction × detector count grid above the ranked list.
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

const NO_JURISDICTION = 'No jurisdiction recorded';

/** Jurisdictions × detectors. Detector names are long next to a count cell, so StateGrid angles or shortens the headers and keeps each full name in its title. */
export function jurisdictionGrid(findings: readonly CrossCorpusFinding[]) {
  const jurisdictions = [...new Set(findings.map((finding) => finding.subject.portalKey ?? NO_JURISDICTION))];
  const count = (jurisdiction: string, kind: CrossCorpusKind) => findings.filter((finding) => (finding.subject.portalKey ?? NO_JURISDICTION) === jurisdiction && finding.kind === kind).length;
  const total = (jurisdiction: string) => CROSS_CORPUS_KINDS.reduce((sum, kind) => sum + count(jurisdiction, kind), 0);
  jurisdictions.sort((a, b) => total(b) - total(a) || a.localeCompare(b));
  const max = Math.max(0, ...jurisdictions.flatMap((jurisdiction) => CROSS_CORPUS_KINDS.map((kind) => count(jurisdiction, kind))));
  const rows: StateGridRow[] = jurisdictions.map((jurisdiction) => ({
    key: jurisdiction,
    label: jurisdiction,
    trailing: { text: total(jurisdiction).toLocaleString('en-US'), value: total(jurisdiction), max: Math.max(...jurisdictions.map(total)) },
    cells: CROSS_CORPUS_KINDS.map((kind): StateGridCell => {
      const n = count(jurisdiction, kind);
      const name = `${jurisdiction}, ${KIND_LABEL[kind].toLowerCase()}: ${n} ${n === 1 ? 'finding' : 'findings'}.`;
      return n === 0
        ? { kind: 'state', tone: 'quiet', glyph: 'bar', text: '0', legend: 'None found by this detector', name }
        : { kind: 'count', value: n, step: seqStep(n, max), glyph: 'circle', text: n.toLocaleString('en-US'), name: `${name} Press Enter to list them.`, actionable: true };
    }),
  }));
  const byKind = CROSS_CORPUS_KINDS.map((kind) => ({ kind, n: findings.filter((finding) => finding.kind === kind).length })).sort((a, b) => b.n - a.n);
  const headline = `${findings.length} cross-corpus ${findings.length === 1 ? 'finding' : 'findings'} across ${jurisdictions.length} ${jurisdictions.length === 1 ? 'jurisdiction' : 'jurisdictions'}; most from ${KIND_LABEL[byKind[0].kind].toLowerCase()} (${byKind[0].n}).`;
  return { rows, jurisdictions, headline };
}

const FIRST_PAGE = 200;

function isCrossCorpusFinding(finding: Finding): finding is CrossCorpusFinding {
  return (CROSS_CORPUS_KINDS as readonly string[]).includes(finding.kind);
}

export default function CrossCorpusExplorer() {
  const [limit, setLimit] = useState(FIRST_PAGE);
  const load = useFindings(null, limit);
  const findings = useMemo(() => load.findings.filter(isCrossCorpusFinding), [load.findings]);
  // A full page may stop short of later findings, so its per-cell counts are not totals and a 0
  // could be an unloaded cell: the grid waits until every finding is loaded.
  const truncated = load.findings.length >= limit;
  const countGate = useLatestRequestGate();
  const [countError, setCountError] = useState<string | null>(null);
  const [counting, setCounting] = useState(false);
  const loadAll = () => {
    const request = countGate.begin();
    setCounting(true);
    setCountError(null);
    invokeCommand('count_findings', { filter: null })
      .then((count) => {
        if (countGate.isCurrent(request)) setLimit((current) => Math.max(current + 1, count));
      })
      .catch((reason) => {
        if (countGate.isCurrent(request)) setCountError(formatNativeError(reason));
      })
      .finally(() => {
        if (countGate.isCurrent(request)) setCounting(false);
      });
  };
  const error = load.status === 'failed' ? load.message : null;
  const isLoading = load.status === 'loading';
  const setActivePortal = useAppStore((state) => state.setActivePortal);
  const setSearchFilters = useAppStore((state) => state.setSearchFilters);
  const setWorkspaceView = useAppStore((state) => state.setWorkspaceView);
  const [cellFilter, setCellFilter] = useState<{ jurisdiction: string; kind: CrossCorpusKind } | null>(null);
  const grid = useMemo(() => jurisdictionGrid(findings), [findings]);
  const shown = cellFilter
    ? findings.filter((finding) => (finding.subject.portalKey ?? NO_JURISDICTION) === cellFilter.jurisdiction && finding.kind === cellFilter.kind)
    : findings;

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
          {truncated ? (
            <div className="flex flex-wrap items-center gap-2 text-caption text-muted">
              <span>Counts by jurisdiction need every finding: the first {load.findings.length.toLocaleString('en-US')} loaded are not totals.</span>
              <Button size="sm" disabled={counting || isLoading} onClick={loadAll}>Load all to compare</Button>
              {countError && <span role="alert" className="text-warning">The total could not be read: {countError}</span>}
            </div>
          ) : (
          <StateGrid
            title="Cross-corpus findings by jurisdiction"
            storageKey="cross-corpus-jurisdictions"
            headline={grid.headline}
            sources={{ tables: ['findings'], note: 'cross-corpus detectors only, over every stored finding; counts are detector output, not legal findings' }}
            summary={`${findings.length} findings in ${grid.rows.length} jurisdictions by ${CROSS_CORPUS_KINDS.length} detectors. Activate a cell to list its findings.`}
            status="ready"
            rowHeader="Jurisdiction"
            trailingHeader="Total"
            columns={CROSS_CORPUS_KINDS.map((kind) => ({ key: kind, label: KIND_LABEL[kind] }))}
            rows={grid.rows}
            selected={cellFilter ? [{ row: grid.jurisdictions.indexOf(cellFilter.jurisdiction), col: CROSS_CORPUS_KINDS.indexOf(cellFilter.kind) }] : []}
            onActivate={(row, col) => setCellFilter({ jurisdiction: grid.jurisdictions[row], kind: CROSS_CORPUS_KINDS[col] })}
            onEscape={() => setCellFilter(null)}
            legend={[{ key: 'scale', label: 'Findings per cell: the stronger the shade, the more findings', swatch: 'rect', tone: 'mark' }]}
          />
          )}
          {cellFilter && (
            <p className="text-caption text-muted">
              Showing {KIND_LABEL[cellFilter.kind].toLowerCase()} in {cellFilter.jurisdiction}.{' '}
              <button type="button" className="focus-ring rounded text-accent underline" onClick={() => setCellFilter(null)}>Show every cell</button>
            </p>
          )}
          <ul className="flex flex-col gap-2" aria-label="Cross-corpus findings list">
            {shown.map((finding) => (
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
