import ChartFrame from '../frame/ChartFrame';
import { Axis } from '../frame/Axis';
import { Grid } from '../frame/Grid';
import type { ChartSources } from '../frame/SourceCaption';
import type { TwinTable } from '../frame/TableTwin';
import { TooltipContent } from '../frame/Tooltip';
import { VIZ } from '../core/claim';
import { formatCurrencyString } from '../core/format';
import { measureText } from '../core/layout';
import { absent } from '../core/nulls';
import { scaleLinear } from '../core/scale';
import { useRovingFocus } from '../hooks/useRovingFocus';
import { useTooltip } from '../hooks/useTooltip';
import { Dot } from '../marks/Dot';
import { LABEL_CLASS } from '../marks/shared';
import { RowLabel, rowLayout } from './BulletRows';

/** A line whose two sides were both complete: a signed difference of decimal strings. */
export interface ComparedLine {
  key: string;
  label: string;
  /** Departments or other context, for the tooltip and table. */
  context: string | null;
  from: string;
  to: string;
  /** to − from, as the source computed it. */
  difference: string;
  /** Percent of |from|, or null where from is zero. */
  percent: string | null;
}

/** A line that was not compared, with the status that says why. */
export interface UncomparedLine {
  key: string;
  label: string;
  status: string;
}

export interface DifferenceDotsProps {
  title: string;
  sources: ChartSources;
  compared: ComparedLine[];
  uncompared: UncomparedLine[];
  /** Words for the two sides: 'budget' and 'actual'. */
  fromLabel: string;
  toLabel: string;
  currency?: string;
  /** The result was cut at the query's row limit, so the counts cover the rows returned only. */
  truncated: boolean;
  onActivate?(key: string): void;
}

const PLOTTED = 20;
const BAND = 14;

const magnitude = (value: string) => Math.abs(Number(value));
const isPositive = (value: string) => !value.trim().startsWith('-') && /[1-9]/.test(value);

function signedPercent(percent: string | null): string {
  if (percent === null) return 'no % (zero base)';
  const negative = percent.trim().startsWith('-');
  const body = percent.replace(/^[-+]/, '');
  return `${negative && /[1-9]/.test(body) ? '−' : /[1-9]/.test(body) ? '+' : ''}${body}%`;
}

function countByStatus(lines: readonly UncomparedLine[]): [string, number][] {
  const counts = new Map<string, number>();
  for (const line of lines) counts.set(line.status, (counts.get(line.status) ?? 0) + 1);
  return [...counts.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]));
}

/** The headline: the largest difference above the 'from' side among compared lines, plus what was not compared. */
export function differenceHeadline(compared: readonly ComparedLine[], uncompared: readonly UncomparedLine[], fromLabel: string, currency = 'USD'): string {
  const over = compared.filter((line) => isPositive(line.difference)).sort((a, b) => Number(b.difference) - Number(a.difference))[0];
  const lead = over
    ? `Of ${compared.length.toLocaleString('en-US')} compared lines, ${over.label} exceeds its ${fromLabel} amount by the most: ${formatCurrencyString(over.difference, { currency, compact: true, signed: true })} (${signedPercent(over.percent)})`
    : `Of ${compared.length.toLocaleString('en-US')} compared lines, none exceeds its ${fromLabel} amount`;
  const missing = countByStatus(uncompared).map(([status, count]) => `${count.toLocaleString('en-US')} ${status.toLowerCase()}`);
  return `${lead}${missing.length ? `; not compared: ${missing.join(', ')}` : ''}.`;
}

function Plot({ lines, width, props }: { lines: ComparedLine[]; width: number; props: DifferenceDotsProps }) {
  const tooltip = useTooltip();
  const currency = props.currency ?? 'USD';
  const { rows, height, plotX } = rowLayout(lines, width, { band: BAND });
  const values = lines.map((line) => Number(line.difference));
  const labelRoom = Math.min(150, Math.round(width * 0.22));
  const x = scaleLinear([Math.min(0, ...values), Math.max(0, ...values)], [plotX + 8, width - labelRoom]);
  const roving = useRovingFocus({ shape: { rows: Math.max(1, lines.length) }, onActivate: (i) => props.onActivate?.(lines[i].key), onEscape: tooltip.hide });
  const zero = Math.round(x(0)) + 0.5;
  return (
    <>
      <Grid scale={x} extent={[0, height]} vertical />
      <line x1={zero} x2={zero} y1={0} y2={height} style={{ stroke: VIZ.baseline, strokeWidth: 1 }} aria-hidden="true" />
      {rows.map((row, index) => {
        const line = lines[index];
        const cx = x(Number(line.difference));
        const cy = row.markY + BAND / 2;
        const label = `${formatCurrencyString(line.difference, { currency, compact: true, signed: true })} (${signedPercent(line.percent)})`;
        const negative = Number(line.difference) < 0;
        const fromText = formatCurrencyString(line.from, { currency });
        const toText = formatCurrencyString(line.to, { currency });
        const name = `${line.label}: ${props.toLabel} ${toText} against ${props.fromLabel} ${fromText}, a difference of ${formatCurrencyString(line.difference, { currency, signed: true })} (${signedPercent(line.percent)}).`;
        const content = (
          <TooltipContent
            title={line.label}
            rows={[
              { key: 'd', value: formatCurrencyString(line.difference, { currency, signed: true }), label: `${props.toLabel} − ${props.fromLabel}` },
              { key: 'p', value: signedPercent(line.percent), label: `of ${props.fromLabel}` },
              { key: 'f', value: fromText, label: props.fromLabel },
              { key: 't', value: toText, label: props.toLabel },
            ]}
            note={line.context ?? undefined}
          />
        );
        // The value label sits on the outer side of the dot; it moves inward when the outer side has no room.
        const width_ = measureText(label);
        const outward = negative ? cx - 10 - width_ >= plotX : cx + 10 + width_ <= width;
        const end = outward ? !negative : negative;
        return (
          <g key={row.key}>
            <RowLabel row={row} title={line.context ? `${line.label} · ${line.context}` : line.label} />
            {Math.abs(cx - zero) > 6 && <line x1={zero} x2={cx + (negative ? 5 : -5)} y1={cy} y2={cy} style={{ stroke: VIZ.context, strokeWidth: 1.5 }} aria-hidden="true" />}
            <Dot
              cx={cx}
              cy={cy}
              role="primary"
              interaction={{
                name,
                item: roving.getItemProps(index),
                onClick: props.onActivate ? () => props.onActivate?.(line.key) : undefined,
                onPointerMove: (event) => tooltip.show(content, event),
                onPointerLeave: tooltip.hide,
                onFocusMark: (box) => tooltip.show(content, box),
                onBlurMark: tooltip.hide,
              }}
            />
            <text x={end ? cx + 10 : cx - 10} y={cy} dy="0.32em" textAnchor={end ? 'start' : 'end'} className={LABEL_CLASS}>{label}</text>
          </g>
        );
      })}
      <Axis scale={x} kind="currency" orient="bottom" offset={height} />
      {tooltip.node}
    </>
  );
}

