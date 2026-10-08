import { useState, type KeyboardEvent, type PointerEvent, type ReactNode } from 'react';
import { VIZ, type MarkRole } from '../core/claim';
import { formatValue, type ValueKind } from '../core/format';
import { measureText, placeLabels } from '../core/layout';
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
  /**
   * The slot's place on a period scale (periods since the first). When every slot has one, x
   * follows it, so a gap is as wide as the periods it spans.
   */
  position?: number;
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

/** Each slot's x: by `position` when every slot has one, else evenly spaced. */
export function slotPositions(slots: readonly SeriesSlot[], x0: number, x1: number): number[] {
  if (slots.length === 1) return [(x0 + x1) / 2];
  const positioned = slots.every((slot) => slot.position !== undefined);
  const first = positioned ? slots[0].position ?? 0 : 0;
  const span = positioned ? (slots[slots.length - 1].position ?? 0) - first : slots.length - 1;
  return slots.map((slot, i) => x0 + (((positioned ? (slot.position ?? 0) - first : i) / (span || 1)) * (x1 - x0)));
}

/**
 * The slots that get a tick label: the `priority` pair (the bracket's from and to; turned away
 * from each other when centred labels would touch), both ends where they do not collide,
 * then as many others as fit. A gap slot (empty label) is never labelled.
 */
export function labelledSlots(slots: readonly SeriesSlot[], xs: readonly number[], priority: readonly number[]): { index: number; anchor: 'start' | 'middle' | 'end' }[] {
  const last = slots.length - 1;
  const anchors = new Map<number, 'start' | 'middle' | 'end'>();
  const anchorOf = (i: number) => anchors.get(i) ?? (slots.length === 1 ? 'middle' : i === 0 ? 'start' : i === last ? 'end' : 'middle');
  const extent = (i: number): [number, number] => {
    const w = measureText(slots[i].label);
    const anchor = anchorOf(i);
    return anchor === 'start' ? [xs[i], xs[i] + w] : anchor === 'end' ? [xs[i] - w, xs[i]] : [xs[i] - w / 2, xs[i] + w / 2];
  };
  const apart = (i: number, j: number) => { const [a0, a1] = extent(i); const [b0, b1] = extent(j); return a1 + 8 <= b0 || b1 + 8 <= a0; };
  const chosen: number[] = [];
  const valid = (i: number) => i >= 0 && i <= last && Boolean(slots[i].label) && !chosen.includes(i);
  const pair = priority.filter(valid);
  if (pair.length === 2 && !apart(pair[0], pair[1])) {
    // Each label turns away from its partner; an end slot keeps its inward anchor so it never
    // runs past the panel.
    const [a, b] = [Math.min(...pair), Math.max(...pair)];
    if (a !== 0) anchors.set(a, 'end');
    if (b !== last) anchors.set(b, 'start');
    // Still touching beside an end slot: the end label stays, and the bracket's own label names
    // the other period.
    if (!apart(a, b)) pair.splice(pair.indexOf(b === last ? a : b), 1);
  }
  chosen.push(...new Set(pair));
  for (const i of [0, last, ...slots.map((_, index) => index)]) {
    if (valid(i) && chosen.every((j) => apart(i, j))) chosen.push(i);
  }
  return chosen.sort((a, b) => a - b).map((index) => ({ index, anchor: anchorOf(index) }));
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
          const xs = slotPositions(slots, LEFT, panelWidth - right);
          const x = (i: number) => xs[i];
          const tickSlots = labelledSlots(slots, xs, bracket ? [bracket.from, bracket.to] : []);
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
            // A keyboard step anchors the tooltip to the crosshair: the slot's column in this panel.
            const showAtSlot = (target: SVGGElement, slot: number) => {
              const svg = target.ownerSVGElement?.getBoundingClientRect();
              const ox = (svg?.left ?? 0) + left + x(slot);
              const oy = (svg?.top ?? 0) + top + TITLE_HEIGHT;
              showTooltip(slot, { x: ox, y: oy, left: ox, top: oy, right: ox + 1, bottom: oy + PANEL_HEIGHT, width: 1, height: PANEL_HEIGHT, toJSON: () => null } as DOMRect);
            };
            const lastObserved = () => {
              for (let i = slots.length - 1; i >= 0; i--) if (panel.lines.some((line) => { const value = line.values[i]; return value !== undefined && isValue(value); })) return i;
              return slots.length - 1;
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
                showAtSlot(event.currentTarget, clamped);
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
                onFocus={(event) => {
                  if (slots.length === 0) return;
                  const slot = activeSlot ?? lastObserved();
                  setActive(slot);
                  showAtSlot(event.currentTarget, slot);
                  announce(panel, slot);
                }}
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
                {tickSlots.map(({ index: i, anchor }) => (
                  <text key={i} aria-hidden="true" data-tick={slots[i].key} x={x(i)} y={TITLE_HEIGHT + PANEL_HEIGHT + 14} textAnchor={anchor} className={TICK_CLASS}>{slots[i].label}</text>
                ))}
                {activeSlot !== null && (
                  <g aria-hidden="true">
                    <line x1={x(activeSlot)} x2={x(activeSlot)} y1={TITLE_HEIGHT + 2} y2={TITLE_HEIGHT + PANEL_HEIGHT} style={{ stroke: VIZ.baseline, strokeWidth: 1 }} />
                    {!tickSlots.some((tick) => tick.index === activeSlot) && measureText(slots[activeSlot].label) < panelWidth / 3 && (
                      <text x={x(activeSlot)} y={TITLE_HEIGHT + PANEL_HEIGHT + 14} textAnchor="middle" className={TICK_CLASS}>{slots[activeSlot].label}</text>
                    )}
                  </g>
                )}
                {(() => {
                  // End labels of two lines that finish close together are pushed apart, never stacked.
                  const lastOf = (line: SeriesLine) => {
                    for (let i = line.values.length - 1; i >= 0; i--) { const v = line.values[i]; if (isValue(v)) return y(v.value); }
                    return null;
                  };
                  const anchored = panel.lines.flatMap((line) => { const at = lastOf(line); return at === null ? [] : [{ key: line.key, anchor: at, width: 12 }]; });
                  const placed = new Map(placeLabels(anchored, TITLE_HEIGHT, TITLE_HEIGHT + PANEL_HEIGHT, 2).map((label) => [label.key, label.pos + 6]));
                  return panel.lines.map((line) => {
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
                      labelY={panel.lines.length > 1 ? placed.get(line.key) : undefined}
                      opacity={legendFocus.opacityFor(line.key)}
                      motionKey={`${panel.key}:${line.key}`}
                    />
                  );
                  });
                })()}
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
