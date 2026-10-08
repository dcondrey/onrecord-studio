import type { ReactNode } from 'react';
import type { GlyphShape } from '../../ui/shapeGlyph';
import { deadlineChipSpec } from '../../tracker/DeadlineChip';
import type {
  ClaimedRequest,
  DeadlineAnalysis,
  LegalComplianceFinding,
  PortalRequestDeadline,
  RequestDeadline,
  RequestFilingPlan,
  RequestSearchResult,
  StatutoryExtensionRecord,
  TimelineEventKind,
} from '../../../types';
import ChartFrame from '../frame/ChartFrame';
import { Axis } from '../frame/Axis';
import { ReferenceBand } from '../frame/ReferenceLine';
import type { ChartSources } from '../frame/SourceCaption';
import type { TwinCell } from '../frame/TableTwin';
import { TooltipContent } from '../frame/Tooltip';
import type { LegendItem } from '../frame/Legend';
import { paintFor, REFERENCE_PAINT, VIZ, type Claim } from '../core/claim';
import { formatDate } from '../core/format';
import { LABEL_FONT, measureText, placeLabels } from '../core/layout';
import { parseDate, scaleLinear, type ContinuousScale } from '../core/scale';
import { useRovingFocus } from '../hooks/useRovingFocus';
import { useTooltip } from '../hooks/useTooltip';
import { MUTED_LABEL_CLASS, hBarPath } from '../marks/shared';
import { TimelineLane } from '../marks/TimelineLane';
import { Dot } from '../marks/Dot';

/*
 * F2 StatutoryClock: one request's determination clock on an axis of days since submission. The
 * deadline comes only from the statute engine (or, for a portal request without it, is not drawn);
 * nothing here recomputes it. The span past the deadline is crimson only for a COUNTED measure: the
 * tracker's violationAsserted or a stored statutory_violations finding. Anything else past the
 * deadline is an ink dashed outline with the word 'provisional', 'not counted' or 'modelled'.
 */

const DAY = 86_400_000;

export type ClockModel = 'tracker' | 'portal-recorded' | 'portal-engine' | 'casework';

export interface ClockEvent {
  key: string;
  /** 'Extension notice', 'Created'. */
  label: string;
  at: string;
  shape: GlyphShape;
  hollow: boolean;
  /** How the date is known: 'confirmed from an email', 'entered by you', 'recorded by the portal'. */
  basisLabel: string;
  /** Printed beside the glyph; only the milestones the story turns on. */
  direct: boolean;
  /** Supporting text for the tooltip (the email's matched sentence). */
  detail?: string | null;
}

export interface ClockDeadline {
  initialAt: string;
  effectiveAt: string;
  allowedDays: number;
  citation: string;
  /** Set when a valid extension moved the deadline past the initial one. */
  extensionReason: string | null;
}

export interface ClockCounted {
  /** Days past the deadline as the evaluator stored them; null when it stored none. */
  varianceDays: number | null;
  citation: string;
  /** 'Violation counted' for the tracker; the finding's review status for a portal request. */
  reviewStatus: 'ASSERTED' | LegalComplianceFinding['reviewStatus'];
  /** The instant the variance was measured to (closure, else the evaluation); null when a deadline is drawn. */
  measuredTo: string | null;
}

interface ClockBase {
  createdAt: string;
  observedAt: string;
  deadline: ClockDeadline | null;
  /** A recorded determination (sourced, or a proxy from the earliest activity); null while none is. */
  determination: { at: string; basis: 'SOURCED' | 'PROXY'; elapsedDays: number | null } | null;
  /** Where the elapsed bar stops, and the words for an open end. */
  end: { at: string; openLabel: string | null };
  events: ClockEvent[];
  /** Engine anomaly markers, as sentences. */
  anomalies: string[];
  /** Set when there is no clock to draw: the sentence replaces the plot, with no axis. */
  noClock: string | null;
  headline: string;
  summary: string;
  sources: ChartSources;
  /** The word on a past-deadline span that is not counted. */
  uncountedWord: 'provisional' | 'not counted' | 'modelled';
}

/** A counted span is representable only where a counted measure exists; never for a casework model. */
export type ClockData =
  | (ClockBase & { model: 'tracker' | 'portal-recorded' | 'portal-engine'; counted: ClockCounted | null })
  | (ClockBase & { model: 'casework'; counted?: never });

export type PastDeadlinePaint = 'counted' | 'dismissed' | 'resolved' | 'uncounted';

/** Review states under which a stored finding still stands as a counted violation. */
const LIVE_REVIEW: ReadonlySet<ClockCounted['reviewStatus']> = new Set(['ASSERTED', 'OPEN', 'CONFIRMED']);

/**
 * How the span past a deadline is painted: crimson only for a live counted measure. A finding
 * the reviewer dismissed, or one re-evaluation withdrew (RESOLVED_BY_NEW_EVIDENCE is written
 * exactly when the evaluator no longer finds a violation), is drawn struck through, never counted.
 */
export function pastDeadlinePaint(data: ClockData): PastDeadlinePaint {
  if (data.model === 'casework' || !data.counted) return 'uncounted';
  if (LIVE_REVIEW.has(data.counted.reviewStatus)) return 'counted';
  return data.counted.reviewStatus === 'DISMISSED' ? 'dismissed' : 'resolved';
}

function dayOf(at: string, created: number): number {
  return (parseDate(at) - created) / DAY;
}

