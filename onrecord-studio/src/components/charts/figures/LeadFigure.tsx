import { useMemo, useState, type ReactNode } from 'react';
import type {
  LeadBinsFigure,
  LeadFigure as LeadFigureData,
  LeadFigureUnit,
  LeadFigureValue,
  LeadPairedFigure,
  LeadRankedFigure,
  LeadSegmentTone,
  LeadSeriesFigure,
  LeadStackedFigure,
} from '../../../types';
import { assertYardstickLabel, REFERENCE_PAINT, VIZ } from '../core/claim';
import { formatValue, type ValueKind } from '../core/format';
import { measureText } from '../core/layout';
import { absent, measured, type Absence, type Measured } from '../core/nulls';
import { scaleLinear, type ContinuousScale } from '../core/scale';
import { Axis, TICK_CLASS } from '../frame/Axis';
import ChartFrame from '../frame/ChartFrame';
import { Grid } from '../frame/Grid';
import type { LegendItem } from '../frame/Legend';
import { ReferenceLine } from '../frame/ReferenceLine';
import type { ChartSources } from '../frame/SourceCaption';
import type { TwinCell, TwinTable } from '../frame/TableTwin';
import { TooltipContent, type TooltipRow } from '../frame/Tooltip';
import { useRovingFocus } from '../hooks/useRovingFocus';
import { useTooltip } from '../hooks/useTooltip';
import { Bar, ExcludedStub, StackedBar } from '../marks/Bar';
import { Bullet } from '../marks/Bullet';
import { Dumbbell } from '../marks/Dumbbell';
import { LineSeries } from '../marks/Line';
import { AbsentSlot, hBarPath, LABEL_CLASS, MarkGroup, MUTED_LABEL_CLASS, type MarkInteraction } from '../marks/shared';

/*
 * One figure for every lead family, drawn from the typed payload the lead's builder wrote from the
 * same rows as its detail text. Five kinds: series (small multiples on a shared x), stacked rows
 * (with excluded stubs beside the bar and an overlay tick above it), paired values (dumbbell or
 * bullet), bins, and a ranked list. Never crimson: a lead is not a counted legal finding, and the
 * payload type cannot express one.
 */

export type LeadFigureSize = 'card' | 'full';

export interface LeadFigureProps {
  figure: LeadFigureData;
  /** The lead's own title. Null on a card, whose heading already is the title. */
  headline: string | null;
  sources: ChartSources;
  size: LeadFigureSize;
  /** Activating a mark: opens the lead's query or dossier. Called with the mark's key (a row,
   * year or bin key of the payload), so a caller can narrow to it. */
  onActivate?(key: string): void;
}

const ROW_LABEL_CLASS = 'fill-fg-secondary text-[12px]';
const ROW_FONT = '12px -apple-system, BlinkMacSystemFont, sans-serif';
const CARD_LIMIT: Record<LeadFigureData['kind'], number> = { series: 40, stacked: 4, paired: 4, bins: 8, ranked: 5 };

const kindOf = (unit: LeadFigureUnit): ValueKind => unit;
/** Compact ('12.9K') only from 10,000: a smaller count or amount prints exact, as ticks do. */
const COMPACT_FROM = 10_000;
const format = (value: number, unit: LeadFigureUnit, compact = false) => formatValue(value, { kind: kindOf(unit), compact: compact && (unit === 'currency' || unit === 'count') && Math.abs(value) >= COMPACT_FROM, digits: unit === 'percent' ? 1 : undefined });

/** A reason's first clause ('no rows in the actuals table; not spending of zero' gives the part
 * before the semicolon), for a slot too small for all of it. */