function heightFor(count: number, lines: readonly ComparedLine[], width: number): number {
  return count === 0 ? 80 : rowLayout(lines, width, { band: BAND }).height + 28;
}

/**
 * F6 DifferenceDots: the compared lines with the largest |difference| (top 20) about a zero line.
 * Direction is carried by position and the sign in each label, never by a colour pair: over budget is
 * not a finding and under budget is not health. Lines that could not be compared are listed by
 * status under 'Not compared' and never placed at zero.
 */
export default function DifferenceDots(props: DifferenceDotsProps) {
  const { compared, uncompared, fromLabel, toLabel } = props;
  const currency = props.currency ?? 'USD';
  const plotted = [...compared].sort((a, b) => magnitude(b.difference) - magnitude(a.difference) || a.key.localeCompare(b.key)).slice(0, PLOTTED);
  const byStatus = countByStatus(uncompared);
  const headline = compared.length === 0 && uncompared.length === 0 ? null : differenceHeadline(compared, uncompared, fromLabel, currency);
  const table: TwinTable = {
    caption: `${toLabel} minus ${fromLabel} by line`,
    columns: [
      { key: 'line', label: 'Line' },
      { key: 'from', label: fromLabel[0].toUpperCase() + fromLabel.slice(1), align: 'end' },
      { key: 'to', label: toLabel[0].toUpperCase() + toLabel.slice(1), align: 'end' },
      { key: 'difference', label: `${toLabel} − ${fromLabel}`, align: 'end' },
      { key: 'percent', label: '%', align: 'end' },
    ],
    rows: [
      ...plotted.map((line) => ({
        line: line.label,
        from: formatCurrencyString(line.from, { currency }),
        to: formatCurrencyString(line.to, { currency }),
        difference: formatCurrencyString(line.difference, { currency, signed: true }),
        percent: signedPercent(line.percent),
      })),
      ...uncompared.map((line) => ({ line: line.label, from: null, to: null, difference: absent('excluded', line.status), percent: null })),
    ],
  };
  const summary = `${compared.length} lines compared, ${plotted.length} plotted by largest difference; ${uncompared.length} not compared${byStatus.length ? ` (${byStatus.map(([status, count]) => `${status}: ${count}`).join('; ')})` : ''}.${props.truncated ? ' The query result was cut at its row limit, so these counts cover the returned rows only.' : ''}`;
  return (
    <div className="grid gap-2">
      <ChartFrame
        title={props.title}
        headline={headline}
        sources={props.sources}
        summary={summary}
        status={compared.length === 0 ? 'empty' : 'ready'}
        emptyText="No line has complete amounts on both sides, so nothing is compared."
        table={table}
        legend={[{ key: 'dot', label: `${toLabel} − ${fromLabel}; sign in the label`, swatch: 'dot', tone: 'ink' }]}
        minHeight={heightFor(plotted.length, plotted, 640)}
        plotHeight={(width) => heightFor(plotted.length, plotted, width)}
        interactive={{ roledescription: 'dot plot' }}
        storageKey={`difference-dots:${props.title}`}
      >
        {({ width }) => <Plot lines={plotted} width={width} props={props} />}
      </ChartFrame>
      {(compared.length > plotted.length || byStatus.length > 0 || props.truncated) && (
        <div className="grid gap-1 text-caption text-muted">
          {compared.length > plotted.length && <p>{`Plotted: the ${plotted.length} largest differences of ${compared.length.toLocaleString('en-US')} compared lines. Every line is in the table below.`}</p>}
          {byStatus.length > 0 && (
            <p aria-label="Not compared">
              <span className="font-medium text-fg-secondary">Not compared: </span>
              {byStatus.map(([status, count]) => `${status} ${count.toLocaleString('en-US')}`).join(' · ')}
              . These lines are not placed at zero.
            </p>
          )}
          {props.truncated && <p className="text-warning">The query result was cut at its row limit; counts cover the returned rows only.</p>}
        </div>
      )}
    </div>
  );
}