function shortDate(at: string): string {
  return formatDate(parseDate(at), 'day');
}

function plural(n: number, word: string): string {
  return `${n.toLocaleString('en-US')} ${word}${n === 1 ? '' : 's'}`;
}

/** UTC calendar days between two recorded instants, by date. */
function calendarDaysBetween(from: string, to: string): number {
  const a = new Date(parseDate(from));
  const b = new Date(parseDate(to));
  return Math.round((Date.UTC(b.getUTCFullYear(), b.getUTCMonth(), b.getUTCDate()) - Date.UTC(a.getUTCFullYear(), a.getUTCMonth(), a.getUTCDate())) / DAY);
}

const REVIEW_WORDS: Record<ClockCounted['reviewStatus'], string> = {
  ASSERTED: 'counted',
  OPEN: 'open for review',
  CONFIRMED: 'confirmed',
  DISMISSED: 'dismissed',
  RESOLVED_BY_NEW_EVIDENCE: 'resolved by new evidence',
};

const ANOMALY_WORDS: Record<string, string> = {
  SUSPICIOUSLY_FAST_CLOSURE: 'Closed within minutes of creation.',
  UPDATE_BEFORE_CREATION: 'An update is dated before the request was created.',
  CLOSE_BEFORE_CREATION: 'The closure is dated before the request was created.',
  OBSERVATION_BEFORE_CREATION: 'Observed before the request was created.',
  STATUTORY_EXTENSION_DEFECTIVE: 'The extension notice does not meet the statute\'s conditions, so the initial deadline applies.',
};

function anomalySentences(markers: readonly string[]): string[] {
  // The latency marker is the clock itself; drawing it again as a callout would double-count it.
  return markers.filter((marker) => marker !== 'STATUTORY_LATENCY_VIOLATION').map((marker) => ANOMALY_WORDS[marker] ?? marker);
}

function engineDeadline(analysis: DeadlineAnalysis): ClockDeadline {
  return {
    initialAt: analysis.initialDeadlineAt,
    effectiveAt: analysis.effectiveDeadlineAt,
    allowedDays: analysis.allowedCalendarDays,
    citation: analysis.statuteCitation,
    extensionReason: analysis.extensionStatus === 'VALID' ? analysis.extensionReason ?? 'reason not recorded' : null,
  };
}

function engineDetermination(analysis: DeadlineAnalysis): ClockData['determination'] {
  if (!analysis.determinationAt || analysis.determinationBasis === 'OBSERVATION_WITHOUT_RECORDED_ACTIVITY') return null;
  return {
    at: analysis.determinationAt,
    basis: analysis.determinationBasis === 'SOURCED_DETERMINATION_EVENT' ? 'SOURCED' : 'PROXY',
    elapsedDays: analysis.calendarDaysElapsed,
  };
}

// ---- Adapters -------------------------------------------------------------------------------

const NO_CLOCK_REASON: Partial<Record<RequestDeadline['countdown']['kind'], string>> = {
  NO_SUBMISSION_DATE: 'no submission date is recorded',
  NO_DEADLINE_MODEL: 'the app has no deadline model for this jurisdiction',
  UNAVAILABLE: 'the recorded dates conflict, so they were not evaluated',
};

const TRACKER_DIRECT: ReadonlySet<TimelineEventKind> = new Set(['SUBMITTED', 'EXTENSION_NOTICE', 'DETERMINATION']);

/** The tracker's clock: the engine's analysis over the dates the user confirmed. */
export function trackerClock(claim: ClaimedRequest, deadline: RequestDeadline, kindLabels: Record<TimelineEventKind, string>): ClockData {
  const analysis = deadline.analysis;
  const chip = deadlineChipSpec(deadline).label;
  const events: ClockEvent[] = claim.events.map((event) => ({
    key: event.id,
    label: kindLabels[event.kind],
    at: event.occurredAt,
    shape: 'circle',
    hollow: event.basis === 'USER_ENTERED',
    basisLabel: event.basis === 'EMAIL_CONFIRMED' ? 'confirmed from an email' : 'entered by you',
    direct: TRACKER_DIRECT.has(event.kind),
    detail: event.evidence?.matchedSentence ?? null,
  }));
  const suggestions = deadline.pendingSuggestionCount;
  const sources: ChartSources = {
    tables: ['tracker.json (local)'],
    note: [
      analysis ? `deadline modelled under ${analysis.statuteCitation} by the statutory engine` : null,
      suggestions > 0 ? `${plural(suggestions, 'suggested date')} not counted` : null,
    ].filter(Boolean).join('; ') || undefined,
  };
  const base = { model: 'tracker' as const, events, anomalies: [], sources, uncountedWord: 'provisional' as const };
  const noClockKinds = deadline.countdown.kind === 'NO_SUBMISSION_DATE' || deadline.countdown.kind === 'NO_DEADLINE_MODEL' || deadline.countdown.kind === 'UNAVAILABLE';
  if (noClockKinds || !analysis) {
    return {
      ...base,
      createdAt: claim.claimedAt,
      observedAt: claim.claimedAt,
      deadline: null,
      determination: null,
      end: { at: claim.claimedAt, openLabel: null },
      counted: null,
      noClock: `No clock is drawn: ${NO_CLOCK_REASON[deadline.countdown.kind] ?? 'the deadline engine returned no dated analysis'}.`,
      headline: chip,
      summary: `${chip}. No clock is drawn.`,
    };
  }
  const determination = engineDetermination(analysis);
  const days = deadline.countdown.days ?? 0;
  const allowed = analysis.allowedCalendarDays;
  const basisWords = determination?.basis === 'SOURCED' ? 'confirmed from an email' : 'your entry, a proxy';
  let headline: string;
  if (deadline.violationAsserted) {
    headline = `Determination recorded ${analysis.calendarDaysElapsed} calendar days after submission; ${allowed} allowed under ${analysis.statuteCitation}. Violation counted (sourced determination).`;
  } else if (deadline.countdown.kind === 'DETERMINATION_RECORDED') {
    headline = `Determination recorded on day ${analysis.calendarDaysElapsed} of ${allowed} (${basisWords}).`;
  } else if (deadline.countdown.kind === 'DAYS_REMAINING') {
    headline = `${plural(days, 'day')} left of ${allowed}; no determination recorded yet (provisional).`;
  } else {
    headline = `Deadline passed ${plural(days, 'day')} ago with no determination recorded (provisional, not counted).`;
  }
  return {
    ...base,
    createdAt: analysis.createdAt,
    observedAt: analysis.observedAt,
    deadline: engineDeadline(analysis),
    determination,
    end: determination ? { at: determination.at, openLabel: null } : { at: analysis.observedAt, openLabel: 'no determination recorded' },
    counted: deadline.violationAsserted
      ? { varianceDays: analysis.calendarDaysElapsed - allowed, citation: analysis.statuteCitation, reviewStatus: 'ASSERTED', measuredTo: null }
      : null,
    anomalies: anomalySentences(analysis.anomalyMarkers),
    noClock: null,
    headline,
    summary: `${headline} Submitted ${shortDate(analysis.createdAt)}; effective deadline ${shortDate(analysis.effectiveDeadlineAt)}; ${plural(events.length, 'recorded date')}.`,
  };
}

