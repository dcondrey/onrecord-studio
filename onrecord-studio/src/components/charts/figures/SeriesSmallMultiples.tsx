import { useState, type KeyboardEvent, type PointerEvent, type ReactNode } from 'react';
import { VIZ, type MarkRole } from '../core/claim';
import { formatValue, type ValueKind } from '../core/format';
import { measureText } from '../core/layout';
import { isValue, type Measured } from '../core/nulls';
import { scaleLinear, type ContinuousScale } from '../core/scale';
import { TICK_CLASS, defaultTickCount } from '../frame/Axis';
import ChartFrame, { type ChartStatus } from '../frame/ChartFrame';
import type { LegendItem } from '../frame/Legend';
import type { ChartSources } from '../frame/SourceCaption';
import type { TwinTable } from '../frame/TableTwin';
import { TooltipContent, type TooltipRow } from '../frame/Tooltip';
import { useSyncedKey } from '../hooks/ChartSync';
import { nearestIndex } from '../hooks/useCrosshair';
import { useLegendFocus } from '../hooks/useLegendFocus';
import { useTooltip } from '../hooks/useTooltip';
import { LineSeries, type LinePoint } from '../marks/Line';
import { LABEL_CLASS, MUTED_LABEL_CLASS } from '../marks/shared';

/** One position on the shared x axis: an observed period, or a gap no group has rows for. */
export interface SeriesSlot {
  key: string;
  /** Short tick label ('2024', 'FY24'); a gap slot's label is shown only in tooltips. */
  label: string;
  /** Spelled out in the tooltip ('no rows between 2020 and 2022'). */
  title?: string;
}

export interface SeriesLine {
  key: string;
  label: string;
  /** primary: ink solid; comparison: context dashed. At most two lines per panel. */
  role: Extract<MarkRole, 'primary' | 'comparison' | 'default'>;
  /** One value per slot, in slot order. */
  values: readonly Measured[];
  /** Slots whose measurement is incomplete (missing values in a period): hollow markers. */
  incomplete?: ReadonlySet<number>;
}

export interface SeriesPanel {
  key: string;
  title: string;
  /** Beside the title: the last value, in tabular numerals. */
  value?: string;
  lines: readonly SeriesLine[];
  /** Accessible summary: periods, gaps, the largest change. */
  summary: string;
}

/** The single largest adjacent change: one violet bracket, in one panel only. */
export interface ChangeBracket {
  panelKey: string;
  lineKey: string;
  from: number;
  to: number;
  label: string;
}

export interface SeriesSmallMultiplesProps {
  title: string;
  headline: string | null;
  sources: ChartSources;
  summary: string;
  status: ChartStatus;
  error?: string | null;
  emptyText?: string;
  slots: readonly SeriesSlot[];
  panels: readonly SeriesPanel[];
  kind: ValueKind;
  /** 'shared': one y scale across panels; 'per-panel': each panel its own. */
  yScale: 'shared' | 'per-panel';
  /** Zero-based (counts and sums). A mean sets false, and the caption must say so. */
  zero?: boolean;
  bracket?: ChangeBracket | null;
  /** Tooltip rows for one panel at one slot. */
  describe(panelKey: string, slot: number): { rows: TooltipRow[]; note?: string };
  onPanelActivate?(panelKey: string): void;
  activateLabel?: string;
  table: TwinTable;
  legend?: LegendItem[];
  footer?: ReactNode;
  /** Shares the crosshair with other aligned frames inside a ChartSync. */
  syncGroup?: string;
  storageKey?: string;
}

const PANEL_HEIGHT = 132;
const TITLE_HEIGHT = 22;
const AXIS_HEIGHT = 18;
const GAP_X = 24;
const GAP_Y = 20;
const LEFT = 44;
const RIGHT = 16;

function columnsFor(width: number, panels: number): number {
  const fit = width >= 960 ? 4 : width >= 640 ? 3 : width >= 400 ? 2 : 1;
  return Math.max(1, Math.min(fit, panels));
}

