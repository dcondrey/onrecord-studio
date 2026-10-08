import { Fragment, useId, useMemo, useRef, useState, type ReactNode } from 'react';
import { cn } from '../../../utils/cn';
import Badge from '../../ui/Badge';
import Button from '../../ui/Button';
import AnalyticsUnavailable from '../AnalyticsUnavailable';
import { CLAIM_TAG, VIZ, type Claim } from '../core/claim';
import { absenceLegend, isAbsence, isValue, type AbsenceState, type Measured } from '../core/nulls';
import { scaleLinear } from '../core/scale';
import { Legend, type LegendItem } from '../frame/Legend';
import { SourceCaption, type ChartSources } from '../frame/SourceCaption';
import { TableTwin, type TwinTable } from '../frame/TableTwin';
import { TooltipContent, type TooltipRow } from '../frame/Tooltip';
import { HatchProvider, VizDefs, useHatchFill, useSvgId } from '../frame/VizDefs';
import { ChartMotionContext, useChartMotion } from '../hooks/useChartMotion';
import { useChartSize } from '../hooks/useChartSize';
import { useDrillDown } from '../hooks/useDrillDown';
import { useRovingFocus } from '../hooks/useRovingFocus';
import { useTooltip } from '../hooks/useTooltip';
import { Bar } from '../marks/Bar';
import { AbsentSlot } from '../marks/shared';
import type { ChartStatus } from '../frame/ChartFrame';

/**
 * - own-0-100: values are fractions, each on its own 0–100% track.
 * - shared: every 'shared' column draws on one scale, 0..`sharedMax`, so equal points are equal length.
 * - own-max: 0..the column's largest value.
 */
export type BarScaleKind = 'own-0-100' | 'shared' | 'own-max';

export interface BarTableColumn {
  key: string;
  label: string;
  /** A second header line: 'not a disposition', 'pattern signal'. */
  note?: string;
  /** The header's full wording, as its tooltip. */
  title?: string;
  scale: BarScaleKind;
  /**
   * Real records outside the measure (an absence count): the bar is the excluded hatch, never a
   * solid value bar.
   */
  hatched?: boolean;
  /**
   * A counted legal measure: the header carries the flag. The bars stay one slate ink, because
   * position in a column already names what each bar measures.
   */
  claim?: Claim;
  /** The scale's top, overriding the column's own maximum: fixed so rows never rescale on filter. */
  max?: number;
  sortable?: boolean;
  /** Bar length at full scale, px. Default 48. */
  barWidth?: number;
  /** Text for a row with no cell in this column; its tooltip says why. */
  missing?: { text: string; title: string };
}

export interface BarTableCell {
  /** The bar's value in the column's units: a fraction for own-0-100. */
  value: Measured;
  /** The visible value text; the bar beside it is decoration. */
  text: string;
  /** Hatched span from the bar's end: the missing share of a completeness bar. */
  remainder?: number;
  /** An estimate (ESTIMATED_FROM_SAMPLE): the context tone, and '≈' in the text. */
  approximate?: boolean;
  tooltip?: { rows: TooltipRow[]; note?: string };
  /** Activation (click or Enter) does something for this cell. */
  active?: boolean;
}

export interface BarTableRow {
  key: string;
  /** Plain-text row name for accessible names and tooltips. */
  name: string;
  /** The first column's content: the name, with any link or chip. */
  header: ReactNode;
  cells: Partial<Record<string, BarTableCell>>;
  /** Too few records for shares to be stable: drawn at 40% with the chip the header carries. */
  dimmed?: boolean;
  /**
   * Rows sharing a group sit under one group header, and an 'own-max' column scales within the
   * group: a breakdown's values are proportional within their own facet.
   */
  group?: { key: string; label: string; note?: string };
  /** The row's accessible label (rank, name and its values as one sentence). */
  label?: string;
}