/** The stored finding that states this request's deadline result; a withdrawn one never wins over a live one. */
export function countedDeadlineFinding(findings: readonly LegalComplianceFinding[]): LegalComplianceFinding | null {
  const withVariance = findings.filter((finding) => finding.varianceDays !== null);
  const live = withVariance.filter((finding) => LIVE_REVIEW.has(finding.reviewStatus));
  const pool = live.length > 0 ? live : withVariance;
  return pool.reduce<LegalComplianceFinding | null>((best, finding) => (best === null || (finding.varianceDays ?? 0) > (best.varianceDays ?? 0) ? finding : best), null);
}

type RecordDates = Pick<RequestSearchResult, 'createdAt' | 'closedAt' | 'status'>;

/** The recorded dates the engine read evaluated from, for a surface that never loaded the request row. */
export function recordFromEngine(engine: PortalRequestDeadline | null): RecordDates | null {
  if (!engine || typeof engine.status !== 'string' || engine.closedAt === undefined) return null;
  return { createdAt: engine.createdAt, closedAt: engine.closedAt, status: engine.status };
}

function findingClause(finding: LegalComplianceFinding): string {
  const days = plural(finding.varianceDays ?? 0, 'day');
  return LIVE_REVIEW.has(finding.reviewStatus)
    ? `${finding.statuteCitation}: counted, exceeded by ${days} (${REVIEW_WORDS[finding.reviewStatus]}).`
    : `${finding.statuteCitation}: a finding of ${days} over was ${REVIEW_WORDS[finding.reviewStatus]}; not counted.`;
}

export interface PortalClockInput {
  /** The request's recorded dates; null when this surface did not load them. */
  record: RecordDates | null;
  extension: StatutoryExtensionRecord | null;
  /** The engine's analysis (phase 2); null when the read command is unavailable. */
  engine: DeadlineAnalysis | null;
  findings: readonly LegalComplianceFinding[] | null;
  observedAt: string;
}

/**
 * A portal request's clock. With the engine's analysis it draws the full deadline (phase 2);
 * without it, the recorded dates only (phase 1), and no deadline line at all.
 */