function extentOf(values: Iterable<Measured>): [number, number] | null {
  let lo = Infinity;
  let hi = -Infinity;
  for (const value of values) {
    if (!isValue(value)) continue;
    lo = Math.min(lo, value.value);
    hi = Math.max(hi, value.value);
  }
  return lo <= hi ? [lo, hi] : null;
}

/**
 * Small multiples on one shared x axis: one panel per group, up to two lines per panel. A line
 * breaks at every absent slot (LineSeries draws the gap's 'no data' tick); an incomplete period is
 * a hollow marker; the one largest adjacent change wears the violet bracket. A crosshair marks the
 * same slot in every panel; Left and Right step it from the keyboard, Enter opens the panel's rows.
 */
export default function SeriesSmallMultiples(props: SeriesSmallMultiplesProps) {
  const { slots, panels, kind, bracket } = props;
  const tooltip = useTooltip();
  const legendFocus = useLegendFocus();
  const [localSlot, setLocalSlot] = useState<number | null>(null);
  const [syncedKey, setSyncedKey] = useSyncedKey(props.syncGroup);
  const active = setSyncedKey ? (syncedKey === null ? null : slots.findIndex((slot) => slot.key === syncedKey)) : localSlot;
  const activeSlot = active === -1 ? null : active;
  const setActive = (index: number | null) => {
    const clamped = index === null || slots.length === 0 ? null : Math.max(0, Math.min(slots.length - 1, index));
    if (setSyncedKey) setSyncedKey(clamped === null ? null : slots[clamped].key);
    else setLocalSlot(clamped);
  };
  const [readout, setReadout] = useState('');

  const zero = props.zero ?? true;
  const sharedExtent = extentOf(panels.flatMap((panel) => panel.lines.flatMap((line) => line.values)));
  const rows = (width: number) => Math.ceil(panels.length / columnsFor(width, panels.length));
  const plotHeight = (width: number) => rows(width) * (TITLE_HEIGHT + PANEL_HEIGHT + AXIS_HEIGHT) + (rows(width) - 1) * GAP_Y;
  const format = (v: number) => formatValue(v, { kind, compact: true });

  const announce = (panel: SeriesPanel, slot: number) => {
    const described = props.describe(panel.key, slot);
    setReadout(`${panel.title}, ${slots[slot].title ?? slots[slot].label}: ${described.rows.map((row) => `${row.value} ${row.label}`).join(', ')}${described.note ? `. ${described.note}` : ''}`);
  };

  return (
    <>
      <ChartFrame
        title={props.title}
        headline={props.headline}
        sources={props.sources}
        summary={props.summary}
        status={props.status}
        error={props.error}
        emptyText={props.emptyText}
        table={props.table}
        legend={props.legend}
        legendFocus={props.legend && props.legend.length > 1 ? legendFocus : undefined}
        minHeight={TITLE_HEIGHT + PANEL_HEIGHT + AXIS_HEIGHT}
        plotHeight={plotHeight}
        interactive={{ roledescription: 'small multiples' }}
        storageKey={props.storageKey}
      >
        {({ width }) => {
          const cols = columnsFor(width, panels.length);
          const panelWidth = (width - (cols - 1) * GAP_X) / cols;
          // Direct end labels need room past the last point; a single line has none.
          const right = Math.max(RIGHT, ...panels.filter((panel) => panel.lines.length > 1).flatMap((panel) => panel.lines.map((line) => measureText(line.label) + 16)));
          const xs = slots.map((_, i) => (slots.length === 1 ? LEFT + (panelWidth - LEFT - right) / 2 : LEFT + (i * (panelWidth - LEFT - right)) / (slots.length - 1)));
          const x = (i: number) => xs[i];
          const yOf = (extent: [number, number] | null): ContinuousScale => scaleLinear(extent ?? [0, 1], [TITLE_HEIGHT + PANEL_HEIGHT, TITLE_HEIGHT + 6], { zero });
          const shared = yOf(sharedExtent);
          return panels.map((panel, index) => {
            const left = (index % cols) * (panelWidth + GAP_X);
            const top = Math.floor(index / cols) * (TITLE_HEIGHT + PANEL_HEIGHT + AXIS_HEIGHT + GAP_Y);
            const y = props.yScale === 'shared' ? shared : yOf(extentOf(panel.lines.flatMap((line) => line.values)));
            const ticks = y.ticks(Math.min(3, defaultTickCount(PANEL_HEIGHT)));
            const step = ticks.length > 1 ? ticks[1] - ticks[0] : 1;
            const showTooltip = (slot: number, at: { clientX: number; clientY: number } | DOMRect) => {
              const described = props.describe(panel.key, slot);
              tooltip.show(<TooltipContent title={`${panel.title} · ${slots[slot].title ?? slots[slot].label}`} rows={described.rows} note={described.note} />, at);
            };
            const onPointerMove = (event: PointerEvent<SVGGElement>) => {
              const box = event.currentTarget.getBoundingClientRect();
              const slot = nearestIndex(xs, event.clientX - box.left);
              if (slot < 0) return;
              setActive(slot);
              showTooltip(slot, { clientX: event.clientX, clientY: event.clientY });
            };
            const onKeyDown = (event: KeyboardEvent<SVGGElement>) => {
              if (event.key === 'ArrowRight' || event.key === 'ArrowLeft') {
                event.preventDefault();
                const next = activeSlot === null ? (event.key === 'ArrowRight' ? 0 : slots.length - 1) : activeSlot + (event.key === 'ArrowRight' ? 1 : -1);
                const clamped = Math.max(0, Math.min(slots.length - 1, next));
                setActive(clamped);
                announce(panel, clamped);
              } else if (event.key === 'Enter' && props.onPanelActivate) {
                event.preventDefault();
                props.onPanelActivate(panel.key);
              } else if (event.key === 'Escape') {
                setActive(null);
                tooltip.hide();
              }
            };
            const bracketHere = bracket && bracket.panelKey === panel.key ? bracket : null;
            const bracketLine = bracketHere ? panel.lines.find((line) => line.key === bracketHere.lineKey) : undefined;
            const fromValue = bracketLine?.values[bracketHere?.from ?? -1];
            const toValue = bracketLine?.values[bracketHere?.to ?? -1];
            return (
              <g
                key={panel.key}
                transform={`translate(${left},${top})`}
                tabIndex={0}
                role="img"
                aria-label={panel.summary}
                className="viz-focusable outline-none"
                onKeyDown={onKeyDown}
                onBlur={() => tooltip.hide()}
              >
                <rect className="viz-focus-ring" x={-4} y={-2} width={panelWidth + 8} height={TITLE_HEIGHT + PANEL_HEIGHT + AXIS_HEIGHT + 4} rx={6} style={{ fill: 'none', stroke: VIZ.focus, strokeWidth: 2 }} />
                <text x={0} y={14} className="fill-fg text-[12px] font-medium">
                  {panel.title.length > 40 ? `${panel.title.slice(0, 39)}…` : panel.title}
                  {panel.title.length > 40 && <title>{panel.title}</title>}
                </text>
                {panel.value && <text x={panelWidth} y={14} textAnchor="end" className={LABEL_CLASS}>{panel.value}</text>}
                {ticks.map((tick) => (
                  <g key={tick} aria-hidden="true">
                    <line x1={LEFT} x2={xs[xs.length - 1] ?? panelWidth - right} y1={Math.round(y(tick)) + 0.5} y2={Math.round(y(tick)) + 0.5} style={{ stroke: tick === 0 ? VIZ.baseline : VIZ.grid, strokeWidth: 1 }} />
                    <text x={LEFT - 6} y={y(tick)} dy="0.32em" textAnchor="end" className={TICK_CLASS}>{formatValue(tick, { kind, compact: true, digits: step < 1 ? 1 : undefined })}</text>
                  </g>
                ))}
                {[0, slots.length - 1].filter((i, at, all) => i >= 0 && all.indexOf(i) === at).map((i) => (
                  <text key={i} aria-hidden="true" x={x(i)} y={TITLE_HEIGHT + PANEL_HEIGHT + 14} textAnchor={slots.length === 1 ? 'middle' : i === 0 ? 'start' : 'end'} className={TICK_CLASS}>{slots[i].label}</text>
                ))}
                {activeSlot !== null && (
                  <g aria-hidden="true">
                    <line x1={x(activeSlot)} x2={x(activeSlot)} y1={TITLE_HEIGHT + 2} y2={TITLE_HEIGHT + PANEL_HEIGHT} style={{ stroke: VIZ.baseline, strokeWidth: 1 }} />
                    {activeSlot !== 0 && activeSlot !== slots.length - 1 && measureText(slots[activeSlot].label) < panelWidth / 3 && (
                      <text x={x(activeSlot)} y={TITLE_HEIGHT + PANEL_HEIGHT + 14} textAnchor="middle" className={TICK_CLASS}>{slots[activeSlot].label}</text>
                    )}
                  </g>
                )}
                {panel.lines.map((line) => {
                  const points: LinePoint[] = line.values.map((value, i) => ({ key: slots[i].key, x: i, y: value, incomplete: line.incomplete?.has(i) }));
                  return (
                    <LineSeries
                      key={line.key}
                      points={points}
                      x={x}
                      y={y}
                      role={line.role}
                      markers="observed"
                      label={panel.lines.length > 1 ? line.label : undefined}
                      opacity={legendFocus.opacityFor(line.key)}
                      motionKey={`${panel.key}:${line.key}`}
                    />
                  );
                })}
                {bracketHere && fromValue && toValue && isValue(fromValue) && isValue(toValue) && (() => {
                  const x0 = x(bracketHere.from);
                  const x1 = x(bracketHere.to);
                  const y0 = y(fromValue.value);
                  const y1 = y(toValue.value);
                  const bx = Math.min(x1 + 8, panelWidth - 4);
                  const labelLeft = bx + 4 + measureText(bracketHere.label) > panelWidth;
                  return (
                    <g data-bracket="">
                      <path d={`M${x0},${y0}H${bx}M${bx - 3},${y0}H${bx}V${y1}H${bx - 3}`} style={{ fill: 'none', stroke: VIZ.lead, strokeWidth: 1.5 }} />
                      <text
                        x={labelLeft ? x0 - 6 : bx + 4}
                        y={Math.max(TITLE_HEIGHT + 10, Math.min(y0, y1) - 6)}
                        textAnchor={labelLeft ? 'end' : 'start'}
                        className="viz-num fill-fg text-[11px] font-medium"
                        style={{ paintOrder: 'stroke', stroke: VIZ.surface, strokeWidth: 3 }}
                      >
                        {bracketHere.label}
                      </text>
                    </g>
                  );
                })()}
                <rect
                  x={0}
                  y={TITLE_HEIGHT}
                  width={panelWidth}
                  height={PANEL_HEIGHT}
                  style={{ fill: 'transparent', cursor: props.onPanelActivate ? 'pointer' : undefined }}
                  onPointerMove={onPointerMove}
                  onPointerLeave={() => {
                    setActive(null);
                    tooltip.hide();
                  }}
                  onClick={() => props.onPanelActivate?.(panel.key)}
                />
                {sharedExtent === null && <text x={panelWidth / 2} y={TITLE_HEIGHT + PANEL_HEIGHT / 2} textAnchor="middle" className={MUTED_LABEL_CLASS}>no values</text>}
                <title>{panel.summary}</title>
                <desc>{`Values: ${panel.lines.map((line) => `${line.label} ${line.values.map((value, i) => `${slots[i].label} ${isValue(value) ? format(value.value) : 'no data'}`).join(', ')}`).join('; ')}`}</desc>
              </g>
            );
          });
        }}
      </ChartFrame>
      <p aria-live="polite" className="sr-only">{readout}</p>
      {props.footer}
      {tooltip.node}
    </>
  );
}
