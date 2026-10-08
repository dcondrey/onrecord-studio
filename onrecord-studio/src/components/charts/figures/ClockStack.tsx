import type { CaseworkCase, CaseworkCommitment, CaseworkRequest } from '../../../types/casework';
import ChartFrame from '../frame/ChartFrame';
import { Axis } from '../frame/Axis';
import { ReferenceLine } from '../frame/ReferenceLine';
import { TooltipContent } from '../frame/Tooltip';
import { REFERENCE_PAINT, VIZ } from '../core/claim';
import { formatDate } from '../core/format';
import { measureText } from '../core/layout';
import { parseDate, scaleLinear, scaleTime } from '../core/scale';
import { useRovingFocus } from '../hooks/useRovingFocus';
import { useTooltip } from '../hooks/useTooltip';
import { Dot } from '../marks/Dot';
import { MUTED_LABEL_CLASS, MarkGroup } from '../marks/shared';
import { ClockBar } from './ClockStrip';
import { fitText } from './ScoreBreakdown';

/*
 * F2 ClockStack: a case's request clocks stacked on one axis. Casework clocks are a MODEL of the
 * CPRA determination period (filingPlan), never a counted violation, so a request past its
 * modelled date is an ink dashed span with the word 'modelled' and is never crimson. A request
 * with no filing plan has no clock and is listed as such, never drawn as a zero-length bar.
 */

const DAY = 86_400_000;
const ROW_LABEL_CLASS = 'fill-fg-secondary text-[12px]';
const LABEL_FONT_12 = '12px -apple-system, BlinkMacSystemFont, sans-serif';

function day(at: string): string {
  return formatDate(parseDate(at), 'day');
}

function plural(n: number, word: string): string {
  return `${n} ${word}${n === 1 ? '' : 's'}`;
}

function isClosed(request: CaseworkRequest): boolean {
  return request.state.toUpperCase() === 'CLOSED' || request.state.toUpperCase() === 'PRODUCED';
}

function noClockWords(request: CaseworkRequest): string {
  return request.submittedAt === null ? 'not filed: no clock' : 'no clock yet: save the case to evaluate it';
}

function requestLabel(request: CaseworkRequest): string {
  return [request.title || 'Untitled request', request.custodian].filter(Boolean).join(' · ');
}

/** The ≠ rule: a recorded due date that disagrees with the model is shown, not reconciled. */
function dueConflict(request: CaseworkRequest): boolean {
  const plan = request.filingPlan;
  return plan !== null && request.responseDue !== null && request.responseDue.slice(0, 10) !== plan.determinationDueOn.slice(0, 10);
}

export function caseClocksHeadline(found: CaseworkCase): string {
  const filed = found.requests.filter((request) => request.filingPlan !== null);
  const open = filed.filter((request) => !isClosed(request));
  const overdue = filed.filter((request) => request.filingPlan?.deadlinePassedWithoutRecordedResponse);
  const unfiled = found.requests.length - filed.length;
  const next = open
    .filter((request) => request.filingPlan && !request.filingPlan.deadlinePassedWithoutRecordedResponse)
    .sort((a, b) => parseDate(a.filingPlan!.determinationDueOn) - parseDate(b.filingPlan!.determinationDueOn))[0];
  const parts = [
    `${plural(open.length, 'filed request')} open`,
    overdue.length > 0 ? `${overdue.length} past the modelled determination date with no recorded response` : null,
    next ? `next due: ${next.title || 'untitled request'} on ${day(next.filingPlan!.determinationDueOn)}` : null,
  ].filter(Boolean);
  return `${parts.join('; ')}.${unfiled > 0 ? ` ${unfiled} not filed (no clock).` : ''}`;
}

// ---- Casework: a shared date axis -------------------------------------------------------------

interface DateRow {
  key: string;
  kind: 'request' | 'commitment' | 'undated';
  label: string;
  labelY: number;
  barY: number;
  request?: CaseworkRequest;
  commitment?: CaseworkCommitment;
  undated?: CaseworkCommitment[];
}

