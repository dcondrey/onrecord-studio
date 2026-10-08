import { Fragment, type ReactNode } from 'react';
import ChartFrame, { type ChartStatus } from '../frame/ChartFrame';
import { Axis } from '../frame/Axis';
import { Grid } from '../frame/Grid';
import type { LegendItem } from '../frame/Legend';
import type { ChartSources } from '../frame/SourceCaption';
import type { TwinTable } from '../frame/TableTwin';
import { TooltipContent, type TooltipRow } from '../frame/Tooltip';
import { VIZ, type YardstickLabel } from '../core/claim';
import { fitsInside, measureText } from '../core/layout';
import { isValue, type Measured } from '../core/nulls';
import { scaleLinear, type ContinuousScale } from '../core/scale';
import { useRovingFocus } from '../hooks/useRovingFocus';
import { useTooltip } from '../hooks/useTooltip';
import { Bullet, type BulletTick } from '../marks/Bullet';
import { Dot } from '../marks/Dot';
import { AbsentSlot, LABEL_CLASS, MUTED_LABEL_CLASS, type MarkInteraction } from '../marks/shared';

const ROW_LABEL_FONT = '12px -apple-system, BlinkMacSystemFont, sans-serif';
export const ROW_LABEL_CLASS = 'fill-fg-secondary text-[12px]';

export interface RowPlacement {
  key: string;
  label: string;
  /** Baseline of the row's label. */
  labelY: number;
  /** Top of the row's mark band. */
  markY: number;
  /** The label sits in the left column (true) or on its own line above the mark (false). */
  inline: boolean;
}

/**
 * Row geometry shared by the row figures: a label column on the left when the label fits, and the
 * label on its own line above the mark when it does not (or below 480px, where there is no column).
 */
export function rowLayout(items: readonly { key: string; label: string }[], width: number, { band = 14, top = 4, gap = 12, extra = 0 }: { band?: number; top?: number; gap?: number; extra?: number } = {}) {
  const labelColumn = width < 480 ? 0 : Math.min(220, Math.round(width * 0.3));
  let y = top;
  const rows: RowPlacement[] = items.map((item) => {
    const inline = labelColumn > 0 && fitsInside(item.label, labelColumn - 8, ROW_LABEL_FONT);
    const row = inline
      ? { key: item.key, label: item.label, labelY: y + band / 2, markY: y, inline }
      : { key: item.key, label: item.label, labelY: y + 7, markY: y + 18, inline };
    y = row.markY + band + extra + gap;
    return row;
  });
  return { rows, height: y, plotX: labelColumn, band };
}

/** A row label, haloed in the surface colour so a long one stays legible over gridlines. */
export function RowLabel({ row, title }: { row: RowPlacement; title?: string }) {
  return (
    <text x={0} y={row.labelY} dy="0.32em" className={ROW_LABEL_CLASS} style={{ paintOrder: 'stroke', stroke: VIZ.surface, strokeWidth: 4, strokeLinejoin: 'round' }}>
      {title && <title>{title}</title>}
      {row.label}
    </text>
  );
}

/** 'not-mapped': no dataset is mapped to the stage's role, so no link was looked for; not the same as a link not found. */
export type StageState = 'linked' | 'not-linked' | 'not-retrieved' | 'not-mapped';

export interface StageNode {
  key: string;
  label: string;
  state: StageState;
  /** Citation rows behind a linked stage. */
  count: number;
  onOpen?(): void;
}

export interface BulletRow {
  key: string;
  label: string;
  track: Measured;
  base?: Measured;
  bar: Measured;
  /** A lighter segment continuing the bar (encumbered beside actual). */
  extension?: Measured;
  ticks: BulletTick[];
  /** Printed at the bar's end; built from decimal strings by the container. */
  barLabel?: string;
  /** Printed in ink when the bar runs past the track. */
  overrunLabel?: string;
  /** Words for an absent track or bar; '' when the bar label already says it. */
  trackSlotText?: string;
  barSlotText?: string;
  /** A signed difference drawn as a small diverging bar in the right-hand column, its label signed. */
  delta?: { value: number; label: string } | null;
  /** This row's app yardstick band on the value axis (a different band on each row). */
  yardstick?: { lo: number; hi: number; label: YardstickLabel };
  /** A full sentence with every figure, 'not retrieved' where one is absent. */
  name: string;
  tooltip: { title: string; rows: TooltipRow[]; note?: string };
  stages?: StageNode[];
  /** Enter or a click on the bullet: open its cited rows. */
  onOpen?(): void;
}

