import type { ReactNode } from 'react';
import { VIZ, type Claim } from '../core/claim';
import { formatValue } from '../core/format';
import { fitsInside, measureText } from '../core/layout';
import { ABSENCE_LABEL, isValue, type Absence, type Measured } from '../core/nulls';
import { scaleLinear, type ContinuousScale } from '../core/scale';
import { Axis } from '../frame/Axis';
import ChartFrame, { type ChartStatus } from '../frame/ChartFrame';
import { Grid } from '../frame/Grid';
import type { LegendItem } from '../frame/Legend';
import type { ChartSources } from '../frame/SourceCaption';
import { TooltipContent } from '../frame/Tooltip';
import type { TwinTable } from '../frame/TableTwin';
import { useRovingFocus } from '../hooks/useRovingFocus';
import { useTooltip } from '../hooks/useTooltip';
import { Dot } from '../marks/Dot';
import { Dumbbell } from '../marks/Dumbbell';
import { LABEL_CLASS, MarkGroup, MUTED_LABEL_CLASS } from '../marks/shared';

/** 'rate' is a 0–1 share drawn on a fixed 0–100% axis, and its change is in percentage points. */
export type DumbbellKind = 'count' | 'rate' | 'days';

export interface DumbbellRow {
  key: string;
  label: string;
  prior: Measured;
  current: Measured;
  /** The all-departments row: drawn first and bold. */
  total?: boolean;
  /** What each value rests on ('412 requests'), for the tooltip and table. */
  priorBasis?: string;
  currentBasis?: string;
  onActivate?(): void;
}

export interface DumbbellsProps {
  title: string;
  headline: string | null;
  claim?: Claim;
  sources: ChartSources;
  status: Exclude<ChartStatus, 'scoped-out'>;
  error?: string | null;
  emptyText?: string;
  kind: DumbbellKind;
  rows: readonly DumbbellRow[];
  /** Rows below the minimum, listed by name under the plot and never drawn. */
  tooFew?: { labels: readonly string[]; reason: string };
  priorLabel: string;
  currentLabel: string;
  controls?: ReactNode;
  storageKey?: string;
}

const ROW_LABEL_FONT = '12px -apple-system, BlinkMacSystemFont, "SF Pro Text", "Segoe UI", Roboto, sans-serif';
const STEP = 28;
const NARROW_STEP = 42;
const AXIS = 26;

function valueText(value: number, kind: DumbbellKind): string {
  return kind === 'rate' ? formatValue(value, { kind: 'percent' }) : formatValue(value, { kind: kind === 'days' ? 'days' : 'count' });
}

/** A change in the measure's own unit: percentage points for a rate, never a relative %. */
export function changeText(prior: number, current: number, kind: DumbbellKind): string {
  if (kind === 'rate') return formatValue(current - prior, { kind: 'pts' });
  return formatValue(current - prior, { kind: kind === 'days' ? 'days' : 'count', signed: true });
}

function change(row: DumbbellRow): number | null {
  return isValue(row.prior) && isValue(row.current) ? row.current.value - row.prior.value : null;
}

/** The total first, then departments by the size of their change; a row missing a side goes last. */
export function dumbbellOrder(rows: readonly DumbbellRow[]): DumbbellRow[] {
  return [...rows].sort((a, b) => {
    if (a.total !== b.total) return a.total ? -1 : 1;
    const da = change(a);
    const db = change(b);
    if (da === null || db === null) return da === null && db === null ? a.label.localeCompare(b.label) : da === null ? 1 : -1;
    return Math.abs(db) - Math.abs(da) || a.label.localeCompare(b.label);
  });
}

function rowLayout(width: number) {
  const labelColumn = width < 480 ? 0 : Math.min(220, Math.round(width * 0.3));
  return { labelColumn, step: labelColumn === 0 ? NARROW_STEP : STEP };
}

/**
 * F7, a paired comparison per row: hollow for the prior window, solid for the current one, joined
 * by a context connector, with the change printed in the measure's unit. A side with no value
 * draws no dot and prints its reason ('no requests in window'); rows under the minimum are named in
 * a footer and never drawn.
 */
