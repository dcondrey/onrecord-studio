import { useState, type ReactNode } from 'react';
import ChartFrame, { type ChartFrameProps } from './frame/ChartFrame';
import { Axis } from './frame/Axis';
import { Grid } from './frame/Grid';
import { ReferenceBand, ReferenceLine } from './frame/ReferenceLine';
import { TooltipContent } from './frame/Tooltip';
import { formatCurrencyString, formatDate, formatValue } from './core/format';
import { fitsInside, measureText, placeLabels } from './core/layout';
import { absent, measured, type Measured } from './core/nulls';
import { parseDate, scaleBand, scaleLinear, scaleTime } from './core/scale';
import { nearestIndex, useCrosshair } from './hooks/useCrosshair';
import { useLegendFocus } from './hooks/useLegendFocus';
import { useRovingFocus } from './hooks/useRovingFocus';
import { useTooltip } from './hooks/useTooltip';
import { Bar, ExcludedStub, StackedBar } from './marks/Bar';
import { Bullet } from './marks/Bullet';
import { Beeswarm, Dot } from './marks/Dot';
import { Dumbbell } from './marks/Dumbbell';
import { HeatCell, seqStep } from './marks/HeatCell';
import { Interval } from './marks/Interval';
import { AreaGradient, LineSeries, type LinePoint } from './marks/Line';
import { Meter } from './marks/Meter';
import { Segments } from './marks/Segments';
import { TimelineLane } from './marks/TimelineLane';
import { VIZ } from './core/claim';
import { BarTableExamples, SeriesExamples } from './TablesSeriesGallery';

/*
 * Every chart primitive with realistic data: gaps, long labels, negative values, single-point and
 * empty series, every frame state, and the 340px inspector width. Rendered only by the UI review
 * harness (scripts/capture-ui-review.mjs --only=chart-gallery); the app never imports it.
 */

const ROW_LABEL_CLASS = 'fill-fg-secondary text-[12px]';

/** Roving props frozen in the focused state, so a capture shows each mark family's focus ring. */
const SHOWN_FOCUSED = { tabIndex: -1 as const, onKeyDown() {}, onFocus() {}, onBlur() {}, ref() {}, 'data-focused': true as const };

interface Row {
  key: string;
  label: string;
  /** Top of the row's label line. */
  labelY: number;
  /** Top of the bar band. */
  barY: number;
  inline: boolean;
}

/**
 * Row layout for row charts: a label column beside each bar when the label fits, otherwise the
 * label on its own line above the bar. Below 480px every label sits above its bar.
 */
function rowLayout(items: readonly { key: string; label: string }[], width: number, band = 14) {
  const labelColumn = width < 480 ? 0 : Math.min(200, Math.round(width * 0.3));
  let y = 4;
  const rows: Row[] = items.map((item) => {
    const inline = labelColumn > 0 && fitsInside(item.label, labelColumn - 4, '12px -apple-system, BlinkMacSystemFont, sans-serif');
    const row = inline
      ? { key: item.key, label: item.label, labelY: y + band / 2, barY: y, inline }
      : { key: item.key, label: item.label, labelY: y + 7, barY: y + 18, inline };
    y = row.barY + band + 12;
    return row;
  });
  return { rows, height: y + 24, plotX: labelColumn, band };
}

function RowLabel({ row }: { row: Row }) {
  return (
    <text x={0} y={row.labelY} dy="0.32em" className={ROW_LABEL_CLASS} style={{ paintOrder: 'stroke', stroke: VIZ.surface, strokeWidth: 4, strokeLinejoin: 'round' }}>
      {row.label}
    </text>
  );
}

function Section({ title, note, children }: { title: string; note: string; children: ReactNode }) {
  return (
    <section data-gallery-section={title} className="app-panel grid gap-4 p-5">
      <header className="grid gap-1">
        <h2 className="text-title text-fg">{title}</h2>
        <p className="max-w-prose text-body text-muted">{note}</p>
      </header>
      {children}
    </section>
  );
}

const SOURCE_PROFILES = { tables: ['agency_performance_profiles', 'statutory_violations'], snapshot: { name: 'San Diego NextRequest', id: '7f3c2a91-5d0e-4b8f-9a61-2c4e8b0d1f77', sha256: '9b2d7f0e4c1a8b3f6e5d2c1a0f9e8d7c6b5a4f3e2d1c0b9a8f7e6d5c4b3a2f1e' }, retrievedAt: '2026-10-02' };

// ---- Bars ---------------------------------------------------------------------------------

const DEPARTMENTS: { key: string; label: string; missed: Measured; requests: number; excluded?: number }[] = [
  { key: 'esd', label: 'Environmental Services', missed: measured(22), requests: 268, excluded: 9 },
  { key: 'pd', label: 'Police', missed: measured(41), requests: 512 },
  { key: 'hcd', label: 'Homelessness Strategies and Solutions Department', missed: measured(7), requests: 96 },
  { key: 'parks', label: 'Parks and Recreation', missed: measured(0), requests: 54 },
  { key: 'dsd', label: 'Development Services', missed: absent('not-recorded', 'the portal published no due dates for this department'), requests: 31 },
];

function CountedBars({ width }: { width: number }) {
  const tooltip = useTooltip();
  const { rows, height, plotX, band } = rowLayout(DEPARTMENTS, width);
  const x = scaleLinear([0, Math.max(...DEPARTMENTS.map((d) => d.requests))], [plotX, width - 150]);
  return (
    <>
      <Grid scale={x} extent={[0, height - 24]} vertical />
      {rows.map((row, index) => {
        const d = DEPARTMENTS[index];
        const missed = d.missed;
        const label = missed.state === 'value' ? `${missed.value} of ${d.requests}` : undefined;
        const name = missed.state === 'value'
          ? `${d.label}: ${missed.value} of ${d.requests} requests missed the statutory deadline (counted).`
          : `${d.label}: no deadline outcome recorded.`;
        return (
          <g key={row.key}>
            <RowLabel row={row} />
            <rect x={x(0)} y={row.barY} width={x(d.requests) - x(0)} height={band} rx={2} style={{ fill: VIZ.track }} />
            <Bar
              band={{ pos: row.barY, thickness: band }}
              value={missed}
              scale={x}
              claim="counted"
              label={label}
              slotExtent={[x(0), x(d.requests)]}
              motionKey={`counted-${row.key}`}
              interaction={{
                name,
                item: row.key === 'pd' ? SHOWN_FOCUSED : undefined,
                onPointerMove: (event) => tooltip.show(
                  <TooltipContent title={d.label} rows={[{ key: 'm', value: label ?? 'not recorded', label: 'counted deadline misses', swatch: 'flag', tone: 'counted' }]} />,
                  event,
                ),
                onPointerLeave: tooltip.hide,
              }}
            />
            {d.excluded && <ExcludedStub band={{ pos: row.barY, thickness: band }} at={x(d.requests) + 8} count={String(d.excluded)} label="cancelled, not counted" />}
          </g>
        );
      })}
      <Axis scale={x} kind="count" orient="bottom" offset={height - 24} />
      {tooltip.node}
    </>
  );
}

