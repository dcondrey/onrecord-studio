import type { CaseworkCase, CaseworkCitation } from '../types/casework';
import type { TimelineCitation, TimelineItem } from './timeline';

/**
 * Case records on the timeline: every event is a date the operator recorded in casework or a
 * date the statutory engine modeled from one, each carrying the citations it rests on and their
 * verification rung. No date here is extracted from a document: the span primitive the workbench
 * plan names does not exist yet, so an event with no citation says so rather than borrowing one.
 */

export type LaneKind = 'CLOCK' | 'HYPOTHESIS';

export interface TimelineLane {
  id: string;
  kind: LaneKind;
  label: string;
}

/** One date reading of a subject, and the basis that gives it. */
export interface DateReading {
  date: string;
  basis: string;
}

/** Two recorded sources give the same subject different dates. Both are shown; neither is chosen. */
export interface DateContradiction {
  id: string;
  subject: string;
  readings: DateReading[];
}

export interface CaseTimeline {
  items: TimelineItem[];
  lanes: TimelineLane[];
  contradictions: DateContradiction[];
}

const NO_DATE = '';

function cited(ids: string[], evidence: CaseworkCitation[]): TimelineCitation[] {
  return ids.flatMap((id) => {
    const cite = evidence.find((candidate) => candidate.id === id);
    return cite ? [{ label: cite.label, grade: cite.verification }] : [];
  });
}

function basisOf(citations: TimelineCitation[]): string {
  if (citations.length === 0) return 'No citation recorded for this date.';
  return citations.map((cite) => `${cite.label} (${cite.grade})`).join('; ');
}

function normalise(text: string): string {
  return text.trim().toLowerCase().replace(/\s+/g, ' ');
}

export function caseItems(cases: CaseworkCase[]): CaseTimeline {
  const items: TimelineItem[] = [];
  const lanes: TimelineLane[] = [];
  const contradictions: DateContradiction[] = [];

  for (const record of cases) {
    const hypotheses = record.questions
      .filter((question) => question.hypothesis.trim())
      .map((question) => ({
        id: `hypothesis:${record.id}:${question.id}`,
        cites: new Set([...question.citationIds, ...question.counterCitationIds]),
      }));
    for (const question of record.questions) {
      if (question.hypothesis.trim()) lanes.push({ id: `hypothesis:${record.id}:${question.id}`, kind: 'HYPOTHESIS', label: `Hypothesis: ${question.hypothesis.trim()}` });
    }
    // A hypothesis lane holds the events whose citations the question names as support or
    // counter-evidence; two questions over the same records lay out competing readings of them.
    const laneIds = (citationIds: string[], ...fixed: string[]) => [
      ...fixed,
      ...hypotheses.filter((lane) => citationIds.some((id) => lane.cites.has(id))).map((lane) => lane.id),
    ];
    const push = (item: Omit<TimelineItem, 'source' | 'provenance' | 'citations'> & { citationIds: string[]; fixedLanes?: string[] }) => {
      const { citationIds, fixedLanes = [], ...rest } = item;
      if (rest.at === NO_DATE) return;
      const citations = cited(citationIds, record.evidence);
      items.push({ ...rest, source: 'CASE', provenance: `${record.title}: ${basisOf(citations)}`, citations, lanes: laneIds(citationIds, ...fixedLanes) });
    };

    for (const request of record.requests) {
      const clock = `clock:${record.id}:${request.id}`;
      lanes.push({ id: clock, kind: 'CLOCK', label: `Statutory clock: ${request.title}` });
      push({ id: `${clock}:filed`, at: request.submittedAt ?? NO_DATE, dateMeaning: 'OCCURRED', title: `Request filed: ${request.title}`, detail: `To ${request.custodian}. Date entered by you; the app sends nothing.`, evidence: null, findingId: null, citationIds: request.citationIds, fixedLanes: [clock] });
      if (request.responseDue) {
        push({ id: `${clock}:recorded-due`, at: request.responseDue, dateMeaning: 'DUE', title: `Recorded response due: ${request.title}`, detail: 'Entered by you from the agency response or your own count.', evidence: null, findingId: null, citationIds: [], fixedLanes: [clock] });
      }
      const plan = request.filingPlan;
      if (plan) {
        push({ id: `${clock}:determination-due`, at: plan.determinationDueOn, dateMeaning: 'DUE', title: `Determination due: ${request.title}`, detail: `Modeled under ${plan.statuteCitation}: ${plan.calendarDaysElapsed} of ${plan.allowedCalendarDays} allowed calendar days elapsed as of ${plan.evaluatedAt}.`, evidence: null, findingId: null, citationIds: [], fixedLanes: [clock] });
        for (const [index, followUp] of plan.followUps.entries()) {
          push({ id: `${clock}:follow-up:${index}`, at: followUp.scheduledFor, dateMeaning: 'DUE', title: `Follow-up (${followUp.trigger.replace(/_/g, ' ').toLowerCase()}): ${request.title}`, detail: followUp.rationale, evidence: null, findingId: null, citationIds: [], fixedLanes: [clock] });
        }
        if (request.responseDue && request.responseDue !== plan.determinationDueOn) {
          contradictions.push({
            id: `${clock}:due`,
            subject: `Response due date for ${request.title}`,
            readings: [
              { date: request.responseDue, basis: 'Recorded response due date, entered by you.' },
              { date: plan.determinationDueOn, basis: `Modeled determination date under ${plan.statuteCitation} from the recorded filing date ${plan.filedOn}.` },
            ],
          });
        }
      }
      for (const production of request.productions) {
        push({ id: `${clock}:production:${production.id}`, at: production.receivedAt, dateMeaning: 'OCCURRED', title: `Production received: ${request.title}`, detail: production.description || null, evidence: null, findingId: null, citationIds: production.citationIds, fixedLanes: [clock] });
      }
    }

    const decisionsBySubject = new Map<string, { subject: string; readings: DateReading[] }>();
    for (const decision of record.decisions) {
      const citations = cited(decision.citationIds, record.evidence);
      push({ id: `decision:${record.id}:${decision.id}`, at: decision.date, dateMeaning: 'OCCURRED', title: `${decision.body}: ${decision.agendaItem}`, detail: decision.proposedDecision || null, evidence: decision.outcome || null, findingId: null, citationIds: decision.citationIds });
      if (!decision.date) continue;
      const key = `${normalise(decision.body)}|${normalise(decision.agendaItem)}`;
      const held = decisionsBySubject.get(key) ?? { subject: `${decision.body}: ${decision.agendaItem}`, readings: [] };
      if (!held.readings.some((reading) => reading.date === decision.date)) held.readings.push({ date: decision.date, basis: basisOf(citations) });
      decisionsBySubject.set(key, held);
    }
    for (const [key, held] of decisionsBySubject) {
      if (held.readings.length > 1) contradictions.push({ id: `decision:${record.id}:${key}`, subject: held.subject, readings: held.readings });
    }

    for (const commitment of record.commitments) {
      push({ id: `commitment:${record.id}:${commitment.id}`, at: commitment.dueDate ?? NO_DATE, dateMeaning: 'DUE', title: `Commitment due: ${commitment.promise}`, detail: commitment.responsibleParty ? `Responsible: ${commitment.responsibleParty}` : null, evidence: null, findingId: null, citationIds: commitment.citationIds });
    }
  }

  return { items, lanes, contradictions };
}
