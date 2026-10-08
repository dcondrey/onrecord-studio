import { useState } from 'react';
import type { GlyphShape } from '../../ui/shapeGlyph';
import Button from '../../ui/Button';
import { deadlineChipSpec } from '../../tracker/DeadlineChip';
import type { ClaimedRequest, RequestDeadline } from '../../../types';
import ChartFrame from '../frame/ChartFrame';
import { Axis } from '../frame/Axis';
import { ReferenceBand } from '../frame/ReferenceLine';
import { TooltipContent } from '../frame/Tooltip';
import { VIZ, type Claim } from '../core/claim';
import { measureText } from '../core/layout';
import { scaleLinear } from '../core/scale';
import { useRovingFocus } from '../hooks/useRovingFocus';
import { useTooltip } from '../hooks/useTooltip';
import { Dot } from '../marks/Dot';
import { LABEL_CLASS, MarkGroup } from '../marks/shared';

/*
 * F2 DeadlineRunway: every tracked request on one axis of days relative to its own deadline, so
 * the requests closest to (or past) the line read first. Negative is time remaining, positive is
 * past. A claim without a running clock is never placed: it is listed under the plot.
 */

const ROW = 24;
const VISIBLE_ROWS = 12;
const TOP = 22;
const AXIS_GAP = 8;

export interface RunwayRow {
  claimId: string;
  reference: string;
  title: string | null;
  /** Days relative to the deadline: negative remaining, positive past. */
  position: number;
  shape: GlyphShape;
  dashed: boolean;
  counted: boolean;
  sentence: string;
}

export interface RunwayModel {
  rows: RunwayRow[];
  offClock: { claimId: string; reference: string; reason: string }[];
  headline: string;
  summary: string;
}

const OFF_CLOCK_WORDS: Record<string, string> = {
  NO_SUBMISSION_DATE: 'no submission date',
  NO_DEADLINE_MODEL: 'no model',
  UNAVAILABLE: 'dates conflict',
};

function plural(n: number, word: string): string {
  return `${n} ${word}${n === 1 ? '' : 's'}`;
}

/** Places each claim from the engine's countdown; nothing here recomputes a deadline. */
export function runwayModel(claims: readonly ClaimedRequest[], deadlines: Readonly<Record<string, RequestDeadline | undefined>>): RunwayModel {
  const rows: RunwayRow[] = [];
  const offClock: RunwayModel['offClock'] = [];
  for (const claim of claims) {
    const deadline = deadlines[claim.id];
    if (!deadline) continue;
    const { kind, days } = deadline.countdown;
    const spec = deadlineChipSpec(deadline);
    let position: number | null = null;
    if (kind === 'DAYS_REMAINING' && days !== null) position = -days;
    else if (kind === 'DEADLINE_PASSED' && days !== null) position = days;
    else if (kind === 'DETERMINATION_RECORDED' && deadline.analysis) position = deadline.analysis.calendarDaysElapsed - deadline.analysis.allowedCalendarDays;
    if (position === null) {
      offClock.push({ claimId: claim.id, reference: claim.reference, reason: OFF_CLOCK_WORDS[kind] ?? 'not evaluated' });
      continue;
    }
    rows.push({
      claimId: claim.id,
      reference: claim.reference,
      title: claim.title,
      position,
      shape: spec.shape,
      dashed: spec.dashed,
      counted: deadline.violationAsserted,
      sentence: `${claim.title || `Request ${claim.reference}`}: ${spec.label}.`,
    });
  }
  rows.sort((a, b) => b.position - a.position || a.reference.localeCompare(b.reference));
  const running = rows.filter((row) => !row.counted && row.shape === 'ring');
  const pastProvisional = running.filter((row) => row.position > 0).length;
  const dueSoon = running.filter((row) => row.position <= 0 && row.position >= -3).length;
  const counted = rows.filter((row) => row.counted).length;
  const clauses = [
    pastProvisional > 0 ? `${pastProvisional} past the deadline with no determination recorded (provisional)` : null,
    dueSoon > 0 ? `${dueSoon} due within 3 days` : null,
    counted > 0 ? `${plural(counted, 'violation')} counted` : null,
  ].filter(Boolean);
  const total = rows.length + offClock.length;
  const headline = `${rows.length} of ${plural(total, 'tracked request')} ${rows.length === 1 ? 'has' : 'have'} a running clock${clauses.length > 0 ? `: ${clauses.join(', ')}` : ''}.${offClock.length > 0 ? ` ${offClock.length} not on the clock.` : ''}`;
  return { rows, offClock, headline, summary: `${headline} Positions are days relative to each request's own deadline.` };
}