const CHANGES = [
  { key: 'a', label: 'Open reports older than a year', value: 412 },
  { key: 'b', label: 'Encampment reports closed as referred', value: -268 },
  { key: 'c', label: 'Shelter beds listed', value: -37 },
  { key: 'd', label: 'Abatement notices posted', value: 0 },
];

function SignedBars({ width }: { width: number }) {
  const { rows, height, plotX, band } = rowLayout(CHANGES, width);
  const x = scaleLinear([-300, 450], [plotX + 40, width - 48]);
  return (
    <>
      <Grid scale={x} extent={[0, height - 24]} vertical />
      {rows.map((row, index) => (
        <g key={row.key}>
          <RowLabel row={row} />
          <Bar band={{ pos: row.barY, thickness: band }} value={measured(CHANGES[index].value)} scale={x} role="default" label={formatValue(CHANGES[index].value, { kind: 'count', signed: true })} />
        </g>
      ))}
      <line x1={x(0) + 0.5} x2={x(0) + 0.5} y1={0} y2={height - 24} style={{ stroke: VIZ.baseline, strokeWidth: 1 }} />
      <Axis scale={x} kind="count" orient="bottom" offset={height - 24} />
    </>
  );
}

function StatisticalBars({ width }: { width: number }) {
  const items = [
    { key: 'esd', label: 'Environmental Services', value: 0.31 },
    { key: 'pd', label: 'Police', value: 0.12 },
    { key: 'parks', label: 'Parks and Recreation', value: 0.04 },
  ];
  const { rows, height, plotX, band } = rowLayout(items, width);
  const x = scaleLinear([0, 0.4], [plotX, width - 56]);
  return (
    <>
      <Grid scale={x} extent={[0, height - 24]} vertical />
      {rows.map((row, index) => (
        <g key={row.key}>
          <RowLabel row={row} />
          <Bar band={{ pos: row.barY, thickness: band }} value={measured(items[index].value)} scale={x} claim="statistical" label={formatValue(items[index].value, { kind: 'percent' })} />
        </g>
      ))}
      <Axis scale={x} kind="percent" orient="bottom" offset={height - 24} />
    </>
  );
}

function Columns({ width }: { width: number }) {
  const years = ['FY21', 'FY22', 'FY23', 'FY24', 'FY25'];
  const values: Measured[] = [measured(1204), measured(1530), absent('not-recorded', 'FY23 actuals are not published'), measured(1877), measured(0)];
  const x = scaleBand(years, [48, width - 8]);
  const y = scaleLinear([0, 2000], [190, 16]);
  return (
    <>
      <Grid scale={y} extent={[48, width - 8]} />
      {years.map((year, index) => (
        <Bar
          key={year}
          orient="vertical"
          band={{ pos: x(year), thickness: x.bandwidth }}
          value={values[index]}
          scale={y}
          label={values[index].state === 'value' ? formatValue((values[index] as { value: number }).value, { kind: 'count' }) : undefined}
        />
      ))}
      <Axis scale={y} kind="count" orient="left" offset={44} />
      <Axis band={x} orient="bottom" offset={190} />
    </>
  );
}

// ---- Stacks -------------------------------------------------------------------------------

function Stacks({ width }: { width: number }) {
  const items = [
    { key: 'esd', label: 'Environmental Services' },
    { key: 'calls', label: 'Sleeper calls by outcome, 2025 (signed: removals below zero)' },
  ];
  const { rows, height, plotX, band } = rowLayout(items, width, 20);
  const x = scaleLinear([0, 268], [plotX, width - 150]);
  const signed = scaleLinear([-120, 420], [plotX, width - 150]);
  return (
    <>
      <RowLabel row={rows[0]} />
      <StackedBar
        band={{ pos: rows[0].barY, thickness: band }}
        scale={x}
        segments={[
          { key: 'measured', value: 174, tone: 'ord-4', label: '174' },
          { key: 'unmeasurable', value: 58, tone: 'ord-2', label: '58' },
          { key: 'open', value: 36, tone: 'ord-1', label: '36' },
        ]}
        motionKey="stack-esd"
      />
      <ExcludedStub band={{ pos: rows[0].barY, thickness: band }} at={x(268) + 8} count="9" />
      <RowLabel row={rows[1]} />
      <StackedBar
        band={{ pos: rows[1].barY, thickness: band }}
        scale={signed}
        segments={[
          { key: 'arrest', value: 96, tone: 'lead', label: '96' },
          { key: 'report', value: 211, tone: 'ord-3', label: '211' },
          { key: 'none', value: 88, tone: 'ord-1', label: '88' },
          { key: 'removed', value: -104, tone: 'default', label: '−104' },
        ]}
      />
      <line x1={signed(0) + 0.5} x2={signed(0) + 0.5} y1={rows[1].barY - 4} y2={rows[1].barY + band + 4} style={{ stroke: VIZ.baseline }} />
      <Axis scale={signed} kind="count" orient="bottom" offset={height - 24} />
    </>
  );
}

// ---- Lines --------------------------------------------------------------------------------

const FY_STARTS = [2018, 2019, 2020, 2021, 2022, 2023, 2024, 2025].map((year) => Date.UTC(year, 6, 1));
const REVENUE: (number | null)[] = [4.1e6, 4.6e6, 5.2e6, null, 6.8e6, 7.4e6, 7.1e6, 8.2e6];
const EXPENSES: (number | null)[] = [3.9e6, 4.8e6, 5.0e6, null, null, 7.9e6, 7.6e6, 7.95e6];

