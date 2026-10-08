import { useCallback, useEffect, useRef, useState } from 'react';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import { useAppStore } from '../../store/useAppStore';
import { openHelp } from '../help/helpEvents';
import type { SeriesGap, SeriesGapReport, StandingQueryFilters, StandingQueryOutcome, StandingQuerySummary } from '../../types/standingSearch';
import { describeCountBasis, describeGap, describeRun, formatFacetCount } from '../../utils/standingSearch';
import type { AgencyPerformanceProfile } from '../../types';
import MethodNote from '../ui/MethodNote';
import Panel, { PanelHeader } from '../ui/Panel';
import Button from '../ui/Button';
import Badge from '../ui/Badge';
import InlineError from '../ui/InlineError';
import Skeleton from '../ui/Skeleton';
import Time from '../ui/Time';
import { formatInstant } from '../objects/labels';
import BarTable, { type BarTableRow } from '../charts/figures/BarTable';
import ChartFrame from '../charts/frame/ChartFrame';
import { Axis } from '../charts/frame/Axis';
import { Bar } from '../charts/marks/Bar';
import { AbsentSlot, LABEL_CLASS, MarkGroup, MUTED_LABEL_CLASS } from '../charts/marks/shared';
import { TooltipContent } from '../charts/frame/Tooltip';
import { useTooltip } from '../charts/hooks/useTooltip';
import { useRovingFocus } from '../charts/hooks/useRovingFocus';
import { measureText } from '../charts/core/layout';
import { VIZ } from '../charts/core/claim';
import { absent, measured } from '../charts/core/nulls';
import { scaleBand, scaleLinear } from '../charts/core/scale';
import { formatValue } from '../charts/core/format';

const FACET_LABELS: Record<string, string> = { status: 'Status', agency: 'Department', portal: 'Source', year: 'Year filed' };

/** The executed query in full: the text plus every flat filter, including ones chosen by dropdown. */
function describeFilters(filters: StandingQueryFilters): string {
  return [
    filters.searchTerm.trim() ? `“${filters.searchTerm.trim()}”` : null,
    filters.status ? `status:${filters.status}` : null,
    filters.policyDeviation ? `flag:${filters.policyDeviation}` : null,
    filters.agencyId !== null ? `agency #${filters.agencyId}` : null,
    filters.portalKey ? `source:${filters.portalKey}` : null,
  ].filter((part): part is string => part !== null).join(' ') || '(every request)';
}

/**
 * Saved searches re-run against each new corpus state, shown as breakdowns and clusters before
 * any row, and the series-number absence search. Nothing here is a row list: open a query in the
 * evidence search to read records.
 */