function RunwayPlot({ rows, width, highlighted, onHover, onOpen }: {
  rows: readonly RunwayRow[];
  width: number;
  highlighted: string | null;
  onHover(claimId: string | null): void;
  onOpen(claimId: string): void;
}) {
  const tooltip = useTooltip();
  const roving = useRovingFocus({ shape: { rows: Math.max(1, rows.length) }, onActivate: (i) => rows[i] && onOpen(rows[i].claimId), onEscape: tooltip.hide });
  const labelWidth = Math.min(120, Math.max(...rows.map((row) => measureText(row.reference, '11px ui-monospace, SFMono-Regular, Menlo, monospace')), 40) + 12);
  const extent = Math.max(5, ...rows.map((row) => Math.abs(row.position)));
  const x = scaleLinear([-extent, extent], [labelWidth + 12, width - 16], { zero: true });
  const bottom = TOP + rows.length * ROW;
  const zero = Math.round(x(0)) + 0.5;
  return (
    <g>
      <ReferenceBand axis="x" from={x(x.domain[0])} to={x(0)} extent={[TOP - 6, bottom]} label="" />
      <text x={x(0) - 6} y={10} textAnchor="end" className="fill-muted text-[11px]">time remaining</text>
      <text x={x(0) + 6} y={10} className="fill-muted text-[11px]">past the deadline</text>
      <line x1={zero} x2={zero} y1={TOP - 6} y2={bottom} data-reference="deadline" style={{ stroke: VIZ.ink, strokeWidth: 2 }} />
      {rows.map((row, index) => {
        const cy = TOP + index * ROW + ROW / 2;
        const cx = x(row.position);
        const name = row.sentence;
        const tip = <TooltipContent title={row.title || `Request ${row.reference}`} rows={[{ key: 'p', value: row.position === 0 ? 'due today' : `${Math.abs(row.position)} d`, label: row.position < 0 ? 'remaining' : row.position > 0 ? 'past the deadline' : '' }]} note={name} />;
        return (
          <MarkGroup
            key={row.claimId}
            box={{ x: 0, y: cy - ROW / 2 + 2, width: width - 8, height: ROW - 4 }}
            interaction={{
              name,
              item: roving.getItemProps(index),
              onPointerMove: (event) => { tooltip.show(tip, { clientX: event.clientX, clientY: event.clientY }); onHover(row.claimId); },
              onPointerLeave: () => { tooltip.hide(); onHover(null); },
              onFocusMark: (box) => { tooltip.show(tip, box); onHover(row.claimId); },
              onBlurMark: () => { tooltip.hide(); onHover(null); },
              onClick: () => onOpen(row.claimId),
            }}
          >
            {highlighted === row.claimId && <rect x={0} y={cy - ROW / 2 + 2} width={width - 8} height={ROW - 4} rx={4} style={{ fill: VIZ.track, opacity: 0.7 }} />}
            <text x={4} y={cy} dy="0.32em" className="fill-fg-secondary font-mono text-[11px]">{row.reference}</text>
            <line x1={labelWidth + 12} x2={width - 16} y1={cy + 0.5} y2={cy + 0.5} style={{ stroke: VIZ.grid, strokeWidth: 1 }} />
            {row.dashed ? (
              <circle cx={cx} cy={cy} r={4.5} style={{ fill: VIZ.surface, stroke: VIZ.ink, strokeWidth: 1.5, strokeDasharray: '2 1.6' }} />
            ) : (
              <Dot cx={cx} cy={cy} r={5} shape={row.shape} claim={row.counted ? 'counted' : 'descriptive'} role="primary" />
            )}
            {row.counted && <text x={cx + 9} y={cy} dy="0.32em" className={LABEL_CLASS} style={{ fill: VIZ.counted }}>violation counted</text>}
          </MarkGroup>
        );
      })}
      <Axis scale={x} kind="days" orient="bottom" offset={bottom + AXIS_GAP} />
      {tooltip.node}
    </g>
  );
}

export default function DeadlineRunway({ model, highlighted, onHover, onOpen }: {
  model: RunwayModel;
  highlighted: string | null;
  onHover(claimId: string | null): void;
  onOpen(claimId: string): void;
}) {
  const [showAll, setShowAll] = useState(false);
  const visible = showAll ? model.rows : model.rows.slice(0, VISIBLE_ROWS);
  const height = TOP + Math.max(1, visible.length) * ROW + AXIS_GAP + 26;
  const claim: Claim = model.rows.some((row) => row.counted) ? 'counted' : 'descriptive';
  return (
    <div className="grid min-w-0 gap-2">
      <ChartFrame
        title="Deadline runway"
        headline={model.headline}
        claim={claim}
        sources={{ tables: ['tracker.json (local)'], note: 'CPRA deadline model' }}
        summary={model.summary}
        status={model.rows.length === 0 ? 'empty' : 'ready'}
        emptyText="No tracked request has a running clock."
        table={{
          caption: 'Tracked requests by days relative to the deadline',
          columns: [{ key: 'reference', label: 'Request' }, { key: 'state', label: 'Reading' }, { key: 'position', label: 'Days relative to deadline', align: 'end' }],
          rows: [
            ...model.rows.map((row) => ({ reference: row.reference, state: row.sentence, position: row.position })),
            ...model.offClock.map((row) => ({ reference: row.reference, state: `Not on the clock: ${row.reason}`, position: { state: 'not-recorded' as const, reason: row.reason } })),
          ],
        }}
        legend={[
          { key: 'provisional', label: 'Provisional (no determination)', swatch: 'ring', tone: 'ink' },
          ...(model.rows.some((row) => row.shape === 'diamond' && !row.counted) ? [{ key: 'sourced', label: 'Sourced determination', swatch: 'dot' as const, tone: 'ink' as const }] : []),
          ...(model.rows.some((row) => row.counted) ? [{ key: 'counted', label: 'Violation counted', swatch: 'flag' as const, tone: 'counted' as const }] : []),
        ]}
        minHeight={height}
        interactive={{ roledescription: 'deadline runway' }}
        storageKey="my-requests-runway"
      >
        {({ width }) => <RunwayPlot rows={visible} width={width} highlighted={highlighted} onHover={onHover} onOpen={onOpen} />}
      </ChartFrame>
      {model.rows.length > VISIBLE_ROWS && (
        <Button size="sm" variant="ghost" className="justify-self-start" onClick={() => setShowAll((value) => !value)}>
          {showAll ? `Show the first ${VISIBLE_ROWS}` : `Show all ${model.rows.length}`}
        </Button>
      )}
      {model.offClock.length > 0 && (
        <p className="text-caption text-muted">
          Not on the clock: {model.offClock.length} ({[...new Set(model.offClock.map((row) => row.reason))].join(' · ')}): {model.offClock.map((row) => row.reference).join(', ')}.
        </p>
      )}
    </div>
  );
}