export interface BarTableProps {
  title: string;
  headline: string | null;
  claim?: Claim | 'mixed';
  sources: ChartSources;
  summary: string;
  status: ChartStatus;
  error?: string | null;
  emptyText?: string;
  rowHeader: string;
  /** Header cells between the row header and the bar columns ('Requests', 'Score'). */
  columns: BarTableColumn[];
  rows: BarTableRow[];
  sharedMax?: number;
  sort?: { key: string; direction: 'ascending' | 'descending' };
  onSort?(key: string): void;
  /** Expand in place: one row (`row`) or one cell's records (`cell`) at a time. */
  drill?: { mode: 'row' | 'cell'; render(rowKey: string, columnKey: string | null): ReactNode };
  /** Without `drill`: a click or Enter on a row's cell (opening its records). */
  onRowActivate?(rowKey: string): void;
  /** The 'View as table' body: a copyable twin, or the exact-values table this figure replaced. */
  twin: TwinTable | { node: ReactNode };
  twinLabel?: string;
  legend?: LegendItem[];
  controls?: ReactNode;
  footer?: ReactNode;
  /** 'stacked' sets each value above its bar, for tables with many narrow columns. */
  layout?: 'inline' | 'stacked';
  /** What the hatch stands for here ('Missing values'), when 'Outside this measure' is too general. */
  hatchLabel?: string;
  className?: string;
}

const BAR_HEIGHT = 8;
const SVG_HEIGHT = 12;

function columnMax(column: BarTableColumn, rows: readonly BarTableRow[], sharedMax: number | undefined): number {
  if (column.max !== undefined) return column.max;
  if (column.scale === 'own-0-100') return 1;
  if (column.scale === 'shared' && sharedMax !== undefined) return sharedMax;
  let max = 0;
  for (const row of rows) {
    const cell = row.cells[column.key];
    if (cell && isValue(cell.value)) max = Math.max(max, cell.value.value);
  }
  return max;
}

function absenceStatesOf(rows: readonly BarTableRow[], columns: readonly BarTableColumn[]): AbsenceState[] {
  const states = new Set<AbsenceState>();
  for (const row of rows) for (const column of columns) {
    const cell = row.cells[column.key];
    if (cell && isAbsence(cell.value)) states.add(cell.value.state);
    if (column.hatched && cell) states.add('excluded');
    if (cell?.remainder) states.add('excluded');
  }
  return [...states];
}

/** The hatched span of excluded records: never a solid value bar, never on the value axis. */
function HatchedSpan({ x0, x1 }: { x0: number; x1: number }) {
  const hatch = useHatchFill();
  if (x1 - x0 < 0.5) return null;
  return <rect data-absence="excluded" x={x0 + 0.5} y={(SVG_HEIGHT - BAR_HEIGHT) / 2 + 0.5} width={x1 - x0 - 1} height={BAR_HEIGHT - 1} rx={2} style={{ fill: hatch, stroke: VIZ.baseline, strokeWidth: 1 }} />;
}

function columnMin(column: BarTableColumn, rows: readonly BarTableRow[]): number {
  if (column.scale === 'own-0-100') return 0;
  let min = 0;
  for (const row of rows) {
    const cell = row.cells[column.key];
    if (cell && isValue(cell.value)) min = Math.min(min, cell.value.value);
  }
  return min;
}

function CellBar({ column, cell, min, max, motionKey }: { column: BarTableColumn; cell: BarTableCell; min: number; max: number; motionKey: string }) {
  const width = column.barWidth ?? 48;
  // A negative value (a sum can be) draws left of the zero line, inside the same track.
  const x = scaleLinear([min, max > min ? max : min + 1], [0, width], { nice: false });
  const band = { pos: (SVG_HEIGHT - BAR_HEIGHT) / 2, thickness: BAR_HEIGHT };
  const value = cell.value;
  let mark: ReactNode;
  if (!isValue(value)) {
    mark = <AbsentSlot absence={value} box={{ x: 0, y: band.pos, width, height: BAR_HEIGHT }} text="" />;
  } else if (value.value === 0) {
    mark = <line x1={x(0) + 0.75} x2={x(0) + 0.75} y1={band.pos - 1} y2={band.pos + BAR_HEIGHT + 1} style={{ stroke: VIZ.mark, strokeWidth: 1.5 }} />;
  } else if (column.hatched) {
    mark = <HatchedSpan x0={0} x1={x(value.value)} />;
  } else {
    mark = <Bar band={band} value={value} scale={x} claim="descriptive" role={cell.approximate ? 'comparison' : 'default'} motionKey={motionKey} />;
  }
  const end = isValue(value) ? x(Math.max(0, value.value)) : 0;
  return (
    <svg width={width + 1} height={SVG_HEIGHT} aria-hidden="true" className="block shrink-0 overflow-visible">
      <rect x={0} y={band.pos} width={width} height={BAR_HEIGHT} rx={2} style={{ fill: VIZ.track }} />
      {cell.remainder ? <HatchedSpan x0={end} x1={Math.min(width, end + x(cell.remainder))} /> : null}
      {mark}
    </svg>
  );
}