export default function Dumbbells(props: DumbbellsProps) {
  const ordered = dumbbellOrder(props.rows);
  const tooFew = props.tooFew && props.tooFew.labels.length > 0 ? props.tooFew : null;
  const legend: LegendItem[] = [
    { key: 'prior', label: props.priorLabel, swatch: 'hollow', tone: 'context' },
    { key: 'current', label: props.currentLabel, swatch: 'dot', tone: 'ink' },
  ];
  const table: TwinTable = {
    caption: props.title,
    columns: [
      { key: 'row', label: 'Department' },
      { key: 'prior', label: props.priorLabel, align: 'end' },
      { key: 'current', label: props.currentLabel, align: 'end' },
      { key: 'change', label: props.kind === 'rate' ? 'Change (pts)' : 'Change', align: 'end' },
    ],
    rows: [
      ...ordered.map((row) => ({
        row: row.label,
        prior: isValue(row.prior) ? `${valueText(row.prior.value, props.kind)}${row.priorBasis ? ` (${row.priorBasis})` : ''}` : `Not measured: ${(row.prior as Absence).reason}`,
        current: isValue(row.current) ? `${valueText(row.current.value, props.kind)}${row.currentBasis ? ` (${row.currentBasis})` : ''}` : `Not measured: ${(row.current as Absence).reason}`,
        change: isValue(row.prior) && isValue(row.current) ? changeText(row.prior.value, row.current.value, props.kind) : 'Not compared',
      })),
      ...(tooFew ? tooFew.labels.map((label) => ({ row: label, prior: 'Not compared', current: 'Not compared', change: tooFew.reason })) : []),
    ],
  };
  const missing = ordered.filter((row) => !isValue(row.prior) || !isValue(row.current)).length;
  const summary = `${ordered.length} rows compared${missing ? `, ${missing} missing a window` : ''}${tooFew ? `; ${tooFew.labels.length} too few to compare` : ''}.`;
  return (
    <ChartFrame
      title={props.title}
      storageKey={props.storageKey}
      headline={props.headline}
      claim={props.claim}
      sources={props.sources}
      summary={summary}
      status={props.status}
      error={props.error}
      emptyText={props.emptyText}
      table={table}
      legend={legend}
      controls={props.controls}
      minHeight={140}
      plotHeight={(width) => ordered.length * rowLayout(width).step + AXIS + (tooFew ? 22 : 0) + 6}
      interactive={{ roledescription: 'paired comparison' }}
    >
      {({ width }) => <DumbbellPlot width={width} rows={ordered} kind={props.kind} tooFew={tooFew} priorLabel={props.priorLabel} currentLabel={props.currentLabel} />}
    </ChartFrame>
  );
}