export function portalClock({ record, extension, engine, findings, observedAt }: PortalClockInput): ClockData {
  const finding = findings ? countedDeadlineFinding(findings) : null;
  const counted: ClockCounted | null = finding
    ? { varianceDays: finding.varianceDays, citation: finding.statuteCitation, reviewStatus: finding.reviewStatus, measuredTo: engine ? null : record?.closedAt ?? (finding.evaluatedAt || null) }
    : null;
  const createdAt = engine?.createdAt ?? record?.createdAt ?? null;
  const tables = ['requests', 'statutory_extensions', 'statutory_violations'];
  const findingNote = findings === null ? 'stored findings could not be read, so no counted result is shown' : undefined;
  const sources: ChartSources = { tables, note: [engine ? `deadline from the statute engine (${engine.statuteCitation})` : 'recorded dates only; no deadline drawn', findingNote].filter(Boolean).join('; ') };
  const base = { model: engine ? 'portal-engine' as const : 'portal-recorded' as const, counted, anomalies: engine ? anomalySentences(engine.anomalyMarkers) : [], sources, uncountedWord: 'not counted' as const };
  if (!createdAt) {
    const stated = finding && finding.varianceDays !== null ? ` ${findingClause(finding)}` : '';
    const sentence = `Open this request in Evidence to see its recorded dates and clock.${stated}`;
    return { ...base, createdAt: observedAt, observedAt, deadline: null, determination: null, end: { at: observedAt, openLabel: null }, events: [], noClock: sentence, headline: `No clock: this request's dates are not loaded here.${stated}`, summary: sentence };
  }
  // Without the request row, closure is unknown here, not absent from the record.
  const closureLoaded = record !== null;
  const closedAt = record?.closedAt ?? null;
  const open = (record?.status ?? '').toLowerCase() !== 'closed';
  const events: ClockEvent[] = [
    { key: 'created', label: 'Created', at: createdAt, shape: 'circle', hollow: false, basisLabel: 'recorded by the portal', direct: true },
    ...(closedAt ? [{ key: 'closed', label: 'Closed', at: closedAt, shape: 'circle' as const, hollow: false, basisLabel: 'recorded by the portal', direct: true }] : []),
    ...(extension ? [
      { key: 'extension', label: 'Extension notice', at: extension.noticeAt, shape: 'circle' as const, hollow: false, basisLabel: `recorded extension (${extension.noticeSource})`, direct: true },
      { key: 'expected', label: 'Expected determination', at: extension.expectedDeterminationAt, shape: 'diamond' as const, hollow: true, basisLabel: 'date the extension notice promised', direct: false },
    ] : []),
  ];
  const elapsed = closedAt ? calendarDaysBetween(createdAt, closedAt) : null;
  let headline = closedAt
    ? `Created ${shortDate(createdAt)}; closed ${shortDate(closedAt)} after ${plural(elapsed ?? 0, 'calendar day')}.`
    : closureLoaded
      ? `Created ${shortDate(createdAt)}; no closing date recorded.`
      : `Created ${shortDate(createdAt)}; closing date not loaded here.`;
  if (finding && finding.varianceDays !== null) headline += ` ${findingClause(finding)}`;
  const determination = engine ? engineDetermination(engine) : null;
  if (engine) {
    headline += !determination
      ? ` No determination recorded by day ${engine.calendarDaysElapsed}; ${engine.allowedCalendarDays} allowed.`
      : determination.basis === 'PROXY'
        ? ` Earliest recorded activity (a proxy for the determination) on day ${engine.calendarDaysElapsed}; ${engine.allowedCalendarDays} allowed.`
        : ` Determination on day ${engine.calendarDaysElapsed}; ${engine.allowedCalendarDays} allowed.`;
  }
  const endAt = engine
    ? determination?.at ?? engine.observedAt
    : closedAt ?? observedAt;
  const openLabel = engine
    ? determination ? null : 'no determination recorded'
    : closedAt ? null : !closureLoaded ? 'closing date not loaded here' : open ? 'open' : 'closing date not recorded';
  return {
    ...base,
    createdAt,
    observedAt: engine?.observedAt ?? observedAt,
    deadline: engine ? engineDeadline(engine) : null,
    determination,
    end: { at: endAt, openLabel },
    events,
    noClock: null,
    headline,
    summary: `${headline} ${plural(events.length, 'recorded date')}${extension ? ', including an extension notice' : ''}.`,
  };
}

/** A casework request's modelled clock. Its past-due span is modelled and can never be counted. */
export function caseworkClock(plan: RequestFilingPlan, productions: readonly { id: string; receivedAt: string }[]): ClockData {
  const measuredTo = plan.measuredTo;
  const headline = plan.deadlinePassedWithoutRecordedResponse
    ? `Modelled: past due, no recorded response; ${plan.calendarDaysElapsed} of ${plan.allowedCalendarDays} allowed calendar days elapsed.`
    : `${plan.calendarDaysElapsed} of ${plan.allowedCalendarDays} allowed calendar days elapsed (modelled).`;
  return {
    model: 'casework',
    createdAt: plan.filedOn,
    observedAt: plan.evaluatedAt,
    deadline: { initialAt: plan.determinationDueOn, effectiveAt: plan.determinationDueOn, allowedDays: plan.allowedCalendarDays, citation: plan.statuteCitation, extensionReason: null },
    determination: null,
    end: measuredTo ? { at: measuredTo, openLabel: null } : { at: plan.evaluatedAt, openLabel: 'no recorded response' },
    events: productions.filter((p) => p.receivedAt).map((p) => ({ key: p.id, label: 'Production received', at: p.receivedAt, shape: 'diamond', hollow: false, basisLabel: 'entered in casework', direct: false })),
    anomalies: [],
    noClock: null,
    headline,
    summary: headline,
    sources: { tables: ['casework_cases', 'casework_events'], note: `clock modelled under ${plan.statuteCitation} as of ${plan.evaluatedAt}` },
    uncountedWord: 'modelled',
  };
}

// ---- Mark ------------------------------------------------------------------------------------

/**
 * The elapsed bar of one clock: ink from day 0 to the deadline, then the span past it, crimson
 * only when counted, struck-through outline when the counted finding was withdrawn, otherwise an
 * ink dashed outline. Crimson stops at `countedEndDay` (deadline + the stored variance): days after
 * the measure was taken are drawn as the uncounted outline. An open end stops in a chevron.
 */