export default function StandingSearchPanel() {
  const refreshRequests = useAppStore((state) => state.findingsRefreshRequests);
  const setSearchFilters = useAppStore((state) => state.setSearchFilters);
  const setActivePortal = useAppStore((state) => state.setActivePortal);
  const setWorkspaceView = useAppStore((state) => state.setWorkspaceView);
  const [summaries, setSummaries] = useState<StandingQuerySummary[] | null>(null);
  const [gaps, setGaps] = useState<SeriesGapReport | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const gate = useLatestRequestGate();
  const lastRunRequest = useRef(0);

  const sync = useCallback(
    async (runDue: boolean) => {
      const request = gate.begin();
      setBusy(true);
      setError(null);
      try {
        let outcomes: StandingQueryOutcome[] = [];
        if (runDue) outcomes = await invokeCommand('run_standing_queries', { queryId: null, force: false });
        const [list, report] = await Promise.all([
          invokeCommand('list_standing_queries', {}),
          invokeCommand('list_series_number_gaps', { limit: 50 }),
        ]);
        if (!gate.isCurrent(request)) return;
        if (!Array.isArray(list)) throw new Error('The native standing query list was invalid.');
        if (!report || !Array.isArray(report.gaps)) throw new Error('The native series gap report was invalid.');
        setSummaries(list);
        setGaps(report);
        const deferred = outcomes.filter((outcome) => outcome.outcome === 'DEFERRED');
        const ran = outcomes.filter((outcome) => outcome.outcome === 'RAN').length;
        setNotice(
          runDue
            ? `${ran} ${ran === 1 ? 'query' : 'queries'} re-run${deferred.length > 0 ? `; ${deferred.length} deferred: ${deferred[0].outcome === 'DEFERRED' ? deferred[0].reason : ''}` : ''}.`
            : null,
        );
      } catch (reason) {
        if (!gate.isCurrent(request)) return;
        setError(formatNativeError(reason));
      } finally {
        if (gate.isCurrent(request)) setBusy(false);
      }
    },
    [gate],
  );

  useEffect(() => {
    const runDue = refreshRequests > 0 && refreshRequests !== lastRunRequest.current;
    lastRunRequest.current = refreshRequests;
    void sync(runDue);
    return gate.invalidate;
  }, [refreshRequests, sync, gate]);

  const remove = async (queryId: string) => {
    setError(null);
    try {
      await invokeCommand('delete_standing_query', { queryId });
      await sync(false);
    } catch (reason) {
      setError(formatNativeError(reason));
    }
  };

  const open = (summary: StandingQuerySummary) => {
    const { searchTerm, agencyId, status, policyDeviation, portalKey } = summary.query.filters;
    setActivePortal(portalKey);
    setSearchFilters(searchTerm, agencyId, status, policyDeviation);
    setWorkspaceView('evidence');
  };

  // A facet value narrows the saved query's own filters in Evidence, when Evidence can filter by it.
  const profiles = useAppStore((state) => state.agencyProfiles);
  const facetFilter = (summary: StandingQuerySummary) => (dimension: string, value: string): (() => void) | null => {
    const filters = summary.query.filters;
    const apply = (portalKey: string | null, agencyId: number | null, status: string | null) => () => {
      setActivePortal(portalKey);
      setSearchFilters(filters.searchTerm, agencyId, status, filters.policyDeviation);
      setWorkspaceView('evidence');
    };
    if (!value) return null;
    if (dimension === 'status') return apply(filters.portalKey, filters.agencyId, value);
    if (dimension === 'portal') return apply(value, filters.agencyId, filters.status);
    if (dimension === 'agency') {
      const agency = agencyNamed(profiles, value, filters.portalKey);
      return agency ? apply(agency.portalSubdomain, agency.agencyId, filters.status) : null;
    }
    return null;
  };

  return (
    <Panel aria-label="Standing searches">
      <PanelHeader
        title="Standing searches"
        description={<>
          Saved queries re-run whenever the corpus changes. Each run names the corpus state it saw.{' '}
          <Button variant="link" className="text-caption" onClick={() => openHelp('standing-search')}>How to read a run</Button>
        </>}
        actions={<Button disabled={busy} onClick={() => void sync(true)}>{busy ? 'Running…' : 'Run due queries'}</Button>}
      />
      <div className="space-y-4 p-5 text-body">
      {notice && <p role="status" className="text-caption text-muted">{notice}</p>}
      {error && <InlineError summary="Standing searches couldn’t be read." detail={error} onRetry={() => void sync(false)} />}

      {summaries === null ? (
        !error && <Skeleton variant="lines" lines={2} label="Reading standing searches" />
      ) : summaries.length === 0 ? (
        <p className="text-muted">None saved. Run a search in the evidence view and choose “Save as standing query”.</p>
      ) : (
        <ul className="space-y-3">
          {summaries.map((summary) => (
            <li key={summary.query.id} className="rounded-lg border border-line p-3">
              <div className="flex flex-wrap items-center gap-2">
                <span className="font-medium text-fg">{summary.query.label || 'Untitled search'}</span>
                <span className="min-w-0 truncate text-caption text-muted">{describeFilters(summary.query.filters)}</span>
                {summary.due && <Badge tone="warning">Corpus changed since last run</Badge>}
                <span className="ml-auto flex gap-1">
                  <Button size="sm" variant="ghost" onClick={() => open(summary)}>Open in evidence</Button>
                  <Button size="sm" variant="ghost" className="text-danger hover:text-danger" onClick={() => void remove(summary.query.id)}>Delete</Button>
                </span>
              </div>
              {summary.lastRun ? (
                <RunView run={summary.lastRun} filter={facetFilter(summary)} />
              ) : (
                <p className="mt-1 text-caption text-muted">Never run.</p>
              )}
            </li>
          ))}
        </ul>
      )}

      <div className="border-t border-line pt-4">
        <h4 className="text-body font-medium text-fg">Missing series numbers</h4>
        <div className="mt-1 text-caption text-muted">
          Gaps in numbered series of resolutions, ordinances, contracts and permits.{' '}
          <MethodNote>
            A gap is a number absent from the corpus between two documents of one labeled series. It is a lead for a request,
            not evidence that the agency withheld anything.
          </MethodNote>
        </div>
        {gaps === null ? null : gaps.gaps.length === 0 ? (
          <p className="mt-2 text-caption text-muted">
            {gaps.seriesCount === 0 ? 'No labeled series with three or more numbers has been read from the corpus.' : `No short gaps in ${gaps.seriesCount} series.`}
          </p>
        ) : (
          <div className="mt-3"><SeriesGapLines report={gaps} /></div>
        )}
      </div>
      </div>
    </Panel>
  );
}