function dateLayout(found: CaseworkCase): { rows: DateRow[]; lanes: { y: number; text: string }[]; height: number } {
  const rows: DateRow[] = [];
  const lanes: { y: number; text: string }[] = [];
  let y = 8;
  lanes.push({ y, text: 'Requests' });
  y += 16;
  for (const request of found.requests) {
    rows.push({ key: request.id, kind: 'request', label: requestLabel(request), labelY: y + 6, barY: y + 18, request });
    y += 18 + 10 + 16;
  }
  if (found.commitments.length > 0) {
    lanes.push({ y: y + 4, text: 'Commitments' });
    y += 20;
    const dated = found.commitments.filter((commitment) => commitment.dueDate !== null);
    const undated = found.commitments.filter((commitment) => commitment.dueDate === null);
    for (const commitment of dated) {
      rows.push({ key: commitment.id, kind: 'commitment', label: `${commitment.promise || 'Unstated promise'}${commitment.responsibleParty ? ` · ${commitment.responsibleParty}` : ''}`, labelY: y + 6, barY: y + 18, commitment });
      y += 18 + 10 + 10;
    }
    if (undated.length > 0) {
      rows.push({ key: 'undated', kind: 'undated', label: `Undated: ${undated.map((commitment) => commitment.promise || 'unstated promise').join('; ')}`, labelY: y + 6, barY: y + 18, undated });
      y += 26;
    }
  }
  return { rows, lanes, height: y + 30 };
}

export function caseClocksHeight(found: CaseworkCase): number {
  return dateLayout(found).height;
}