export function ClockBar({ x, y, thickness, endDay, deadlineDay, countedEndDay = null, past, open }: {
  x: ContinuousScale;
  y: number;
  thickness: number;
  endDay: number;
  deadlineDay: number | null;
  countedEndDay?: number | null;
  past: PastDeadlinePaint;
  open: boolean;
}) {
  const x0 = x(0);
  const xe = x(Math.max(0, endDay));
  const split = deadlineDay !== null && endDay > deadlineDay ? x(Math.max(0, deadlineDay)) : null;
  const ink = paintFor('descriptive', 'fill', 'primary').fill;
  const chevron = open ? (
    <path
      d={`M${xe + 3},${y - 1}L${xe + 8},${y + thickness / 2}L${xe + 3},${y + thickness + 1}`}
      style={{ fill: 'none', stroke: VIZ.ink, strokeWidth: 1.5, strokeLinecap: 'round', strokeLinejoin: 'round' }}
    />
  ) : null;
  if (split === null) {
    const d = open || xe - x0 < 1 ? `M${x0},${y}H${Math.max(x0 + 1, xe)}V${y + thickness}H${x0}Z` : hBarPath(x0, xe, y, thickness);
    return <g data-clock-bar="">{<path d={d} style={{ fill: ink }} />}{chevron}</g>;
  }
  const inside = `M${x0},${y}H${split - 1}V${y + thickness}H${x0}Z`;
  const pastStart = split + 1;
  const outline = paintFor('descriptive', 'outline', 'primary');
  const outlineSpan = (kind: PastDeadlinePaint, from: number) => (
    <g data-past={kind}>
      <rect x={from + 0.75} y={y + 0.75} width={Math.max(0, xe - from - 1.5)} height={thickness - 1.5} rx={1} style={{ fill: 'none', stroke: outline.stroke, strokeWidth: outline.strokeWidth, strokeDasharray: kind === 'uncounted' ? '3 2' : undefined }} />
      {kind !== 'uncounted' && <line x1={from} x2={xe} y1={y + thickness / 2} y2={y + thickness / 2} style={{ stroke: VIZ.ink, strokeWidth: 1.5 }} />}
    </g>
  );
  let pastNode: ReactNode;
  if (past === 'counted') {
    const paint = paintFor('counted', 'fill');
    const xc = countedEndDay === null ? xe : Math.min(xe, Math.max(pastStart, x(countedEndDay)));
    const tail = xe - xc > 1;
    pastNode = (
      <>
        <path data-past="counted" d={open || tail ? `M${pastStart},${y}H${xc}V${y + thickness}H${pastStart}Z` : hBarPath(pastStart, xe, y, thickness)} style={{ fill: paint.fill }} />
        {tail && outlineSpan('uncounted', xc + 1)}
      </>
    );
  } else {
    pastNode = outlineSpan(past, pastStart);
  }
  return (
    <g data-clock-bar="">
      <path d={inside} style={{ fill: ink }} />
      {pastNode}
      {chevron}
    </g>
  );
}

// ---- Figure ----------------------------------------------------------------------------------

const LABEL_Y = 10;
const LANE_Y = 32;
const BAR_Y = 50;
const BAR_H = 6;
const RULE_TOP = 20;
const RULE_BOTTOM = 62;
const NOTE_Y = 74;
const END_Y = 89;
const AXIS_Y = 98;
const HEIGHT = 118;

interface Plotted {
  created: number;
  endDay: number;
  observedDay: number;
  initialDay: number | null;
  effectiveDay: number | null;
  /** Where the past-deadline span starts: the engine's deadline, or closure − stored variance. */
  splitDay: number | null;
  /** Where the crimson stops: the deadline plus the stored variance; null when not counted. */
  countedEndDay: number | null;
  showToday: boolean;
  domain: [number, number];
}

/**
 * Day the past-deadline span starts: the engine's effective deadline, or, with recorded dates only,
 * the stored variance counted back from the instant it was measured to (never from today, which
 * would move a counted claim later every day the request stays open).
 */
export function countedSpanStartDay(data: ClockData): number | null {
  const created = parseDate(data.createdAt);
  if (data.deadline) return dayOf(data.deadline.effectiveAt, created);
  if (data.model === 'casework' || !data.counted) return null;
  const { varianceDays, measuredTo } = data.counted;
  return varianceDays !== null && varianceDays > 0 && measuredTo ? dayOf(measuredTo, created) - varianceDays : null;
}

function plot(data: ClockData): Plotted {
  const created = parseDate(data.createdAt);
  const endDay = dayOf(data.end.at, created);
  const observedDay = dayOf(data.observedAt, created);
  const initialDay = data.deadline ? dayOf(data.deadline.initialAt, created) : null;
  const effectiveDay = data.deadline ? dayOf(data.deadline.effectiveAt, created) : null;
  // Phase 1 has no deadline line: a stored variance places the counted span back from closure.
  const splitDay = countedSpanStartDay(data);
  const eventDays = data.events.map((event) => dayOf(event.at, created));
  const lo = Math.min(0, ...eventDays);
  // Today matters only to a clock still running; a closed one is scaled to its own span.
  const showToday = data.end.openLabel !== null;
  const hi = Math.max(1, endDay, showToday ? observedDay : 0, effectiveDay ?? 0, ...eventDays);
  const variance = data.model !== 'casework' ? data.counted?.varianceDays ?? null : null;
  const countedEndDay = pastDeadlinePaint(data) === 'counted' && splitDay !== null && variance !== null ? splitDay + variance : null;
  return { created, endDay, observedDay, initialDay, effectiveDay, splitDay, countedEndDay, showToday, domain: [lo, hi + (hi - lo) * 0.1] };
}

/** Days drawn past the counted span (after the measure was taken); 0 when none are. */
function uncountedTailDays(p: Plotted): number {
  return p.countedEndDay !== null && p.splitDay !== null && p.endDay > p.splitDay ? Math.max(0, p.endDay - Math.max(p.splitDay, p.countedEndDay)) : 0;
}