function DumbbellPlot({ width, rows, kind, tooFew, priorLabel, currentLabel }: {
  width: number;
  rows: readonly DumbbellRow[];
  kind: DumbbellKind;
  tooFew: { labels: readonly string[]; reason: string } | null;
  priorLabel: string;
  currentLabel: string;
}) {
  const tooltip = useTooltip();
  const roving = useRovingFocus({ shape: { rows: rows.length }, onActivate: (i) => rows[i].onActivate?.(), onEscape: tooltip.hide });
  const { labelColumn, step } = rowLayout(width);
  const deltaRoom = 84;
  const values = rows.flatMap((row) => [row.prior, row.current]).filter(isValue).map((value) => value.value);
  const x: ContinuousScale = kind === 'rate'
    ? scaleLinear([0, 1], [labelColumn + 8, width - deltaRoom])
    : scaleLinear([0, Math.max(1, ...values)], [labelColumn + 8, width - deltaRoom]);
  const plotBottom = rows.length * step;

  const describe = (row: DumbbellRow) => {
    const side = (value: Measured, basis?: string) => (isValue(value) ? `${valueText(value.value, kind)}${basis ? ` (${basis})` : ''}` : value.reason);
    const delta = isValue(row.prior) && isValue(row.current) ? `, change ${changeText(row.prior.value, row.current.value, kind)}` : '';
    return `${row.label}: ${priorLabel} ${side(row.prior, row.priorBasis)}; ${currentLabel} ${side(row.current, row.currentBasis)}${delta}.`;
  };
  const tip = (row: DumbbellRow) => (
    <TooltipContent
      title={row.label}
      rows={[
        { key: 'prior', value: isValue(row.prior) ? valueText(row.prior.value, kind) : 'not measured', label: `${priorLabel}${row.priorBasis ? `, ${row.priorBasis}` : ''}`, swatch: 'hollow', tone: 'context' },
        { key: 'current', value: isValue(row.current) ? valueText(row.current.value, kind) : 'not measured', label: `${currentLabel}${row.currentBasis ? `, ${row.currentBasis}` : ''}`, swatch: 'dot', tone: 'ink' },
      ]}
      note={!isValue(row.prior) ? row.prior.reason : !isValue(row.current) ? row.current.reason : undefined}
    />
  );

  return (
    <g>
      <Grid scale={x} extent={[0, plotBottom]} vertical />
      {rows.map((row, index) => {
        const top = index * step;
        const inline = labelColumn > 0;
        const y = inline ? top + step / 2 : top + 28;
        const labelY = inline ? y : top + 10;
        const labelWidth = inline ? labelColumn - 8 : width;
        const label = fitsInside(row.label, labelWidth + 8, ROW_LABEL_FONT) ? row.label : row.label.slice(0, Math.max(4, Math.floor(labelWidth / 7))) + '…';
        const both = isValue(row.prior) && isValue(row.current);
        const absentSide = !isValue(row.prior) ? row.prior : !isValue(row.current) ? row.current : null;
        const presentX = isValue(row.current) ? x(row.current.value) : isValue(row.prior) ? x(row.prior.value) : null;
        return (
          <MarkGroup
            key={row.key}
            box={{ x: 0, y: top + 2, width, height: step - 4 }}
            interaction={{
              name: describe(row),
              item: roving.getItemProps(index),
              onClick: row.onActivate,
              onPointerMove: (event) => tooltip.show(tip(row), event),
              onPointerLeave: tooltip.hide,
              onFocusMark: (box) => tooltip.show(tip(row), box),
              onBlurMark: tooltip.hide,
            }}
          >
            <text x={0} y={labelY} dy="0.32em" className={row.total ? 'fill-fg text-[12px] font-semibold' : 'fill-fg-secondary text-[12px]'}>
              <title>{row.label}</title>
              {label}
            </text>
            {both ? (
              <Dumbbell y={y} scale={x} from={row.prior} to={row.current} deltaLabel={changeText((row.prior as { value: number }).value, (row.current as { value: number }).value, kind)} />
            ) : (
              <>
                {isValue(row.prior) && <Dot cx={x(row.prior.value)} cy={y} role="comparison" hollow />}
                {isValue(row.current) && <Dot cx={x(row.current.value)} cy={y} role="primary" />}
                {absentSide && (
                  <text x={(presentX ?? x.range[0]) + (presentX === null ? 0 : 10)} y={y} dy="0.32em" className={MUTED_LABEL_CLASS}>
                    <title>{`${ABSENCE_LABEL[absentSide.state]}: ${absentSide.reason}`}</title>
                    {absentSide.reason}
                  </text>
                )}
              </>
            )}
          </MarkGroup>
        );
      })}
      <Axis scale={x} kind={kind === 'rate' ? 'percent' : kind === 'days' ? 'days' : 'count'} orient="bottom" offset={plotBottom + 2} />
      {tooFew && (
        <text x={0} y={plotBottom + AXIS + 14} className={MUTED_LABEL_CLASS}>
          <title>{`${tooFew.reason}: ${tooFew.labels.join(', ')}`}</title>
          <tspan className={LABEL_CLASS}>{`Too few to compare (${tooFew.labels.length}): `}</tspan>
          {truncateList(tooFew.labels, width - measureText(`Too few to compare (${tooFew.labels.length}): `) - 4)}
        </text>
      )}
      <line x1={x.range[0] + 0.5} x2={x.range[0] + 0.5} y1={0} y2={plotBottom} style={{ stroke: VIZ.baseline, strokeWidth: 1 }} />
      {tooltip.node}
    </g>
  );
}

function truncateList(labels: readonly string[], width: number): string {
  let text = '';
  for (let i = 0; i < labels.length; i++) {
    const next = text ? `${text}, ${labels[i]}` : labels[i];
    const rest = labels.length - i - 1;
    if (measureText(`${next}${rest ? `, +${rest} more` : ''}`) > width) return text ? `${text}, +${labels.length - i} more` : `${labels.length} departments`;
    text = next;
  }
  return text;
}
