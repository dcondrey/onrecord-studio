import { useMemo } from 'react';
import type { DailyCounts } from '../../../utils/serviceAccountability';
import { formatDate, formatValue } from '../core/format';
import { absent, measured } from '../core/nulls';
import { parseDate } from '../core/scale';
import ChartFrame from '../frame/ChartFrame';
import type { ChartSources } from '../frame/SourceCaption';
import type { TwinTable } from '../frame/TableTwin';
import { TooltipContent } from '../frame/Tooltip';
import { TICK_CLASS } from '../frame/Axis';
import { useRovingFocus } from '../hooks/useRovingFocus';
import { useTooltip } from '../hooks/useTooltip';
import { HeatCell, type SeqStep } from '../marks/HeatCell';
import type { MarkInteraction } from '../marks/shared';

/*
 * A year-by-week calendar of one family's daily counts on the sequential slate ramp. A covered day
 * with no record is a measured zero, the lightest filled step; a day the source did not cover
 * (before its first day, after its last, or in a year it holds nothing for) is an unfilled dotted
 * cell, never a zero.
 */

export interface CalendarHeatProps {
  title: string;
  /** What is counted, plural ('SLEEPER calls'). */
  noun: string;
  daily: DailyCounts;
  sources: ChartSources;
  size: 'card' | 'full';
}

const DAY = 86_400_000;
const WEEKDAY_LABELS = ['Mon', '', 'Wed', '', 'Fri', '', ''];
const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];

interface Day {
  iso: string;
  ms: number;
  year: number;
  /** Monday 0 to Sunday 6. */
  weekday: number;
  covered: boolean;
  count: number;
}

const isoOf = (ms: number) => new Date(ms).toISOString().slice(0, 10);
const weekdayOf = (ms: number) => (new Date(ms).getUTCDay() + 6) % 7;

function daysOf(daily: DailyCounts): Day[] {
  const first = parseDate(daily.first);
  const last = parseDate(daily.last);
  const from = Date.UTC(new Date(first).getUTCFullYear(), 0, 1);
  const to = Date.UTC(new Date(last).getUTCFullYear(), 11, 31);
  const days: Day[] = [];
  for (let ms = from; ms <= to; ms += DAY) {
    const year = new Date(ms).getUTCFullYear();
    const iso = isoOf(ms);
    const covered = ms >= first && ms <= last && daily.years.has(year);
    days.push({ iso, ms, year, weekday: weekdayOf(ms), covered, count: daily.counts.get(iso) ?? 0 });
  }
  return days;
}

/** Steps: a measured zero is step 1; positive days split into quartiles for steps 2 to 5. */
function thresholds(days: readonly Day[]): number[] {
  const positive = days.filter((day) => day.covered && day.count > 0).map((day) => day.count).sort((a, b) => a - b);
  if (!positive.length) return [];
  const at = (q: number) => positive[Math.min(positive.length - 1, Math.floor(q * positive.length))];
  return [...new Set([at(0.25), at(0.5), at(0.75)])];
}

function stepOf(count: number, bounds: readonly number[]): SeqStep {
  if (count <= 0) return 1;
  const above = bounds.filter((bound) => count > bound).length;
  return Math.min(5, 2 + above) as SeqStep;
}

function median(values: readonly number[]): number | null {
  if (!values.length) return null;
  const sorted = [...values].sort((a, b) => a - b);
  const mid = Math.floor(sorted.length / 2);
  return sorted.length % 2 ? sorted[mid] : (sorted[mid - 1] + sorted[mid]) / 2;
}

const dayLabel = (ms: number) => `${new Intl.DateTimeFormat('en-US', { weekday: 'short', timeZone: 'UTC' }).format(ms)} ${formatDate(ms, 'day')}`;

function layoutOf(years: readonly number[], width: number, size: 'card' | 'full') {
  const left = 30;
  const cell = Math.max(6, Math.min(size === 'card' ? 10 : 14, Math.floor((width - left - 4) / 54)));
  const block = 16 + 7 * cell + 12;
  return { left, cell, block, height: years.length * block };
}

