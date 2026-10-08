import type { DataPackChange } from '../../../types';
import { VIZ } from '../core/claim';
import { formatDate, formatValue } from '../core/format';
import { fitsInside } from '../core/layout';
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
import { AbsentSlot, LABEL_CLASS, MarkGroup, MUTED_LABEL_CLASS } from '../marks/shared';

export interface ChangeTimelineProps {
  table: string;
  changes: readonly DataPackChange[];
  unchangedChecks: number;
  lastCheckedAt: string | null;
  storageKey?: string;
}

const HEIGHT = 196;
const TOP = 22;
const AXIS_Y = HEIGHT - 22;
const LEFT = 8;

const NOT_SEPARATED = 'not separated';
const NOT_SEPARATED_REASON = 'more than half the records changed: listed as removed and added';

function day(iso: string | null): string {
  if (!iso) return 'an unrecorded time';
  const ms = parseDate(iso);
  return Number.isFinite(ms) ? formatDate(ms, 'day') : 'an unrecorded time';
}

function fileName(path: string): string {
  return path.split('/').pop() ?? path;
}

/** '<table>: +A / −R / C changed across N retrievals since <first>; last checked <date>, no change since'. */
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
  if (unchangedChecks > 0 && lastCheckedAt) return `${lead}; last checked ${day(lastCheckedAt)}, no change since.`;
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
  const placed = changes
    .map((change, index) => ({ change, index, at: change.currentRetrievedAt ? parseDate(change.currentRetrievedAt) : Number.NaN }))
    .filter((pair) => Number.isFinite(pair.at));
  const unplaced = changes.length - placed.length;
  const checkedAt = lastCheckedAt ? parseDate(lastCheckedAt) : Number.NaN;
  const roving = useRovingFocus({ shape: { rows: Math.max(1, placed.length) }, onActivate: () => {}, onEscape: tooltip.hide });

  const twinRows: Record<string, TwinCell>[] = changes.map((change) => ({
    pair: `${fileName(change.previous)} → ${fileName(change.current)}`,
    previous: change.previousRetrievedAt ? day(change.previousRetrievedAt) : absent('not-recorded', 'the manifest no longer lists the earlier file'),
    current: change.currentRetrievedAt ? day(change.currentRetrievedAt) : absent('not-recorded', 'the manifest no longer lists the later file'),
    added: change.added,
    removed: change.removed,
    changed: change.changed === null ? absent('not-recorded', `${NOT_SEPARATED}: ${NOT_SEPARATED_REASON}`) : change.changed,
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
          { key: 'changed', label: 'Changed field by field', swatch: 'rect', tone: 'ink' },
          { key: 'checked', label: 'Checked, unchanged', swatch: 'hollow', tone: 'ink' },
        ]}
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
          const times = [...placed.map((pair) => pair.at), ...placed.flatMap((pair) => (pair.change.previousRetrievedAt ? [parseDate(pair.change.previousRetrievedAt)] : [])), ...(Number.isFinite(checkedAt) ? [checkedAt] : [])].filter(Number.isFinite);
          const lo = times.length ? Math.min(...times) : Date.now();
          const hi = times.length ? Math.max(...times) : Date.now();
          const pad = Math.max((hi - lo) * 0.08, 86_400_000 * 3);
          const right = width - 16;
          const x = scaleTime([lo - pad, hi + pad], [LEFT + 24, right - 24]);
          const extent = Math.max(1, ...placed.map(({ change }) => Math.max(change.added, change.removed, change.changed ?? 0)));
          const y = scaleLinear([-extent, extent], [AXIS_Y - 30, TOP], { nice: false });
          const base = y(0);
          const xs = placed.map((pair) => x(pair.at)).sort((a, b) => a - b);
          const gap = xs.length > 1 ? Math.min(...xs.slice(1).map((v, i) => v - xs[i])) : 80;
          const column = Math.max(6, Math.min(18, gap * 0.35));
          return (
            <>
              <line x1={LEFT} x2={right} y1={base} y2={base} style={{ stroke: VIZ.baseline, strokeWidth: 1 }} />
              <text x={LEFT} y={TOP - 8} className={MUTED_LABEL_CLASS}>added ↑</text>
              <text x={LEFT} y={AXIS_Y - 20} className={MUTED_LABEL_CLASS}>removed ↓</text>
              {placed.map(({ change, at }, i) => {
                const cx = x(at);
                const name = `${day(change.previousRetrievedAt)} to ${day(change.currentRetrievedAt)}: ${change.added.toLocaleString('en-US')} added, ${change.removed.toLocaleString('en-US')} removed, ${change.changed === null ? `changed ${NOT_SEPARATED} (${NOT_SEPARATED_REASON})` : `${change.changed.toLocaleString('en-US')} changed`}.`;
                const tip = (
                  <TooltipContent
                    title={`${fileName(change.previous)} → ${fileName(change.current)}`}
                    rows={[
                      { key: 'a', value: `+${change.added.toLocaleString('en-US')}`, label: 'added' },
                      { key: 'r', value: `−${change.removed.toLocaleString('en-US')}`, label: 'removed' },
                      { key: 'c', value: change.changed === null ? NOT_SEPARATED : change.changed.toLocaleString('en-US'), label: 'changed' },
                    ]}
                    note={`${day(change.previousRetrievedAt)} → ${day(change.currentRetrievedAt)}`}
                  />
                );
                const top = y(Math.max(change.added, change.changed ?? 0));
                const bottom = y(-change.removed);
                const addedLabel = `+${formatValue(change.added, { kind: 'count', compact: true })}`;
                const removedLabel = `−${formatValue(change.removed, { kind: 'count', compact: true })}`;
                const showLabels = fitsInside(addedLabel, gap - 4) || placed.length === 1;
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
                      <AbsentSlot absence={absent('not-recorded', NOT_SEPARATED_REASON)} box={{ x: cx + column / 2 + 2, y: y(extent * 0.6), width: 8, height: base - y(extent * 0.6) - 1 }} text="" />
                    ) : (
                      <Bar orient="vertical" band={{ pos: cx + column / 2 + 2, thickness: 4 }} value={measured(change.changed)} scale={y} role="primary" motionKey={`changed-${i}`} />
                    )}
                    {showLabels && (
                      <>
                        <text x={cx} y={top - 6} textAnchor="middle" className={LABEL_CLASS}>{addedLabel}</text>
                        <text x={cx} y={bottom + 12} textAnchor="middle" className={LABEL_CLASS}>{removedLabel}</text>
                        {change.changed === null && <text x={cx + column / 2 + 12} y={y(extent * 0.6) + 4} className={MUTED_LABEL_CLASS}>{NOT_SEPARATED}</text>}
                      </>
                    )}
                  </MarkGroup>
                );
              })}
              {unchangedChecks > 0 && Number.isFinite(checkedAt) && (
                <Dot
                  cx={x(checkedAt)}
                  cy={base}
                  hollow
                  role="primary"
                  label={`${unchangedChecks} unchanged ${unchangedChecks === 1 ? 'check' : 'checks'}, last`}
                  labelSide={x(checkedAt) > width * 0.6 ? 'start' : 'end'}
                  title={`${unchangedChecks} ${unchangedChecks === 1 ? 'check' : 'checks'} found the file unchanged; the last on ${day(lastCheckedAt)}. Earlier check times are counted, not drawn.`}
                />
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