function toPoints(prefix: string, values: (number | null)[], incompleteLast = false): LinePoint[] {
  return values.map((value, index) => ({
    key: `${prefix}-${index}`,
    x: FY_STARTS[index],
    y: value === null ? absent('not-covered', `index year FY${String((2019 + index) % 100).padStart(2, '0')} was not checked`) : measured(value),
    incomplete: incompleteLast && index === values.length - 1,
  }));
}

function Lines({ width, legend }: { width: number; legend: ReturnType<typeof useLegendFocus> }) {
  const tooltip = useTooltip();
  const x = scaleTime([FY_STARTS[0], FY_STARTS[FY_STARTS.length - 1]], [52, width - 84], { fiscalYearStartMonth: 7 });
  const y = scaleLinear([0, 9e6], [196, 12]);
  const xs = FY_STARTS.map((at) => x(at));
  // The two end labels converge; placeLabels separates them and the series draw leader lines.
  const ends = placeLabels([
    { key: 'revenue', anchor: y(REVENUE[REVENUE.length - 1] ?? 0), width: 14 },
    { key: 'expenses', anchor: y(EXPENSES[EXPENSES.length - 1] ?? 0), width: 14 },
  ], 12, 196, 2);
  const crosshair = useCrosshair({ xs, keys: FY_STARTS.map(String), syncGroup: undefined });
  const revenue = toPoints('rev', REVENUE, true);
  const expenses = toPoints('exp', EXPENSES);
  const index = crosshair.index;
  return (
    <g
      {...crosshair.bind}
      onPointerLeave={() => {
        crosshair.bind.onPointerLeave();
        tooltip.hide();
      }}
      onPointerMove={(event) => {
        crosshair.bind.onPointerMove(event);
        const i = nearestIndex(xs, event.clientX - event.currentTarget.getBoundingClientRect().left);
        const r = REVENUE[i];
        const e = EXPENSES[i];
        tooltip.show(
          <TooltipContent
            title={formatDate(FY_STARTS[i], 'fiscal-year')}
            rows={[
              { key: 'r', value: r === null ? 'not checked' : formatValue(r, { kind: 'currency', compact: true }), label: 'revenue', swatch: 'line' },
              { key: 'e', value: e === null ? 'not checked' : formatValue(e, { kind: 'currency', compact: true }), label: 'expenses', swatch: 'dashed-line', tone: 'context' },
            ]}
          />,
          event,
        );
      }}
    >
      <rect x={0} y={0} width={width} height={220} fill="transparent" />
      <Grid scale={y} extent={[52, width - 84]} />
      <LineSeries points={expenses} x={x} y={y} role="comparison" label="Expenses" labelY={ends[1].pos + 7} opacity={legend.opacityFor('expenses')} />
      <LineSeries points={revenue} x={x} y={y} role="primary" label="Revenue" labelY={ends[0].pos + 7} opacity={legend.opacityFor('revenue')} motionKey="revenue" />
      {index !== null && <line x1={xs[index] + 0.5} x2={xs[index] + 0.5} y1={12} y2={196} style={{ stroke: VIZ.baseline }} />}
      <Axis scale={y} kind="currency" orient="left" offset={48} />
      <Axis time={x} orient="bottom" offset={196} />
      {tooltip.node}
    </g>
  );
}

const WEEKLY: (number | null)[] = [12, 18, 15, 22, 27, null, null, 31, null, 19, 24, 26];
const WEEK_STARTS = WEEKLY.map((_, i) => parseDate('2026-06-01') + i * 7 * 86_400_000);

function WeeklyCounts({ width }: { width: number }) {
  const x = scaleTime([WEEK_STARTS[0], WEEK_STARTS[WEEK_STARTS.length - 1]], [48, width - 72]);
  const y = scaleLinear([0, 35], [120, 12]);
  const points: LinePoint[] = WEEKLY.map((value, i) => ({ key: `w${i}`, x: WEEK_STARTS[i], y: value === null ? absent('not-recorded', 'the weekly export was not published') : measured(value) }));
  return (
    <>
      <Grid scale={y} extent={[48, width - 72]} />
      <AreaGradient points={points} x={x} y={y} />
      <LineSeries points={points} x={x} y={y} label="26 reports" motionKey="weekly" />
      <Axis scale={y} kind="count" orient="left" offset={44} />
      <Axis time={x} orient="bottom" offset={120} />
    </>
  );
}

function LonePoint({ width }: { width: number }) {
  const at = parseDate('2024-06-30');
  const x = scaleTime([at, at], [48, width - 72], { fiscalYearStartMonth: 7 });
  const y = scaleLinear([1_240_000, 1_240_000], [120, 12]);
  return (
    <>
      <Grid scale={y} extent={[48, width - 72]} />
      <LineSeries points={[{ key: 'fy24', x: at, y: measured(1_240_000) }]} x={x} y={y} label="$1.24M" />
      <Axis scale={y} kind="currency" orient="left" offset={44} />
      <Axis time={x} orient="bottom" offset={120} />
    </>
  );
}

function AllAbsent({ width }: { width: number }) {
  const x = scaleTime([FY_STARTS[0], FY_STARTS[FY_STARTS.length - 1]], [48, width - 16], { fiscalYearStartMonth: 7 });
  const y = scaleLinear([0, 0], [120, 12]);
  const points: LinePoint[] = FY_STARTS.map((at, i) => ({ key: `g${i}`, x: at, y: absent('not-covered', 'the archive has no e-filed return for this year') }));
  // No value axis: with nothing observed, its ticks would read as values.
  return (
    <>
      <LineSeries points={points} x={x} y={y} />
      <Axis time={x} orient="bottom" offset={120} />
    </>
  );
}

// ---- Bullets ------------------------------------------------------------------------------

const CONTRACTS = [
  { key: 'c1', label: 'Harborview Shelter Partners: storage', track: '1240000.00', base: '900000.00', paid: '1476200.00' },
  { key: 'c2', label: 'Mesa Outreach Collective', track: '650000.00', base: '650000.00', paid: '412500.50' },
  { key: 'c3', label: 'Bridge housing operations (amendments not retrieved)', track: null, base: null, paid: '210000.00' },
];