function ClaimTags({ claim }: { claim: BarTableProps['claim'] }) {
  if (!claim || claim === 'descriptive') return null;
  return (
    <span className="flex flex-wrap gap-1.5">
      {(claim === 'counted' || claim === 'mixed') && <Badge tone="violation">{CLAIM_TAG.counted}</Badge>}
      {(claim === 'statistical' || claim === 'mixed') && <Badge dashed>{CLAIM_TAG.statistical}</Badge>}
    </span>
  );
}

/**
 * A ranked table whose numeric columns each carry an inline bar: the real <table> is the figure,
 * every value is visible text, and the bar beside it is aria-hidden decoration. Absence columns
 * draw the excluded hatch; a not-recorded value draws a dashed slot, never a zero bar. Arrow keys
 * move a roving focus across cells; Enter (or a click) expands a row or a cell in place.
 */
export default function BarTable(props: BarTableProps) {
  const { title, headline, claim, sources, summary, status, columns, rows, sort, drill, className } = props;
  const titleId = useId();
  const summaryId = useId();
  const hatchId = useSvgId('viz-hatch');
  const motion = useChartMotion();
  const tooltip = useTooltip();
  const drillDown = useDrillDown();
  const [view, setView] = useState<'chart' | 'table'>('chart');
  const showsData = status === 'ready' || status === 'stale';
  const frameRef = useRef<HTMLElement>(null);
  const { width } = useChartSize(frameRef, { fallbackWidth: 1024 });
  // Too narrow for a name column beside the values: each row's name moves to a line above them.
  const narrow = width > 0 && width < 160 + columns.length * (props.layout === 'stacked' ? 70 : 110);
  const span = columns.length + (narrow ? 0 : 1);
  const maxes = useMemo(() => {
    const byGroup = new Map<string, number[]>();
    for (const row of rows) {
      const key = row.group?.key ?? '';
      if (!byGroup.has(key)) byGroup.set(key, columns.map((column) => columnMax(column, rows.filter((other) => (other.group?.key ?? '') === key), props.sharedMax)));
    }
    return byGroup;
  }, [columns, rows, props.sharedMax]);
  const mins = useMemo(() => columns.map((column) => columnMin(column, rows)), [columns, rows]);
  const legend = [...(props.legend ?? []), ...absenceLegend(absenceStatesOf(rows, columns)).map((item) => (item.key === 'absence-excluded' && props.hatchLabel ? { ...item, label: props.hatchLabel } : item))];

  const keyOf = (rowKey: string, columnKey: string | null) => (drill?.mode === 'cell' && columnKey ? `${rowKey}\u0000${columnKey}` : rowKey);
  const activate = (i: number, j: number | undefined) => {
    const row = rows[i];
    const column = j === undefined ? null : columns[j];
    if (!row) return;
    if (!drill) {
      props.onRowActivate?.(row.key);
      return;
    }
    if (drill.mode === 'cell') {
      if (!column || !row.cells[column.key]?.active) return;
      drillDown.toggle(keyOf(row.key, column.key));
    } else {
      drillDown.toggle(row.key);
    }
  };
  const roving = useRovingFocus({ shape: { rows: rows.length, cols: columns.length }, onActivate: activate, onEscape: tooltip.hide });

  let body: ReactNode;
  if (status === 'loading') {
    body = <p className="px-1 py-6 text-body text-muted">Loading…</p>;
  } else if (status === 'empty') {
    body = <p className="px-1 py-6 text-body text-muted">{props.emptyText ?? 'Nothing to show for this selection.'}</p>;
  } else if (status === 'error') {
    body = <AnalyticsUnavailable heading="This chart could not be drawn" detail={props.error ?? undefined} />;
  } else if (status === 'scoped-out') {
    body = <p className="px-1 py-6 text-body text-muted">Hidden while a case scope is active.</p>;
  } else if (view === 'table') {
    body = 'node' in props.twin ? props.twin.node : <TableTwin table={props.twin} />;
  } else {
    body = (
      <div className="overflow-x-auto">
        <table aria-labelledby={titleId} className="w-full border-collapse text-body">
          <thead>
            <tr>
              {!narrow && <th scope="col" className="border-b border-line px-2 pb-2 text-left align-bottom text-caption font-medium text-fg-secondary">{props.rowHeader}</th>}
              {columns.map((column) => {
                const sorted = sort?.key === column.key;
                const label = (
                  <span className="grid justify-items-start gap-0.5 text-left">
                    <span className="flex items-center gap-1">
                      {column.claim === 'counted' && <span aria-hidden="true" className="text-[rgb(var(--violation))]">§</span>}
                      <span>{column.label}</span>
                      {sorted && <span aria-hidden="true">{sort?.direction === 'ascending' ? '↑' : '↓'}</span>}
                    </span>
                    {column.note && <span className="text-caption normal-case tracking-normal text-muted">{column.note}</span>}
                  </span>
                );
                return (
                  <th
                    key={column.key}
                    scope="col"
                    title={column.title}
                    aria-sort={column.sortable ? (sorted ? sort?.direction : 'none') : undefined}
                    className={cn('border-b border-line pb-2 align-bottom text-caption font-medium leading-4 text-fg-secondary', props.layout === 'stacked' ? 'min-w-[3rem] px-1.5' : 'min-w-[4.5rem] px-2')}
                  >
                    {column.sortable && props.onSort ? (
                      <button type="button" className="focus-ring rounded text-left hover:text-fg" onClick={() => props.onSort?.(column.key)}>{label}</button>
                    ) : label}
                  </th>
                );
              })}
            </tr>
          </thead>
          <tbody>
            {rows.map((row, i) => {
              const rowOpen = drill?.mode === 'row' && drillDown.expanded === row.key;
              const openColumn = drill?.mode === 'cell' ? columns.find((column) => drillDown.expanded === keyOf(row.key, column.key)) : undefined;
              const expansionKey = drill?.mode === 'row' ? row.key : openColumn ? keyOf(row.key, openColumn.key) : null;
              return (
                <Fragment key={row.key}>
                  {row.group && row.group.key !== rows[i - 1]?.group?.key && (
                    <tr>
                      <th scope="colgroup" colSpan={span} className="border-b border-line px-2 pb-1 pt-3 text-left text-caption font-medium text-fg">
                        {row.group.label}
                        {row.group.note && <span className="ml-2 font-normal text-muted">{row.group.note}</span>}
                      </th>
                    </tr>
                  )}
                  {narrow && (
                    <tr>
                      <th scope="row" colSpan={span} className="px-2 pt-2 text-left font-normal">
                        {row.header}
                        {drill?.mode === 'row' && (
                          <button type="button" {...drillDown.buttonProps(row.key)} className="focus-ring mt-1 block rounded text-caption text-muted hover:text-fg">
                            {rowOpen ? 'Hide details' : 'Details'}
                          </button>
                        )}
                      </th>
                    </tr>
                  )}
                  <tr
                    aria-label={row.label}
                    {...(drill?.mode === 'row' ? drillDown.rowProps(row.key) : {})}
                    className={cn(
                      'group border-b border-line align-middle hover:bg-raised/50',
                      rowOpen && 'bg-raised/40 outline outline-2 -outline-offset-2 outline-focus',
                    )}
                  >
                    {!narrow && (
                      <th scope="row" className="min-w-[10rem] px-2 py-2 text-left font-normal">
                        {row.header}
                        {drill?.mode === 'row' && (
                          <button type="button" {...drillDown.buttonProps(row.key)} className="focus-ring mt-1 block rounded text-caption text-muted hover:text-fg">
                            {rowOpen ? 'Hide details' : 'Details'}
                          </button>
                        )}
                      </th>
                    )}
                    {columns.map((column, j) => {
                      const cell = row.cells[column.key];
                      const item = roving.getItemProps(i, j);
                      const cellOpen = openColumn?.key === column.key;
                      const show = (target: Element | null, at?: { clientX: number; clientY: number }) => {
                        if (!cell?.tooltip) return;
                        const content = <TooltipContent title={`${row.name} · ${column.label}`} rows={cell.tooltip.rows} note={cell.tooltip.note} />;
                        tooltip.show(content, at ?? target?.getBoundingClientRect() ?? { clientX: 0, clientY: 0 });
                      };
                      return (
                        <td
                          key={column.key}
                          ref={item.ref}
                          tabIndex={item.tabIndex}
                          data-focused={item['data-focused']}
                          aria-expanded={drill?.mode === 'cell' && cell?.active ? cellOpen : undefined}
                          title={!cell && column.missing ? column.missing.title : undefined}
                          onKeyDown={item.onKeyDown}
                          onFocus={(event) => {
                            item.onFocus();
                            show(event.currentTarget);
                          }}
                          onBlur={() => {
                            item.onBlur();
                            tooltip.hide();
                          }}
                          onPointerMove={(event) => show(null, { clientX: event.clientX, clientY: event.clientY })}
                          onPointerLeave={tooltip.hide}
                          onClick={() => activate(i, j)}
                          className={cn(
                            props.layout === 'stacked' ? 'px-1.5' : 'px-2', 'py-2 align-middle outline-none focus-visible:outline focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-focus',
                            cell?.active && drill?.mode === 'cell' && 'cursor-pointer',
                            (drill?.mode === 'row' || (!drill && props.onRowActivate)) && 'cursor-pointer',
                            cellOpen && 'bg-raised/60',
                            row.dimmed && 'opacity-40',
                          )}
                        >
                          {cell ? (
                            <span className={cn('flex', props.layout === 'stacked' ? 'flex-col-reverse items-start gap-1' : 'items-center gap-1.5')}>
                              <CellBar column={column} cell={cell} min={mins[j]} max={maxes.get(row.group?.key ?? '')?.[j] ?? 0} motionKey={`${row.key}:${column.key}`} />
                              <span className={cn('viz-num whitespace-nowrap text-caption', isValue(cell.value) ? 'text-fg-secondary' : 'text-muted', cellOpen && 'font-semibold text-fg')}>{cell.text}</span>
                            </span>
                          ) : (
                            <span className="text-caption text-muted">{column.missing?.text ?? ''}</span>
                          )}
                        </td>
                      );
                    })}
                  </tr>
                  {drill && expansionKey && (
                    <tr>
                      <td colSpan={span} className={cn('p-0', drillDown.expanded === expansionKey && 'border-b border-line')}>
                        <div {...drillDown.regionProps(expansionKey)}>
                          {/* A collapsed region stays mounted so it can animate open, but out of reach. */}
                          <div className="min-h-0 overflow-hidden" {...(drillDown.expanded === expansionKey ? {} : { inert: '', 'aria-hidden': true })}>
                            <div className="bg-raised/40 px-3 py-3">{drill.render(row.key, openColumn?.key ?? null)}</div>
                          </div>
                        </div>
                      </td>
                    </tr>
                  )}
                </Fragment>
              );
            })}
          </tbody>
        </table>
      </div>
    );
  }

  return (
    <figure ref={frameRef} aria-labelledby={titleId} aria-describedby={summaryId} className={cn('m-0 grid min-w-0 gap-3', className)}>
      <svg width={0} height={0} aria-hidden="true" className="absolute">
        <VizDefs hatchId={hatchId} />
      </svg>
      <figcaption className="grid gap-1">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <div className="flex min-w-0 flex-wrap items-center gap-2">
            <h3 id={titleId} className="text-heading text-fg">{title}</h3>
            <ClaimTags claim={claim} />
          </div>
          {showsData && (
            <Button size="sm" variant="ghost" aria-pressed={view === 'table'} onClick={() => setView(view === 'chart' ? 'table' : 'chart')}>
              {props.twinLabel ?? 'View as table'}
            </Button>
          )}
        </div>
        {headline && <p className="text-headline text-fg">{headline}</p>}
      </figcaption>
      {props.controls && <div className="flex flex-wrap items-center gap-2">{props.controls}</div>}
      {status === 'stale' && (
        <p role="status" className="text-caption text-warning">Controls changed. The chart still shows the previous result until it is run again.</p>
      )}
      <div className="min-w-0" style={{ opacity: status === 'stale' ? 0.5 : undefined }}>
        <HatchProvider value={hatchId}>
          <ChartMotionContext.Provider value={motion}>{body}</ChartMotionContext.Provider>
        </HatchProvider>
      </div>
      <p id={summaryId} className="sr-only">{summary}</p>
      {showsData && view === 'chart' && props.footer}
      {showsData && legend.length > 0 && view === 'chart' && <Legend items={legend} />}
      <SourceCaption sources={sources} />
      {tooltip.node}
    </figure>
  );
}
