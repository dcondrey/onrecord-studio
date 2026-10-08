import { useMemo, type ReactNode } from 'react';
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
import { REFERENCE_PAINT, VIZ } from '../core/claim';
import { formatValue, type ValueKind } from '../core/format';
import { fitsInside, measureText } from '../core/layout';
import { absent, isValue, measured, type Absence, type Measured } from '../core/nulls';
import { scaleBand, scaleLinear, type ContinuousScale } from '../core/scale';
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
import { AbsentSlot, LABEL_CLASS, MarkGroup, MUTED_LABEL_CLASS, type MarkInteraction } from '../marks/shared';

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
  /** Activating a mark: opens the lead's query or dossier. */
  onActivate?(): void;
}

const ROW_LABEL_CLASS = 'fill-fg-secondary text-[12px]';
const ROW_FONT = '12px -apple-system, BlinkMacSystemFont, sans-serif';
const CARD_LIMIT: Record<LeadFigureData['kind'], number> = { series: 40, stacked: 4, paired: 4, bins: 8, ranked: 5 };

const kindOf = (unit: LeadFigureUnit): ValueKind => unit;
const format = (value: number, unit: LeadFigureUnit, compact = false) => formatValue(value, { kind: kindOf(unit), compact: compact && (unit === 'currency' || unit === 'count'), digits: unit === 'percent' ? 1 : undefined });
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
    default:
      return { figure, hidden: 0 };
  }
}

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