type Run = StandingQuerySummary['lastRun'] & object;
const RUN_SOURCES = { tables: ['standing_query_runs', 'requests'] };
/** A wider year span is listed in the table only: the columns would be slivers. */
const MAX_YEAR_COLUMNS = 40;

function matchHeadline(run: Run): string {
  const count = `${run.result.matchCount.toLocaleString('en-US')} match${run.result.matchCount === 1 ? '' : 'es'}`;
  if (run.previousMatchCount === null) return `${count} (first run).`;
  const delta = run.result.matchCount - run.previousMatchCount;
  return `${count} (${formatValue(delta, { kind: 'count', signed: true })} since the last run, from ${run.previousMatchCount.toLocaleString('en-US')}).`;
}

/** Status, department and source breakdowns: each value's bar is proportional within its facet. */
type FacetFilter = (dimension: string, value: string) => (() => void) | null;

/** The one profile with this department name in the query's source, or null when none or several. */
function agencyNamed(profiles: readonly AgencyPerformanceProfile[], name: string, portalKey: string | null) {
  const found = profiles.filter((profile) => profile.agencyName === name && (portalKey === null || profile.portalSubdomain === portalKey));
  return found.length === 1 ? found[0] : null;
}

function FacetBars({ run, filter }: { run: Run; filter: FacetFilter }) {
  const basis = run.result.countBasis;
  const estimated = basis.basis !== 'EXACT';
  const facets = run.result.facets.filter((facet) => facet.dimension !== 'year');
  const actions = new Map<string, () => void>();
  const rows: BarTableRow[] = facets.flatMap((facet) => facet.values.map((value) => {
    const label = value.value || '(none)';
    const text = formatFacetCount(value.count, basis);
    const action = filter(facet.dimension, value.value);
    if (action) actions.set(`${facet.dimension}\u0000${value.value}`, action);
    const filterNote = action ? 'Enter or click: open these matches in Evidence.' : 'Evidence cannot filter by this value.';
    return {
      key: `${facet.dimension}\u0000${value.value}`,
      name: label,
      label: `${FACET_LABELS[facet.dimension] ?? facet.dimension} ${label}: ${text}`,
      group: { key: facet.dimension, label: FACET_LABELS[facet.dimension] ?? facet.dimension, note: facet.truncated ? 'more values not listed' : undefined },
      header: <span className="line-clamp-2 text-fg-secondary" title={label}>{label}</span>,
      cells: {
        count: {
          value: measured(value.count),
          text,
          approximate: estimated,
          tooltip: { rows: [{ key: 'count', value: text, label: 'matching requests' }], note: estimated ? `${describeCountBasis(basis)}. ${filterNote}` : filterNote },
        },
      },
    };
  }));
  return (
    <BarTable
      title="Matches by facet"
      headline={matchHeadline(run)}
      sources={{ ...RUN_SOURCES, note: describeCountBasis(basis) }}
      summary={`${rows.length} facet values across ${facets.length} facets. Bars are proportional within each facet.${facets.some((facet) => facet.truncated) ? ' Some facets list only their largest values.' : ''}`}
      status={rows.length === 0 ? 'empty' : 'ready'}
      emptyText="This run recorded no breakdown."
      rowHeader="Value"
      columns={[{ key: 'count', label: estimated ? 'Requests (estimated)' : 'Requests', short: estimated ? 'Requests ≈' : undefined, scale: 'own-max', barWidth: 80 }]}
      rows={rows}
      onRowActivate={(key) => actions.get(key)?.()}
      twin={{
        caption: 'Facet counts',
        columns: [{ key: 'facet', label: 'Facet' }, { key: 'value', label: 'Value' }, { key: 'count', label: 'Requests', align: 'end' }],
        rows: facets.flatMap((facet) => [
          ...facet.values.map((value) => ({ facet: FACET_LABELS[facet.dimension] ?? facet.dimension, value: value.value || '(none)', count: formatFacetCount(value.count, basis) })),
          ...(facet.truncated ? [{ facet: FACET_LABELS[facet.dimension] ?? facet.dimension, value: 'More values not listed', count: null }] : []),
        ]),
      }}
      legend={estimated ? [{ key: 'estimated', label: 'Estimated from a sample (≈)', swatch: 'rect', tone: 'context' }] : undefined}
    />
  );
}

