import type { CaseworkCase, CaseworkCommitment, CaseworkRequest } from '../../../types/casework';
import ChartFrame from '../frame/ChartFrame';
import { Axis } from '../frame/Axis';
import { ReferenceLine } from '../frame/ReferenceLine';
import { TooltipContent } from '../frame/Tooltip';
import type { LegendItem } from '../frame/Legend';
import { REFERENCE_PAINT, VIZ } from '../core/claim';
import { formatDate } from '../core/format';
import { measureText, placeLabels } from '../core/layout';
import { parseDate, scaleLinear, scaleTime } from '../core/scale';
import { useRovingFocus } from '../hooks/useRovingFocus';
import { useTooltip } from '../hooks/useTooltip';
import { Dot } from '../marks/Dot';
import { AbsentSlot, MUTED_LABEL_CLASS, MarkGroup } from '../marks/shared';
import { absent } from '../core/nulls';
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
  // Rows are requests and dated commitments; a request's follow-ups are the columns after it.
  const focusable = rows.filter((row) => row.kind === 'request' || row.kind === 'commitment');
  const cols = 1 + Math.max(0, ...focusable.map((row) => row.request?.filingPlan?.followUps.length ?? 0));
  const roving = useRovingFocus({
    shape: { rows: Math.max(1, focusable.length), cols },
    onActivate: (i, j = 0) => {
      const row = focusable[i];
      if (j === 0 && row?.request) onOpenRequest(row.request.id);
    },
    onEscape: tooltip.hide,
  });
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
          focusIndex += 1;
          const at = t(parseDate(row.commitment.dueDate));
          const name = `${row.label}: due ${day(row.commitment.dueDate)}, state ${row.commitment.state.toLowerCase().replace(/_/g, ' ')}.`;
          const tip = <TooltipContent title={row.commitment.promise || 'Unstated promise'} rows={[{ key: 'due', value: day(row.commitment.dueDate), label: 'due' }]} note={name} />;
          return (
            <MarkGroup
              key={row.key}
              box={{ x: 0, y: row.labelY - 8, width: width - 2, height: 28 }}
              interaction={{
                name,
                item: roving.getItemProps(focusIndex),
                onPointerMove: (event) => tooltip.show(tip, { clientX: event.clientX, clientY: event.clientY }),
                onPointerLeave: tooltip.hide,
                onFocusMark: (box) => tooltip.show(tip, box),
                onBlurMark: tooltip.hide,
              }}
            >
              {label}
              <Dot cx={at} cy={row.barY + 4} shape="diamond" hollow role="primary" r={4.5} />
              <text x={at + 9} y={row.barY + 4} dy="0.32em" className={MUTED_LABEL_CLASS}>{`due ${day(row.commitment.dueDate)}`}</text>
            </MarkGroup>
          );
        }
        const request = row.request!;
        const plan = request.filingPlan;
        focusIndex += 1;
        const rowIndex = focusIndex;
        if (!plan) {
          return (
            <MarkGroup key={row.key} box={{ x: 0, y: row.labelY - 8, width: width - 2, height: 30 }} interaction={{ name: `${row.label}: ${noClockWords(request)}.`, item: roving.getItemProps(focusIndex), onClick: () => onOpenRequest(request.id) }}>
              {label}
              <AbsentSlot absence={absent('not-recorded', noClockWords(request))} box={{ x: 0, y: row.barY - 4, width: Math.min(width - 2, 260), height: 14 }} text={noClockWords(request)} />
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
        const conflict = dueConflict(request) && request.responseDue ? parseDate(request.responseDue) : null;
        // The ≠ badge and the past-due words share one row below the bar, pushed apart so neither covers the other.
        const noteRow = placeLabels([
          ...(conflict !== null ? [{ key: 'ne', anchor: (t(conflict) + t(due)) / 2, width: 10 }] : []),
          ...(pastWords ? [{ key: 'past', anchor: t(endAt) + 12 + measureText(pastWords) / 2, width: measureText(pastWords) }] : []),
        ], 0, width - 2);
        const notePos = (key: string) => noteRow.find((placed) => placed.key === key)?.pos ?? 0;
        return (
          <g key={row.key}>
          <MarkGroup
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
            {conflict !== null && request.responseDue && (
              <g data-due-conflict="">
                <title>{`Recorded response due ${day(request.responseDue)} differs from the modelled ${day(plan.determinationDueOn)}`}</title>
                <line x1={Math.round(t(conflict)) + 0.5} x2={Math.round(t(conflict)) + 0.5} y1={row.barY - 5} y2={row.barY + 11} style={{ stroke: VIZ.ink, strokeWidth: 1, strokeDasharray: '2 2' }} />
                <line x1={t(conflict)} x2={t(due)} y1={row.barY + 11} y2={row.barY + 11} style={{ stroke: VIZ.ink, strokeWidth: 1, strokeDasharray: '3 2' }} />
                <text x={notePos('ne')} y={row.barY + 19} dy="0.32em" className="fill-fg text-[11px] font-semibold">≠</text>
              </g>
            )}
            {pastWords && <text x={notePos('past')} y={row.barY + 19} dy="0.32em" className="fill-fg-secondary text-[11px]">{pastWords}</text>}
          </MarkGroup>
          {plan.followUps.map((followUp, j) => {
            const at = t(parseDate(followUp.scheduledFor));
            const followName = `Follow-up on ${row.label}, scheduled ${day(followUp.scheduledFor)}: ${followUp.rationale}`;
            const followTip = <TooltipContent title={`Follow-up ${day(followUp.scheduledFor)}`} rows={[]} note={followUp.rationale} />;
            return (
              <MarkGroup
                key={`${followUp.trigger}-${followUp.scheduledFor}`}
                box={{ x: at - 5, y: row.barY - 2, width: 10, height: 10 }}
                interaction={{
                  name: followName,
                  item: roving.getItemProps(rowIndex, j + 1),
                  onPointerMove: (event) => tooltip.show(followTip, { clientX: event.clientX, clientY: event.clientY }),
                  onPointerLeave: tooltip.hide,
                  onFocusMark: (box) => tooltip.show(followTip, box),
                  onBlurMark: tooltip.hide,
                }}
              >
                <Dot cx={at} cy={row.barY + 3} hollow role="primary" r={3.5} />
              </MarkGroup>
            );
          })}
          </g>
        );
      })}
      <Axis time={t} orient="bottom" offset={axisY} />
      {tooltip.node}
    </g>
  );
}

