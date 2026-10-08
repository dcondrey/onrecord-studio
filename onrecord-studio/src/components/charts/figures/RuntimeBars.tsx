import { ShapeGlyph } from '../../ui/shapeGlyph';
import type { PortalRuntimeSnapshot } from '../../../types';
import { VIZ } from '../core/claim';
import { measureText } from '../core/layout';
import { measured } from '../core/nulls';
import { scaleLinear } from '../core/scale';
import ChartFrame from '../frame/ChartFrame';
import type { LegendItem } from '../frame/Legend';
import { TooltipContent } from '../frame/Tooltip';
import type { TwinTable } from '../frame/TableTwin';
import { useRovingFocus } from '../hooks/useRovingFocus';
import { useTooltip } from '../hooks/useTooltip';
import { Bar, StackedBar } from '../marks/Bar';
import { LABEL_CLASS, MarkGroup, MUTED_LABEL_CLASS } from '../marks/shared';

export interface RuntimeRow {
  key: string;
  label: string;
  /** 'Running' or 'Idle', printed after the name. */
  state?: string;
  /** No snapshot means the source has not run this session: words, never an empty bar. */
  runtime: Pick<PortalRuntimeSnapshot, 'completedTasks' | 'failedTasks' | 'activeTasks' | 'queuedTasks' | 'frontierExclusions' | 'stoppedHosts'> | null;
}

export const NOT_RUN = 'not run this session';
const NOT_COLLECTED_LINES = 3;
const BAND = 14;

const count = (n: number) => n.toLocaleString('en-US');
const plural = (n: number, one: string, many = `${one}s`) => `${count(n)} ${n === 1 ? one : many}`;

function total(runtime: NonNullable<RuntimeRow['runtime']>): number {
  return runtime.completedTasks + runtime.failedTasks + runtime.activeTasks + runtime.queuedTasks;
}

/** Bounds on coverage, as text: they state what was not collected, not a quantity to compare. */
export function notCollectedLines(runtime: NonNullable<RuntimeRow['runtime']>): string[] {
  return [
    ...runtime.frontierExclusions.map((exclusion) => `${exclusion.rule}: ${plural(exclusion.excludedLinks, 'link')} refused`),
    ...runtime.stoppedHosts.map((host) => `${host.host} stopped: ${host.reason}`),
  ];
}

function countsLine(runtime: NonNullable<RuntimeRow['runtime']>): string {
  return `${count(runtime.completedTasks)} done · ${count(runtime.failedTasks)} failed · ${count(runtime.activeTasks)} active · ${count(runtime.queuedTasks)} queued`;
}

export function runtimeHeadline(rows: readonly RuntimeRow[]): string | null {
  const run = rows.filter((row) => row.runtime !== null) as (RuntimeRow & { runtime: NonNullable<RuntimeRow['runtime']> })[];
  if (run.length === 0) return null;
  const tasks = run.reduce((sum, row) => sum + total(row.runtime), 0);
  const completed = run.reduce((sum, row) => sum + row.runtime.completedTasks, 0);
  const failed = run.reduce((sum, row) => sum + row.runtime.failedTasks, 0);
  const hosts = run.reduce((sum, row) => sum + row.runtime.stoppedHosts.length, 0);
  const failing = run.filter((row) => row.runtime.failedTasks > 0).sort((a, b) => b.runtime.failedTasks - a.runtime.failedTasks);
  const where = failing.length === 1 && run.length > 1 ? `, all in ${failing[0].label}` : failing.length > 1 ? `, most in ${failing[0].label}` : '';
  const refusing = run.filter((row) => row.runtime.frontierExclusions.some((exclusion) => exclusion.excludedLinks > 0));
  const links = run.reduce((sum, row) => sum + row.runtime.frontierExclusions.reduce((s, exclusion) => s + exclusion.excludedLinks, 0), 0);
  const refused = links > 0 ? `; ${plural(refusing.length, 'source')} refused ${plural(links, 'link')} by frontier rule` : '';
  return `${count(completed)} of ${plural(tasks, 'collection task')} done across ${plural(run.length, 'source')}; ${count(failed)} failed${where}; ${plural(hosts, 'host')} stopped${refused}.`;
}

function rowHeight(row: RuntimeRow): number {
  if (!row.runtime) return 40;
  const lines = Math.min(NOT_COLLECTED_LINES + 1, notCollectedLines(row.runtime).length);
  return 16 + BAND + 26 + lines * 15 + 10;
}