function CaseworkPlot({ found, width, onOpenRequest }: { found: CaseworkCase; width: number; onOpenRequest(requestId: string): void }) {
  const tooltip = useTooltip();
  const { rows, lanes, height } = dateLayout(found);
  const focusable = rows.filter((row) => row.kind === 'request');
  const roving = useRovingFocus({ shape: { rows: Math.max(1, focusable.length) }, onActivate: (i) => focusable[i]?.request && onOpenRequest(focusable[i].request!.id), onEscape: tooltip.hide });
  const instants: number[] = [];
  let today: number | null = null;
  for (const request of found.requests) {
    const plan = request.filingPlan;
    if (!plan) continue;
    instants.push(parseDate(plan.filedOn), parseDate(plan.determinationDueOn), parseDate(plan.measuredTo ?? plan.evaluatedAt));
    today = Math.max(today ?? 0, parseDate(plan.evaluatedAt));
    if (request.responseDue) instants.push(parseDate(request.responseDue));
    for (const production of request.productions) if (production.receivedAt) instants.push(parseDate(production.receivedAt));
    for (const followUp of plan.followUps) instants.push(parseDate(followUp.scheduledFor));
  }
  for (const commitment of found.commitments) if (commitment.dueDate) instants.push(parseDate(commitment.dueDate));
  const valid = instants.filter(Number.isFinite);
  const lo = valid.length ? Math.min(...valid) : Date.now() - 30 * DAY;
  const hi = valid.length ? Math.max(...valid) : Date.now();
  const pad = Math.max(DAY * 2, (hi - lo) * 0.06);
  const t = scaleTime([lo - pad, hi + pad], [12, width - 12]);
  const axisY = height - 26;
  let focusIndex = -1;
  return (
    <g>
      {today !== null && <ReferenceLine kind="today" axis="x" at={t(today)} extent={[0, axisY]} label="last evaluated" labelSide={t(today) > width - 100 ? 'start' : 'end'} />}
      {lanes.map((lane) => (
        <text key={lane.text} x={0} y={lane.y} dy="0.32em" className="fill-muted text-[11px] font-semibold uppercase tracking-wide">{lane.text}</text>
      ))}
      {rows.map((row) => {
        const label = <text x={0} y={row.labelY} dy="0.32em" className={ROW_LABEL_CLASS}><title>{row.label}</title>{fitText(row.label, width - 4, LABEL_FONT_12)}</text>;
        if (row.kind === 'undated') return <g key={row.key}>{label}</g>;
        if (row.kind === 'commitment' && row.commitment?.dueDate) {
          const at = t(parseDate(row.commitment.dueDate));
          return (
            <g key={row.key}>
              {label}
              <Dot cx={at} cy={row.barY + 4} shape="diamond" hollow role="primary" r={4.5} title={`${row.label}: due ${day(row.commitment.dueDate)}, state ${row.commitment.state.toLowerCase().replace(/_/g, ' ')}`} />
              <text x={at + 9} y={row.barY + 4} dy="0.32em" className={MUTED_LABEL_CLASS}>{`due ${day(row.commitment.dueDate)}`}</text>
            </g>
          );
        }
        const request = row.request!;
        const plan = request.filingPlan;
        focusIndex += 1;
        if (!plan) {
          return (
            <MarkGroup key={row.key} box={{ x: 0, y: row.labelY - 8, width: width - 2, height: 30 }} interaction={{ name: `${row.label}: ${noClockWords(request)}.`, item: roving.getItemProps(focusIndex), onClick: () => onOpenRequest(request.id) }}>
              {label}
              <text x={0} y={row.barY + 4} dy="0.32em" className={MUTED_LABEL_CLASS}>{noClockWords(request)}</text>
            </MarkGroup>
          );
        }
        const filed = parseDate(plan.filedOn);
        const due = parseDate(plan.determinationDueOn);
        const endAt = parseDate(plan.measuredTo ?? plan.evaluatedAt);
        const span = Math.max(1, (Math.max(due, endAt) - filed) / DAY);
        // One request's day axis laid onto the shared date axis: day d sits at filed + d days.
        const dx = scaleLinear([0, span], [t(filed), t(filed + span * DAY)], { nice: false, zero: false });
        const endDay = (endAt - filed) / DAY;
        const dueDay = (due - filed) / DAY;
        const past = plan.deadlinePassedWithoutRecordedResponse;
        const name = `${row.label}: filed ${day(plan.filedOn)}, determination due ${day(plan.determinationDueOn)} under ${plan.statuteCitation}, ${plan.calendarDaysElapsed} of ${plan.allowedCalendarDays} allowed days elapsed as of ${day(plan.evaluatedAt)}${past ? ', past the modelled date with no recorded response' : ''}.`;
        const tip = (
          <TooltipContent
            title={row.label}
            rows={[
              { key: 'e', value: `${plan.calendarDaysElapsed} of ${plan.allowedCalendarDays}`, label: 'allowed calendar days elapsed' },
              { key: 'f', value: day(plan.filedOn), label: 'filed' },
              { key: 'd', value: day(plan.determinationDueOn), label: 'determination due (modelled)' },
            ]}
            note={`Basis: ${plan.determinationBasis.toLowerCase().replace(/_/g, ' ')}; evaluated ${day(plan.evaluatedAt)}.`}
          />
        );
        const pastWords = past ? 'modelled: past due, no recorded response' : null;
        return (
          <MarkGroup
            key={row.key}
            box={{ x: 0, y: row.labelY - 8, width: width - 2, height: 34 }}
            interaction={{
              name,
              item: roving.getItemProps(focusIndex),
              onPointerMove: (event) => tooltip.show(tip, { clientX: event.clientX, clientY: event.clientY }),
              onPointerLeave: tooltip.hide,
              onFocusMark: (box) => tooltip.show(tip, box),
              onBlurMark: tooltip.hide,
              onClick: () => onOpenRequest(request.id),
            }}
          >
            {label}
            <rect x={t(filed)} y={row.barY - 2} width={Math.max(1, t(due) - t(filed))} height={10} rx={2} style={{ fill: VIZ.track }} />
            <line x1={Math.round(t(due)) + 0.5} x2={Math.round(t(due)) + 0.5} y1={row.barY - 5} y2={row.barY + 11} style={{ stroke: REFERENCE_PAINT.statute.stroke, strokeWidth: 1 }} />
            <ClockBar x={dx} y={row.barY + 1} thickness={4} endDay={endDay} deadlineDay={past ? dueDay : null} past="uncounted" open={plan.measuredTo === null} />
            {request.productions.filter((production) => production.receivedAt).map((production) => (
              <Dot key={production.id} cx={t(parseDate(production.receivedAt))} cy={row.barY + 3} shape="diamond" role="primary" r={4.5} title={`Production received ${day(production.receivedAt)}: ${production.description || 'no description'}`} />
            ))}
            {plan.followUps.map((followUp) => (
              <Dot key={`${followUp.trigger}-${followUp.scheduledFor}`} cx={t(parseDate(followUp.scheduledFor))} cy={row.barY + 3} hollow role="primary" r={3.5} title={`Follow-up scheduled ${day(followUp.scheduledFor)}: ${followUp.rationale}`} />
            ))}
            {dueConflict(request) && request.responseDue && (
              <g>
                <title>{`Recorded response due ${day(request.responseDue)} differs from the modelled ${day(plan.determinationDueOn)}`}</title>
                <line x1={t(parseDate(request.responseDue))} x2={t(due)} y1={row.barY + 14} y2={row.barY + 14} style={{ stroke: VIZ.ink, strokeWidth: 1, strokeDasharray: '3 2' }} />
                <text x={(t(parseDate(request.responseDue)) + t(due)) / 2} y={row.barY + 14} dy="0.32em" textAnchor="middle" className="fill-fg text-[11px] font-semibold" style={{ paintOrder: 'stroke', stroke: VIZ.surface, strokeWidth: 3 }}>≠</text>
              </g>
            )}
            {pastWords && <text x={Math.min(width - measureText(pastWords) - 2, t(endAt) + 12)} y={row.barY + 20} dy="0.32em" className="fill-fg-secondary text-[11px]">{pastWords}</text>}
          </MarkGroup>
        );
      })}
      <Axis time={t} orient="bottom" offset={axisY} />
      {tooltip.node}
    </g>
  );
}