function Bullets({ width }: { width: number }) {
  const { rows, height, plotX, band } = rowLayout(CONTRACTS, width, 16);
  // Room on the right for the overrun words at the bar's end.
  const x = scaleLinear([0, 1600000], [plotX, width - 220]);
  const thresholdX = x(1000000);
  return (
    <>
      <Grid scale={x} extent={[0, height - 24]} vertical />
      {rows.map((row, index) => {
        const c = CONTRACTS[index];
        const over = c.track !== null && Number(c.paid) > Number(c.track);
        return (
          <g key={row.key}>
            <RowLabel row={row} />
            <Bullet
              band={{ pos: row.barY, thickness: band }}
              scale={x}
              track={c.track === null ? absent('not-recorded', 'no award or amendment documents retrieved') : measured(Number(c.track))}
              base={c.base === null ? undefined : measured(Number(c.base))}
              bar={measured(Number(c.paid))}
              ticks={c.base === null ? [] : [{ key: 'base', at: measured(Number(c.base)), label: `Base authority ${formatCurrencyString(c.base)}` }]}
              barLabel={`${formatCurrencyString(c.paid, { compact: true })} paid`}
              labelLimit={width - 8}
              overrunLabel={over && c.track ? `${formatCurrencyString((Number(c.paid) - Number(c.track)).toFixed(2), { compact: true, signed: true })} over retrieved authority` : undefined}
            />
          </g>
        );
      })}
      <ReferenceLine kind="yardstick" axis="x" at={thresholdX} extent={[0, height - 24]} label="$1M lead threshold: app yardstick, not a budget rule" labelSide="start" />
      <Axis scale={x} kind="currency" orient="bottom" offset={height - 24} />
    </>
  );
}

// ---- Dots ---------------------------------------------------------------------------------

const PEERS = [
  { key: 'p1', value: 9.4, n: 61 }, { key: 'p2', value: 12.1, n: 88 }, { key: 'p3', value: 12.4, n: 3 }, { key: 'p4', value: 14.8, n: 40 },
  { key: 'p5', value: 15.0, n: 22 }, { key: 'p6', value: 15.3, n: 19 }, { key: 'p7', value: 18.9, n: 54 }, { key: 'p8', value: 21.7, n: 33 },
  { key: 'p9', value: 22.0, n: 12 }, { key: 'p10', value: 26.5, n: 47 }, { key: 'p11', value: 29.8, n: 4 },
];

function PeerDots({ width, showFocus }: { width: number; showFocus?: boolean }) {
  const tooltip = useTooltip();
  const x = scaleLinear([0, 45], [16, width - 16]);
  const mean = 18.0;
  const sd = 5.6;
  const focusables = [...PEERS.map((peer) => ({ key: peer.key, value: peer.value, name: `Peer agency: ${formatValue(peer.value, { kind: 'days' })} average closure (n=${peer.n})${peer.n < 5 ? ', small sample' : ''}.` })), { key: 'esd', value: 41.2, name: 'Environmental Services: 41.2 d average closure (n=74), slower than peers (statistical).' }];
  const roving = useRovingFocus({ shape: { rows: focusables.length }, onActivate: () => {}, onEscape: tooltip.hide });
  const interaction = (index: number) => ({
    name: focusables[index].name,
    item: showFocus && index === PEERS.length ? { ...roving.getItemProps(index), ...SHOWN_FOCUSED } : roving.getItemProps(index),
    onFocusMark: (box: DOMRect) => tooltip.show(<TooltipContent rows={[{ key: 'v', value: formatValue(focusables[index].value, { kind: 'days' }), label: 'average closure' }]} note={focusables[index].name} />, box),
    onBlurMark: tooltip.hide,
  });
  return (
    <g>
      <ReferenceBand axis="x" from={x(mean - 2 * sd)} to={x(mean + 2 * sd)} extent={[8, 92]} label="typical range (within 2 sd)" />
      <ReferenceLine kind="peer-mean" axis="x" at={x(mean)} extent={[8, 76]} label="peer mean 18.0 d" />
      <Beeswarm
        items={PEERS.map((peer, index) => ({ key: peer.key, value: peer.value, role: 'comparison', hollow: peer.n < 5, title: focusables[index].name, interaction: interaction(index) }))}
        x={x}
        cy={56}
      />
      <Dot cx={x(41.2)} cy={56} r={7} shape="ring" claim="statistical" />
      <Dot cx={x(41.2)} cy={56} r={4} role="primary" label="41.2 d (n=74)" labelSide={x(41.2) - x(30) > measureText('41.2 d (n=74)') + 16 ? 'start' : 'above'} interaction={interaction(PEERS.length)} />
      <Axis scale={x} kind="days" orient="bottom" offset={92} />
      {tooltip.node}
    </g>
  );
}

// ---- Intervals and dumbbells --------------------------------------------------------------

function Intervals({ width }: { width: number }) {
  const x = scaleTime([parseDate('2022-01-01'), parseDate('2026-10-08')], [16, width - 16]);
  const items: { key: string; label: string; start: Measured; end: Measured }[] = [
    { key: 'v1', label: 'Ordinance version 1', start: measured(parseDate('2022-06-14')), end: measured(parseDate('2023-07-29')) },
    { key: 'v2', label: 'Ordinance version 2 (start unknown)', start: absent('not-recorded', 'validFrom is null'), end: measured(parseDate('2024-11-02')) },
    { key: 'v3', label: 'Ordinance version 3 (open ended)', start: measured(parseDate('2024-11-02')), end: absent('not-recorded', 'no repeal recorded') },
  ];
  return (
    <>
      {items.map((item, index) => (
        <g key={item.key}>
          <text x={16} y={14 + index * 40} className={ROW_LABEL_CLASS}>{item.label}</text>
          <Interval y={30 + index * 40} x={x} bounds={[16, width - 16]} start={item.start} end={item.end} role="primary" interaction={item.key === 'v3' ? { name: `${item.label}: in force since Nov 2, 2024, no recorded end.`, item: SHOWN_FOCUSED } : undefined} />
        </g>
      ))}
      <Axis time={x} orient="bottom" offset={132} />
    </>
  );
}