/**
 * The 'Year filed' facet over its continuous year range. With an exact, untruncated facet every
 * year in the match set is listed, so an unlisted year is a measured zero; with a truncated or
 * estimated facet it is a 'none listed' slot. Labels sit on the endpoints, the largest year and the
 * hovered or focused column; every value is in the tooltip and the table.
 */
function YearColumns({ run }: { run: Run }) {
  const basis = run.result.countBasis;
  const estimated = basis.basis !== 'EXACT';
  const facet = run.result.facets.find((candidate) => candidate.dimension === 'year');
  const tooltip = useTooltip();
  const [hot, setHot] = useState<number | null>(null);
  const counts = new Map((facet?.values ?? []).flatMap((value) => (/^\d{4}$/.test(value.value) ? [[Number(value.value), value.count] as const] : [])));
  const listed = [...counts.keys()];
  const [first, last] = listed.length ? [Math.min(...listed), Math.max(...listed)] : [0, -1];
  const years = last - first + 1 <= MAX_YEAR_COLUMNS ? Array.from({ length: Math.max(0, last - first + 1) }, (_, i) => first + i) : [...listed].sort((a, b) => a - b);
  const roving = useRovingFocus({ shape: { rows: years.length }, onActivate: () => {}, onEscape: tooltip.hide });
  // A run that did not break matches down by year has no year figure to draw.
  if (!facet) return null;
  const complete = !estimated && !facet.truncated;
  const unplaced = facet.values.filter((value) => !/^\d{4}$/.test(value.value));
  const unlisted = years.filter((year) => !counts.has(year));
  const reason = facet.truncated ? 'No count listed for this year; the facet lists only its largest values' : 'No count listed for this estimated breakdown';
  const countOf = (year: number) => counts.get(year) ?? (complete ? 0 : undefined);
  const textOf = (year: number) => { const count = countOf(year); return count === undefined ? 'none listed' : formatFacetCount(count, basis); };
  const peak = years.reduce<number | null>((best, year) => (best === null || (countOf(year) ?? -1) > (countOf(best) ?? -1) ? year : best), null);
  return (
    <ChartFrame
      title="Year filed"
      headline={years.length ? `Matches filed ${first}–${last}${unlisted.length ? `; ${unlisted.length} ${unlisted.length === 1 ? 'year' : 'years'} with ${complete ? 'no matches' : 'none listed'}` : ''}.` : null}
      sources={{ ...RUN_SOURCES, note: describeCountBasis(basis) }}
      summary={`${years.length} years from ${first} to ${last}. ${unlisted.length ? `${complete ? 'No matches' : 'None listed'} in ${unlisted.join(', ')}.` : ''}${unplaced.length ? ` ${unplaced.length} values are not a year and are listed in the table.` : ''}`}
      status={years.length ? 'ready' : 'empty'}
      emptyText="This run recorded no year breakdown."
      minHeight={140}
      interactive={{ roledescription: 'column chart' }}
      table={{
        caption: 'Matches by year filed',
        columns: [{ key: 'year', label: 'Year filed' }, { key: 'count', label: 'Requests', align: 'end' }],
        rows: [
          ...years.map((year) => ({ year: String(year), count: countOf(year) === undefined ? absent('not-recorded', reason) : textOf(year) })),
          ...unplaced.map((value) => ({ year: value.value || '(none)', count: formatFacetCount(value.count, basis) })),
        ],
      }}
    >
      {({ width, height }) => {
        const band = scaleBand(years.map(String), [0, width]);
        const y = scaleLinear([0, Math.max(1, ...counts.values())], [height - 22, 14], { nice: false });
        // Direct labels: the hovered or focused column first, then the peak and both ends, never touching.
        const placed: [number, number][] = [];
        const labelled = new Set<number>();
        for (const year of [hot === null ? null : years[hot], peak, years[0], years[years.length - 1]]) {
          if (year === null || year === undefined || labelled.has(year) || countOf(year) === undefined) continue;
          const mid = band(String(year)) + band.bandwidth / 2;
          const half = measureText(textOf(year)) / 2 + 3;
          if (placed.some(([a, b]) => mid - half < b && mid + half > a)) continue;
          placed.push([mid - half, mid + half]);
          labelled.add(year);
        }
        return (
          <g>
            <Axis band={band} orient="bottom" offset={height - 22} format={(key) => (years.length > 12 ? `’${key.slice(2)}` : key)} />
            {years.map((year, index) => {
              const pos = band(String(year));
              const count = countOf(year);
              const box = { x: pos, y: 14, width: band.bandwidth, height: height - 36 };
              const name = `${year}: ${count === undefined ? `none listed. ${reason}` : `${textOf(year)} matching ${count === 1 ? 'request' : 'requests'}`}. Evidence search has no year filter.`;
              const showAt = (at: { clientX: number; clientY: number } | DOMRect) => tooltip.show(<TooltipContent title={`Filed ${year}`} rows={count === undefined ? [] : [{ key: 'count', value: textOf(year), label: 'matching requests' }]} note={count === undefined ? reason : estimated ? describeCountBasis(basis) : undefined} />, at);
              return (
                <MarkGroup
                  key={year}
                  box={box}
                  interaction={{
                    name,
                    item: roving.getItemProps(index),
                    onPointerMove: (event) => { setHot(index); showAt({ clientX: event.clientX, clientY: event.clientY }); },
                    onPointerLeave: () => { setHot(null); tooltip.hide(); },
                    onFocusMark: (rect) => { setHot(index); showAt(rect); },
                    onBlurMark: () => { setHot(null); tooltip.hide(); },
                  }}
                >
                  {count === undefined
                    ? <AbsentSlot absence={absent('not-recorded', reason)} box={box} text="none listed" />
                    : count === 0
                      ? <line x1={pos} x2={pos + band.bandwidth} y1={y(0) - 0.75} y2={y(0) - 0.75} style={{ stroke: VIZ.mark, strokeWidth: 1.5 }} />
                      : <Bar orient="vertical" band={{ pos, thickness: band.bandwidth }} value={measured(count)} scale={y} role={estimated ? 'comparison' : 'default'} motionKey={`year:${year}`} />}
                  {labelled.has(year) && count !== undefined && (
                    <text x={pos + band.bandwidth / 2} y={y(count) - 4} textAnchor="middle" className={LABEL_CLASS}>{textOf(year)}</text>
                  )}
                </MarkGroup>
              );
            })}
            {tooltip.node}
          </g>
        );
      }}
    </ChartFrame>
  );
}