const firstClause = (reason: string) => reason.split(/[;(]/)[0].trim() || reason;

/** Words cut to fit `px` (with the slot's 4px padding each side), ending in an ellipsis; '' when
 * not even one letter fits. The full words stay in the mark's title. */
function fitWords(text: string, px: number): string {
  if (measureText(text) + 8 <= px) return text;
  for (let n = text.length - 1; n > 0; n -= 1) {
    const cut = `${text.slice(0, n).trimEnd()}…`;
    if (measureText(cut) + 8 <= px) return cut;
  }
  return '';
}
const toMeasured = (value: LeadFigureValue): Measured => value;
const valueOf = (value: LeadFigureValue): number | null => (value.state === 'value' ? value.value : null);
const cell = (value: LeadFigureValue): TwinCell => (value.state === 'value' ? value.value : (value satisfies Absence));
const valueText = (value: LeadFigureValue, unit: LeadFigureUnit) => (value.state === 'value' ? format(value.value, unit) : `${value.state === 'excluded' ? 'set aside' : 'not recorded'}: ${value.reason}`);

/** The figure as drawn at this size: a card keeps the first rows, and says so. */
function visible(figure: LeadFigureData, size: LeadFigureSize): { figure: LeadFigureData; hidden: number } {
  if (size === 'full') return { figure, hidden: 0 };
  const limit = CARD_LIMIT[figure.kind];
  switch (figure.kind) {
    case 'stacked': {
      const group = figure.rows[0]?.group;
      const rows = figure.rows.filter((row) => row.group === group).slice(0, limit);
      return { figure: { ...figure, rows }, hidden: figure.rows.length - rows.length };
    }
    case 'paired':
      return { figure: { ...figure, rows: figure.rows.slice(0, limit) }, hidden: Math.max(0, figure.rows.length - limit) };
    case 'ranked':
      return { figure: { ...figure, rows: figure.rows.slice(0, limit) }, hidden: Math.max(0, figure.rows.length - limit) };
    case 'series':
      // A card keeps the first panel: more panels would outgrow the card's plot budget.
      return { figure: { ...figure, panels: figure.panels.slice(0, 1) }, hidden: Math.max(0, figure.panels.length - 1) };
    default:
      return { figure, hidden: 0 };
  }
}

/** What a card left out, in the figure's own unit: panels for a series, rows otherwise. */
const hiddenText = (figure: LeadFigureData, hidden: number) => (figure.kind === 'series'
  ? `+${hidden} ${hidden === 1 ? 'panel' : 'panels'} in the lead's dossier.`
  : `${hidden} more ${hidden === 1 ? 'row is' : 'rows are'} in the lead's dossier.`);

// ---- Row layout shared by ranked, paired and stacked ----------------------------------------

interface RowSlot {
  key: string;
  label: string;
  /** Baseline of the row's label. */
  labelY: number;
  /** Top of the bar band. */
  barY: number;
  /** Space under the bar for stubs and notes, px. */
  under: number;
}

/**
 * The label column every row shares: as wide as the longest label, or, when that would take more
 * than two fifths of the plot, none, and every label sits on its own line above its bar. One rule
 * for all rows, so the bars of a figure always start at the same x.
 */
function labelColumnOf(labels: readonly string[], width: number): number {
  const widest = Math.max(0, ...labels.map((label) => measureText(label, ROW_FONT)));
  return widest + 12 <= width * 0.42 ? Math.ceil(widest + 12) : 0;
}

function rowLayout(items: readonly { key: string; label: string; under?: number; above?: number; header?: string }[], width: number, band: number, top = 4) {
  const labelColumn = labelColumnOf(items.map((item) => item.label), width);
  let y = top;
  const headers: { text: string; y: number }[] = [];
  const rows: RowSlot[] = items.map((item) => {
    if (item.header) {
      headers.push({ text: item.header, y: y + 10 });
      y += 18;
    }
    y += item.above ?? 0;
    const slot = labelColumn > 0
      ? { key: item.key, label: item.label, labelY: y + band / 2, barY: y, under: item.under ?? 0 }
      : { key: item.key, label: item.label, labelY: y + 7, barY: y + 16, under: item.under ?? 0 };
    // A label above its bar needs more air from the row before, or it reads as that row's.
    y = slot.barY + band + slot.under + (labelColumn > 0 ? 10 : 16);
    return slot;
  });
  return { rows, headers, height: y + 4, plotX: labelColumn };
}

function RowLabel({ slot, x = 0 }: { slot: RowSlot; x?: number }) {
  return (
    <text x={x} y={slot.labelY} dy="0.32em" className={ROW_LABEL_CLASS} style={{ paintOrder: 'stroke', stroke: VIZ.surface, strokeWidth: 4, strokeLinejoin: 'round' }}>
      {slot.label}
    </text>
  );
}

/** Lays out excluded stubs left to right, wrapping, from `x0`. */
function stubPositions(stubs: readonly { count: number; label: string }[], x0: number, right: number) {
  let x = x0;
  let line = 0;
  return stubs.map((stub) => {
    const width = 14 + 4 + measureText(`${stub.count.toLocaleString('en-US')} ${stub.label}`) + 12;
    if (x > x0 && x + width > right) {
      x = x0;
      line += 1;
    }
    const at = { x, line };
    x += width;
    return at;
  });
}

const STUB_LINE = 14;

// ---- Kind: ranked ------------------------------------------------------------------------------

function rankedLayout(figure: LeadRankedFigure, width: number, size: LeadFigureSize) {
  const band = size === 'card' ? 12 : 16;
  const layout = rowLayout(figure.rows, width, band);
  const stubs = stubPositions(figure.excluded, layout.plotX, width);
  const stubLines = figure.excluded.length ? Math.max(...stubs.map((stub) => stub.line)) + 1 : 0;
  return { ...layout, band, stubs, stubTop: layout.height, height: layout.height + stubLines * STUB_LINE + (stubLines ? 4 : 0) };
}

function RankedPlot({ figure, width, size, interaction }: { figure: LeadRankedFigure; width: number; size: LeadFigureSize; interaction(i: number): MarkInteraction }) {
  const layout = rankedLayout(figure, width, size);
  const values = figure.rows.map((row) => valueOf(row.value)).filter((v): v is number => v !== null);
  const labels = figure.rows.map((row) => (row.value.state === 'value' ? format(row.value.value, figure.unit, true) : ''));
  const reserve = Math.max(24, ...labels.map((label) => measureText(label) + 10));
  const x = scaleLinear([0, Math.max(0, ...values)], [layout.plotX, Math.max(layout.plotX + 40, width - reserve)]);
  return (
    <g>
      {layout.rows.map((slot, index) => {
        const row = figure.rows[index];
        return (
          <g key={slot.key}>
            <RowLabel slot={slot} />
            <Bar band={{ pos: slot.barY, thickness: layout.band }} value={toMeasured(row.value)} scale={x} role={row.subject ? 'lead' : 'default'} label={labels[index]} interaction={interaction(index)} motionKey={`ranked:${slot.key}`} />
          </g>
        );
      })}
      {figure.excluded.map((stub, index) => (
        <ExcludedStub key={stub.key} band={{ pos: layout.stubTop + layout.stubs[index].line * STUB_LINE, thickness: 10 }} at={layout.stubs[index].x} count={stub.count.toLocaleString('en-US')} label={stub.label} />
      ))}
    </g>
  );
}

// ---- Kind: paired ------------------------------------------------------------------------------

function pairedLayout(figure: LeadPairedFigure, width: number, size: LeadFigureSize) {
  const band = figure.display === 'bullet' ? (size === 'card' ? 12 : 16) : 8;
  const layout = rowLayout(figure.rows, width, band);
  return { ...layout, band, axisY: layout.height, height: layout.height + 22 };
}

function deltaText(subject: number, comparison: number, unit: LeadFigureUnit): string {
  const difference = subject - comparison;
  if (unit === 'percent') return formatValue(difference, { kind: 'pts' });
  return formatValue(difference, { kind: kindOf(unit), signed: true, compact: unit === 'currency' && Math.abs(difference) >= COMPACT_FROM });
}

/** The yardstick half-width about a comparison value: the larger of the share and the floor. */
const bandHalf = (comparison: number, band: NonNullable<LeadPairedFigure['band']>) => Math.max(Math.abs(comparison) * band.percent / 100, band.minimum);

function PairedPlot({ figure, width, size, interaction }: { figure: LeadPairedFigure; width: number; size: LeadFigureSize; interaction(i: number): MarkInteraction }) {
  // The band is an app yardstick; its words must say so, as a ReferenceLine's must.
  if (figure.band) assertYardstickLabel(figure.band.label);
  const layout = pairedLayout(figure, width, size);
  const values = figure.rows.flatMap((row) => [valueOf(row.subject), valueOf(row.comparison)]).filter((v): v is number => v !== null);
  const bandMax = figure.band ? Math.max(0, ...figure.rows.map((row) => valueOf(row.comparison)).filter((v): v is number => v !== null).map((v) => v + bandHalf(v, figure.band!))) : 0;
  const deltas = figure.rows.map((row) => {
    const s = valueOf(row.subject); const c = valueOf(row.comparison);
    if (s === null || c === null) return '';
    if (figure.display === 'bullet') return c === 0 ? deltaText(s, c, figure.unit) : `${formatValue((s - c) / Math.abs(c), { kind: 'percent', signed: true, digits: 0 })}, ${deltaText(s, c, figure.unit)}`;
    return deltaText(s, c, figure.unit);
  });
  // Room after the plot for each row's words: its delta, or an absent bullet subject's reason.
  const reasons = figure.display === 'bullet' ? figure.rows.map((row) => (row.subject.state === 'value' ? 0 : Math.min(width * 0.3, measureText(firstClause(row.subject.reason)) + 20))) : [];
  const reserve = Math.max(40, ...deltas.map((text) => measureText(text) + 16), ...reasons);
  const x = scaleLinear([0, Math.max(...values, bandMax, 0)], [layout.plotX + 6, Math.max(layout.plotX + 60, width - reserve)]);
  return (
    <g>
      <Grid scale={x} extent={[0, layout.axisY]} vertical tickCount={size === 'card' ? 3 : undefined} />
      {layout.rows.map((slot, index) => {
        const row = figure.rows[index];
        const comparison = valueOf(row.comparison);
        const subject = valueOf(row.subject);
        if (figure.display === 'bullet') {
          const half = figure.band && comparison !== null ? bandHalf(comparison, figure.band) : null;
          const bandEnd = half !== null && comparison !== null ? x(comparison + half) : x(0);
          const trackEnd = comparison === null ? x(0) : x(comparison);
          const tick = comparison === null ? null : Math.round(x(comparison)) + 0.5;
          // The delta goes after everything the row draws, so it never overprints the band or tick.
          const deltaX = Math.max(subject === null ? x(0) : x(subject), trackEnd, bandEnd) + 6;
          return (
            <g key={slot.key}>
              <RowLabel slot={slot} />
              {half !== null && comparison !== null && (
                <rect
                  data-reference="yardstick"
                  x={x(Math.max(0, comparison - half))}
                  y={slot.barY - 3}
                  width={Math.max(0, bandEnd - x(Math.max(0, comparison - half)))}
                  height={layout.band + 6}
                  rx={2}
                  style={{ fill: 'none', stroke: REFERENCE_PAINT.yardstick.stroke, strokeWidth: 1, strokeDasharray: REFERENCE_PAINT.yardstick.dash }}
                />
              )}
              {row.subject.state === 'value' ? (
                <Bullet
                  band={{ pos: slot.barY, thickness: layout.band }}
                  scale={x}
                  track={toMeasured(row.comparison)}
                  bar={toMeasured(row.subject)}
                  ticks={[{ key: 'comparison', at: toMeasured(row.comparison), label: `${figure.comparisonLabel}: ${valueText(row.comparison, figure.unit)}` }]}
                  labelLimit={width}
                  interaction={interaction(index)}
                />
              ) : (
                // No subject value: the comparison track and tick stand, and the subject's would-be
                // footprint is a slot after them carrying the builder's reason, never a bar.
                <MarkGroup interaction={interaction(index)} box={{ x: x(0), y: slot.barY, width: Math.max(1, width - x(0)), height: layout.band }}>
                  {comparison !== null && <rect x={x(0)} y={slot.barY} width={Math.max(0, trackEnd - x(0))} height={layout.band} rx={2} style={{ fill: VIZ.track }} />}
                  {tick !== null && <line x1={tick} x2={tick} y1={slot.barY - 2} y2={slot.barY + layout.band + 2} style={{ stroke: VIZ.ink, strokeWidth: 2 }} />}
                  <AbsentSlot
                    absence={row.subject}
                    box={{ x: Math.max(trackEnd, bandEnd) + 6, y: slot.barY, width: Math.max(0, width - Math.max(trackEnd, bandEnd) - 6), height: layout.band }}
                    text={fitWords(firstClause(row.subject.reason), width - Math.max(trackEnd, bandEnd) - 6)}
                  />
                </MarkGroup>
              )}
              {deltas[index] && deltaX + measureText(deltas[index]) <= width + 2 && (
                <text x={deltaX} y={slot.barY + layout.band / 2} dy="0.32em" className={LABEL_CLASS}>{deltas[index]}</text>
              )}
            </g>
          );
        }
        return (
          <g key={slot.key}>
            <RowLabel slot={slot} />
            <Dumbbell y={slot.barY + layout.band / 2} scale={x} from={toMeasured(row.comparison)} to={toMeasured(row.subject)} deltaLabel={deltas[index] || undefined} interaction={interaction(index)} />
          </g>
        );
      })}
      <Axis orient="bottom" offset={layout.axisY} scale={x} kind={kindOf(figure.unit)} tickCount={size === 'card' ? 3 : undefined} />
    </g>
  );
}

// ---- Kind: stacked -----------------------------------------------------------------------------

const SEGMENT_FILL: Record<LeadSegmentTone, string> = {
  lead: VIZ.lead,
  'ord-1': 'var(--viz-ord-1)',
  'ord-2': 'var(--viz-ord-2)',
  'ord-3': 'var(--viz-ord-3)',
  'ord-4': 'var(--viz-ord-4)',
};

const rowTotal = (values: readonly number[]) => values.reduce((sum, value) => sum + Math.max(0, value), 0);

function stackedLayout(figure: LeadStackedFigure, width: number, size: LeadFigureSize) {
  const band = size === 'card' ? 12 : 16;
  const combine = size === 'card';
  const labelColumn = labelColumnOf(figure.rows.map((row) => row.label), width);
  const stubsOf = (row: LeadStackedFigure['rows'][number]) => (combine && row.excluded.length > 1
    ? [{ key: 'outside', label: 'outside the bar', count: row.excluded.reduce((sum, stub) => sum + stub.count, 0) }]
    : row.excluded);
  const stubs = figure.rows.map((row) => stubPositions(stubsOf(row), labelColumn, width));
  const groups = new Set(figure.rows.map((row) => row.group)).size;
  const layout = rowLayout(figure.rows.map((row, index) => {
    const lines = row.excluded.length ? Math.max(...stubs[index].map((stub) => stub.line)) + 1 : 0;
    return {
      key: row.key,
      label: row.label,
      above: row.overlay ? 6 : 0,
      under: lines * STUB_LINE + (row.note ? STUB_LINE : 0) + (lines || row.note ? 2 : 0),
      header: groups > 1 && (index === 0 || figure.rows[index - 1].group !== row.group) ? row.group : undefined,
    };
  }), width, band);
  // Counts (not shares) get a value axis under the rows.
  return { ...layout, band, stubs, stubsOf, axisY: layout.height, height: layout.height + (figure.normalise ? 0 : 22) };
}

/** Text on a segment's fill: the ramp's own on-colour, or the surface on the lead hue. */
const onFill = (tone: LeadSegmentTone) => (tone === 'lead' ? VIZ.surface : `var(--viz-on-${tone})`);
const SMALL_LABEL_FONT = '10px -apple-system, BlinkMacSystemFont, "SF Pro Text", "Segoe UI", Roboto, sans-serif';

function StackedPlot({ figure, width, size, interaction }: { figure: LeadStackedFigure; width: number; size: LeadFigureSize; interaction(i: number): MarkInteraction }) {
  const layout = stackedLayout(figure, width, size);
  const totals = figure.rows.map((row) => rowTotal(row.values));
  const negatives = figure.rows.map((row) => row.values.reduce((sum, value) => sum + Math.min(0, value), 0));
  // A row's end label: its total, or with units removed below zero, the net and the removal.
  const totalLabels = totals.map((total, index) => (!figure.normalise && negatives[index] < 0
    ? `${format(total + negatives[index], figure.unit, true)} net (${format(negatives[index], figure.unit)} removed)`
    : format(total, figure.unit, true)));
  const reserve = Math.max(28, ...totalLabels.map((label) => measureText(label) + 10));
  const right = Math.max(layout.plotX + 60, width - reserve);
  const least = Math.min(0, ...negatives);
  // The positive side gets nice ticks; the negative side ends at the data, so a few units removed
  // never push zero a whole tick step in from the left.
  const top = scaleLinear([0, Math.max(0, ...totals)], [0, 1]).domain[1];
  const x: ContinuousScale = figure.normalise
    ? scaleLinear([0, 1], [layout.plotX, right], { nice: false })
    : scaleLinear([least, top], [layout.plotX, right], { nice: false });
  const zero = x(0);
  return (
    <g>
      {!figure.normalise && (
        <>
          <Grid scale={x} extent={[0, layout.axisY]} vertical tickCount={size === 'card' ? 3 : undefined} />
          <line x1={Math.round(zero) + 0.5} x2={Math.round(zero) + 0.5} y1={0} y2={layout.axisY} style={{ stroke: VIZ.baseline, strokeWidth: 1 }} />
          <Axis orient="bottom" offset={layout.axisY} scale={x} kind={kindOf(figure.unit)} tickCount={size === 'card' ? 3 : undefined} line={false} />
        </>
      )}
      {layout.headers.map((header) => <text key={`${header.text}-${header.y}`} x={0} y={header.y} className="fill-fg text-[12px] font-medium">{header.text}</text>)}
      {layout.rows.map((slot, index) => {
        const row = figure.rows[index];
        const total = totals[index];
        const negative = negatives[index];
        // Positive segments stack outward from zero; units removed are one slate block below it.
        const segments = figure.segments.map((segment, s) => {
          const raw = Math.max(0, row.values[s] ?? 0);
          const value = figure.normalise ? (total > 0 ? raw / total : 0) : raw;
          const text = figure.normalise ? formatValue(value, { kind: 'percent', digits: 0 }) : format(raw, figure.unit, true);
          return { key: segment.key, value, tone: segment.tone, label: text };
        });
        const extent = figure.normalise ? [x(0), x(total > 0 ? 1 : 0)] : [x(negative), x(total)];
        const stubTop = slot.barY + layout.band + 4;
        const stubLines = row.excluded.length ? Math.max(...layout.stubs[index].map((stub) => stub.line)) + 1 : 0;
        // StackedBar labels a segment only on a band of 14px or more; a card's 12px band gets
        // 10px labels here, inside each segment wide enough to hold one.
        let running = 0;
        const smallLabels = layout.band < 14 ? segments.flatMap((segment) => {
          const a0 = x(running); running += segment.value; const a1 = x(running);
          return segment.value > 0 && measureText(segment.label, SMALL_LABEL_FONT) + 6 <= a1 - a0 - 2
            ? [{ key: segment.key, x: (a0 + a1 - 2) / 2, label: segment.label, fill: onFill(segment.tone) }] : [];
        }) : [];
        const removed = !figure.normalise && negative < 0;
        return (
          <g key={slot.key}>
            <RowLabel slot={slot} />
            {row.overlay && total > 0 && (
              <Bar band={{ pos: slot.barY - 5, thickness: 3 }} value={measured(figure.normalise ? row.overlay.value / total : row.overlay.value)} scale={x} role="primary" />
            )}
            {total === 0 && negative === 0 ? (
              <AbsentSlot absence={absent('not-recorded', 'no records in any segment')} box={{ x: zero, y: slot.barY, width: right - zero, height: layout.band }} text="none recorded" />
            ) : (
              <StackedBar band={{ pos: slot.barY, thickness: layout.band }} segments={segments} scale={x} interaction={interaction(index)} motionKey={`stacked:${slot.key}`} />
            )}
            {removed && (
              <g data-removed="">
                <path d={hBarPath(zero - 1, x(negative), slot.barY, layout.band)} style={{ fill: VIZ.context }} />
              </g>
            )}
            {smallLabels.map((label) => (
              <text key={label.key} x={label.x} y={slot.barY + layout.band / 2} dy="0.32em" textAnchor="middle" pointerEvents="none" className="viz-num text-[10px]" style={{ fill: label.fill }}>{label.label}</text>
            ))}
            {row.partial && (
              <rect x={extent[0] - 2} y={slot.barY - 2} width={Math.max(0, extent[1] - extent[0] + 4)} height={layout.band + 4} rx={3} style={{ fill: 'none', stroke: VIZ.baseline, strokeWidth: 1, strokeDasharray: '3 2' }} />
            )}
            {(figure.normalise || total > 0 || removed) && (
              <text x={figure.normalise ? right + 6 : x(total) + 6} y={slot.barY + layout.band / 2} dy="0.32em" className={LABEL_CLASS}>{totalLabels[index]}</text>
            )}
            {layout.stubsOf(row).map((stub, s) => (
              <ExcludedStub key={stub.key} band={{ pos: stubTop + layout.stubs[index][s].line * STUB_LINE, thickness: 10 }} at={layout.stubs[index][s].x} count={stub.count.toLocaleString('en-US')} label={stub.label} />
            ))}
            {row.note && <text x={zero + 2} y={stubTop + stubLines * STUB_LINE + 9} className={MUTED_LABEL_CLASS}>{row.note}</text>}
          </g>
        );
      })}
    </g>
  );
}

// ---- Kind: series ------------------------------------------------------------------------------

function seriesLayout(figure: LeadSeriesFigure, size: LeadFigureSize) {
  const panelHeight = size === 'card' ? 40 : 64;
  const header = 16;
  const gap = 10;
  return { panelHeight, header, gap, height: figure.panels.length * (header + panelHeight + gap) + 18 };
}

/** Index runs of consecutive recorded values: each is drawn as its own line, so no line crosses a gap. */
function runsOf(values: readonly LeadFigureValue[]): number[][] {
  const runs: number[][] = [];
  values.forEach((value, i) => {
    if (value.state !== 'value') return;
    const last = runs[runs.length - 1];
    if (last && last[last.length - 1] === i - 1) last.push(i);
    else runs.push([i]);
  });
  return runs;
}

/** Runs of consecutive absent values with the same state and reason. */
function absentRuns(values: readonly LeadFigureValue[]): { from: number; to: number; absence: Absence }[] {
  const runs: { from: number; to: number; absence: Absence }[] = [];
  values.forEach((value, i) => {
    if (value.state === 'value') return;
    const last = runs[runs.length - 1];
    if (last && last.to === i - 1 && last.absence.state === value.state && last.absence.reason === value.reason) last.to = i;
    else runs.push({ from: i, to: i, absence: value });
  });
  return runs;
}

/** Change from the first recorded value to the latest: percentage points for a rate (a relative
 * change of a rate overstates it), a signed percent otherwise; '' when the first is zero. */
function changeText(first: number, last: number, unit: LeadFigureUnit): string {
  if (unit === 'percent') return formatValue(last - first, { kind: 'pts' });
  return first === 0 ? '' : formatValue((last - first) / Math.abs(first), { kind: 'percent', signed: true, digits: 0 });
}

function SeriesPlot({ figure, width, size, interaction }: { figure: LeadSeriesFigure; width: number; size: LeadFigureSize; interaction(i: number): MarkInteraction }) {
  const { panelHeight, header, gap, height } = seriesLayout(figure, size);
  // The year under the pointer or keyboard focus, drawn as one rule through every panel.
  const [active, setActive] = useState<number | null>(null);
  const n = figure.points.length;
  const tickWidth = Math.max(...figure.panels.flatMap((panel) => {
    const max = Math.max(0, ...[...panel.values, ...(panel.context?.values ?? [])].map(valueOf).filter((v): v is number => v !== null));
    return [measureText(format(max, panel.unit, true))];
  }), 24);
  const left = Math.ceil(tickWidth) + 10;
  const ends = figure.panels.map((panel) => {
    const defined = panel.values.map(valueOf).filter((v): v is number => v !== null);
    if (!defined.length) return { first: '', last: '' };
    const first = defined[0]; const last = defined[defined.length - 1];
    const change = defined.length > 1 ? changeText(first, last, panel.unit) : '';
    return { first: format(first, panel.unit), last: `${format(last, panel.unit)}${change ? ` (${change})` : ''}` };
  });
  const right = width - Math.max(16, ...ends.map((end) => measureText(end.last) + 14));
  // Points sit inside the plot by half a column, so a gap's slot never covers the axis labels.
  const inset = n > 1 ? Math.min(20, (right - left) / (2 * n)) : 0;
  const step = n > 1 ? (right - left - 2 * inset) / (n - 1) : right - left;
  const xOf = (i: number) => (n > 1 ? left + inset + i * step : (left + right) / 2);
  const column = Math.max(12, Math.min(2 * inset, step * 0.6, 40));
  const axisY = height - 18;
  // Thin the x labels so none collides with its neighbour.
  const widest = Math.max(...figure.points.map((point) => measureText(point.label)));
  const every = Math.max(1, Math.ceil((widest + 8) / Math.max(1, step)));
  const track = (i: number): MarkInteraction => {
    const base = interaction(i);
    return {
      ...base,
      onPointerMove: (event) => { setActive(i); base.onPointerMove?.(event); },
      onPointerLeave: () => { setActive(null); base.onPointerLeave?.(); },
      onFocusMark: (box) => { setActive(i); base.onFocusMark?.(box); },
      onBlurMark: () => { setActive(null); base.onBlurMark?.(); },
    };
  };
  // A slot's words: the reason's first clause cut to the slot, upright when the slot is tall and
  // narrow. The whole reason is the slot's title.
  const slotWords = (reason: string, box: { width: number; height: number }) => fitWords(firstClause(reason), box.height > box.width * 2 && box.width >= 12 ? box.height : box.width);
  return (
    <g>
      {figure.panels.map((panel, p) => {
        const top = p * (header + panelHeight + gap) + header;
        const all = [...panel.values, ...(panel.context?.values ?? [])].map(valueOf).filter((v): v is number => v !== null);
        const y = scaleLinear([0, Math.max(0, ...all)], [top + panelHeight, top + 4]);
        const runs = runsOf(panel.values);
        const contextRuns = panel.context ? runsOf(panel.context.values) : [];
        const firstIndex = runs[0]?.[0];
        const lastRun = runs[runs.length - 1];
        const lastIndex = lastRun?.[lastRun.length - 1];
        return (
          <g key={panel.key}>
            <text x={0} y={top - 4} className="fill-fg-secondary text-[12px]">{panel.label}</text>
            <Grid scale={y} extent={[left, right]} tickCount={2} />
            <Axis orient="left" offset={left} scale={y} kind={kindOf(panel.unit)} tickCount={2} />
            {absentRuns(panel.values).map(({ from, to, absence }) => {
              // Neighbouring years absent for the same reason share one slot, so the reason has room.
              const box = { x: xOf(from) - column / 2, y: top + 2, width: xOf(to) - xOf(from) + column, height: panelHeight - 2 };
              return <AbsentSlot key={`gap-${from}`} absence={absence} box={box} text={slotWords(absence.reason, box)} />;
            })}
            {panel.values.map((value, i) => {
              if (value.state !== 'value') return null;
              const context = panel.context?.values[i];
              if (!context || context.state === 'value') return null;
              // The comparison line's own absence: a low slot on the baseline, under the line.
              const box = { x: xOf(i) - column / 2, y: top + panelHeight - 12, width: column, height: 12 };
              return <AbsentSlot key={`context-gap-${i}`} absence={context} box={box} text={slotWords(context.reason, box)} />;
            })}
            {panel.context && contextRuns.map((run) => (
              <LineSeries key={`context:${run[0]}`} points={run.map((i) => ({ key: `${panel.key}:context:${figure.points[i].key}`, x: i, y: toMeasured(panel.context!.values[i]) }))} x={xOf} y={y} role="comparison" markers="observed" />
            ))}
            {runs.map((run) => (
              <LineSeries
                key={`run:${run[0]}`}
                points={run.map((i) => ({ key: `${panel.key}:${figure.points[i].key}`, x: i, y: toMeasured(panel.values[i]), incomplete: figure.points[i].partial }))}
                x={xOf}
                y={y}
                role="primary"
                markers="observed"
                label={run === lastRun ? ends[p].last || undefined : undefined}
                motionKey={`series:${panel.key}:${run[0]}`}
              />
            ))}
            {firstIndex !== undefined && firstIndex !== lastIndex && (
              <text x={xOf(firstIndex)} y={y(valueOf(panel.values[firstIndex])!) - 8} textAnchor="middle" className={LABEL_CLASS} style={{ paintOrder: 'stroke', stroke: VIZ.surface, strokeWidth: 3 }}>
                {ends[p].first}
              </text>
            )}
          </g>
        );
      })}
      {active !== null && (
        <line data-crosshair="" x1={Math.round(xOf(active)) + 0.5} x2={Math.round(xOf(active)) + 0.5} y1={header - 2} y2={axisY} pointerEvents="none" style={{ stroke: VIZ.ink, strokeWidth: 1 }} />
      )}
      <line x1={left} x2={right} y1={axisY + 0.5} y2={axisY + 0.5} style={{ stroke: VIZ.baseline, strokeWidth: 1 }} />
      {figure.points.map((point, i) => (
        <g key={point.key}>
          {(i % every === 0 || i === n - 1) && (i === n - 1 || n - 1 - i >= every) && (
            <text x={xOf(i)} y={axisY + 14} textAnchor="middle" className={TICK_CLASS}>{point.label}</text>
          )}
          <MarkGroup interaction={track(i)} box={{ x: xOf(i) - Math.max(6, Math.min(step, 48) / 2), y: 0, width: Math.max(12, Math.min(step, 48)), height: axisY }}>
            <g />
          </MarkGroup>
        </g>
      ))}
    </g>
  );
}

// ---- Kind: bins --------------------------------------------------------------------------------

const DENOMINATOR_TONES = ['ord-1', 'ord-2', 'ord-3', 'ord-4'] as const;

/**
 * Bins on one linear hour axis: a closed bin spans its hours, and its height is records per hour,
 * so a wide bin's count is its area, not a spike. The open-ended last bin has no width to divide
 * by; it sits past an axis break as a fixed tile labelled with its count, off the height scale.
 */
function binsGeometry(figure: LeadBinsFigure, width: number, size: LeadFigureSize) {
  const plot = size === 'card' ? 96 : 150;
  // Room above the plot for the reference line's label, clear of the tallest column's count.
  const top = 30;
  const bottom = top + plot;
  const left = 34;
  const closed = figure.bins.filter((bin) => bin.to !== null);
  const open = figure.bins.find((bin) => bin.to === null) ?? null;
  const tileWidth = open ? (size === 'card' ? 48 : 64) : 0;
  const breakGap = open ? 10 : 0;
  const end = Math.max(1, ...closed.map((bin) => bin.to!), figure.reference?.at ?? 0);
  const plotRight = width - 8 - tileWidth - breakGap;
  const x = scaleLinear([0, end], [left, Math.max(left + 40, plotRight)], { nice: false });
  const tileX = x(end) + breakGap;
  const densities = closed.map((bin) => bin.count / Math.max(1e-9, bin.to! - bin.from));
  const y = scaleLinear([0, Math.max(0, ...densities)], [bottom, top]);
  let cursor = bottom + 22;
  const medianY = figure.median === null ? null : cursor;
  if (figure.median !== null) cursor += 20;
  const beforeY = figure.before ? cursor : null;
  if (figure.before) cursor += STUB_LINE + 4;
  const strip = figure.denominator.length ? (() => {
    const total = figure.denominator.reduce((sum, item) => sum + item.count, 0);
    let at = left; let line = 0;
    const labels = figure.denominator.map((item) => {
      const text = `${item.count.toLocaleString('en-US')} ${item.label}`;
      const w = 14 + measureText(text) + 12;
      if (at > left && at + w > width) { at = left; line += 1; }
      const placed = { key: item.key, text, x: at, line };
      at += w;
      return placed;
    });
    const y0 = cursor + 4;
    cursor = y0 + 10 + 6 + (Math.max(...labels.map((label) => label.line)) + 1) * STUB_LINE;
    return { total, y: y0, labels };
  })() : null;
  return { plot, top, bottom, left, closed, open, tileX, tileWidth, x, y, densities, medianY, beforeY, strip, height: cursor + 4 };
}

function BinsPlot({ figure, width, size, interaction }: { figure: LeadBinsFigure; width: number; size: LeadFigureSize; interaction(i: number): MarkInteraction }) {
  const g = binsGeometry(figure, width, size);
  const { top, bottom, left, x, y, open, tileX, tileWidth } = g;
  const edges = [...new Set(g.closed.flatMap((bin) => [bin.from, bin.to!]))].sort((a, b) => a - b);
  // Edge ticks, then the open tile's words, kept left to right while none touches the last kept.
  const candidates = [
    ...edges.map((edge, i) => ({ key: `edge-${edge}`, at: x(edge), text: i === edges.length - 1 && !open ? `${edge} h` : String(edge) })),
    ...(open ? [{ key: 'open', at: tileX + tileWidth / 2, text: `${open.from}+ h` }] : []),
  ];
  let lastRight = -Infinity;
  const ticks = candidates.filter((tick) => {
    const half = measureText(tick.text) / 2;
    if (tick.at - half < lastRight + 6) return false;
    lastRight = tick.at + half;
    return true;
  });
  // Column counts, thinned the same way so a narrow column's count never runs into the next.
  let countRight = -Infinity;
  const counts = g.closed.map((bin, i) => {
    const text = format(bin.count, 'count', true);
    const at = (x(bin.from) + x(bin.to!)) / 2;
    const half = measureText(text) / 2;
    const show = at - half >= countRight + 4;
    if (show) countRight = at + half;
    return { key: bin.key, text, at, y: y(g.densities[i]) - 4, show };
  });
  const median = figure.median === null ? null : open && figure.median >= open.from ? tileX + tileWidth / 2 : x(figure.median);
  const indexOf = (key: string) => figure.bins.findIndex((bin) => bin.key === key);
  return (
    <g>
      <text x={0} y={top - 8} className={TICK_CLASS}>per hour</text>
      <Axis orient="left" offset={left - 4} scale={y} kind="count" tickCount={3} />
      {figure.reference && (
        <ReferenceLine kind="yardstick" axis="x" at={x(figure.reference.at)} extent={[2, bottom]} label={`${formatValue(figure.reference.at, { kind: 'hours' })} reference`} labelSide="start" />
      )}
      {g.closed.map((bin, i) => {
        const x0 = x(bin.from); const x1 = x(bin.to!);
        return (
          <Bar key={bin.key} orient="vertical" band={{ pos: x0 + 0.5, thickness: Math.max(1, x1 - x0 - 1.5) }} value={measured(g.densities[i])} scale={y} interaction={interaction(indexOf(bin.key))} motionKey={`bins:${bin.key}`} />
        );
      })}
      {counts.filter((item) => item.show).map((item) => (
        <text key={`count-${item.key}`} x={item.at} y={item.y} textAnchor="middle" className={LABEL_CLASS}>{item.text}</text>
      ))}
      {open && (
        <g data-open-bin="">
          {/* The axis break: the open bin is past it, at no scale. */}
          <path d={`M${tileX - 8},${bottom + 4}l3,-8M${tileX - 4},${bottom + 4}l3,-8`} style={{ stroke: VIZ.baseline, strokeWidth: 1, fill: 'none' }} />
          <MarkGroup interaction={interaction(indexOf(open.key))} box={{ x: tileX, y: bottom - 18, width: tileWidth, height: 18 }}>
            <rect x={tileX + 0.5} y={bottom - 17.5} width={tileWidth - 1} height={17} rx={2} style={{ fill: VIZ.track, stroke: VIZ.mark, strokeWidth: 1 }} />
            <path d={`M${tileX + tileWidth - 9},${bottom - 13}l4,4.5l-4,4.5`} style={{ stroke: VIZ.ink, strokeWidth: 1.5, fill: 'none' }} />
            <text x={tileX + tileWidth / 2} y={bottom - 22} textAnchor="middle" className={LABEL_CLASS}>{format(open.count, 'count', true)}</text>
          </MarkGroup>
        </g>
      )}
      <line x1={left} x2={x.range[1]} y1={bottom + 0.5} y2={bottom + 0.5} style={{ stroke: VIZ.baseline, strokeWidth: 1 }} />
      {ticks.map((tick) => (
        <text key={tick.key} x={tick.at} y={bottom + 14} textAnchor="middle" className={TICK_CLASS}>{tick.text}</text>
      ))}
      {median !== null && g.medianY !== null && (
        <g data-median="">
          <path d={`M${median},${g.medianY}l-4,6h8Z`} style={{ fill: VIZ.ink }} />
          <text x={Math.min(Math.max(median, left + 44), width - 44)} y={g.medianY + 16} textAnchor="middle" className={LABEL_CLASS}>{`median ${formatValue(figure.median!, { kind: 'hours', digits: 1 })}`}</text>
        </g>
      )}
      {figure.before && g.beforeY !== null && (
        <ExcludedStub band={{ pos: g.beforeY, thickness: 10 }} at={left} count={figure.before.count.toLocaleString('en-US')} label={figure.before.label} />
      )}
      {g.strip && g.strip.total > 0 && (
        <g data-denominator="">
          <StackedBar
            band={{ pos: g.strip.y, thickness: 10 }}
            segments={figure.denominator.map((item, i) => ({ key: item.key, value: item.count / g.strip!.total, tone: DENOMINATOR_TONES[i % DENOMINATOR_TONES.length], label: '' }))}
            scale={scaleLinear([0, 1], [left, width - 8], { nice: false })}
          />
          {g.strip.labels.map((label, i) => (
            <g key={label.key}>
              <rect x={label.x} y={g.strip!.y + 16 + label.line * STUB_LINE} width={10} height={10} rx={2} style={{ fill: `var(--viz-${DENOMINATOR_TONES[i % DENOMINATOR_TONES.length]})` }} />
              <text x={label.x + 14} y={g.strip!.y + 21 + label.line * STUB_LINE} dy="0.32em" className={MUTED_LABEL_CLASS}>{label.text}</text>
            </g>
          ))}
        </g>
      )}
    </g>
  );
}

// ---- Table twins, legends, summaries and focusable marks -------------------------------------

function tableOf(figure: LeadFigureData): TwinTable {
  const kind = (unit: LeadFigureUnit): ValueKind => kindOf(unit);
  switch (figure.kind) {
    case 'series':
      return {
        caption: figure.measure,
        columns: [{ key: 'point', label: 'Year' }, ...figure.panels.flatMap((panel) => [
          { key: panel.key, label: panel.label, kind: kind(panel.unit) },
          ...(panel.context ? [{ key: `${panel.key}:context`, label: panel.context.label, kind: kind(panel.unit) }] : []),
        ])],
        rows: figure.points.map((point, i) => Object.fromEntries([['point', `${point.label}${point.partial ? ' (so far)' : ''}`], ...figure.panels.flatMap((panel) => [
          [panel.key, cell(panel.values[i])],
          ...(panel.context ? [[`${panel.key}:context`, cell(panel.context.values[i])]] : []),
        ])])),
      };
    case 'stacked': {
      const stubs = [...new Map(figure.rows.flatMap((row) => row.excluded).map((stub) => [stub.key, stub.label])).entries()];
      const overlay = figure.rows.find((row) => row.overlay)?.overlay?.label;
      return {
        caption: figure.measure,
        columns: [{ key: 'group', label: 'Group' }, { key: 'label', label: 'Row' }, ...figure.segments.map((segment) => ({ key: segment.key, label: segment.label, kind: kind(figure.unit) })),
          ...stubs.map(([key, label]) => ({ key: `x:${key}`, label: `Outside the bar: ${label}`, kind: kind(figure.unit) })),
          ...(overlay ? [{ key: 'overlay', label: overlay, kind: kind(figure.unit) }] : []), { key: 'note', label: 'Note' }],
        rows: figure.rows.map((row) => ({
          group: row.group, label: row.label, note: row.note ?? '',
          ...Object.fromEntries(figure.segments.map((segment, s) => [segment.key, row.values[s] ?? 0])),
          ...Object.fromEntries(stubs.map(([key]) => [`x:${key}`, row.excluded.find((stub) => stub.key === key)?.count ?? 0])),
          ...(overlay ? { overlay: row.overlay?.value ?? null } : {}),
        })),
      };
    }
    case 'paired':
      return {
        caption: figure.measure,
        columns: [{ key: 'label', label: 'Row' }, { key: 'subject', label: figure.subjectLabel, kind: kind(figure.unit) }, { key: 'subjectBasis', label: `${figure.subjectLabel}: count` },
          { key: 'comparison', label: figure.comparisonLabel, kind: kind(figure.unit) }, { key: 'comparisonBasis', label: `${figure.comparisonLabel}: count` }],
        rows: figure.rows.map((row) => ({ label: row.label, subject: cell(row.subject), subjectBasis: row.subjectBasis ?? '', comparison: cell(row.comparison), comparisonBasis: row.comparisonBasis ?? '' })),
      };
    case 'bins':
      return {
        caption: figure.measure,
        columns: [{ key: 'label', label: 'Hours after posting' }, { key: 'count', label: 'Count', kind: 'count' }],
        rows: [
          ...(figure.before ? [{ label: figure.before.label, count: absent('excluded', `${figure.before.count.toLocaleString('en-US')} ${figure.before.label}`) }] : []),
          ...figure.bins.map((bin) => ({ label: bin.label, count: bin.count })),
          ...(figure.median === null ? [] : [{ label: 'Median hours', count: figure.median }]),
          ...figure.denominator.map((item) => ({ label: item.label, count: item.count })),
        ],
      };
    case 'ranked':
      return {
        caption: figure.measure,
        columns: [{ key: 'label', label: 'Row' }, { key: 'value', label: 'Value', kind: kind(figure.unit) }, { key: 'detail', label: 'Detail' }],
        rows: [
          ...figure.rows.map((row) => ({ label: row.label, value: cell(row.value), detail: row.detail ?? '' })),
          ...figure.excluded.map((stub) => ({ label: stub.label, value: absent('excluded', `${stub.count.toLocaleString('en-US')} ${stub.label}`), detail: '' })),
        ],
      };
  }
}

function legendOf(figure: LeadFigureData): LegendItem[] {
  switch (figure.kind) {
    case 'series':
      return figure.panels.some((panel) => panel.context)
        ? [{ key: 'series', label: 'Recorded value', swatch: 'line', tone: 'ink' }, ...figure.panels.flatMap((panel) => (panel.context ? [{ key: `${panel.key}:context`, label: panel.context.label, swatch: 'dashed-line' as const, tone: 'context' as const }] : []))]
        : [];
    case 'paired':
      return figure.display === 'bullet'
        ? [{ key: 'subject', label: figure.subjectLabel, swatch: 'rect', tone: 'ink' }, { key: 'comparison', label: `${figure.comparisonLabel} (track and tick)`, swatch: 'rect', tone: 'track' },
          ...(figure.band ? [{ key: 'band', label: figure.band.label, swatch: 'dashed-line' as const, tone: 'context' as const }] : [])]
        : [{ key: 'subject', label: figure.subjectLabel, swatch: 'dot', tone: 'ink' }, { key: 'comparison', label: figure.comparisonLabel, swatch: 'hollow', tone: 'context' }];
    case 'bins':
      return figure.reference ? [{ key: 'reference', label: figure.reference.label, swatch: 'dashed-line', tone: 'context' }] : [];
    case 'ranked':
      return figure.rows.some((row) => row.subject) && figure.rows.some((row) => !row.subject)
        ? [{ key: 'subject', label: 'The lead\'s subject', swatch: 'rect', tone: 'lead' }, { key: 'other', label: 'Others', swatch: 'rect', tone: 'mark' }]
        : [];
    case 'stacked':
      return [];
  }
}

/** The ordinal and lead tones are not legend tones, so the stacked kind keys its own swatches. */
function SegmentLegend({ figure }: { figure: LeadStackedFigure }) {
  const overlay = figure.rows.find((row) => row.overlay)?.overlay;
  return (
    <ul aria-label="Segments" className="flex flex-wrap items-center gap-x-4 gap-y-1 text-caption text-fg-secondary">
      {figure.segments.map((segment) => (
        <li key={segment.key} className="flex h-6 items-center gap-1.5">
          <svg width={16} height={12} viewBox="0 0 16 12" aria-hidden="true" className="shrink-0"><rect x={2} y={2} width={12} height={8} rx={2} style={{ fill: SEGMENT_FILL[segment.tone] }} /></svg>
          <span>{segment.label}</span>
        </li>
      ))}
      {overlay && (
        <li className="flex h-6 items-center gap-1.5">
          <svg width={16} height={12} viewBox="0 0 16 12" aria-hidden="true" className="shrink-0"><rect x={1} y={5} width={14} height={3} rx={1} style={{ fill: VIZ.ink }} /></svg>
          <span>{overlay.label}</span>
        </li>
      )}
    </ul>
  );
}

interface FocusMark {
  /** The payload's own key for the mark (row, year or bin), passed on activation. */
  key: string;
  name: string;
  title: string;
  rows: TooltipRow[];
  note?: string;
}

function marksOf(figure: LeadFigureData): FocusMark[] {
  switch (figure.kind) {
    case 'series':
      return figure.points.map((point, i) => {
        const rows = figure.panels.flatMap((panel) => [
          { key: panel.key, value: valueText(panel.values[i], panel.unit), label: panel.label },
          ...(panel.context ? [{ key: `${panel.key}:context`, value: valueText(panel.context.values[i], panel.unit), label: panel.context.label }] : []),
        ]);
        return { key: point.key, title: `${point.label}${point.partial ? ' so far' : ''}`, rows, name: `${point.label}: ${rows.map((row) => `${row.label} ${row.value}`).join('; ')}.` };
      });
    case 'stacked':
      return figure.rows.map((row) => {
        const total = rowTotal(row.values);
        const share = (value: number) => (figure.normalise && total > 0 ? ` (${formatValue(value / total, { kind: 'percent' })})` : '');
        const rows: TooltipRow[] = [
          ...figure.segments.map((segment, s) => ({ key: segment.key, value: `${format(row.values[s] ?? 0, figure.unit)}${share(row.values[s] ?? 0)}`, label: segment.label })),
          ...(row.overlay ? [{ key: 'overlay', value: `${format(row.overlay.value, figure.unit)}${share(row.overlay.value)}`, label: row.overlay.label }] : []),
          ...row.excluded.map((stub) => ({ key: `x:${stub.key}`, value: format(stub.count, figure.unit), label: `${stub.label} (outside the bar)` })),
        ];
        return { key: row.key, title: `${row.group}, ${row.label}`, rows, note: row.note ?? undefined, name: `${row.group}, ${row.label}: ${rows.map((item) => `${item.value} ${item.label}`).join('; ')}.` };
      });
    case 'paired':
      return figure.rows.map((row) => {
        const rows = [
          { key: 'subject', value: `${valueText(row.subject, figure.unit)}${row.subjectBasis ? ` (${row.subjectBasis})` : ''}`, label: figure.subjectLabel },
          { key: 'comparison', value: `${valueText(row.comparison, figure.unit)}${row.comparisonBasis ? ` (${row.comparisonBasis})` : ''}`, label: figure.comparisonLabel },
        ];
        return { key: row.key, title: row.label, rows, note: figure.band ? figure.band.label : undefined, name: `${row.label}: ${rows.map((item) => `${item.label} ${item.value}`).join('; ')}.` };
      });
    case 'bins': {
      const total = figure.bins.reduce((sum, bin) => sum + bin.count, 0);
      return figure.bins.map((bin) => {
        const rows = [{ key: 'count', value: `${format(bin.count, 'count')}${total > 0 ? ` (${formatValue(bin.count / total, { kind: 'percent' })})` : ''}`, label: bin.label }];
        return { key: bin.key, title: bin.label, rows, name: `${bin.label}: ${rows[0].value}.` };
      });
    }
    case 'ranked':
      return figure.rows.map((row) => {
        const rows = [{ key: 'value', value: valueText(row.value, figure.unit), label: row.detail ?? figure.measure }];
        return { key: row.key, title: row.label, rows, name: `${row.label}: ${rows[0].value}${row.detail ? `, ${row.detail}` : ''}.` };
      });
  }
}

function summaryOf(figure: LeadFigureData, hidden: number): string {
  const absences = (values: readonly LeadFigureValue[]) => values.filter((value) => value.state !== 'value').length;
  const parts: string[] = [figure.measure];
  switch (figure.kind) {
    case 'series': {
      const gaps = figure.panels.reduce((sum, panel) => sum + absences(panel.values), 0);
      parts.push(`${figure.points.length} years from ${figure.points[0]?.label ?? ''} to ${figure.points.at(-1)?.label ?? ''} in ${figure.panels.length} panels`);
      if (gaps) parts.push(`${gaps} values not recorded or set aside, drawn as gaps`);
      break;
    }
    case 'stacked':
      parts.push(`${figure.rows.length} rows of ${figure.segments.map((segment) => segment.label).join(', ')}`);
      if (figure.rows.some((row) => row.excluded.length)) parts.push(`records outside each bar: ${figure.rows.reduce((sum, row) => sum + row.excluded.reduce((s, stub) => s + stub.count, 0), 0).toLocaleString('en-US')}`);
      break;
    case 'paired':
      parts.push(`${figure.rows.length} rows comparing ${figure.subjectLabel} with ${figure.comparisonLabel}`);
      break;
    case 'bins':
      parts.push(`${figure.bins.length} bins`);
      if (figure.before) parts.push(`${figure.before.count.toLocaleString('en-US')} ${figure.before.label}, outside the bins`);
      break;
    case 'ranked':
      parts.push(`${figure.rows.length} rows`);
      if (figure.excluded.length) parts.push(figure.excluded.map((stub) => `${stub.count.toLocaleString('en-US')} ${stub.label}`).join(', '));
      break;
  }
  if (hidden) parts.push(hiddenText(figure, hidden).replace(/\.$/, ''));
  return `${parts.join('. ')}.`;
}

function plotHeightOf(figure: LeadFigureData, width: number, size: LeadFigureSize): number {
  switch (figure.kind) {
    case 'ranked':
      return rankedLayout(figure, width, size).height;
    case 'paired':
      return pairedLayout(figure, width, size).height;
    case 'stacked':
      return stackedLayout(figure, width, size).height;
    case 'series':
      return seriesLayout(figure, size).height;
    case 'bins':
      return binsGeometry(figure, width, size).height;
  }
}

function rowCount(figure: LeadFigureData): number {
  switch (figure.kind) {
    case 'series':
      return figure.points.length;
    case 'bins':
      return figure.bins.length;
    default:
      return figure.rows.length;
  }
}

/**
 * The lead figure slot. It composes the shared marks directly, inside the shared frame: heading,
 * claim tag, 'View as table', the figure's absences in the legend, and the tables it was read from.
 */
export default function LeadFigure({ figure: full, headline, sources, size, onActivate }: LeadFigureProps) {
  const { figure, hidden } = useMemo(() => visible(full, size), [full, size]);
  const marks = useMemo(() => marksOf(figure), [figure]);
  const table = useMemo(() => tableOf(figure), [figure]);
  const tooltip = useTooltip();
  const roving = useRovingFocus({ shape: { rows: Math.max(1, marks.length) }, onActivate: (i) => { if (marks[i]) onActivate?.(marks[i].key); }, onEscape: tooltip.hide });
  const interaction = (index: number): MarkInteraction => {
    const mark = marks[index];
    const content = <TooltipContent title={mark.title} rows={mark.rows} note={mark.note} />;
    return {
      name: mark.name,
      item: roving.getItemProps(index),
      onPointerMove: (event) => tooltip.show(content, { clientX: event.clientX, clientY: event.clientY }),
      onPointerLeave: tooltip.hide,
      onFocusMark: (box) => tooltip.show(content, box),
      onBlurMark: tooltip.hide,
      onClick: onActivate ? () => onActivate(mark.key) : undefined,
    };
  };
  const empty = rowCount(figure) === 0;
  const notes = [...figure.notes, ...(figure.kind === 'bins' && new Set(figure.bins.flatMap((bin) => (bin.to === null ? [] : [bin.to - bin.from]))).size > 1
    ? ['Bins span different hours, so a column\'s height is records per hour and its area is its count.'] : []),
    ...(figure.kind === 'bins' && figure.bins.some((bin) => bin.to === null) ? ['The last bin has no end, so it is a count past the axis break, not on the height scale.'] : []),
    ...(hidden ? [hiddenText(figure, hidden)] : [])];
  const minHeight = size === 'card' ? 120 : 240;
  let draw: (width: number) => ReactNode;
  switch (figure.kind) {
    case 'ranked':
      draw = (width) => <RankedPlot figure={figure} width={width} size={size} interaction={interaction} />;
      break;
    case 'paired':
      draw = (width) => <PairedPlot figure={figure} width={width} size={size} interaction={interaction} />;
      break;
    case 'stacked':
      draw = (width) => <StackedPlot figure={figure} width={width} size={size} interaction={interaction} />;
      break;
    case 'series':
      draw = (width) => <SeriesPlot figure={figure} width={width} size={size} interaction={interaction} />;
      break;
    case 'bins':
      draw = (width) => <BinsPlot figure={figure} width={width} size={size} interaction={interaction} />;
      break;
  }
  return (
    <div className="grid min-w-0 gap-2" data-lead-figure={figure.kind}>
      <ChartFrame
        title={figure.measure}
        headline={headline}
        claim={figure.claim}
        sources={sources}
        summary={summaryOf(figure, hidden)}
        status={empty ? 'empty' : 'ready'}
        emptyText="This lead's result has nothing to draw."
        table={table}
        legend={legendOf(figure)}
        controls={figure.caption || (figure.kind === 'stacked' && !empty) ? (
          <div className="grid gap-1">
            {figure.caption && <p className="text-caption text-muted">{figure.caption}</p>}
            {figure.kind === 'stacked' && !empty && <SegmentLegend figure={figure} />}
          </div>
        ) : undefined}
        minHeight={Math.min(minHeight, plotHeightOf(figure, 640, size))}
        plotHeight={(width) => plotHeightOf(figure, width, size)}
        interactive={{ roledescription: 'lead figure' }}
        storageKey={`lead-figure:${figure.kind}:${figure.measure}`}
      >
        {({ width }) => draw(width)}
      </ChartFrame>
      {notes.length > 0 && (
        <ul className="list-none space-y-0.5 text-caption text-muted">
          {notes.map((note) => <li key={note}>{note}</li>)}
        </ul>
      )}
      {tooltip.node}
    </div>
  );
}