function Dumbbells({ width }: { width: number }) {
  const items: { key: string; label: string; from: Measured; to: Measured }[] = [
    { key: 'total', label: 'All departments', from: measured(0.08), to: measured(0.082) },
    { key: 'esd', label: 'Environmental Services', from: measured(0.21), to: measured(0.093) },
    { key: 'pd', label: 'Police', from: absent('not-recorded', 'no requests in the prior window'), to: measured(0.14) },
  ];
  const { rows, height, plotX } = rowLayout(items, width, 12);
  const x = scaleLinear([0, 0.25], [plotX + 8, width - 140]);
  return (
    <>
      <Grid scale={x} extent={[0, height - 24]} vertical />
      {rows.map((row, index) => {
        const item = items[index];
        const delta = item.from.state === 'value' && item.to.state === 'value' ? formatValue(item.to.value - item.from.value, { kind: 'pts' }) : undefined;
        return (
          <g key={row.key}>
            <RowLabel row={row} />
            <Dumbbell y={row.barY + 6} scale={x} from={item.from} to={item.to} deltaLabel={delta} />
          </g>
        );
      })}
      <Axis scale={x} kind="percent" orient="bottom" offset={height - 24} />
    </>
  );
}

// ---- Cells --------------------------------------------------------------------------------

const WEEKDAYS = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'];

function Calendar({ width }: { width: number }) {
  const weeks = Math.max(8, Math.min(26, Math.floor((width - 40) / 18)));
  const cell = Math.min(18, (width - 40) / weeks);
  const value = (w: number, d: number): Measured => {
    if (w < 2) return absent('not-covered', 'before the file’s first recorded day');
    if (w === 5 && d === 2) return absent('not-recorded', 'the row has no call date');
    if (w === 7 && d > 4) return absent('excluded', 'cancelled calls are outside this count');
    return measured(((w * 7 + d) * 37) % 23 === 0 ? 0 : ((w * 13 + d * 7) % 19));
  };
  return (
    <>
      {WEEKDAYS.map((day, d) => <text key={day} x={0} y={d * cell + cell / 2 + 4} dy="0.32em" className="fill-muted text-[10px]">{day}</text>)}
      {Array.from({ length: weeks }, (_, w) => WEEKDAYS.map((_, d) => {
        const v = value(w, d);
        return <HeatCell key={`${w}-${d}`} x={36 + w * cell} y={4 + d * cell} width={cell} height={cell} value={v} step={v.state === 'value' ? seqStep(v.value, 18) : undefined} />;
      }))}
    </>
  );
}

const WATCH_ROWS = ['Contract ceiling', 'Board agenda', 'Shelter bed count'];
const WATCH_COLS = ['Amount', 'Vendor', 'Posting time', 'Document hash'];
const WATCH_STATE: { text: string; glyph: 'circle' | 'ring' | 'diamond'; value: Measured; step?: 1 | 2 | 3 | 4 | 5 }[][] = [
  [{ text: 'changed', glyph: 'diamond', value: measured(1), step: 5 }, { text: 'same', glyph: 'circle', value: measured(0), step: 1 }, { text: 'n/a', glyph: 'ring', value: absent('not-covered', 'this watch does not track posting time') }, { text: 'same', glyph: 'circle', value: measured(0), step: 1 }],
  [{ text: 'same', glyph: 'circle', value: measured(0), step: 1 }, { text: 'not observed', glyph: 'ring', value: absent('not-recorded', 'the page did not load on the last check') }, { text: 'changed', glyph: 'diamond', value: measured(1), step: 5 }, { text: 'same', glyph: 'circle', value: measured(0), step: 1 }],
  [{ text: 'changed', glyph: 'diamond', value: measured(1), step: 5 }, { text: 'same', glyph: 'circle', value: measured(0), step: 1 }, { text: 'same', glyph: 'circle', value: measured(0), step: 1 }, { text: 'not observed', glyph: 'ring', value: absent('not-recorded', 'no snapshot this week') }],
];

function StateGridDemo({ width }: { width: number }) {
  const [selected, setSelected] = useState<string | null>(null);
  const roving = useRovingFocus({ shape: { rows: WATCH_ROWS.length, cols: WATCH_COLS.length }, onActivate: (i, j) => setSelected(`${WATCH_ROWS[i]} · ${WATCH_COLS[j ?? 0]}`), onEscape: () => setSelected(null) });
  const labelColumn = width < 480 ? 110 : 150;
  const cellWidth = (width - labelColumn) / WATCH_COLS.length;
  return (
    <>
      {WATCH_COLS.map((col, j) => <text key={col} x={labelColumn + j * cellWidth + 4} y={12} className="fill-muted text-[11px]">{fitsInside(col, cellWidth - 6) ? col : `${col.slice(0, 4)}…`}<title>{col}</title></text>)}
      {WATCH_ROWS.map((row, i) => (
        <g key={row}>
          <text x={0} y={24 + i * 30 + 14} dy="0.32em" className={ROW_LABEL_CLASS}>{row}</text>
          {WATCH_COLS.map((col, j) => {
            const state = WATCH_STATE[i][j];
            return (
              <HeatCell
                key={col}
                x={labelColumn + j * cellWidth}
                y={24 + i * 30}
                width={cellWidth}
                height={28}
                value={state.value}
                step={state.step}
                glyph={state.glyph}
                text={state.text}
                interaction={{ name: `${row}, ${col}: ${state.text}.`, item: i === 1 && j === 2 ? { ...roving.getItemProps(i, j), ...SHOWN_FOCUSED } : roving.getItemProps(i, j) }}
              />
            );
          })}
        </g>
      ))}
      {selected && <text x={0} y={128} className="fill-fg-secondary text-[11px]">Selected: {selected}</text>}
    </>
  );
}

// ---- Timeline, segments, meter ------------------------------------------------------------

