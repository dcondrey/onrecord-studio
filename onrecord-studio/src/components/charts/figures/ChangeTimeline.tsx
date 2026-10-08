import type { DataPackChange } from '../../../types';
import { VIZ } from '../core/claim';
import { formatDate, formatValue } from '../core/format';
import { measureText, placeLabels } from '../core/layout';
import { absent, measured } from '../core/nulls';
import { parseDate, scaleLinear, scaleTime } from '../core/scale';
import { Axis } from '../frame/Axis';
import ChartFrame from '../frame/ChartFrame';
import { TooltipContent } from '../frame/Tooltip';
import type { TwinCell } from '../frame/TableTwin';
import { useRovingFocus } from '../hooks/useRovingFocus';
import { useTooltip } from '../hooks/useTooltip';
import { Bar } from '../marks/Bar';
import { Dot } from '../marks/Dot';
import { LABEL_CLASS, MarkGroup, MUTED_LABEL_CLASS } from '../marks/shared';

export interface ChangeTimelineProps {
  table: string;
  changes: readonly DataPackChange[];
  unchangedChecks: number;
  lastCheckedAt: string | null;
  storageKey?: string;
}

const HEIGHT = 196;
const TOP = 30;
const AXIS_Y = HEIGHT - 22;
const LEFT = 8;

const NOT_SEPARATED = 'not separated';
const NOT_SEPARATED_REASON = 'more than half the records changed: listed as removed and added';
const NOT_SEPARATED_TWIN = 'not separated: counted in added and removed';
/** The side of the fixed 'not separated' marker; it has no extent on the value axis. */
const MARKER = 8;

/** A signed count label; a measured zero prints '0', never '+0' or '−0'. */
function signed(sign: '+' | '−', value: number): string {
  return value === 0 ? '0' : `${sign}${formatValue(value, { kind: 'count', compact: true })}`;
}

function day(iso: string | null): string {
  if (!iso) return 'an unrecorded time';
  const ms = parseDate(iso);
  return Number.isFinite(ms) ? formatDate(ms, 'day') : 'an unrecorded time';
}

function fileName(path: string): string {
  return path.split('/').pop() ?? path;
}

/** '<table>: +A / −R / C changed across N retrievals since <first>; last checked <date>: unchanged from the latest snapshot'. */
export function changeHeadline({ table, changes, unchangedChecks, lastCheckedAt }: ChangeTimelineProps): string {
  const added = changes.reduce((sum, change) => sum + change.added, 0);
  const removed = changes.reduce((sum, change) => sum + change.removed, 0);
  const separated = changes.filter((change) => change.changed !== null);
  const changed = separated.reduce((sum, change) => sum + (change.changed ?? 0), 0);
  const unseparated = changes.length - separated.length;
  const changedText = separated.length === 0 ? `changed ${NOT_SEPARATED}` : `${changed.toLocaleString('en-US')} changed${unseparated > 0 ? ` (${unseparated} ${unseparated === 1 ? 'pair' : 'pairs'} ${NOT_SEPARATED})` : ''}`;
  const first = changes.map((change) => change.previousRetrievedAt).find((at) => at !== null) ?? null;
  const retrievals = changes.length + 1;
  const lead = `${table}: +${added.toLocaleString('en-US')} / −${removed.toLocaleString('en-US')} / ${changedText} across ${retrievals} retrievals since ${day(first)}`;
  if (unchangedChecks > 0 && lastCheckedAt) return `${lead}; last checked ${day(lastCheckedAt)}: unchanged from the latest snapshot.`;
  return `${lead}.`;
}

/**
 * F18: what each change record between two retrievals of one publisher file counts. Added rows
 * stand above the baseline and removed rows below it, both slate (adding records is not a good
 * or bad thing); changed records are a thin ink column beside them, or, when more than half the
 * records changed, a dashed 'not separated' slot, never a 0. A check that found the file
 * unchanged is a hollow dot on the baseline at the last check; checks are counted, not dated.
 */