function rowLayout(items: readonly { key: string; label: string; under?: number; above?: number; header?: string }[], width: number, band: number, top = 4) {
  const labelColumn = width < 420 ? 0 : Math.min(200, Math.round(width * 0.3));
  let y = top;
  const headers: { text: string; y: number }[] = [];
  const rows: RowSlot[] = items.map((item) => {
    if (item.header) {
      headers.push({ text: item.header, y: y + 10 });
      y += 18;
    }
    y += item.above ?? 0;
    const inline = labelColumn > 0 && fitsInside(item.label, labelColumn - 4, ROW_FONT);
    const slot = inline
      ? { key: item.key, label: item.label, labelY: y + band / 2, barY: y, under: item.under ?? 0 }
      : { key: item.key, label: item.label, labelY: y + 7, barY: y + 16, under: item.under ?? 0 };
    y = slot.barY + band + slot.under + 10;
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
  return formatValue(difference, { kind: kindOf(unit), signed: true, compact: unit === 'currency' });
}

/** The yardstick half-width about a comparison value: the larger of the share and the floor. */
const bandHalf = (comparison: number, band: NonNullable<LeadPairedFigure['band']>) => Math.max(Math.abs(comparison) * band.percent / 100, band.minimum);

function PairedPlot({ figure, width, size, interaction }: { figure: LeadPairedFigure; width: number; size: LeadFigureSize; interaction(i: number): MarkInteraction }) {
  const layout = pairedLayout(figure, width, size);
  const values = figure.rows.flatMap((row) => [valueOf(row.subject), valueOf(row.comparison)]).filter((v): v is number => v !== null);
  const bandMax = figure.band ? Math.max(0, ...figure.rows.map((row) => valueOf(row.comparison)).filter((v): v is number => v !== null).map((v) => v + bandHalf(v, figure.band!))) : 0;
  const deltas = figure.rows.map((row) => {
    const s = valueOf(row.subject); const c = valueOf(row.comparison);
    if (s === null || c === null) return '';
    if (figure.display === 'bullet') return c === 0 ? deltaText(s, c, figure.unit) : `${formatValue((s - c) / Math.abs(c), { kind: 'percent', signed: true, digits: 0 })}, ${deltaText(s, c, figure.unit)}`;
    return deltaText(s, c, figure.unit);
  });
  const reserve = Math.max(40, ...deltas.map((text) => measureText(text) + 16));
  const x = scaleLinear([0, Math.max(...values, bandMax, 0)], [layout.plotX + 6, Math.max(layout.plotX + 60, width - reserve)]);
  return (
    <g>
      <Grid scale={x} extent={[0, layout.axisY]} vertical tickCount={size === 'card' ? 3 : undefined} />
      {layout.rows.map((slot, index) => {
        const row = figure.rows[index];
        const comparison = valueOf(row.comparison);
        if (figure.display === 'bullet') {
          const half = figure.band && comparison !== null ? bandHalf(comparison, figure.band) : null;
          return (
            <g key={slot.key}>
              <RowLabel slot={slot} />
              {half !== null && comparison !== null && (
                <rect
                  data-reference="yardstick"
                  x={x(Math.max(0, comparison - half))}
                  y={slot.barY - 3}
                  width={Math.max(0, x(comparison + half) - x(Math.max(0, comparison - half)))}
                  height={layout.band + 6}
                  rx={2}
                  style={{ fill: 'none', stroke: REFERENCE_PAINT.yardstick.stroke, strokeWidth: 1, strokeDasharray: REFERENCE_PAINT.yardstick.dash }}
                />
              )}
              <Bullet
                band={{ pos: slot.barY, thickness: layout.band }}
                scale={x}
                track={toMeasured(row.comparison)}
                bar={toMeasured(row.subject)}
                ticks={[{ key: 'comparison', at: toMeasured(row.comparison), label: `${figure.comparisonLabel}: ${valueText(row.comparison, figure.unit)}` }]}
                barLabel={deltas[index] || undefined}
                labelLimit={width}
                interaction={interaction(index)}
              />
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
  const labelColumn = width < 420 ? 0 : Math.min(200, Math.round(width * 0.3));
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
  return { ...layout, band, stubs, stubsOf };
}

function StackedPlot({ figure, width, size, interaction }: { figure: LeadStackedFigure; width: number; size: LeadFigureSize; interaction(i: number): MarkInteraction }) {
  const layout = stackedLayout(figure, width, size);
  const totals = figure.rows.map((row) => rowTotal(row.values));
  const totalLabels = totals.map((total) => format(total, figure.unit, true));
  const reserve = Math.max(28, ...totalLabels.map((label) => measureText(label) + 10));
  const right = Math.max(layout.plotX + 60, width - reserve);
  const negatives = figure.rows.map((row) => row.values.reduce((sum, value) => sum + Math.min(0, value), 0));
  const x: ContinuousScale = figure.normalise
    ? scaleLinear([0, 1], [layout.plotX, right], { nice: false })
    : scaleLinear([Math.min(0, ...negatives), Math.max(0, ...totals)], [layout.plotX, right]);
  return (
    <g>
      {layout.headers.map((header) => <text key={`${header.text}-${header.y}`} x={0} y={header.y} className="fill-fg text-[12px] font-medium">{header.text}</text>)}
      {layout.rows.map((slot, index) => {
        const row = figure.rows[index];
        const total = totals[index];
        const segments = figure.segments.map((segment, s) => {
          const raw = row.values[s] ?? 0;
          const value = figure.normalise ? (total > 0 ? raw / total : 0) : raw;
          const text = figure.normalise ? formatValue(value, { kind: 'percent', digits: 0 }) : format(raw, figure.unit, true);
          return { key: segment.key, value, tone: segment.tone, label: text };
        });
        const extent = figure.normalise ? [x(0), x(total > 0 ? 1 : 0)] : [x(negatives[index]), x(total)];
        const stubTop = slot.barY + layout.band + 4;
        const stubLines = row.excluded.length ? Math.max(...layout.stubs[index].map((stub) => stub.line)) + 1 : 0;
        return (
          <g key={slot.key}>
            <RowLabel slot={slot} />
            {row.overlay && total > 0 && (
              <Bar band={{ pos: slot.barY - 5, thickness: 3 }} value={measured(figure.normalise ? row.overlay.value / total : row.overlay.value)} scale={x} role="primary" />
            )}
            {total === 0 && negatives[index] === 0 ? (
              <AbsentSlot absence={absent('not-recorded', 'no records in any segment')} box={{ x: layout.plotX, y: slot.barY, width: right - layout.plotX, height: layout.band }} text="none recorded" />
            ) : (
              <StackedBar band={{ pos: slot.barY, thickness: layout.band }} segments={segments} scale={x} interaction={interaction(index)} motionKey={`stacked:${slot.key}`} />
            )}
            {row.partial && (
              <rect x={extent[0] - 2} y={slot.barY - 2} width={Math.max(0, extent[1] - extent[0] + 4)} height={layout.band + 4} rx={3} style={{ fill: 'none', stroke: VIZ.baseline, strokeWidth: 1, strokeDasharray: '3 2' }} />
            )}
            {figure.normalise && <text x={right + 6} y={slot.barY + layout.band / 2} dy="0.32em" className={LABEL_CLASS}>{totalLabels[index]}</text>}
            {layout.stubsOf(row).map((stub, s) => (
              <ExcludedStub key={stub.key} band={{ pos: stubTop + layout.stubs[index][s].line * STUB_LINE, thickness: 10 }} at={layout.stubs[index][s].x} count={stub.count.toLocaleString('en-US')} label={stub.label} />
            ))}
            {row.note && <text x={layout.plotX} y={stubTop + stubLines * STUB_LINE + 9} className={MUTED_LABEL_CLASS}>{row.note}</text>}
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

function SeriesPlot({ figure, width, size, interaction }: { figure: LeadSeriesFigure; width: number; size: LeadFigureSize; interaction(i: number): MarkInteraction }) {
  const { panelHeight, header, gap, height } = seriesLayout(figure, size);
  const n = figure.points.length;
  const tickWidth = Math.max(...figure.panels.flatMap((panel) => {
    const max = Math.max(0, ...[...panel.values, ...(panel.context?.values ?? [])].map(valueOf).filter((v): v is number => v !== null));
    return [measureText(format(max, panel.unit, true))];
  }), 24);
  const left = Math.ceil(tickWidth) + 10;
  const endLabels = figure.panels.map((panel) => {
    const defined = panel.values.map(valueOf).filter((v): v is number => v !== null);
    return defined.length ? format(defined[defined.length - 1], panel.unit, true) : '';
  });
  const right = width - Math.max(16, ...endLabels.map((label) => measureText(label) + 14));
  const step = n > 1 ? (right - left) / (n - 1) : right - left;
  const xOf = (i: number) => (n > 1 ? left + i * step : (left + right) / 2);
  const column = Math.max(12, Math.min(40, step * 0.6));
  const axisY = height - 18;
  // Thin the x labels so none collides with its neighbour.
  const widest = Math.max(...figure.points.map((point) => measureText(point.label)));
  const every = Math.max(1, Math.ceil((widest + 8) / Math.max(1, step)));
  return (
    <g>
      {figure.panels.map((panel, p) => {
        const top = p * (header + panelHeight + gap) + header;
        const all = [...panel.values, ...(panel.context?.values ?? [])].map(valueOf).filter((v): v is number => v !== null);
        const y = scaleLinear([0, Math.max(0, ...all)], [top + panelHeight, top + 4]);
        const points = panel.values.map((value, i) => ({ key: `${panel.key}:${figure.points[i].key}`, x: i, y: toMeasured(value), incomplete: figure.points[i].partial }));
        const first = points.find((point) => isValue(point.y));
        return (
          <g key={panel.key}>
            <text x={0} y={top - 4} className="fill-fg-secondary text-[12px]">{panel.label}</text>
            <Grid scale={y} extent={[left, right]} tickCount={2} />
            <Axis orient="left" offset={left} scale={y} kind={kindOf(panel.unit)} tickCount={2} />
            {panel.values.map((value, i) => (value.state === 'value' ? null : (
              <AbsentSlot key={`gap-${i}`} absence={value} box={{ x: xOf(i) - column / 2, y: top + 2, width: column, height: panelHeight - 2 }} text="" />
            )))}
            {panel.context && (
              <LineSeries points={panel.context.values.map((value, i) => ({ key: `${panel.key}:context:${figure.points[i].key}`, x: i, y: toMeasured(value) }))} x={xOf} y={y} role="comparison" markers="observed" />
            )}
            <LineSeries points={points} x={xOf} y={y} role="primary" markers="observed" label={endLabels[p] || undefined} motionKey={`series:${panel.key}`} />
            {first && first !== [...points].reverse().find((point) => isValue(point.y)) && (
              <text x={xOf(first.x)} y={y((first.y as { value: number }).value) - 8} textAnchor="middle" className={LABEL_CLASS} style={{ paintOrder: 'stroke', stroke: VIZ.surface, strokeWidth: 3 }}>
                {format((first.y as { value: number }).value, panel.unit, true)}
              </text>
            )}
          </g>
        );
      })}
      <line x1={left} x2={right} y1={axisY + 0.5} y2={axisY + 0.5} style={{ stroke: VIZ.baseline, strokeWidth: 1 }} />
      {figure.points.map((point, i) => (
        <g key={point.key}>
          {(i % every === 0 || i === n - 1) && (i === n - 1 || n - 1 - i >= every) && (
            <text x={xOf(i)} y={axisY + 14} textAnchor="middle" className={TICK_CLASS}>{point.label}</text>
          )}
          <MarkGroup interaction={interaction(i)} box={{ x: xOf(i) - Math.max(6, Math.min(step, 48) / 2), y: 0, width: Math.max(12, Math.min(step, 48)), height: axisY }}>
            <line className="viz-crosshair" x1={xOf(i)} x2={xOf(i)} y1={0} y2={axisY} style={{ stroke: 'transparent' }} />
          </MarkGroup>
        </g>
      ))}
    </g>
  );
}

// ---- Kind: bins --------------------------------------------------------------------------------

function binLabel(bin: LeadBinsFigure['bins'][number]): string {
  return bin.to === null ? `${bin.from}+ h` : bin.from === 0 ? `<${bin.to} h` : `${bin.from}–${bin.to} h`;
}

function binsLayout(figure: LeadBinsFigure, size: LeadFigureSize) {
  const plot = size === 'card' ? 96 : 150;
  return { plot, top: 16, height: 16 + plot + 34 + (figure.before ? STUB_LINE + 4 : 0) };
}

function BinsPlot({ figure, width, size, interaction }: { figure: LeadBinsFigure; width: number; size: LeadFigureSize; interaction(i: number): MarkInteraction }) {
  const { plot, top } = binsLayout(figure, size);
  const bottom = top + plot;
  const left = 8;
  const right = width - 8;
  const x = scaleBand(figure.bins.map((bin) => bin.key), [left, right], { maxBand: size === 'card' ? 36 : 56 });
  const y = scaleLinear([0, Math.max(0, ...figure.bins.map((bin) => bin.count))], [bottom, top]);
  const boundary = (at: number) => {
    // A bin edge on the band axis: midway between the bin ending at `at` and the next.
    const index = figure.bins.findIndex((bin) => bin.from === at);
    if (index <= 0) return null;
    return (x(figure.bins[index - 1].key) + x.bandwidth + x(figure.bins[index].key)) / 2;
  };
  const referenceAt = figure.reference ? boundary(figure.reference.at) : null;
  // The median's position inside its bin, read linearly between the bin's edges.
  const median = figure.median === null ? null : (() => {
    const bin = figure.bins.find((item) => figure.median! >= item.from && (item.to === null || figure.median! < item.to));
    if (!bin) return null;
    const start = x(bin.key); const span = bin.to === null ? 0 : (figure.median! - bin.from) / (bin.to - bin.from);
    return start + x.bandwidth * Math.min(1, Math.max(0, span));
  })();
  return (
    <g>
      <Grid scale={y} extent={[left, right]} tickCount={3} />
      {referenceAt !== null && figure.reference && (
        <ReferenceLine kind="yardstick" axis="x" at={referenceAt} extent={[top - 12, bottom]} label={`${formatValue(figure.reference.at, { kind: 'hours' })} reference`} labelSide="start" />
      )}
      {figure.bins.map((bin, index) => (
        <g key={bin.key}>
          <Bar orient="vertical" band={{ pos: x(bin.key), thickness: x.bandwidth }} value={measured(bin.count)} scale={y} label={format(bin.count, 'count', true)} interaction={interaction(index)} motionKey={`bins:${bin.key}`} />
          <text x={x(bin.key) + x.bandwidth / 2} y={bottom + 14} textAnchor="middle" className={TICK_CLASS}>{binLabel(bin)}</text>
        </g>
      ))}
      <line x1={left} x2={right} y1={bottom + 0.5} y2={bottom + 0.5} style={{ stroke: VIZ.baseline, strokeWidth: 1 }} />
      {median !== null && (
        <g>
          <line x1={median} x2={median} y1={bottom + 18} y2={bottom + 24} style={{ stroke: VIZ.ink, strokeWidth: 2 }} />
          <text x={Math.min(Math.max(median, left + 40), right - 40)} y={bottom + 33} textAnchor="middle" className={LABEL_CLASS}>{`median ${formatValue(figure.median!, { kind: 'hours', digits: 1 })}`}</text>
        </g>
      )}
      {figure.before && (
        <ExcludedStub band={{ pos: bottom + 38, thickness: 10 }} at={left} count={figure.before.count.toLocaleString('en-US')} label={figure.before.label} />
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
        return { title: `${point.label}${point.partial ? ' so far' : ''}`, rows, name: `${point.label}: ${rows.map((row) => `${row.label} ${row.value}`).join('; ')}.` };
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
        return { title: `${row.group}, ${row.label}`, rows, note: row.note ?? undefined, name: `${row.group}, ${row.label}: ${rows.map((item) => `${item.value} ${item.label}`).join('; ')}.` };
      });
    case 'paired':
      return figure.rows.map((row) => {
        const rows = [
          { key: 'subject', value: `${valueText(row.subject, figure.unit)}${row.subjectBasis ? ` (${row.subjectBasis})` : ''}`, label: figure.subjectLabel },
          { key: 'comparison', value: `${valueText(row.comparison, figure.unit)}${row.comparisonBasis ? ` (${row.comparisonBasis})` : ''}`, label: figure.comparisonLabel },
        ];
        return { title: row.label, rows, note: figure.band ? figure.band.label : undefined, name: `${row.label}: ${rows.map((item) => `${item.label} ${item.value}`).join('; ')}.` };
      });
    case 'bins': {
      const total = figure.bins.reduce((sum, bin) => sum + bin.count, 0);
      return figure.bins.map((bin) => {
        const rows = [{ key: 'count', value: `${format(bin.count, 'count')}${total > 0 ? ` (${formatValue(bin.count / total, { kind: 'percent' })})` : ''}`, label: bin.label }];
        return { title: bin.label, rows, name: `${bin.label}: ${rows[0].value}.` };
      });
    }
    case 'ranked':
      return figure.rows.map((row) => {
        const rows = [{ key: 'value', value: valueText(row.value, figure.unit), label: row.detail ?? figure.measure }];
        return { title: row.label, rows, name: `${row.label}: ${rows[0].value}${row.detail ? `, ${row.detail}` : ''}.` };
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
  if (hidden) parts.push(`${hidden} more rows are in the dossier`);
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
      return binsLayout(figure, size).height;
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
  const roving = useRovingFocus({ shape: { rows: Math.max(1, marks.length) }, onActivate: () => onActivate?.(), onEscape: tooltip.hide });
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
      onClick: onActivate,
    };
  };
  const empty = rowCount(figure) === 0;
  const notes = [...figure.notes, ...(figure.kind === 'bins' ? figure.denominator.map((item) => `${item.count.toLocaleString('en-US')} ${item.label}.`) : []),
    ...(hidden ? [`${hidden} more ${hidden === 1 ? 'row is' : 'rows are'} in the lead's dossier.`] : [])];
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