const GAP_ROW = 46;

/**
 * One row per numbered series: a neutral number line from the first gap's documented neighbour to
 * the last one's. Solid ticks are the documented members either side of each gap; a hollow bracket
 * spans the missing numbers. The line between gaps makes no claim that those numbers are present.
 */
function SeriesGapLines({ report }: { report: SeriesGapReport }) {
  const tooltip = useTooltip();
  const series = [...new Map(report.gaps.map((gap) => [`${gap.label}\u0000${gap.prefix}`, report.gaps.filter((other) => other.label === gap.label && other.prefix === gap.prefix)])).entries()];
  const flat = series.flatMap(([, gaps]) => gaps);
  const roving = useRovingFocus({ shape: { rows: flat.length }, onActivate: () => {}, onEscape: tooltip.hide });
  const missing = flat.reduce((sum, gap) => sum + gap.toNumber - gap.fromNumber + 1, 0);
  const pad = (gap: SeriesGap, n: number) => `${gap.prefix}${String(n).padStart(gap.width, '0')}`;
  const range = (gap: SeriesGap) => (gap.fromNumber === gap.toNumber ? pad(gap, gap.fromNumber) : `${pad(gap, gap.fromNumber)} to ${pad(gap, gap.toNumber)}`);
  return (
    <ChartFrame
      title="Gaps in numbered series"
      headline={`${missing.toLocaleString('en-US')} ${missing === 1 ? 'number' : 'numbers'} missing across ${flat.length} ${flat.length === 1 ? 'gap' : 'gaps'} in ${report.seriesCount} series${report.truncated ? ' (more gaps exist than are listed)' : ''}.`}
      sources={{ tables: ['corpus_entities', 'corpus_entity_mentions'], note: 'Gaps in numbering are a lead to request, not evidence of withholding' }}
      summary={flat.map(describeGap).join(' ')}
      status="ready"
      minHeight={GAP_ROW}
      plotHeight={() => series.length * GAP_ROW}
      interactive={{ roledescription: 'number lines' }}
      legend={[
        { key: 'member', label: 'documented number (either side of a gap)', swatch: 'rect', tone: 'ink' },
        { key: 'gap', label: 'missing numbers', swatch: 'hollow' },
      ]}
      table={{
        caption: report.truncated ? 'Series gaps (more exist than are listed)' : 'Series gaps',
        columns: [{ key: 'series', label: 'Series' }, { key: 'missing', label: 'Missing' }, { key: 'count', label: 'Numbers', align: 'end' }, { key: 'before', label: 'Before (documents)' }, { key: 'after', label: 'After (documents)' }],
        rows: flat.map((gap) => ({ series: `${gap.label} ${gap.prefix}`.trim(), missing: range(gap), count: gap.toNumber - gap.fromNumber + 1, before: `${gap.before.displayValue} (${gap.before.documentCount})`, after: `${gap.after.displayValue} (${gap.after.documentCount})` })),
      }}
    >
      {({ width }) => {
        let index = 0;
        return (
          <g>
            {series.map(([key, gaps], row) => {
              const lo = Math.min(...gaps.map((gap) => gap.fromNumber - 1));
              const hi = Math.max(...gaps.map((gap) => gap.toNumber + 1));
              const x = scaleLinear([lo - 0.5, hi + 0.5], [8, width - 8], { nice: false, zero: false });
              const top = row * GAP_ROW;
              const lineY = top + 30;
              const labels: [number, number][] = [];
              return (
                <g key={key}>
                  <text x={0} y={top + 12} className="fill-fg text-[12px] font-medium">{`${gaps[0].label} ${gaps[0].prefix}`.trim()}<tspan className={MUTED_LABEL_CLASS}>{` · series of ${gaps[0].seriesMemberCount}`}</tspan></text>
                  <line x1={x(lo)} x2={x(hi)} y1={lineY + 0.5} y2={lineY + 0.5} style={{ stroke: VIZ.grid, strokeWidth: 1 }} aria-hidden="true" />
                  <text x={x(lo)} y={lineY + 14} className={MUTED_LABEL_CLASS} aria-hidden="true">{gaps.find((gap) => gap.fromNumber - 1 === lo)?.before.displayValue}</text>
                  <text x={x(hi)} y={lineY + 14} textAnchor="end" className={MUTED_LABEL_CLASS} aria-hidden="true">{gaps.find((gap) => gap.toNumber + 1 === hi)?.after.displayValue}</text>
                  {gaps.map((gap) => {
                    const i = index++;
                    const x0 = x(gap.fromNumber - 0.5);
                    const x1 = Math.max(x(gap.toNumber + 0.5), x0 + 6);
                    const count = gap.toNumber - gap.fromNumber + 1;
                    const text = `${count} missing`;
                    const mid = (x0 + x1) / 2;
                    const half = measureText(text) / 2 + 3;
                    const showLabel = !labels.some(([a, b]) => mid - half < b && mid + half > a);
                    if (showLabel) labels.push([mid - half, mid + half]);
                    const show = (at: { clientX: number; clientY: number } | DOMRect) => tooltip.show(
                      <TooltipContent title={`${gap.label} ${range(gap)}`} rows={[
                        { key: 'before', value: gap.before.displayValue, label: `before (${gap.before.documentCount} documents)` },
                        { key: 'after', value: gap.after.displayValue, label: `after (${gap.after.documentCount} documents)` },
                      ]} note="A lead to request, not evidence of withholding." />, at);
                    return (
                      <MarkGroup
                        key={`${gap.fromNumber}`}
                        box={{ x: x0, y: lineY - 6, width: x1 - x0, height: 12 }}
                        interaction={{
                          name: `${gap.label} ${range(gap)} missing between ${gap.before.displayValue} and ${gap.after.displayValue}`,
                          item: roving.getItemProps(i),
                          onPointerMove: (event) => show({ clientX: event.clientX, clientY: event.clientY }),
                          onPointerLeave: tooltip.hide,
                          onFocusMark: show,
                          onBlurMark: tooltip.hide,
                        }}
                      >
                        {[x(gap.fromNumber - 1), x(gap.toNumber + 1)].map((at) => <line key={at} x1={at} x2={at} y1={lineY - 6} y2={lineY + 6} style={{ stroke: VIZ.ink, strokeWidth: 2 }} />)}
                        <rect x={x0} y={lineY - 5} width={x1 - x0} height={10} rx={2} style={{ fill: VIZ.surface, stroke: VIZ.ink, strokeWidth: 1.25 }} />
                        {showLabel && <text x={mid} y={lineY - 9} textAnchor="middle" className={LABEL_CLASS}>{text}</text>}
                      </MarkGroup>
                    );
                  })}
                </g>
              );
            })}
            {tooltip.node}
          </g>
        );
      }}
    </ChartFrame>
  );
}