function Timeline({ width }: { width: number }) {
  const x = scaleTime([parseDate('2026-03-01'), parseDate('2026-10-15')], [96, width - 12]);
  const cluster = Array.from({ length: 7 }, (_, i) => ({ key: `c${i}`, at: parseDate('2026-06-02') + i * 3_600_000, name: `Document released ${i + 1} of 7 on Jun 2, 2026` }));
  return (
    <>
      <ReferenceLine kind="statute" axis="x" at={x(parseDate('2026-04-22'))} extent={[4, 112]} label="Determination due" citation="Cal. Gov. Code § 7922.535(a)" />
      <ReferenceLine kind="today" axis="x" at={x(parseDate('2026-10-08'))} extent={[4, 112]} label="Today" labelSide="start" />
      <TimelineLane
        y={44}
        x={x}
        label="Requests"
        spans={[{ key: 'clock', from: parseDate('2026-04-12'), to: parseDate('2026-04-22'), role: 'comparison', name: 'Determination period Apr 12 to Apr 22' }]}
        events={[
          { key: 'filed', at: parseDate('2026-04-12'), name: 'Request filed Apr 12, 2026' },
          { key: 'due', at: parseDate('2026-04-22'), shape: 'diamond', hollow: true, name: 'Determination due Apr 22, 2026' },
          { key: 'ext', at: parseDate('2026-04-22'), shape: 'triangle', name: 'Extension notice Apr 22, 2026' },
          ...cluster,
        ]}
      />
      <TimelineLane
        y={92}
        x={x}
        label="Findings"
        events={[
          { key: 'f1', at: parseDate('2026-05-09'), role: 'lead', name: 'Finding: storage contract ceiling exceeded (lead)' },
          { key: 'f2', at: parseDate('2026-08-21'), role: 'lead', name: 'Finding: agenda posted 41 h before start (lead)' },
          { key: 'seen', at: parseDate('2026-09-30'), shape: 'ring', name: 'Last seen by this app Sep 30, 2026' },
        ]}
      />
      <Axis time={x} orient="bottom" offset={116} />
    </>
  );
}

const FACTORS = [
  { key: 'money', max: 3, value: measured(3), label: 'Money: $236,200 above ceiling (3 of 3)' },
  { key: 'recency', max: 2, value: measured(1), label: 'Recency: 2 months old (1 of 2)' },
  { key: 'corroboration', max: 3, value: measured(0), label: 'Corroboration: none yet (0 of 3)' },
  { key: 'strength', max: 3, value: absent('not-recorded', 'evidence strength not assessed') as Measured, label: 'Evidence strength: not assessed' },
];

function ScoreSegments({ width }: { width: number }) {
  const unit = Math.min(36, (width - 240) / 11);
  return (
    <>
      <text x={0} y={16} className={ROW_LABEL_CLASS}>Storage contract ceiling</text>
      <Segments x={0} y={26} unit={unit} segments={FACTORS} />
      <text x={unit * 11 + 14} y={32} dy="0.32em" className="viz-num fill-fg-secondary text-[11px]">4 of 11 (one factor not assessed)</text>
    </>
  );
}

// ---- Page ---------------------------------------------------------------------------------

type FrameBase = Omit<ChartFrameProps, 'children'>;

const BASE: Pick<FrameBase, 'sources' | 'status'> = { sources: SOURCE_PROFILES, status: 'ready' };