function ClockPlot({ data, width, highlightedEvent, onEventHover, onEventActivate }: {
  data: ClockData;
  width: number;
  highlightedEvent?: string | null;
  onEventHover?(key: string | null): void;
  onEventActivate?(key: string): void;
}) {
  const tooltip = useTooltip();
  const p = plot(data);
  const x = scaleLinear(p.domain, [16, width - 16], { nice: false, zero: true });
  const ordered = [...data.events].sort((a, b) => parseDate(a.at) - parseDate(b.at) || (a.key < b.key ? -1 : 1));
  const roving = useRovingFocus({
    shape: { rows: Math.max(1, ordered.length) },
    onActivate: (i) => ordered[i] && onEventActivate?.(ordered[i].key),
    onEscape: tooltip.hide,
  });
  const past = pastDeadlinePaint(data);
  const tip = (event: ClockEvent) => (
    <TooltipContent
      title={event.label}
      rows={[{ key: 'at', value: formatDate(parseDate(event.at), 'hour'), label: event.basisLabel }]}
      note={event.detail ?? undefined}
    />
  );
  const eventName = (event: ClockEvent) => `${event.label} on ${shortDate(event.at)}, ${event.basisLabel}.`;

  const directLabels = placeLabels(
    ordered.filter((event) => event.direct).map((event) => ({ key: event.key, anchor: x(dayOf(event.at, p.created)), width: measureText(event.label) })),
    0,
    width,
    10,
  );
  const directText = new Map(ordered.map((event) => [event.key, event.label]));
  const directAnchor = new Map(ordered.map((event) => [event.key, x(dayOf(event.at, p.created))]));

  // Rule and band labels share one row, pushed apart so none overlaps.
  const notes: { key: string; anchor: number; text: string }[] = [];
  if (data.deadline && p.initialDay !== null) notes.push({ key: 'initial', anchor: x(p.initialDay), text: `day ${Math.round(p.initialDay)} deadline` });
  if (data.deadline?.extensionReason && p.effectiveDay !== null && p.initialDay !== null && p.effectiveDay > p.initialDay) {
    notes.push({ key: 'effective', anchor: x(p.effectiveDay), text: `extended to day ${data.deadline.allowedDays}` });
  }
  if (p.showToday) notes.push({ key: 'today', anchor: x(p.observedDay), text: 'today' });
  const placedNotes = placeLabels(notes.map((note) => ({ key: note.key, anchor: note.anchor, width: measureText(note.text) })), 0, width);

  const variance = data.model !== 'casework' ? data.counted?.varianceDays ?? null : null;
  let endWords: string | null = null;
  if (p.splitDay !== null && p.endDay > p.splitDay) {
    endWords = past === 'counted'
      ? `Counted: exceeded by ${plural(variance ?? Math.round(p.endDay - p.splitDay), 'day')}`
      : past === 'dismissed'
        ? `exceeded by ${plural(variance ?? 0, 'day')} (finding dismissed)`
        : past === 'resolved'
          ? `exceeded by ${plural(variance ?? 0, 'day')} (resolved by new evidence: not counted)`
          : `past the deadline (${data.uncountedWord})`;
  } else if (data.end.openLabel) {
    endWords = data.end.openLabel;
  } else if (data.determination) {
    endWords = `determination day ${data.determination.elapsedDays ?? Math.round(p.endDay)} (${data.determination.basis === 'SOURCED' ? 'sourced' : 'proxy'})`;
  }
  const flagged = past === 'counted' && endWords !== null && endWords.startsWith('Counted');
  const endWidth = endWords ? measureText(endWords, flagged ? `600 ${LABEL_FONT}` : LABEL_FONT) + 6 : 0;
  const endX = Math.min(width - endWidth - 2, Math.max(flagged ? 16 : 2, x(p.endDay) - endWidth / 2));
  const rule = REFERENCE_PAINT.statute;

  return (
    <g>
      {data.deadline && p.initialDay !== null && p.effectiveDay !== null && p.effectiveDay > p.initialDay && (
        <ReferenceBand axis="x" from={x(p.initialDay)} to={x(p.effectiveDay)} extent={[RULE_TOP, RULE_BOTTOM]} label="" />
      )}
      {data.deadline && p.initialDay !== null && (
        <line x1={Math.round(x(p.initialDay)) + 0.5} x2={Math.round(x(p.initialDay)) + 0.5} y1={RULE_TOP} y2={RULE_BOTTOM} data-reference="statute" style={{ stroke: rule.stroke, strokeWidth: rule.strokeWidth, shapeRendering: 'crispEdges' }} />
      )}
      {data.deadline && p.effectiveDay !== null && p.initialDay !== null && p.effectiveDay > p.initialDay && (
        <line x1={Math.round(x(p.effectiveDay)) + 0.5} x2={Math.round(x(p.effectiveDay)) + 0.5} y1={RULE_TOP} y2={RULE_BOTTOM} data-reference="statute" style={{ stroke: rule.stroke, strokeWidth: rule.strokeWidth, shapeRendering: 'crispEdges' }} />
      )}
      {p.showToday && <line x1={Math.round(x(p.observedDay)) + 0.5} x2={Math.round(x(p.observedDay)) + 0.5} y1={RULE_TOP} y2={RULE_BOTTOM} data-reference="today" style={{ stroke: REFERENCE_PAINT.today.stroke, strokeWidth: REFERENCE_PAINT.today.strokeWidth, shapeRendering: 'crispEdges' }} />}
      <ClockBar x={x} y={BAR_Y} thickness={BAR_H} endDay={p.endDay} deadlineDay={p.splitDay} countedEndDay={p.countedEndDay} past={past} open={data.end.openLabel !== null} />
      {data.determination && (
        <Dot
          cx={x(dayOf(data.determination.at, p.created))}
          cy={BAR_Y + BAR_H / 2}
          shape={data.determination.basis === 'SOURCED' ? 'diamond' : 'triangle'}
          role={data.determination.basis === 'SOURCED' && (p.splitDay === null || p.endDay <= p.splitDay) ? 'positive' : 'primary'}
          r={5}
          title={`Determination ${data.determination.basis === 'SOURCED' ? 'confirmed from a source' : 'inferred from the earliest recorded activity (proxy)'} on ${shortDate(data.determination.at)}`}
        />
      )}
      <TimelineLane
        y={LANE_Y}
        x={(ms) => x((ms - p.created) / DAY)}
        events={ordered.map((event, index) => ({
          key: event.key,
          at: parseDate(event.at),
          shape: event.shape,
          hollow: event.hollow,
          role: 'primary',
          name: eventName(event),
          interaction: {
            name: eventName(event),
            item: roving.getItemProps(index),
            onPointerMove: (pointer) => {
              tooltip.show(tip(event), { clientX: pointer.clientX, clientY: pointer.clientY });
              onEventHover?.(event.key);
            },
            onPointerLeave: () => {
              tooltip.hide();
              onEventHover?.(null);
            },
            onFocusMark: (box) => {
              tooltip.show(tip(event), box);
              onEventHover?.(event.key);
            },
            onBlurMark: () => {
              tooltip.hide();
              onEventHover?.(null);
            },
            onClick: () => onEventActivate?.(event.key),
          },
        }))}
      />
      {highlightedEvent && ordered.some((event) => event.key === highlightedEvent) && (
        <circle
          data-highlighted={highlightedEvent}
          cx={x(dayOf(ordered.find((event) => event.key === highlightedEvent)!.at, p.created))}
          cy={LANE_Y}
          r={8}
          pointerEvents="none"
          style={{ fill: 'none', stroke: VIZ.focus, strokeWidth: 2 }}
        />
      )}
      {directLabels.map((label) => {
        const anchor = directAnchor.get(label.key) ?? label.pos;
        const centre = label.pos + measureText(directText.get(label.key) ?? '') / 2;
        return (
          <g key={label.key}>
            {label.leader && <line data-leader="" x1={centre} y1={LABEL_Y + 6} x2={anchor} y2={LANE_Y - 6} style={{ stroke: VIZ.baseline, strokeWidth: 1 }} />}
            <text x={label.pos} y={LABEL_Y} dy="0.32em" className={MUTED_LABEL_CLASS}>{directText.get(label.key)}</text>
          </g>
        );
      })}
      {placedNotes.map((label, index) => (
        <text key={label.key} x={label.pos} y={NOTE_Y} dy="0.32em" className="fill-fg-secondary text-[11px]">{notes[index].text}</text>
      ))}
      {endWords && flagged && (
        <path data-flag="" d={`M${endX - 12},${END_Y + 5}V${END_Y - 5}h8l-2 2.5 2 2.5H${endX - 10.5}`} style={{ fill: VIZ.counted, stroke: VIZ.counted, strokeWidth: 1, strokeLinejoin: 'round' }} />
      )}
      {endWords && (
        <text x={endX} y={END_Y} dy="0.32em" className={flagged ? 'fill-fg text-[11px] font-semibold' : 'fill-fg-secondary text-[11px]'}>
          {endWords}
        </text>
      )}
      <Axis scale={x} kind="days" orient="bottom" offset={AXIS_Y} />
      {tooltip.node}
    </g>
  );
}