function RunView({ run, filter }: { run: StandingQuerySummary['lastRun'] & object; filter: FacetFilter }) {
  const basis = run.result.countBasis;
  return (
    <div className="mt-2 space-y-2 text-fg-secondary">
      <p className="text-caption tabular-nums text-muted" title={describeRun(run)}>
        Ran <Time value={run.ranAt} format={formatInstant} className="whitespace-nowrap" /> · {run.result.matchCount.toLocaleString()} matching request{run.result.matchCount === 1 ? '' : 's'}
        {' · '}{run.changedSincePrevious === null ? 'first run' : run.changedSincePrevious ? `match set changed from ${run.previousMatchCount?.toLocaleString() ?? 'an unrecorded count'} to ${run.result.matchCount.toLocaleString()}` : 'no change since the previous run'}
      </p>
      <p className="text-caption text-muted">Breakdowns below are {describeCountBasis(basis)}.</p>
      <div className="grid gap-4 lg:grid-cols-2">
        <FacetBars run={run} filter={filter} />
        <YearColumns run={run} />
      </div>
      {run.result.clusters.length > 0 && (
        <div>
          <p className="font-medium text-fg">Identical documents across requests</p>
          <ul className="mt-1 space-y-0.5 text-caption">
            {run.result.clusters.map((cluster) => (
              <li key={cluster.extractedTextSha256}>
                {cluster.exampleFileName || 'Document'}: identical extracted text under {cluster.requestCount} requests ({cluster.documentCount} copies) <span className="font-mono text-code text-muted" title={cluster.extractedTextSha256}>sha256:{cluster.extractedTextSha256.slice(0, 16)}</span>
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}