export default function CalendarHeat({ title, noun, daily, sources, size }: CalendarHeatProps) {
  const days = useMemo(() => daysOf(daily), [daily]);
  const covered = useMemo(() => days.filter((day) => day.covered), [days]);
  const bounds = useMemo(() => thresholds(days), [days]);
  const years = useMemo(() => [...new Set(days.map((day) => day.year))], [days]);
  const tooltip = useTooltip();
  const interactive = size === 'full';
  const roving = useRovingFocus({ shape: { rows: Math.max(1, covered.length) }, onActivate: () => {}, onEscape: tooltip.hide });

  const total = covered.reduce((sum, day) => sum + day.count, 0);
  const busiest = covered.reduce<Day | null>((best, day) => (best === null || day.count > best.count ? day : best), null);
  const weekdays = covered.filter((day) => day.weekday < 5).reduce((sum, day) => sum + day.count, 0);
  const middle = median(covered.map((day) => day.count));
  const headline = busiest && total > 0
    ? `Busiest day: ${dayLabel(busiest.ms)}, ${formatValue(busiest.count, { kind: 'count' })}; ${formatValue(weekdays / total, { kind: 'percent' })} of ${noun} on weekdays`
    : null;

  const table: TwinTable = useMemo(() => {
    const months = new Map<string, { label: string; days: number; total: number; busiest: Day | null }>();
    for (const day of covered) {
      const key = day.iso.slice(0, 7);
      const month = months.get(key) ?? { label: `${MONTHS[Number(key.slice(5)) - 1]} ${key.slice(0, 4)}`, days: 0, total: 0, busiest: null };
      month.days += 1;
      month.total += day.count;
      if (!month.busiest || day.count > month.busiest.count) month.busiest = day;
      months.set(key, month);
    }
    return {
      caption: `${title}, by month`,
      columns: [{ key: 'month', label: 'Month' }, { key: 'days', label: 'Days covered', kind: 'count' }, { key: 'total', label: noun[0].toUpperCase() + noun.slice(1), kind: 'count' }, { key: 'busiest', label: 'Busiest day' }],
      rows: [
        ...[...months.values()].map((month) => ({ month: month.label, days: month.days, total: month.total, busiest: month.busiest ? `${formatDate(month.busiest.ms, 'day')} (${month.busiest.count})` : '' })),
        ...(daily.undated ? [{ month: 'No readable date (on no day)', days: null, total: daily.undated, busiest: '' }] : []),
      ],
    };
  }, [covered, daily.undated, noun, title]);

  const coveredIndex = new Map(covered.map((day, index) => [day.iso, index]));
  const interaction = (day: Day): MarkInteraction | undefined => {
    const index = coveredIndex.get(day.iso);
    if (!interactive || index === undefined) return undefined;
    const name = `${dayLabel(day.ms)}: ${formatValue(day.count, { kind: 'count' })} ${noun}`;
    const content = <TooltipContent title={dayLabel(day.ms)} rows={[{ key: 'n', value: formatValue(day.count, { kind: 'count' }), label: noun }]} />;
    return {
      name,
      item: roving.getItemProps(index),
      onPointerMove: (event) => tooltip.show(content, { clientX: event.clientX, clientY: event.clientY }),
      onPointerLeave: tooltip.hide,
      onFocusMark: (box) => tooltip.show(content, box),
      onBlurMark: tooltip.hide,
    };
  };

  const stepLabels = [formatValue(0, { kind: 'count' }), ...[0, ...bounds].map((bound, i, all) => {
    const upper = all[i + 1];
    return upper === undefined ? `more than ${formatValue(bound, { kind: 'count' })}` : `${formatValue(bound + 1, { kind: 'count' })} to ${formatValue(upper, { kind: 'count' })}`;
  })];

  const legend = (
    <div className="grid gap-1">
      <ul aria-label="Steps" className="flex flex-wrap items-center gap-x-3 gap-y-1 text-caption text-fg-secondary">
        {stepLabels.map((label, index) => (
          <li key={label} className="flex h-6 items-center gap-1.5">
            <svg width={12} height={12} viewBox="0 0 12 12" aria-hidden="true" className="shrink-0"><rect x={0.5} y={0.5} width={11} height={11} rx={2} style={{ fill: `var(--viz-seq-${index + 1})` }} /></svg>
            <span className="viz-num">{label}</span>
          </li>
        ))}
        {middle !== null && <li className="text-muted">{`Median day: ${formatValue(middle, { kind: 'count', digits: middle % 1 ? 1 : 0 })}`}</li>}
      </ul>
      <p className="text-caption text-muted">{`Covered ${formatDate(parseDate(daily.first), 'day')} to ${formatDate(parseDate(daily.last), 'day')}; days outside it are left unfilled, not zero.`}</p>
    </div>
  );

  return (
    <div className="grid min-w-0 gap-2" data-calendar-heat="">
      <ChartFrame
        title={title}
        headline={headline}
        claim="descriptive"
        sources={sources}
        summary={`${title}. ${formatValue(total, { kind: 'count' })} ${noun} on ${covered.length} covered days${daily.undated ? `; ${daily.undated} with no readable date` : ''}.`}
        status={covered.length ? 'ready' : 'empty'}
        emptyText="The source covers no day."
        table={table}
        controls={legend}
        legend={covered.length < days.length ? [{ key: 'not-covered', label: 'Outside the period the source covers', swatch: 'slot-dotted' }] : []}
        minHeight={size === 'card' ? 96 : 140}
        plotHeight={(width) => layoutOf(years, width, size).height}
        interactive={interactive ? { roledescription: 'calendar' } : undefined}
        storageKey={`calendar:${title}`}
      >
        {({ width }) => {
          const { left, cell, block } = layoutOf(years, width, size);
          return (
            <g>
              {years.map((year, y) => {
                const top = y * block + 16;
                const start = Date.UTC(year, 0, 1);
                const lead = weekdayOf(start);
                const column = (day: Day) => Math.floor((Math.round((day.ms - start) / DAY) + lead) / 7);
                const yearDays = days.filter((day) => day.year === year);
                return (
                  <g key={year}>
                    <text x={0} y={top - 4} className="fill-fg-secondary text-[11px] font-medium viz-num">{year}</text>
                    {MONTHS.map((month, m) => {
                      const firstOfMonth = yearDays.find((day) => day.iso.slice(5, 7) === String(m + 1).padStart(2, '0'));
                      if (!firstOfMonth || (cell < 8 && m % 2)) return null;
                      return <text key={month} x={left + column(firstOfMonth) * cell} y={top - 4} className={TICK_CLASS}>{month}</text>;
                    })}
                    {WEEKDAY_LABELS.map((label, d) => label && cell >= 8 && (
                      <text key={label} x={left - 4} y={top + d * cell + cell / 2} dy="0.32em" textAnchor="end" className={TICK_CLASS}>{label}</text>
                    ))}
                    {yearDays.map((day) => (
                      <HeatCell
                        key={day.iso}
                        x={left + column(day) * cell}
                        y={top + day.weekday * cell}
                        width={cell}
                        height={cell}
                        value={day.covered ? measured(day.count) : absent('not-covered', 'outside the period the source covers')}
                        step={stepOf(day.count, bounds)}
                        interaction={interaction(day)}
                      />
                    ))}
                  </g>
                );
              })}
            </g>
          );
        }}
      </ChartFrame>
      {daily.undated > 0 && <p className="text-caption text-muted">{`${daily.undated.toLocaleString('en-US')} ${noun} have no readable date and are on no day.`}</p>}
      {tooltip.node}
    </div>
  );
}