const PAST_LEGEND: Record<Exclude<PastDeadlinePaint, 'counted'>, (word: string) => string> = {
  dismissed: () => 'Past the deadline, finding dismissed',
  resolved: () => 'Past the deadline, finding resolved by new evidence (not counted)',
  uncounted: (word) => `Past the deadline (${word})`,
};

function legendFor(data: ClockData, past: PastDeadlinePaint, crossesDeadline: boolean, tail: boolean, today: boolean): LegendItem[] {
  const items: LegendItem[] = [{ key: 'elapsed', label: 'Days elapsed', swatch: 'rect', tone: 'ink' }];
  if (data.deadline) items.push({ key: 'deadline', label: `Deadline (${data.deadline.citation})`, swatch: 'line', tone: 'ink' });
  if (data.deadline?.extensionReason) items.push({ key: 'extension', label: `Extension (${data.deadline.extensionReason.toLowerCase().replace(/_/g, ' ')})`, swatch: 'rect', tone: 'track' });
  if (crossesDeadline && past === 'counted') items.push({ key: 'counted', label: 'Counted violation', swatch: 'flag', tone: 'counted' });
  if (crossesDeadline && past !== 'counted') items.push({ key: 'past', label: PAST_LEGEND[past](data.uncountedWord), swatch: past === 'uncounted' ? 'dashed-line' : 'line', tone: 'ink' });
  if (crossesDeadline && past === 'counted' && tail) items.push({ key: 'tail', label: 'Past the deadline (not counted)', swatch: 'dashed-line', tone: 'ink' });
  if (data.determination?.basis === 'SOURCED') items.push({ key: 'sourced', label: 'Determination (sourced)', swatch: 'diamond', tone: crossesDeadline ? 'ink' : 'positive' });
  if (data.determination?.basis === 'PROXY') items.push({ key: 'proxy', label: 'Earliest activity (proxy)', swatch: 'triangle', tone: 'ink' });
  if (today) items.push({ key: 'today', label: 'Today', swatch: 'line', tone: 'mark' });
  if (data.events.some((event) => !event.hollow)) items.push({ key: 'filled', label: data.model === 'tracker' ? 'Confirmed from an email' : 'Recorded date', swatch: 'dot', tone: 'ink' });
  if (data.events.some((event) => event.hollow && event.shape !== 'diamond')) items.push({ key: 'hollow', label: data.model === 'tracker' ? 'Entered by you' : 'Promised date', swatch: 'hollow', tone: 'ink' });
  if (data.events.some((event) => event.hollow && event.shape === 'diamond')) items.push({ key: 'promised', label: 'Promised date', swatch: 'hollow-diamond', tone: 'ink' });
  return items;
}