export default function ChangeTimeline(props: ChangeTimelineProps) {
  const { table, changes, unchangedChecks, lastCheckedAt } = props;
  const tooltip = useTooltip();
  // In time order, so the arrow keys move left to right whatever order the manifest lists.
  const placed = changes
    .map((change, index) => ({ change, index, at: change.currentRetrievedAt ? parseDate(change.currentRetrievedAt) : Number.NaN }))
    .filter((pair) => Number.isFinite(pair.at))
    .sort((a, b) => a.at - b.at || a.index - b.index);
  const unplaced = changes.length - placed.length;
  const checkedAt = lastCheckedAt ? parseDate(lastCheckedAt) : Number.NaN;
  const roving = useRovingFocus({ shape: { rows: Math.max(1, placed.length) }, onActivate: () => {}, onEscape: tooltip.hide });

  const twinRows: Record<string, TwinCell>[] = changes.map((change) => ({
    pair: `${fileName(change.previous)} → ${fileName(change.current)}`,
    previous: change.previousRetrievedAt ? day(change.previousRetrievedAt) : absent('not-recorded', 'the manifest no longer lists the earlier file'),
    current: change.currentRetrievedAt ? day(change.currentRetrievedAt) : absent('not-recorded', 'the manifest no longer lists the later file'),
    added: change.added,
    removed: change.removed,
    // Not an absence: the changed records are counted, inside added and removed.
    changed: change.changed === null ? NOT_SEPARATED_TWIN : change.changed,
  }));
  if (unchangedChecks > 0) {
    twinRows.push({ pair: `${unchangedChecks} ${unchangedChecks === 1 ? 'check' : 'checks'} found the file unchanged`, previous: null, current: lastCheckedAt ? `last ${day(lastCheckedAt)}` : absent('not-recorded', 'the check time was not recorded'), added: 0, removed: 0, changed: 0 });
  }

  const empty = changes.length === 0 && unchangedChecks === 0;
  const summary = `${changes.length} change ${changes.length === 1 ? 'record' : 'records'} and ${unchangedChecks} unchanged ${unchangedChecks === 1 ? 'check' : 'checks'} for ${table}.${unplaced > 0 ? ` ${unplaced} not placed: no retrieval time recorded.` : ''}`;

  return (
    <>
      <ChartFrame
        title={`Changes between retrievals · ${table}`}
        storageKey={props.storageKey ?? `pack-changes:${table}`}
        headline={empty ? null : changeHeadline(props)}
        sources={{ tables: ['pack manifest: change records', 'pack manifest: checks'], note: 'change records are listed, never imported' }}
        summary={summary}
        status={empty ? 'empty' : 'ready'}
        emptyText="This pack recorded no change and no check for this table."
        minHeight={HEIGHT}
        interactive={{ roledescription: 'change timeline' }}
        legend={[
          { key: 'added', label: 'Added (above) / removed (below)', swatch: 'rect', tone: 'mark' },
          ...(changes.some((change) => change.changed !== null) ? [{ key: 'changed', label: 'Changed field by field', swatch: 'rect' as const, tone: 'ink' as const }] : []),
          ...(changes.some((change) => change.changed === null) ? [{ key: 'not-separated', label: 'Changed not separated: counted in added and removed', swatch: 'slot-dashed' as const }] : []),
          ...(unchangedChecks > 0 ? [{ key: 'checked', label: 'Checked, unchanged', swatch: 'hollow' as const, tone: 'ink' as const }] : []),
        ]}
        autoAbsenceLegend={false}
        table={{
          caption: `Change records for ${table}`,
          columns: [
            { key: 'pair', label: 'Pair' },
            { key: 'previous', label: 'Earlier retrieval' },
            { key: 'current', label: 'Later retrieval' },
            { key: 'added', label: 'Added', kind: 'count' },
            { key: 'removed', label: 'Removed', kind: 'count' },
            { key: 'changed', label: 'Changed', kind: 'count' },
          ],
          rows: twinRows,
        }}
      >
        {({ width }) => {
          // The domain spans the retrievals drawn (each pair at its later retrieval) and the last
          // check; a lone pair sits centred rather than at the end of its earlier retrieval's span.
          const times = [...placed.map((pair) => pair.at), ...(Number.isFinite(checkedAt) ? [checkedAt] : [])];
          const lo = times.length ? Math.min(...times) : Date.now();
          const hi = times.length ? Math.max(...times) : Date.now();
          const pad = Math.max((hi - lo) * 0.08, 86_400_000 * 3);
          const right = width - 16;
          const x = scaleTime([lo - pad, hi + pad], [LEFT + 24, right - 24]);
          const extent = Math.max(1, ...placed.map(({ change }) => Math.max(change.added, change.removed, change.changed ?? 0)));
          const y = scaleLinear([-extent, extent], [AXIS_Y - 30, TOP], { nice: false });
          const base = y(0);
          const xs = placed.map((pair) => x(pair.at));
          const gap = xs.length > 1 ? Math.min(...xs.slice(1).map((v, i) => v - xs[i])) : 80;
          const column = Math.max(6, Math.min(18, gap * 0.35));
          const tickX = (cx: number) => cx + column / 2 + 2;
          // Every per-column label is placed in its lane, pushed apart and kept inside the plot.
          const aboveText = placed.map(({ change }) => `${signed('+', change.added)} · ${change.changed === null ? NOT_SEPARATED : `${change.changed.toLocaleString('en-US')} changed`}`);
          const belowText = placed.map(({ change }) => signed('−', change.removed));
          const lane = (texts: string[]) => placeLabels(texts.map((text, i) => ({ key: String(i), anchor: xs[i], width: measureText(text) })), LEFT, width - 4);
          const above = lane(aboveText);
          const below = lane(belowText);
          const checkText = `${unchangedChecks} unchanged`;
          const checkX = Number.isFinite(checkedAt) ? x(checkedAt) : Number.NaN;
          const check = unchangedChecks > 0 && Number.isFinite(checkX) ? placeLabels([{ key: 'check', anchor: checkX, width: measureText(checkText) }], LEFT, width - 4)[0] : null;
          return (
            <>
              <line x1={LEFT} x2={right} y1={base} y2={base} style={{ stroke: VIZ.baseline, strokeWidth: 1 }} />
              <text x={LEFT} y={TOP - 16} className={MUTED_LABEL_CLASS}>added ↑</text>
              <text x={LEFT} y={AXIS_Y - 20} className={MUTED_LABEL_CLASS}>removed ↓</text>
              {placed.map(({ change, at }, i) => {
                const cx = x(at);
                const name = `${day(change.previousRetrievedAt)} to ${day(change.currentRetrievedAt)}: ${change.added.toLocaleString('en-US')} added, ${change.removed.toLocaleString('en-US')} removed, ${change.changed === null ? `changed ${NOT_SEPARATED} (${NOT_SEPARATED_REASON})` : `${change.changed.toLocaleString('en-US')} changed`}.`;
                const tip = (
                  <TooltipContent
                    title={`${fileName(change.previous)} → ${fileName(change.current)}`}
                    rows={[
                      { key: 'a', value: signed('+', change.added), label: 'added' },
                      { key: 'r', value: signed('−', change.removed), label: 'removed' },
                      { key: 'c', value: change.changed === null ? NOT_SEPARATED : change.changed.toLocaleString('en-US'), label: change.changed === null ? 'changed: counted in added and removed' : 'changed' },
                    ]}
                    note={`${day(change.previousRetrievedAt)} → ${day(change.currentRetrievedAt)}`}
                  />
                );
                const top = y(Math.max(change.added, change.changed ?? 0));
                const bottom = y(-change.removed);
                // A small changed count still shows: at least 3px tall, its value in the label.
                const changedTop = change.changed === null ? base : change.changed === 0 ? base - 1 : Math.min(base - 3, y(change.changed));
                const aboveY = Math.min(top, changedTop, change.changed === null ? base - MARKER / 2 : base) - 6;
                const aboveX = above[i].pos + measureText(aboveText[i]) / 2;
                const belowX = below[i].pos + measureText(belowText[i]) / 2;
                return (
                  <MarkGroup
                    key={`${change.previous}->${change.current}`}
                    box={{ x: cx - column / 2 - 1, y: top, width: column + 8, height: Math.max(8, bottom - top) }}
                    interaction={{
                      name,
                      item: roving.getItemProps(i),
                      onPointerMove: (event) => tooltip.show(tip, { clientX: event.clientX, clientY: event.clientY }),
                      onPointerLeave: tooltip.hide,
                      onFocusMark: (box) => tooltip.show(tip, box),
                      onBlurMark: tooltip.hide,
                    }}
                  >
                    <Bar orient="vertical" band={{ pos: cx - column / 2, thickness: column }} value={measured(change.added)} scale={y} motionKey={`added-${i}`} />
                    <Bar orient="vertical" band={{ pos: cx - column / 2, thickness: column }} value={measured(-change.removed)} scale={y} motionKey={`removed-${i}`} />
                    {change.changed === null ? (
                      <rect data-absence="not-separated" x={tickX(cx) + 0.5} y={base - MARKER / 2 + 0.5} width={MARKER - 1} height={MARKER - 1} rx={1.5} style={{ fill: 'none', stroke: VIZ.baseline, strokeWidth: 1, strokeDasharray: '3 2' }}>
                        <title>{`Changed ${NOT_SEPARATED}: ${NOT_SEPARATED_REASON}. They are counted in added and removed.`}</title>
                      </rect>
                    ) : (
                      <rect x={tickX(cx)} y={changedTop} width={4} height={base - changedTop} rx={1} style={{ fill: VIZ.ink }} />
                    )}
                    {above[i].leader && <line x1={cx} x2={aboveX} y1={aboveY + 2} y2={aboveY - 4} style={{ stroke: VIZ.baseline, strokeWidth: 1 }} />}
                    <text x={aboveX} y={aboveY - (above[i].leader ? 6 : 0)} textAnchor="middle" className={LABEL_CLASS}>{aboveText[i]}</text>
                    {below[i].leader && <line x1={cx} x2={belowX} y1={bottom + 2} y2={bottom + 6} style={{ stroke: VIZ.baseline, strokeWidth: 1 }} />}
                    <text x={belowX} y={bottom + 12 + (below[i].leader ? 4 : 0)} textAnchor="middle" className={LABEL_CLASS}>{belowText[i]}</text>
                  </MarkGroup>
                );
              })}
              {check && (
                <>
                  <Dot
                    cx={checkX}
                    cy={base}
                    hollow
                    role="primary"
                    title={`${unchangedChecks} ${unchangedChecks === 1 ? 'check' : 'checks'} found the file unchanged; the last on ${day(lastCheckedAt)}. Earlier check times are counted, not drawn.`}
                  />
                  {/* Below the marks, just above the axis, with a dotted leader up to its dot. */}
                  <line x1={checkX} x2={check.pos + measureText(checkText) / 2} y1={base + 6} y2={AXIS_Y - 16} style={{ stroke: VIZ.baseline, strokeWidth: 1, strokeDasharray: '1 2' }} />
                  <text x={check.pos} y={AXIS_Y - 6} className={MUTED_LABEL_CLASS}>{checkText}</text>
                </>
              )}
              <Axis time={x} orient="bottom" offset={AXIS_Y} />
              {unplaced > 0 && <text x={LEFT} y={HEIGHT - 2} className={MUTED_LABEL_CLASS}>{unplaced} not placed: no retrieval time recorded</text>}
            </>
          );
        }}
      </ChartFrame>
      {tooltip.node}
    </>
  );
}