function Frames({ narrow }: { narrow: boolean }) {
  const legend = useLegendFocus();
  const widthClass = narrow ? 'w-[340px]' : '';
  const deptTable = {
    caption: 'Counted deadline misses by department',
    columns: [{ key: 'department', label: 'Department' }, { key: 'missed', label: 'Counted misses', kind: 'count' as const }, { key: 'requests', label: 'Requests', kind: 'count' as const }],
    rows: [
      ...DEPARTMENTS.map((d) => ({ department: d.label, missed: d.missed.state === 'value' ? d.missed.value : d.missed, requests: d.requests })),
      { department: 'Environmental Services, cancelled', missed: absent('excluded', '9 cancelled requests are outside the deadline count'), requests: 9 },
    ],
  };
  const deptHeight = rowLayout(DEPARTMENTS, narrow ? 340 : 900).height;
  return (
    <div className={narrow ? 'flex flex-wrap gap-6' : 'grid gap-6'}>
      <div className={widthClass}>
        <ChartFrame
          {...BASE}
          title="Counted deadline misses"
          claim="counted"
          headline="Police missed the statutory deadline on 41 of 512 requests, the most of any department."
          summary="Five departments. Counts of requests whose determination came after the statutory deadline; Development Services has no recorded due dates; 9 cancelled requests are outside the count."
          table={deptTable}
          legend={[{ key: 'counted', label: 'Counted deadline miss', swatch: 'flag', tone: 'counted' }, { key: 'track', label: 'Requests processed', swatch: 'rect', tone: 'track' }]}
          minHeight={deptHeight}
          plotHeight={(width) => rowLayout(DEPARTMENTS, width).height}
          storageKey={`gallery-counted-${narrow}`}
        >
          {({ width }) => <CountedBars width={width} />}
        </ChartFrame>
      </div>
      <div className={widthClass}>
        <ChartFrame
          {...BASE}
          title="Closure time vs peers"
          claim="statistical"
          headline="Environmental Services averaged 41.2 days, slower than peers (statistical)."
          summary="Twelve agencies' average closure days on one axis; peer mean 18.0 days; typical range 6.8 to 29.2 days; two peers have fewer than five closures."
          table={{ caption: 'Average closure days by agency', columns: [{ key: 'agency', label: 'Agency' }, { key: 'days', label: 'Average closure', kind: 'days' }, { key: 'n', label: 'Closures', kind: 'count' }], rows: [{ agency: 'Environmental Services', days: 41.2, n: 74 }, ...PEERS.map((peer) => ({ agency: `Peer ${peer.key}`, days: peer.value, n: peer.n }))] }}
          legend={[{ key: 'this', label: 'This agency', swatch: 'dot', tone: 'ink' }, { key: 'peers', label: 'Peer agencies', swatch: 'hollow', tone: 'context' }, { key: 'z', label: 'Slower than peers (statistical)', swatch: 'ring', tone: 'ink' }]}
          minHeight={116}
          interactive={{ roledescription: 'dot plot' }}
          storageKey={`gallery-peers-${narrow}`}
        >
          {({ width }) => <PeerDots width={width} showFocus={!narrow} />}
        </ChartFrame>
      </div>
      {!narrow && (
        <>
          <ChartFrame
            {...BASE}
            title="Revenue and expenses by fiscal year"
            claim="descriptive"
            headline="Revenue rose from $4.1M in FY19 to $8.2M in FY26; FY22 and FY23 expenses were not checked."
            summary="Two series over eight fiscal years. Revenue has one gap (FY22); expenses have two (FY22, FY23). FY26 revenue is a partial year."
            sources={{ tables: ['irs_990_filings'], note: 'FY26 is a partial year' }}
            table={{ caption: 'Revenue and expenses by fiscal year', columns: [{ key: 'fy', label: 'Fiscal year' }, { key: 'rev', label: 'Revenue ($M)' }, { key: 'exp', label: 'Expenses ($M)' }], rows: FY_STARTS.map((at, i) => ({ fy: formatDate(at, 'fiscal-year'), rev: REVENUE[i] ?? absent('not-covered', 'index year not checked'), exp: EXPENSES[i] ?? absent('not-covered', 'index year not checked') })) }}
            legend={[{ key: 'revenue', label: 'Revenue', swatch: 'line' }, { key: 'expenses', label: 'Expenses', swatch: 'dashed-line', tone: 'context' }]}
            legendFocus={legend}
            minHeight={220}
          >
            {({ width }) => <Lines width={width} legend={legend} />}
          </ChartFrame>
          <div className="grid gap-6 lg:grid-cols-2">
            <ChartFrame {...BASE} title="Encampment reports by week" headline="Weekly reports peaked at 31; three weekly exports were not published." summary="Twelve weeks; weeks 6, 7 and 9 have no published export, leaving week 8 as a single observed point." table={{ caption: 'Encampment reports by week', columns: [{ key: 'week', label: 'Week of' }, { key: 'n', label: 'Reports', kind: 'count' }], rows: WEEKLY.map((value, i) => ({ week: formatDate(WEEK_STARTS[i], 'day'), n: value ?? absent('not-recorded', 'the weekly export was not published') })) }} minHeight={140} sources={{ tables: ['get_it_done_reports'] }}>
              {({ width }) => <WeeklyCounts width={width} />}
            </ChartFrame>
            <ChartFrame {...BASE} title="Adopted operating budget actuals" headline="FY25 actuals are 1,877; FY23 actuals are not published." summary="Five fiscal years; FY23 not recorded; FY26 a measured zero." table={{ caption: 'Actuals by fiscal year', columns: [{ key: 'fy', label: 'Fiscal year' }, { key: 'v', label: 'Actuals', kind: 'count' }], rows: [{ fy: 'FY21', v: 1204 }, { fy: 'FY22', v: 1530 }, { fy: 'FY23', v: absent('not-recorded', 'FY23 actuals are not published') }, { fy: 'FY24', v: 1877 }, { fy: 'FY25', v: 0 }] }} minHeight={210} sources={{ tables: ['budget_actuals'] }}>
              {({ width }) => <Columns width={width} />}
            </ChartFrame>
          </div>
          <div className="grid gap-6 lg:grid-cols-2">
            <ChartFrame {...BASE} title="Government grants, one filing" headline="One return reports grants: $1.24M for FY24." summary="A single observed value; both axes collapse to it." table={{ caption: 'Government grants', columns: [{ key: 'fy', label: 'Fiscal year' }, { key: 'v', label: 'Grants' }], rows: [{ fy: 'FY24', v: '$1,240,000.00' }] }} minHeight={140} sources={{ tables: ['irs_990_filings'] }}>
              {({ width }) => <LonePoint width={width} />}
            </ChartFrame>
            <ChartFrame {...BASE} title="Program service revenue" headline="No year of program service revenue was checked." summary="Eight fiscal years, none checked; nothing is plotted as zero." table={{ caption: 'Program service revenue', columns: [{ key: 'fy', label: 'Fiscal year' }, { key: 'v', label: 'Revenue' }], rows: FY_STARTS.map((at) => ({ fy: formatDate(at, 'fiscal-year'), v: absent('not-covered', 'the archive has no e-filed return for this year') })) }} minHeight={140} sources={{ tables: ['irs_990_filings'] }}>
              {({ width }) => <AllAbsent width={width} />}
            </ChartFrame>
          </div>
        </>
      )}
    </div>
  );
}

function Plain({ title, headline, height, tables, children }: { title: string; headline: string; height: number | ((width: number) => number); tables: string[]; children(width: number): ReactNode }) {
  return (
    <ChartFrame
      title={title}
      headline={headline}
      status="ready"
      sources={{ tables }}
      summary={headline}
      table={{ caption: title, columns: [{ key: 'note', label: 'Note' }], rows: [{ note: headline }] }}
      minHeight={typeof height === 'number' ? height : height(340)}
      plotHeight={typeof height === 'number' ? undefined : height}
    >
      {({ width }) => children(width)}
    </ChartFrame>
  );
}

function StateFrames() {
  const common = {
    title: 'Dispositions by department',
    sources: { tables: ['request_dispositions'] },
    summary: 'Disposition counts by department.',
    table: { caption: 'Dispositions', columns: [{ key: 'd', label: 'Department' }, { key: 'n', label: 'Requests', kind: 'count' as const }], rows: [{ d: 'Police', n: 512 }, { d: 'Parks and Recreation', n: absent('not-recorded', 'disposition not recorded') }] },
    minHeight: 96,
  };
  return (
    <div className="grid gap-6 lg:grid-cols-3">
      <ChartFrame {...common} headline={null} status="loading">{() => null}</ChartFrame>
      <ChartFrame {...common} headline={null} status="empty" emptyText="No requests have a recorded disposition in this window.">{() => null}</ChartFrame>
      <ChartFrame {...common} headline={null} status="error" error="The analytics query did not complete, so this chart has nothing to report either way.">{() => null}</ChartFrame>
      <ChartFrame {...common} headline="Police closed 512 requests." status="stale" storageKey="gallery-stale">
        {({ width }) => {
          const x = scaleLinear([0, 600], [8, width - 60]);
          return <Bar band={{ pos: 30, thickness: 14 }} value={measured(512)} scale={x} label="512" />;
        }}
      </ChartFrame>
      <ChartFrame {...common} headline={null} status="scoped-out">{() => null}</ChartFrame>
      <ChartFrame {...common} headline="Police closed 512 requests; Parks has no recorded disposition." status="ready" tableDefault="table" storageKey="gallery-table">{() => null}</ChartFrame>
    </div>
  );
}