export default function ClockStrip({ data, title = 'Statutory clock', storageKey, highlightedEvent, onEventHover, onEventActivate, status = 'ready', error }: {
  data: ClockData;
  title?: string;
  /** Distinct per surface, so each remembers its own chart/table choice. */
  storageKey: string;
  highlightedEvent?: string | null;
  onEventHover?(key: string | null): void;
  onEventActivate?(key: string): void;
  status?: 'ready' | 'loading' | 'error';
  error?: string | null;
}) {
  const past = pastDeadlinePaint(data);
  const p = data.noClock ? null : plot(data);
  const crosses = p !== null && p.splitDay !== null && p.endDay > p.splitDay;
  const claim: Claim = crosses && past === 'counted' ? 'counted' : 'descriptive';
  const created = parseDate(data.createdAt);
  const rows: Record<string, TwinCell>[] = [...data.events]
    .sort((a, b) => parseDate(a.at) - parseDate(b.at))
    .map((event) => ({ what: event.label, day: Math.round(dayOf(event.at, created) * 10) / 10, past: null, date: formatDate(parseDate(event.at), 'hour'), basis: event.basisLabel }));
  if (data.deadline) {
    rows.push({ what: `Initial deadline (${data.deadline.citation})`, day: Math.round(dayOf(data.deadline.initialAt, created)), past: null, date: shortDate(data.deadline.initialAt), basis: 'statute engine' });
    if (data.deadline.effectiveAt !== data.deadline.initialAt) rows.push({ what: 'Effective deadline after extension', day: data.deadline.allowedDays, past: null, date: shortDate(data.deadline.effectiveAt), basis: 'statute engine' });
  }
  if (data.model !== 'casework' && data.counted) {
    const { varianceDays, reviewStatus } = data.counted;
    rows.push({
      what: `Stored finding: ${data.counted.citation}`,
      day: null,
      past: varianceDays ?? { state: 'not-recorded', reason: 'the finding stored no variance' },
      date: REVIEW_WORDS[reviewStatus],
      basis: LIVE_REVIEW.has(reviewStatus) ? 'stored finding (counted measure)' : 'stored finding, withdrawn (not counted)',
    });
  }
  const frameStatus = status !== 'ready' ? status : data.noClock ? 'empty' : 'ready';
  return (
    <div className="grid min-w-0 gap-2">
      <ChartFrame
        title={title}
        headline={status === 'loading' ? null : data.headline}
        claim={claim}
        sources={data.sources}
        summary={data.summary}
        status={frameStatus}
        error={error}
        emptyText={data.noClock ?? undefined}
        table={{
          caption: `${title}: recorded dates`,
          columns: [{ key: 'what', label: 'Date' }, { key: 'day', label: 'Day since submission', align: 'end' }, { key: 'past', label: 'Days past deadline', align: 'end' }, { key: 'date', label: 'When' }, { key: 'basis', label: 'Basis' }],
          rows,
        }}
        legend={p ? legendFor(data, past, crosses, uncountedTailDays(p) > 0, p.showToday) : undefined}
        minHeight={data.noClock ? 64 : HEIGHT}
        interactive={data.events.length > 0 ? { roledescription: 'statutory clock' } : undefined}
        storageKey={storageKey}
      >
        {({ width }) => (
          <ClockPlot data={data} width={width} highlightedEvent={highlightedEvent} onEventHover={onEventHover} onEventActivate={onEventActivate} />
        )}
      </ChartFrame>
      {data.anomalies.length > 0 && (
        <ul className="grid gap-1 text-caption text-warning" aria-label="Date anomalies">
          {data.anomalies.map((anomaly) => (
            <li key={anomaly} className="flex items-start gap-1.5">
              <svg width={10} height={10} viewBox="0 0 10 10" aria-hidden="true" className="mt-[3px] shrink-0"><path d="M5 1L9 9H1Z" style={{ fill: 'none', stroke: VIZ.ink, strokeWidth: 1.2 }} /></svg>
              {anomaly}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

/** Day arithmetic shared with the clock stack: whole days between a start and an instant. */
export function daysSince(fromAt: string, at: string): number {
  return dayOf(at, parseDate(fromAt));
}