function stackLegend(found: CaseworkCase): LegendItem[] {
  const plans = found.requests.map((request) => request.filingPlan).filter((plan) => plan !== null);
  const items: LegendItem[] = [];
  if (plans.length > 0) {
    items.push({ key: 'elapsed', label: 'Elapsed', swatch: 'rect', tone: 'ink' }, { key: 'allowed', label: 'Allowed period (modelled)', swatch: 'rect', tone: 'track' });
  }
  if (plans.some((plan) => plan.deadlinePassedWithoutRecordedResponse)) items.push({ key: 'past', label: 'Past due (modelled, not counted)', swatch: 'dashed-line', tone: 'ink' });
  if (found.requests.some((request) => request.filingPlan && request.productions.some((production) => production.receivedAt))) items.push({ key: 'production', label: 'Production received', swatch: 'diamond', tone: 'ink' });
  if (found.commitments.some((commitment) => commitment.dueDate)) items.push({ key: 'commitment', label: 'Commitment due', swatch: 'hollow-diamond', tone: 'ink' });
  if (plans.some((plan) => plan.followUps.length > 0)) items.push({ key: 'followup', label: 'Follow-up', swatch: 'hollow', tone: 'ink' });
  if (found.requests.some(dueConflict)) items.push({ key: 'conflict', label: 'Recorded due ≠ modelled', swatch: 'dashed-line', tone: 'ink' });
  return items;
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
      legend={stackLegend(found)}
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
/** Below this width a title sits on its own line above its bar rather than beside it. */
const MINI_STACK_BELOW = 380;
const MINI_STACKED_ROW = 36;

function miniRowHeight(width: number): number {
  return width < MINI_STACK_BELOW ? MINI_STACKED_ROW : MINI_ROW;
}

function miniLabel(request: CaseworkRequest): string {
  return request.title || request.custodian || 'Untitled request';
}

/** The words after a mini clock's bar: elapsed of allowed, and its state. */
function miniEndText(request: CaseworkRequest): string {
  const plan = request.filingPlan;
  if (!plan) return '';
  const received = isClosed(request) || request.productions.some((production) => production.receivedAt);
  return [
    `${plan.calendarDaysElapsed}/${plan.allowedCalendarDays} d`,
    plan.deadlinePassedWithoutRecordedResponse ? 'modelled' : null,
    received ? 'received' : null,
  ].filter(Boolean).join(' · ');
}

export function miniClocksHeadline(found: CaseworkCase): string {
  const total = found.requests.length;
  const open = found.requests.filter((request) => !isClosed(request)).length;
  const past = found.requests.filter((request) => request.filingPlan?.deadlinePassedWithoutRecordedResponse).length;
  const unfiled = found.requests.filter((request) => request.filingPlan === null).length;
  const strongest = found.evidence.filter((cite) => cite.verification === 'SAVED_REPORT' || cite.verification === 'VERIFIED').length;
  const clauses = [
    `${open} of ${plural(total, 'request')} open`,
    past > 0 ? `${past} past the modelled determination date` : null,
    unfiled > 0 ? `${unfiled} without a clock` : null,
    found.evidence.length > 0 ? `${plural(found.evidence.length, 'source')} cited, ${strongest} at the strongest grade` : null,
  ].filter(Boolean);
  return `${clauses.join('; ')}.`;
}

/** The recorded response-due date as a day on the request's own axis; null when none conflicts with the model. */
function recordedDueDay(request: CaseworkRequest): number | null {
  const plan = request.filingPlan;
  if (!plan || !request.responseDue || !dueConflict(request)) return null;
  return (parseDate(request.responseDue) - parseDate(plan.filedOn)) / DAY;
}

/** Where a mini clock's end words start: past its furthest mark (bar end with chevron, deadline tick, recorded-due tick). */
function miniEndX(x: (value: number) => number, request: CaseworkRequest): number {
  const plan = request.filingPlan!;
  const barEnd = x(plan.calendarDaysElapsed) + (plan.measuredTo === null ? 10 : 2);
  return Math.max(barEnd, x(plan.allowedCalendarDays), x(recordedDueDay(request) ?? 0)) + 6;
}

function MiniPlot({ found, width, onOpen }: { found: CaseworkCase; width: number; onOpen(requestId: string): void }) {
  const tooltip = useTooltip();
  const requests = found.requests;
  const roving = useRovingFocus({ shape: { rows: Math.max(1, requests.length) }, onActivate: (i) => requests[i] && onOpen(requests[i].id), onEscape: tooltip.hide });
  const stacked = width < MINI_STACK_BELOW;
  const rowHeight = miniRowHeight(width);
  const labelColumn = stacked ? 0 : Math.min(Math.round(width * 0.45), Math.max(40, ...requests.map((request) => measureText(miniLabel(request), LABEL_FONT_12))) + 4);
  const barLeft = stacked ? 0 : labelColumn + 8;
  const max = Math.max(1, ...requests.map((request) => request.filingPlan ? Math.max(request.filingPlan.allowedCalendarDays, request.filingPlan.calendarDaysElapsed, recordedDueDay(request) ?? 0) : 0));
  const endRoom = Math.max(40, ...requests.map((request) => measureText(miniEndText(request)))) + 26;
  const x = scaleLinear([0, max * 1.08], [barLeft, Math.max(barLeft + 48, width - endRoom)], { zero: true });
  const axisY = requests.length * rowHeight + 6;
  return (
    <g>
      {requests.map((request, index) => {
        const top = index * rowHeight;
        const cy = stacked ? top + 24 : top + rowHeight / 2;
        const labelY = stacked ? top + 8 : cy;
        const plan = request.filingPlan;
        const label = miniLabel(request);
        const received = isClosed(request) || request.productions.some((production) => production.receivedAt);
        const name = plan
          ? `${label}: ${plan.calendarDaysElapsed} of ${plan.allowedCalendarDays} allowed days elapsed as of ${day(plan.evaluatedAt)}${plan.deadlinePassedWithoutRecordedResponse ? ', past the modelled date' : ''}${received ? ', production received' : ''}.`
          : `${label}: ${noClockWords(request)}.`;
        const tip = <TooltipContent title={label} rows={plan ? [{ key: 'e', value: `${plan.calendarDaysElapsed} of ${plan.allowedCalendarDays}`, label: `days, as of ${day(plan.evaluatedAt)}` }, { key: 'f', value: day(plan.filedOn), label: 'filed' }, { key: 'd', value: day(plan.determinationDueOn), label: 'due (modelled)' }] : []} note={request.custodian || undefined} />;
        return (
          <MarkGroup
            key={request.id}
            box={{ x: 0, y: top + 1, width: width - 2, height: rowHeight - 2 }}
            interaction={{
              name,
              item: roving.getItemProps(index),
              onPointerMove: (event) => tooltip.show(tip, { clientX: event.clientX, clientY: event.clientY }),
              onPointerLeave: tooltip.hide,
              onFocusMark: (box) => tooltip.show(tip, box),
              onBlurMark: tooltip.hide,
              onClick: () => onOpen(request.id),
            }}
          >
            <text x={0} y={labelY} dy="0.32em" className={ROW_LABEL_CLASS}><title>{label}</title>{fitText(label, stacked ? width - 4 : labelColumn, LABEL_FONT_12)}</text>
            {plan ? (
              <>
                <line x1={Math.round(x(plan.allowedCalendarDays)) + 0.5} x2={Math.round(x(plan.allowedCalendarDays)) + 0.5} y1={cy - 7} y2={cy + 7} style={{ stroke: REFERENCE_PAINT.statute.stroke, strokeWidth: 1 }} />
                {dueConflict(request) && request.responseDue && (() => {
                  // The recorded due date as its own day on this request's axis, tied to the model's tick by the ≠ connector.
                  const rx = Math.round(x(Math.max(0, recordedDueDay(request) ?? 0))) + 0.5;
                  const mx = Math.round(x(plan.allowedCalendarDays)) + 0.5;
                  return (
                    <g data-due-conflict="">
                      <title>{`Recorded response due ${day(request.responseDue)} differs from the modelled ${day(plan.determinationDueOn)}`}</title>
                      <line x1={rx} x2={rx} y1={cy - 7} y2={cy + 7} style={{ stroke: VIZ.ink, strokeWidth: 1, strokeDasharray: '2 2' }} />
                      <line x1={Math.min(rx, mx)} x2={Math.max(rx, mx)} y1={cy - 8.5} y2={cy - 8.5} style={{ stroke: VIZ.ink, strokeWidth: 1, strokeDasharray: '3 2' }} />
                      <text x={(rx + mx) / 2} y={cy - 8.5} dy="0.32em" textAnchor="middle" className="fill-fg text-[10px] font-semibold" style={{ paintOrder: 'stroke', stroke: VIZ.surface, strokeWidth: 3 }}>≠</text>
                    </g>
                  );
                })()}
                <ClockBar x={x} y={cy - 3} thickness={6} endDay={plan.calendarDaysElapsed} deadlineDay={plan.deadlinePassedWithoutRecordedResponse ? plan.allowedCalendarDays : null} past="uncounted" open={plan.measuredTo === null} />
                {received && <Dot cx={miniEndX(x, request) + 3} cy={cy} r={3} role="positive" title="Production received" />}
                <text x={miniEndX(x, request) + (received ? 11 : 0)} y={cy} dy="0.32em" className="viz-num fill-fg-secondary text-[11px]">
                  {miniEndText(request)}
                </text>
              </>
            ) : (
              <AbsentSlot absence={absent('not-recorded', noClockWords(request))} box={{ x: barLeft, y: cy - 7, width: Math.max(40, Math.min(200, width - barLeft - 4)), height: 14 }} text={request.submittedAt === null ? 'not filed: no clock' : 'no clock yet'} />
            )}
          </MarkGroup>
        );
      })}
      {requests.some((request) => request.filingPlan) && <Axis scale={x} kind="days" orient="bottom" offset={axisY} />}
      {tooltip.node}
    </g>
  );
}

export function MiniClocks({ found, onOpen }: { found: CaseworkCase; onOpen(requestId: string): void }) {
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
      minHeight={Math.max(1, found.requests.length) * MINI_ROW + (found.requests.some((request) => request.filingPlan) ? 30 : 4)}
      plotHeight={(width) => Math.max(1, found.requests.length) * miniRowHeight(width) + (found.requests.some((request) => request.filingPlan) ? 30 : 4)}
      interactive={{ roledescription: 'request clocks' }}
      storageKey="inspector-case-clocks"
    >
      {({ width }) => <MiniPlot found={found} width={width} onOpen={onOpen} />}
    </ChartFrame>
  );
}