/** Rows that share one currency share one axis; a different currency is a different group and axis. */
export interface BulletGroup {
  key: string;
  label: string;
  currency: string;
  rows: BulletRow[];
}

export interface BulletRowsProps {
  title: string;
  headline: string | null;
  sources: ChartSources;
  summary: string;
  status: ChartStatus;
  emptyText?: string;
  table: TwinTable;
  legend: LegendItem[];
  groups: BulletGroup[];
  /** Heading of the right-hand delta column, when rows carry a delta ('actual − adopted'). */
  deltaTitle?: string;
  storageKey: string;
  controls?: ReactNode;
}

const BAND = 14;
const STAGE_STRIP = 22;
const GROUP_HEADER = 22;
const AXIS = 28;

function groupLayout(groups: readonly BulletGroup[], width: number) {
  let y = 0;
  return groups.map((group) => {
    const header = groups.length > 1 || group.currency === '' ? GROUP_HEADER : 0;
    const hasStages = group.rows.some((row) => row.stages && row.stages.length > 0);
    const layout = rowLayout(group.rows, width, { band: BAND, top: y + header + 4, extra: hasStages ? STAGE_STRIP : 0 });
    const block = { group, headerY: y, rows: layout.rows, plotX: layout.plotX, axisY: layout.height, hasStages };
    y = layout.height + AXIS + 8;
    return { ...block, bottom: y };
  });
}

function plotHeight(groups: readonly BulletGroup[], width: number): number {
  const blocks = groupLayout(groups, width);
  return blocks.length ? blocks[blocks.length - 1].bottom : 80;
}

function currencyMax(groups: readonly BulletGroup[], currency: string): number {
  let max = 0;
  for (const group of groups) {
    if (group.currency !== currency) continue;
    for (const row of group.rows) {
      for (const m of [row.track, row.base, row.bar, ...row.ticks.map((tick) => tick.at)]) {
        if (m && isValue(m)) max = Math.max(max, m.value);
      }
    }
  }
  return max;
}

function StageStrip({ y, x0, x1, stages, interaction }: { y: number; x0: number; x1: number; stages: StageNode[]; interaction(j: number): MarkInteraction }) {
  const step = Math.min(150, (x1 - x0) / Math.max(1, stages.length));
  const cy = y + STAGE_STRIP / 2 + 2;
  const xs = stages.map((_, j) => x0 + 6 + j * step);
  return (
    <g>
      {/* The strip is a row of its own: the surface hides the value gridlines, so a node never reads as an amount. */}
      <rect x={x0} y={y + 2} width={Math.max(0, x1 - x0)} height={STAGE_STRIP - 2} style={{ fill: VIZ.surface }} aria-hidden="true" />
      <line x1={xs[0]} x2={xs[xs.length - 1]} y1={cy} y2={cy} style={{ stroke: VIZ.track, strokeWidth: 2 }} aria-hidden="true" />
      {stages.map((stage, j) => {
        const words = stage.state === 'linked' ? `${stage.label} ${stage.count}` : `${stage.label}: ${STAGE_WORDS[stage.state]}`;
        const room = step - 18;
        const text = fitsInside(words, room) ? words : fitsInside(stage.label, room) ? stage.label : '';
        return (
          <Fragment key={stage.key}>
            {stage.state === 'not-retrieved' || stage.state === 'not-mapped'
              ? <AbsentSlot absence={stage.state === 'not-mapped' ? { state: 'not-covered', reason: `${stage.label}: no dataset is mapped to this role, so no link was looked for` } : { state: 'not-recorded', reason: `${stage.label}: no records of this stage were retrieved` }} box={{ x: xs[j] - 5, y: cy - 5, width: 10, height: 10 }} text="" />
              : <Dot cx={xs[j]} cy={cy} r={4} role="primary" hollow={stage.state === 'not-linked'} interaction={interaction(j)} />}
            {text && <text x={xs[j] + 9} y={cy} dy="0.32em" className={MUTED_LABEL_CLASS}>{text}</text>}
          </Fragment>
        );
      })}
    </g>
  );
}