export default function ChartGallery() {
  return (
    <main className="app-theme min-h-screen p-6">
      <div className="mx-auto grid max-w-content gap-6">
        <header className="grid gap-1">
          <h1 className="text-display text-fg">Chart system gallery</h1>
          <p className="text-body text-muted">Every primitive in src/components/charts, with gaps, long labels, negative values, single-point and empty series. Review-harness only.</p>
        </header>
        <Section title="Frames and bars" note="Counted bars are crimson with the flag and the word Counted; a statistical comparison is an ink outline or ring. The dot plot's agency dot, the Police bar, one watch cell and one interval are drawn in their keyboard-focus state; the pinned tooltip is in the cells section.">
          <Frames narrow={false} />
        </Section>
        <Section title="Inspector width (340px)" note="Labels move above their bars; tick density drops; nothing scrolls sideways.">
          <Frames narrow />
        </Section>
        <Section title="Signed, statistical and stacked bars" note="Polarity by position about zero with a signed label; statistical bars are outlines; stacked segments are separated by 2px gaps and the excluded stub sits beside the bar.">
          <div className="grid gap-6 lg:grid-cols-2">
            <Plain title="Change between snapshots" headline="Open reports older than a year rose by 412; referred closures fell by 268." height={(width) => rowLayout(CHANGES, width).height} tables={['service_backlog_changes']}>
              {(width) => <SignedBars width={width} />}
            </Plain>
            <Plain title="Rapid closures (statistical)" headline="Environmental Services closed 31% of requests within a day (statistical, not a finding)." height={(width) => rowLayout([{ key: 'a', label: 'Environmental Services' }, { key: 'b', label: 'Police' }, { key: 'c', label: 'Parks and Recreation' }], width).height} tables={['agency_performance_profiles']}>
              {(width) => <StatisticalBars width={width} />}
            </Plain>
          </div>
          <Plain title="Request accounting and call outcomes" headline="174 of 268 closures were measurable; 9 cancelled requests are outside the measure." height={(width) => rowLayout([{ key: 'a', label: 'Environmental Services' }, { key: 'b', label: 'Sleeper calls by outcome, 2025 (signed: removals below zero)' }], width, 20).height} tables={['agency_performance_profiles', 'police_calls']}>
            {(width) => <Stacks width={width} />}
          </Plain>
        </Section>
        <Section title="Bullets, intervals and dumbbells" note="Overrun is drawn by position and ink words, never crimson; an absent track is a 'not retrieved' slot. Open interval ends are chevrons with words. Rate changes are in points.">
          <Plain title="Contract authority vs payments" headline="Payments to Harborview run $236K over retrieved authority." height={(width) => rowLayout(CONTRACTS, width, 16).height} tables={['contracts', 'payments']}>
            {(width) => <Bullets width={width} />}
          </Plain>
          <div className="grid gap-6 lg:grid-cols-2">
            <Plain title="Ordinance versions" headline="Version 3 has been in force since Nov 2, 2024 with no recorded end." height={150} tables={['legal_statutes']}>
              {(width) => <Intervals width={width} />}
            </Plain>
            <Plain title="Deadline-miss rate, this window vs prior" headline="The overall rate moved +0.2 pts; Environmental Services fell 11.7 pts." height={(width) => rowLayout([{ key: 'a', label: 'All departments' }, { key: 'b', label: 'Environmental Services' }, { key: 'c', label: 'Police' }], width, 12).height} tables={['agency_performance_profiles']}>
              {(width) => <Dumbbells width={width} />}
            </Plain>
          </div>
        </Section>
        <Section title="Cells, timeline, segments and meter" note="Absence is never a ramp step: dotted = not checked, dashed = not recorded, hatched = outside this measure. The state grid is one tab stop with 2-D arrows.">
          <div className="grid gap-6 lg:grid-cols-2">
            <Plain title="Sleeper calls by day" headline="Calls cluster mid-week; the first two weeks are outside the file's coverage." height={140} tables={['police_calls']}>
              {(width) => <Calendar width={width} />}
            </Plain>
            <Plain title="Watch outcomes" headline="Three dimensions changed since the last check; two were not observed." height={140} tables={['watch_observations']}>
              {(width) => <StateGridDemo width={width} />}
            </Plain>
          </div>
          <Plain title="Request and finding timeline" headline="Seven documents were released on Jun 2; two findings were surfaced." height={140} tables={['claimed_requests', 'findings']}>
            {(width) => <Timeline width={width} />}
          </Plain>
          <div className="grid gap-6 lg:grid-cols-2">
            <Plain title="Case priority (not confidence or wrongdoing)" headline="Ordered by money and recency; evidence strength is not assessed." height={56} tables={['casework_leads']}>
              {(width) => <ScoreSegments width={width} />}
            </Plain>
            <div className="grid content-start gap-2">
              <h3 className="text-heading text-fg">Tooltip (pinned for review)</h3>
              <div className="glass w-fit max-w-72 rounded-lg px-3 py-2 text-caption text-fg">
                <TooltipContent title="Environmental Services" rows={[{ key: 'm', value: '22 of 268', label: 'counted deadline misses', swatch: 'flag', tone: 'counted' }, { key: 'd', value: '41.2 d', label: 'average closure', swatch: 'dot' }]} note="9 cancelled requests are outside the count." />
              </div>
              <h3 className="mt-4 text-heading text-fg">Reading depth</h3>
              <Meter
                label="Reading depth"
                max={1842}
                valueText="1,842 documents: 1,714 read in full, 120 read to 20,000 characters, 8 with no stored text."
                segments={[
                  { key: 'full', value: 1714, text: '1,714 read in full', step: 4 },
                  { key: 'partial', value: 120, text: '120 read to 20,000 characters', step: 2 },
                  { key: 'none', value: 8, text: '8 with no stored text', step: 1 },
                ]}
              />
            </div>
          </div>
        </Section>
        <Section title="Frame states" note="Loading, empty, error and scoped-out frames contain text only; stale keeps the previous render at half opacity; the table twin prints each absence's reason.">
          <StateFrames />
        </Section>
        <Section title="Bar-tables" note="Seven disposition columns and eight lead columns at 1024px: bars halve, then headers abbreviate, then the table twin; never a sideways scroll. Hatched absence columns, a dimmed small sample, a counted column, not assessed, an absent factor and a negative sum.">
          <BarTableExamples />
        </Section>
        <Section title="Series small multiples" note="A four-year hole is four years wide; one group's lone missing year breaks its line; hollow markers are incomplete periods; the bracket's two periods carry axis labels. Labels follow the period mode.">
          <SeriesExamples />
        </Section>
      </div>
    </main>
  );
}