const LEGEND: LegendItem[] = [
  { key: 'done', label: 'Done', swatch: 'rect', tone: 'mark' },
  { key: 'failed', label: 'Failed (ink, with ▲)', swatch: 'rect', tone: 'ink' },
  { key: 'active', label: 'Active', swatch: 'rect', tone: 'context' },
  { key: 'queued', label: 'Queued (outline)', swatch: 'hollow', tone: 'context' },
];

/**
 * F10, collection tasks per source on one shared task scale: done, failed, active, then queued as
 * an outline. Failed tasks are an operational state, drawn as their own ink segment with a ▲ and
 * a count, never folded into progress and never crimson. Frontier refusals and stopped hosts are
 * printed as 'Not collected' lines under each bar.
 */
export default function RuntimeBars({ title = 'Collection tasks by source', rows, status, error, storageKey, onShowFailures }: {
  title?: string;
  rows: readonly RuntimeRow[];
  status: 'ready' | 'loading' | 'error';
  error?: string | null;
  storageKey?: string;
  /** Enter or a click on a source with failures shows them. */
  onShowFailures?(key: string): void;
}) {
  const headline = status === 'ready' ? runtimeHeadline(rows) : null;
  const table: TwinTable = {
    caption: title,
    columns: [
      { key: 'source', label: 'Source' },
      { key: 'done', label: 'Done', kind: 'count' },
      { key: 'failed', label: 'Failed', kind: 'count' },
      { key: 'active', label: 'Active', kind: 'count' },
      { key: 'queued', label: 'Queued', kind: 'count' },
      { key: 'notCollected', label: 'Not collected' },
    ],
    rows: rows.map((row) => (row.runtime
      ? { source: row.label, done: row.runtime.completedTasks, failed: row.runtime.failedTasks, active: row.runtime.activeTasks, queued: row.runtime.queuedTasks, notCollected: notCollectedLines(row.runtime).join('; ') || 'none reported' }
      : { source: row.label, done: NOT_RUN, failed: NOT_RUN, active: NOT_RUN, queued: NOT_RUN, notCollected: NOT_RUN })),
  };
  const notRun = rows.filter((row) => row.runtime === null).length;
  return (
    <ChartFrame
      title={title}
      storageKey={storageKey}
      headline={headline}
      sources={{ tables: ['portal_sync_runs', 'ingestion_task_frontier'], note: 'live queue counts for this session' }}
      summary={`${plural(rows.length, 'source')}${notRun ? `, ${notRun} ${NOT_RUN}` : ''}. Failed tasks are drawn as their own segment.`}
      status={status === 'ready' && rows.length === 0 ? 'empty' : status}
      emptyText="No source has a collection queue yet. Connect or resume a source to see its tasks here."
      error={error}
      table={table}
      legend={LEGEND}
      minHeight={Math.max(60, rows.reduce((sum, row) => sum + rowHeight(row), 0) + 4)}
      interactive={{ roledescription: 'collection task bars' }}
    >
      {({ width }) => <RuntimePlot width={width} rows={rows} onShowFailures={onShowFailures} />}
    </ChartFrame>
  );
}

function fit(text: string, width: number): string {
  if (measureText(text) <= width) return text;
  let end = text.length;
  while (end > 1 && measureText(`${text.slice(0, end)}…`) > width) end -= 1;
  return `${text.slice(0, end).trimEnd()}…`;
}

