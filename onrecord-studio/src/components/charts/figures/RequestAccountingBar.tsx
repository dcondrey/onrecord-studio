import type { AgencyPerformanceProfile } from '../../../types';
import ChartFrame from '../frame/ChartFrame';
import { TooltipContent } from '../frame/Tooltip';
import { measureText } from '../core/layout';
import { measured } from '../core/nulls';
import { scaleLinear } from '../core/scale';
import { useTooltip } from '../hooks/useTooltip';
import { Bar, StackedBar, type StackSegment } from '../marks/Bar';
import { MUTED_LABEL_CLASS } from '../marks/shared';

/*
 * F1c RequestAccountingBar: where one agency's requests stand, as one stacked bar of neutral steps
 * (measured closures | closed but not measurable | still open), and beneath it, on the same scale,
 * the counted statutory violations as a crimson bar. Documents released are a different unit and
 * are printed as text, not put on this axis.
 */

const ROW_LABEL_CLASS = 'fill-fg-secondary text-[12px]';
const HEIGHT = 116;

function count(n: number): string {
  return n.toLocaleString('en-US');
}

export function accountingHeadline(profile: AgencyPerformanceProfile): string {
  return `${count(profile.totalClosed)} of ${count(profile.totalRequestsProcessed)} requests closed, ${count(profile.validLatencySampleSize)} with a measurable closure. ${count(profile.statutoryViolationCount)} counted statutory violation${profile.statutoryViolationCount === 1 ? '' : 's'}.`;
}

function segmentsOf(profile: AgencyPerformanceProfile): (StackSegment & { definition: string })[] {
  const measuredClosures = profile.validLatencySampleSize;
  const unmeasurable = Math.max(0, profile.totalClosed - profile.validLatencySampleSize);
  const open = Math.max(0, profile.totalRequestsProcessed - profile.totalClosed);
  return [
    { key: 'measured', value: measuredClosures, tone: 'ord-4', label: count(measuredClosures), definition: 'closed with a valid created/closed pair' },
    { key: 'unmeasurable', value: unmeasurable, tone: 'ord-2', label: count(unmeasurable), definition: `closed, not measurable (includes ${count(profile.excludedInvalidLifetimeCount)} with an invalid lifetime)` },
    { key: 'open', value: open, tone: 'ord-1', label: count(open), definition: 'still open' },
  ];
}

const SHORT: Record<string, string> = { measured: 'measured', unmeasurable: 'not measurable', open: 'open' };

/** Direct labels for the stack: a swatch of each segment's own step with its count and name. */
function SegmentKey({ segments, y, width }: { segments: readonly StackSegment[]; y: number; width: number }) {
  let cursor = 0;
  return (
    <g aria-hidden="true">
      {segments.map((segment) => {
        const text = `${segment.label} ${SHORT[segment.key] ?? segment.key}`;
        const left = cursor;
        cursor += 14 + measureText(text) + 14;
        if (cursor - 14 > width) return null;
        return (
          <g key={segment.key}>
            <rect x={left} y={y - 5} width={10} height={10} rx={2} style={{ fill: `var(--viz-${segment.tone})` }} />
            <text x={left + 14} y={y} dy="0.32em" className="viz-num fill-fg-secondary text-[11px]">{text}</text>
          </g>
        );
      })}
    </g>
  );
}

function AccountingPlot({ profile, width, onSearchRecords }: { profile: AgencyPerformanceProfile; width: number; onSearchRecords(): void }) {
  const tooltip = useTooltip();
  const total = profile.totalRequestsProcessed;
  const x = scaleLinear([0, Math.max(1, total)], [0, width - 8], { nice: false });
  const segments = segmentsOf(profile);
  const countedLabel = `${count(profile.statutoryViolationCount)} of ${count(total)}`;
  const countedEnd = x(profile.statutoryViolationCount);
  const countedRoom = width - 8 - countedEnd >= measureText(countedLabel) + 8;
  const segmentTip = (
    <TooltipContent
      title="Requests processed"
      rows={segments.map((segment) => ({ key: segment.key, value: segment.label, label: segment.definition, swatch: 'rect' as const, tone: 'mark' as const }))}
    />
  );
  return (
    <g>
      <text x={0} y={10} dy="0.32em" className={ROW_LABEL_CLASS}>{`Requests processed: ${count(total)}`}</text>
      <StackedBar
        band={{ pos: 22, thickness: 14 }}
        segments={segments}
        scale={x}
        motionKey="accounting"
        interaction={{
          name: `Requests processed: ${segments.map((segment) => `${segment.label} ${segment.definition}`).join('; ')}.`,
          onPointerMove: (event) => tooltip.show(segmentTip, { clientX: event.clientX, clientY: event.clientY }),
          onPointerLeave: tooltip.hide,
        }}
      />
      <SegmentKey segments={segments} y={48} width={width - 8} />
      <text x={0} y={68} dy="0.32em" className={ROW_LABEL_CLASS}>Counted statutory violations</text>
      <rect x={0} y={80} width={x(total)} height={10} rx={2} style={{ fill: 'var(--viz-track)' }} />
      <Bar
        band={{ pos: 80, thickness: 10 }}
        value={measured(profile.statutoryViolationCount)}
        scale={x}
        claim="counted"
        label={countedRoom ? countedLabel : undefined}
        motionKey="counted"
        interaction={{
          name: `Counted statutory violations: ${countedLabel} requests. Opens a search of this agency's records.`,
          onClick: onSearchRecords,
        }}
      />
      {!countedRoom && <text x={0} y={102} dy="0.32em" className="viz-num fill-fg-secondary text-[11px]">{countedLabel}</text>}
      <text x={0} y={countedRoom ? 106 : 118} dy="0.32em" className={MUTED_LABEL_CLASS}>{`Documents released: ${count(profile.totalDocumentsReleased)} (a different unit, not on this scale)`}</text>
      {tooltip.node}
    </g>
  );
}

export default function RequestAccountingBar({ profile, onSearchRecords }: { profile: AgencyPerformanceProfile; onSearchRecords(): void }) {
  const segments = segmentsOf(profile);
  const headline = accountingHeadline(profile);
  return (
    <ChartFrame
      title="Request accounting"
      headline={headline}
      claim="counted"
      sources={{ tables: ['requests', 'statutory_extensions', 'statutory_violations'], note: 'agency profile aggregate' }}
      summary={`${headline} ${segments.map((segment) => `${segment.label} ${segment.definition}`).join('; ')}. Documents released: ${count(profile.totalDocumentsReleased)}.`}
      status={profile.totalRequestsProcessed === 0 ? 'empty' : 'ready'}
      emptyText="No request from this agency is stored."
      table={{
        caption: 'Request accounting for this agency',
        columns: [{ key: 'measure', label: 'Measure' }, { key: 'value', label: 'Count', kind: 'count' }],
        rows: [
          { measure: 'Requests processed', value: profile.totalRequestsProcessed },
          { measure: 'Closed', value: profile.totalClosed },
          ...segments.map((segment) => ({ measure: segment.definition.charAt(0).toUpperCase() + segment.definition.slice(1), value: segment.value })),
          { measure: 'Counted statutory violations', value: profile.statutoryViolationCount },
          { measure: 'Documents released', value: profile.totalDocumentsReleased },
        ],
      }}
      legend={[
        { key: 'counted', label: 'Counted violation', swatch: 'flag', tone: 'counted' },
      ]}
      minHeight={HEIGHT}
      storageKey="inspector-agency-accounting"
    >
      {({ width }) => <AccountingPlot profile={profile} width={width} onSearchRecords={onSearchRecords} />}
    </ChartFrame>
  );
}