export default function ClockStack({ found, onOpenRequest }: { found: CaseworkCase; onOpenRequest(requestId: string): void }) {
  const headline = caseClocksHeadline(found);
  const citation = found.requests.find((request) => request.filingPlan)?.filingPlan?.statuteCitation;
  const evaluated = found.requests.map((request) => request.filingPlan?.evaluatedAt).filter((at): at is string => Boolean(at)).sort().slice(-1)[0];
  return (
    <ChartFrame
      title="Request determination clocks"
      headline={found.requests.length === 0 && found.commitments.length === 0 ? null : headline}
      sources={{ tables: ['casework_cases', 'casework_events'], note: citation ? `clock modelled under ${citation}${evaluated ? ` as of ${day(evaluated)}` : ''}; a model, not a counted violation` : 'no filed request, so no clock is modelled' }}
      summary={`${headline} ${found.commitments.length} commitments, ${found.commitments.filter((commitment) => commitment.dueDate === null).length} undated.`}
      status={found.requests.length === 0 && found.commitments.length === 0 ? 'empty' : 'ready'}
      emptyText="No request or commitment is tracked in this case."
      table={{
        caption: 'Request clocks and commitments',
        columns: [{ key: 'item', label: 'Request or commitment' }, { key: 'filed', label: 'Filed' }, { key: 'due', label: 'Due' }, { key: 'elapsed', label: 'Elapsed of allowed' }, { key: 'state', label: 'State' }],
        rows: [
          ...found.requests.map((request) => ({
            item: requestLabel(request),
            filed: request.filingPlan?.filedOn ?? request.submittedAt ?? { state: 'not-recorded' as const, reason: 'no submission date recorded' },
            due: request.filingPlan?.determinationDueOn ?? { state: 'not-recorded' as const, reason: noClockWords(request) },
            elapsed: request.filingPlan ? `${request.filingPlan.calendarDaysElapsed} of ${request.filingPlan.allowedCalendarDays} days` : { state: 'not-recorded' as const, reason: 'no clock' },
            state: `${request.state.toLowerCase()}${request.filingPlan?.deadlinePassedWithoutRecordedResponse ? '; modelled: past due, no recorded response' : ''}${dueConflict(request) ? `; recorded due ${request.responseDue} ≠ modelled` : ''}`,
          })),
          ...found.commitments.map((commitment) => ({
            item: `Commitment: ${commitment.promise || 'unstated promise'}`,
            filed: '',
            due: commitment.dueDate ?? { state: 'not-recorded' as const, reason: 'undated' },
            elapsed: '',
            state: commitment.state.toLowerCase().replace(/_/g, ' '),
          })),
        ],
      }}
      legend={[
        { key: 'elapsed', label: 'Elapsed', swatch: 'rect', tone: 'ink' },
        { key: 'allowed', label: 'Allowed period (modelled)', swatch: 'rect', tone: 'track' },
        { key: 'past', label: 'Past due (modelled, not counted)', swatch: 'dashed-line', tone: 'ink' },
        { key: 'followup', label: 'Follow-up', swatch: 'hollow', tone: 'ink' },
      ]}
      minHeight={caseClocksHeight(found)}
      interactive={{ roledescription: 'request clocks' }}
      storageKey="casework-clock-stack"
    >
      {({ width }) => <CaseworkPlot found={found} width={width} onOpenRequest={onOpenRequest} />}
    </ChartFrame>
  );
}