function RuntimePlot({ width, rows, onShowFailures }: { width: number; rows: readonly RuntimeRow[]; onShowFailures?(key: string): void }) {
  const tooltip = useTooltip();
  const roving = useRovingFocus({
    shape: { rows: rows.length },
    onActivate: (i) => {
      const runtime = rows[i].runtime;
      if (runtime && runtime.failedTasks > 0) onShowFailures?.(rows[i].key);
    },
    onEscape: tooltip.hide,
  });
  const maxTasks = Math.max(1, ...rows.map((row) => (row.runtime ? total(row.runtime) : 0)));
  const x = scaleLinear([0, maxTasks], [0, width - 2], { nice: false });
  let y = 0;
  return (
    <g>
      {rows.map((row, index) => {
        const top = y;
        y += rowHeight(row);
        const runtime = row.runtime;
        const heading = row.state ? `${row.label} · ${row.state}` : row.label;
        const name = runtime
          ? `${row.label}: ${countsLine(runtime)} of ${plural(total(runtime), 'task')}.${runtime.failedTasks > 0 && onShowFailures ? ' Enter shows the problems.' : ''}`
          : `${row.label}: ${NOT_RUN}.`;
        const tip = runtime && (
          <TooltipContent
            title={row.label}
            rows={[
              { key: 'done', value: count(runtime.completedTasks), label: 'done', swatch: 'rect', tone: 'mark' },
              { key: 'failed', value: count(runtime.failedTasks), label: 'failed', swatch: 'rect', tone: 'ink' },
              { key: 'active', value: count(runtime.activeTasks), label: 'active', swatch: 'rect', tone: 'context' },
              { key: 'queued', value: count(runtime.queuedTasks), label: 'queued', swatch: 'hollow', tone: 'context' },
            ]}
          />
        );
        if (!runtime) {
          return (
            <MarkGroup key={row.key} box={{ x: 0, y: top + 1, width, height: 32 }} interaction={{ name, item: roving.getItemProps(index) }}>
              <text x={0} y={top + 9} dy="0.32em" className="fill-fg text-[12px] font-medium">{fit(heading, width)}</text>
              <text x={0} y={top + 26} dy="0.32em" className={`${MUTED_LABEL_CLASS} italic`}>{NOT_RUN}</text>
            </MarkGroup>
          );
        }
        const barY = top + 16;
        const stacked = runtime.completedTasks + runtime.failedTasks + runtime.activeTasks;
        const queuedScale = scaleLinear([0, Math.max(1, runtime.queuedTasks)], [x(stacked) + (stacked > 0 ? 2 : 0), x(stacked) + (stacked > 0 ? 2 : 0) + (x(runtime.queuedTasks) - x(0))], { nice: false });
        const failedAt = x(runtime.completedTasks + runtime.failedTasks / 2);
        const lines = notCollectedLines(runtime);
        const shownLines = lines.length > NOT_COLLECTED_LINES + 1 ? [...lines.slice(0, NOT_COLLECTED_LINES), `+${lines.length - NOT_COLLECTED_LINES} more (see table)`] : lines;
        return (
          <MarkGroup
            key={row.key}
            box={{ x: 0, y: top + 1, width, height: rowHeight(row) - 8 }}
            interaction={{
              name,
              item: roving.getItemProps(index),
              onClick: runtime.failedTasks > 0 && onShowFailures ? () => onShowFailures(row.key) : undefined,
              onPointerMove: (event) => tooltip.show(tip, event),
              onPointerLeave: tooltip.hide,
              onFocusMark: (box) => tooltip.show(tip, box),
              onBlurMark: tooltip.hide,
            }}
          >
            <text x={0} y={top + 7} dy="0.32em" className="fill-fg text-[12px] font-medium">{fit(heading, width)}</text>
            <rect x={x(0)} y={barY} width={Math.max(0, x(maxTasks) - x(0))} height={BAND} rx={2} style={{ fill: VIZ.track, opacity: 0.5 }} />
            <StackedBar
              band={{ pos: barY, thickness: BAND }}
              scale={x}
              segments={[
                { key: 'done', value: runtime.completedTasks, tone: 'default', label: count(runtime.completedTasks) },
                { key: 'failed', value: runtime.failedTasks, tone: 'primary', label: count(runtime.failedTasks) },
                { key: 'active', value: runtime.activeTasks, tone: 'ord-1', label: count(runtime.activeTasks) },
              ]}
              motionKey={`runtime-${row.key}`}
            />
            {runtime.queuedTasks > 0 && <Bar band={{ pos: barY, thickness: BAND }} value={measured(runtime.queuedTasks)} scale={queuedScale} role="comparison" />}
            {runtime.failedTasks > 0 && <ShapeGlyph shape="triangle" cx={Math.max(5, Math.min(width - 5, failedAt))} cy={barY + BAND + 6} radius={4} style={{ fill: VIZ.ink }} />}
            <text x={0} y={barY + BAND + 19} dy="0.32em" className={LABEL_CLASS}>
              {fit(countsLine(runtime), width)}
            </text>
            {shownLines.map((line, i) => (
              <text key={line} x={0} y={barY + BAND + 35 + i * 15} dy="0.32em" className={MUTED_LABEL_CLASS}>
                <title>{line}</title>
                {fit(`${i === 0 ? 'Not collected: ' : ''}${line}`, width)}
              </text>
            ))}
          </MarkGroup>
        );
      })}
      {tooltip.node}
    </g>
  );
}