const STAGE_WORDS: Record<Exclude<StageState, 'linked'>, string> = { 'not-linked': 'not linked', 'not-retrieved': 'not retrieved', 'not-mapped': 'not mapped' };
const DELTA_COLUMN = 150;

/** The signed delta as a small bar about its own zero in the right-hand column; neutral both ways, the sign in words. */
function DeltaBar({ delta, x0, y, max }: { delta: { value: number; label: string }; x0: number; y: number; max: number }) {
  const half = 32;
  const centre = x0 + half + 2;
  const length = max > 0 ? (Math.abs(delta.value) / max) * half : 0;
  const from = delta.value < 0 ? centre - length : centre;
  return (
    <g aria-hidden="true">
      <line x1={centre + 0.5} x2={centre + 0.5} y1={y - 1} y2={y + BAND + 1} style={{ stroke: VIZ.baseline, strokeWidth: 1 }} />
      <rect x={from} y={y + BAND / 2 - 3} width={Math.max(length, delta.value === 0 ? 0 : 1)} height={6} style={{ fill: VIZ.context }} />
      <text x={centre + half + 6} y={y + BAND / 2} dy="0.32em" className={LABEL_CLASS}>{delta.label}</text>
    </g>
  );
}

function Plot({ groups, width, deltaTitle }: { groups: BulletGroup[]; width: number; deltaTitle?: string }) {
  const tooltip = useTooltip();
  const blocks = groupLayout(groups, width);
  const flat = blocks.flatMap((block) => block.rows.map((row, i) => ({ row, data: block.group.rows[i] })));
  const rowFocus = useRovingFocus({ shape: { rows: Math.max(1, flat.length) }, onActivate: (i) => flat[i]?.data.onOpen?.(), onEscape: tooltip.hide });
  const stageRows = flat.filter(({ data }) => data.stages && data.stages.length > 0);
  const stageCols = Math.max(1, ...stageRows.map(({ data }) => data.stages?.length ?? 0));
  const stageFocus = useRovingFocus({
    shape: { rows: Math.max(1, stageRows.length), cols: stageCols },
    onActivate: (i, j = 0) => stageRows[i]?.data.stages?.[j]?.onOpen?.(),
    onEscape: tooltip.hide,
  });
  // Room on the right for the longest words at a bar's end, so an overrun label is never dropped.
  const words = Math.max(0, ...groups.flatMap((group) => group.rows.map((row) => {
    const slot = isValue(row.track) || !row.trackSlotText ? 0 : measureText(` · ${row.trackSlotText}`);
    return Math.max(measureText(row.overrunLabel ?? ''), measureText(row.barLabel ?? row.barSlotText ?? '')) + slot;
  })));
  const plotLeft = Math.max(0, ...blocks.map((block) => block.plotX));
  const hasDelta = groups.some((group) => group.rows.some((row) => row.delta));
  const deltaX = width - (hasDelta ? DELTA_COLUMN : 0);
  const right = Math.max(plotLeft + 80, deltaX - Math.min(words + 12, Math.round(width * 0.55)));
  const deltaMax = (currency: string) => Math.max(0, ...groups.filter((group) => group.currency === currency).flatMap((group) => group.rows.map((row) => Math.abs(row.delta?.value ?? 0))));
  const scales = new Map<string, ContinuousScale>();
  for (const block of blocks) {
    if (!scales.has(block.group.currency)) {
      scales.set(block.group.currency, scaleLinear([0, Math.max(1, currencyMax(groups, block.group.currency))], [block.plotX, right]));
    }
  }
  let flatIndex = 0;
  let stageIndex = 0;
  return (
    <>
      {blocks.map((block) => {
        const x = scales.get(block.group.currency) as ContinuousScale;
        const top = block.rows[0] ? block.rows[0].markY - 4 : block.axisY;
        return (
          <g key={block.group.key}>
            {(groups.length > 1 || block.group.currency === '') && (
              <text x={0} y={block.headerY + 12} className="fill-fg text-[12px] font-medium">{block.group.label}</text>
            )}
            <Grid scale={x} extent={[top, block.axisY]} vertical />
            {hasDelta && deltaTitle && <text x={deltaX + 2} y={top - 2} className={MUTED_LABEL_CLASS}>{deltaTitle}</text>}
            {block.rows.map((row, i) => {
              const data = block.group.rows[i];
              const index = flatIndex++;
              const stagesAt = data.stages && data.stages.length > 0 ? stageIndex++ : null;
              const content = <TooltipContent title={data.tooltip.title} rows={data.tooltip.rows} note={data.tooltip.note} />;
              return (
                <g key={row.key}>
                  <RowLabel row={row} title={data.label} />
                  {data.yardstick && (
                    <rect x={x(data.yardstick.lo)} y={row.markY - 3} width={Math.max(1, x(data.yardstick.hi) - x(data.yardstick.lo))} height={BAND + 6} rx={2} style={{ fill: 'none', stroke: VIZ.baseline, strokeWidth: 1, strokeDasharray: '3 2' }}>
                      <title>{data.yardstick.label}</title>
                    </rect>
                  )}
                  <Bullet
                    band={{ pos: row.markY, thickness: BAND }}
                    scale={x}
                    track={data.track}
                    base={data.base}
                    bar={data.bar}
                    extension={data.extension}
                    ticks={data.ticks}
                    barLabel={data.barLabel}
                    overrunLabel={data.overrunLabel}
                    trackSlotText={data.trackSlotText}
                    barSlotText={data.barSlotText}
                    labelLimit={deltaX - 4}
                    interaction={{
                      name: data.name,
                      item: rowFocus.getItemProps(index),
                      onClick: data.onOpen,
                      onPointerMove: (event) => tooltip.show(content, event),
                      onPointerLeave: tooltip.hide,
                      onFocusMark: (box) => tooltip.show(content, box),
                      onBlurMark: tooltip.hide,
                    }}
                  />
                  {data.delta && <DeltaBar delta={data.delta} x0={deltaX} y={row.markY} max={deltaMax(block.group.currency)} />}
                  {stagesAt !== null && data.stages && (
                    <StageStrip
                      y={row.markY + BAND + 2}
                      x0={block.plotX}
                      x1={right}
                      stages={data.stages}
                      interaction={(j) => {
                        const stage = (data.stages as StageNode[])[j];
                        const sentence = stage.state === 'linked'
                          ? `${data.label}, ${stage.label}: ${stage.count} cited ${stage.count === 1 ? 'row' : 'rows'}. Press Enter to inspect.`
                          : `${data.label}, ${stage.label}: ${STAGE_WORDS[stage.state]}.`;
                        return {
                          name: sentence,
                          item: stageFocus.getItemProps(stagesAt, j),
                          onClick: stage.onOpen,
                          onFocusMark: (box) => tooltip.show(<TooltipContent rows={[]} note={sentence} />, box),
                          onBlurMark: tooltip.hide,
                        };
                      }}
                    />
                  )}
                </g>
              );
            })}
            <Axis scale={x} kind={block.group.currency === 'USD' ? 'currency' : 'count'} orient="bottom" offset={block.axisY} />
          </g>
        );
      })}
      {tooltip.node}
    </>
  );
}

/**
 * F6 BulletRows: one bullet per line on a currency's own axis. Track = authority (or revised), an
 * inner band for the base, ink bar = the measured amount with an optional lighter extension, ink
 * ticks = targets, an optional per-row yardstick band, and an optional signed delta column. An overrun is drawn
 * by position past the track and words in ink, never crimson: retrieved authority can be
 * incomplete, so it compares retrieved figures. Mixed currencies are separate groups and axes.
 */
export default function BulletRows(props: BulletRowsProps) {
  const { groups } = props;
  const rows = groups.reduce((sum, group) => sum + group.rows.length, 0);
  return (
    <ChartFrame
      title={props.title}
      headline={props.headline}
      sources={props.sources}
      summary={props.summary}
      status={rows === 0 && props.status === 'ready' ? 'empty' : props.status}
      emptyText={props.emptyText}
      table={props.table}
      legend={props.legend}
      controls={props.controls}
      minHeight={plotHeight(groups, 640)}
      plotHeight={(width) => plotHeight(groups, width)}
      interactive={{ roledescription: 'bullet chart' }}
      storageKey={props.storageKey}
    >
      {({ width }) => <Plot groups={groups} width={width} deltaTitle={props.deltaTitle} />}
    </ChartFrame>
  );
}