// ---- Inspector: mini clocks on a shared day axis ----------------------------------------------

const MINI_ROW = 22;

export function miniClocksHeadline(found: CaseworkCase): string {
  const total = found.requests.length;
  const open = found.requests.filter((request) => !isClosed(request)).length;
  const past = found.requests.filter((request) => request.filingPlan?.deadlinePassedWithoutRecordedResponse).length;
  const unfiled = found.requests.filter((request) => request.filingPlan === null).length;
  return `${open} of ${plural(total, 'request')} open; ${past} past the modelled determination date; ${unfiled} without a clock.`;
}

function MiniPlot({ found, width, onOpen }: { found: CaseworkCase; width: number; onOpen(): void }) {
  const tooltip = useTooltip();
  const requests = found.requests;
  const roving = useRovingFocus({ shape: { rows: Math.max(1, requests.length) }, onActivate: onOpen, onEscape: tooltip.hide });
  const labelColumn = Math.min(130, Math.round(width * 0.36));
  const max = Math.max(1, ...requests.map((request) => request.filingPlan ? Math.max(request.filingPlan.allowedCalendarDays, request.filingPlan.calendarDaysElapsed) : 0));
  const x = scaleLinear([0, max * 1.08], [labelColumn + 8, width - 64], { zero: true });
  const axisY = requests.length * MINI_ROW + 6;
  return (
    <g>
      {requests.map((request, index) => {
        const cy = index * MINI_ROW + MINI_ROW / 2;
        const plan = request.filingPlan;
        const label = request.title || request.custodian || 'Untitled request';
        const received = isClosed(request) || request.productions.some((production) => production.receivedAt);
        const name = plan
          ? `${label}: ${plan.calendarDaysElapsed} of ${plan.allowedCalendarDays} allowed days elapsed as of ${day(plan.evaluatedAt)}${plan.deadlinePassedWithoutRecordedResponse ? ', past the modelled date' : ''}${received ? ', production received' : ''}.`
          : `${label}: ${noClockWords(request)}.`;
        const tip = <TooltipContent title={label} rows={plan ? [{ key: 'e', value: `${plan.calendarDaysElapsed} of ${plan.allowedCalendarDays}`, label: `days, as of ${day(plan.evaluatedAt)}` }, { key: 'f', value: day(plan.filedOn), label: 'filed' }, { key: 'd', value: day(plan.determinationDueOn), label: 'due (modelled)' }] : []} note={request.custodian || undefined} />;
        const conflict = dueConflict(request);
        return (
          <MarkGroup
            key={request.id}
            box={{ x: 0, y: cy - MINI_ROW / 2 + 1, width: width - 2, height: MINI_ROW - 2 }}
            interaction={{
              name,
              item: roving.getItemProps(index),
              onPointerMove: (event) => tooltip.show(tip, { clientX: event.clientX, clientY: event.clientY }),
              onPointerLeave: tooltip.hide,
              onFocusMark: (box) => tooltip.show(tip, box),
              onBlurMark: tooltip.hide,
              onClick: onOpen,
            }}
          >
            <text x={0} y={cy} dy="0.32em" className={ROW_LABEL_CLASS}><title>{label}</title>{fitText(label, labelColumn, LABEL_FONT_12)}</text>
            {plan ? (
              <>
                <line x1={Math.round(x(plan.allowedCalendarDays)) + 0.5} x2={Math.round(x(plan.allowedCalendarDays)) + 0.5} y1={cy - 7} y2={cy + 7} style={{ stroke: REFERENCE_PAINT.statute.stroke, strokeWidth: 1 }} />
                <ClockBar x={x} y={cy - 3} thickness={6} endDay={plan.calendarDaysElapsed} deadlineDay={plan.deadlinePassedWithoutRecordedResponse ? plan.allowedCalendarDays : null} past="uncounted" open={plan.measuredTo === null} />
                {received && <Dot cx={x(plan.calendarDaysElapsed) + (plan.measuredTo === null ? 14 : 6)} cy={cy} r={3} role="positive" title="Production received" />}
                <text x={x(plan.calendarDaysElapsed) + (plan.measuredTo === null ? 14 : 6) + (received ? 8 : 0)} y={cy} dy="0.32em" className="viz-num fill-fg-secondary text-[11px]">
                  {`${plan.calendarDaysElapsed}/${plan.allowedCalendarDays} d${plan.deadlinePassedWithoutRecordedResponse ? ' modelled' : ''}${received ? ' received' : ''}${conflict ? ' ≠' : ''}`}
                </text>
              </>
            ) : (
              <text x={labelColumn + 8} y={cy} dy="0.32em" className={MUTED_LABEL_CLASS}>{noClockWords(request).split(':')[0]}: no clock</text>
            )}
          </MarkGroup>
        );
      })}
      <Axis scale={x} kind="days" orient="bottom" offset={axisY} />
      {tooltip.node}
    </g>
  );
}

export function MiniClocks({ found, onOpen }: { found: CaseworkCase; onOpen(): void }) {
  const headline = miniClocksHeadline(found);
  const citation = found.requests.find((request) => request.filingPlan)?.filingPlan?.statuteCitation;
  return (
    <ChartFrame
      title="Request clocks"
      headline={found.requests.length === 0 ? null : headline}
      sources={{ tables: ['casework_cases', 'casework_events'], note: citation ? `modelled under ${citation}; not a counted violation` : 'no filed request, so no clock is modelled' }}
      summary={headline}
      status={found.requests.length === 0 ? 'empty' : 'ready'}
      emptyText="No request is tracked in this case."
      table={{
        caption: 'Requests in this case',
        columns: [{ key: 'title', label: 'Request' }, { key: 'state', label: 'State' }, { key: 'elapsed', label: 'Days elapsed', align: 'end' }, { key: 'allowed', label: 'Allowed', align: 'end' }, { key: 'due', label: 'Due' }],
        rows: found.requests.map((request) => ({
          title: request.title || request.custodian || 'Untitled request',
          state: request.state.toLowerCase(),
          elapsed: request.filingPlan?.calendarDaysElapsed ?? { state: 'not-recorded' as const, reason: noClockWords(request) },
          allowed: request.filingPlan?.allowedCalendarDays ?? { state: 'not-recorded' as const, reason: 'no clock' },
          due: request.filingPlan?.determinationDueOn ?? request.responseDue ?? { state: 'not-recorded' as const, reason: 'no due date' },
        })),
      }}
      minHeight={Math.max(1, found.requests.length) * MINI_ROW + 30}
      interactive={{ roledescription: 'request clocks' }}
      storageKey="inspector-case-clocks"
    >
      {({ width }) => <MiniPlot found={found} width={width} onOpen={onOpen} />}
    </ChartFrame>
  );
}
