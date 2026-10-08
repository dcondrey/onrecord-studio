import { describe, expect, it } from 'vitest';
import { buildTimeline, findingItems, type TimelineItem, type TimelineSource } from '../utils/timeline';
import type { Finding } from '../types';

const item = (id: string, source: TimelineSource, at: string, title = id): TimelineItem => ({
  id,
  source,
  at,
  dateMeaning: source === 'REQUEST' ? 'OCCURRED' : 'DETECTED',
  title,
  detail: null,
  provenance: '',
  evidence: null,
  findingId: null,
});
const everything = new Set<TimelineSource>(['REQUEST', 'FINDING', 'WATCH']);

describe('buildTimeline', () => {
  const all = [
    item('a', 'REQUEST', '2026-03-02T10:00:00Z'),
    item('b', 'FINDING', '2026-03-31T23:59:59Z'),
    item('c', 'WATCH', '2026-04-01T00:00:00Z'),
    item('bad', 'FINDING', 'not a date'),
  ];

  it('orders newest first, includes both bound days, and counts unreadable dates instead of placing them', () => {
    const view = buildTimeline(all, { sources: everything, from: '2026-03-02', to: '2026-03-31', text: '' });
    expect(view.items.map((entry) => entry.id)).toEqual(['b', 'a']);
    expect(view.unreadableDates).toBe(1);
    expect(buildTimeline(all, { sources: everything, from: '', to: '', text: '' }).items.map((entry) => entry.id)).toEqual(['c', 'b', 'a']);
  });

  it('narrows by source and text without losing the unreadable count', () => {
    const view = buildTimeline(all, { sources: new Set(['REQUEST']), from: '', to: '', text: 'A' });
    expect(view.items.map((entry) => entry.id)).toEqual(['a']);
    expect(view.unreadableDates).toBe(0);
  });
});

describe('caseItems', () => {
  it('lists both readings of a contradicted date, lanes events by clock and hypothesis, and marks an uncited date', async () => {
    const { caseItems } = await import('../utils/caseTimeline');
    const cite = { id: 'e1', reportId: 'r', evidenceId: 'e1', label: 'Agenda', quote: '', sourceUrl: null, sourceSha256: 'a', sourceReferences: [], portalKey: null, documentId: null, characterOffset: null, verification: 'SAVED_REPORT' };
    const plan = { statuteCitation: 'Gov. Code § 7922.535', evaluatedAt: '2026-09-20', filedOn: '2026-09-01', determinationDueOn: '2026-09-11', calendarDaysElapsed: 19, allowedCalendarDays: 10, determinationBasis: 'OBSERVATION_WITHOUT_RECORDED_ACTIVITY' as const, measuredTo: null, deadlinePassedWithoutRecordedResponse: true, followUps: [] };
    const request = { id: 'q', title: 'Reports', custodian: 'Agency', gap: '', resolves: '', draft: '', state: 'SUBMITTED', expectedRecordTypes: [], expectedFields: [], expectedPeriod: '', submittedAt: '2026-09-01', responseDue: '2026-09-15', externalReference: '', productions: [], unresolvedOmissions: [], followUpDraft: '', citationIds: ['e1'], compiledDraft: '', filingPlan: plan };
    const decision = (id: string, date: string) => ({ id, body: 'Council', date, agendaItem: 'Item 4', proposedDecision: '', action: '', outcome: '', vote: '', state: 'PROPOSED', citationIds: id === 'd1' ? ['e1'] : [], outcomeCitationIds: [] });
    const built = caseItems([{ id: 'c', reportId: 'r', topic: '', title: 'Case', owner: '', disposition: '', revision: 1, createdAt: '', updatedAt: '', notes: '', evidence: [cite], requests: [request], decisions: [decision('d1', '2026-08-03'), decision('d2', '2026-08-10')], commitments: [], questions: [{ id: 'h', question: '', hypothesis: 'Delayed on purpose', nextTest: '', disposition: 'OPEN', owner: '', citationIds: ['e1'], counterCitationIds: [] }] }]);

    expect(built.contradictions.map((c) => c.readings.map((r) => r.date))).toEqual([['2026-09-15', '2026-09-11'], ['2026-08-03', '2026-08-10']]);
    expect(built.contradictions[1].readings[1].basis).toBe('No citation recorded for this date.');
    const filed = built.items.find((i) => i.id === 'clock:c:q:filed');
    expect(filed?.lanes).toEqual(['clock:c:q', 'hypothesis:c:h']);
    expect(filed?.citations).toEqual([{ label: 'Agenda', grade: 'SAVED_REPORT' }]);
    expect(built.items.find((i) => i.id === 'clock:c:q:determination-due')?.citations).toEqual([]);
    expect(buildTimeline(built.items, { sources: new Set(['CASE']), from: '', to: '', text: '', lane: 'hypothesis:c:h' }).items.map((i) => i.id)).toEqual(['clock:c:q:filed', 'decision:c:d1']);
  });

  // The timeline and the inbox read the claim basis from one source, so a counted violation is never
  // also called "not a legal claim" here.
  it('states each finding\'s claim basis from its kind', () => {
    const finding = (kind: Finding['kind']): Finding => ({
      id: kind, kind, severity: 'SERIOUS', confidence: { kind: 'MEASURED', basisPoints: 5000 }, subject: { kind: 'AGENCY' } as unknown as Finding['subject'],
      headline: 'h', explanation: 'e', citations: [], discoveredAt: '2026-10-01T00:00:00Z', lastSeenAt: '2026-10-02T00:00:00Z', dismissedAt: null, sourceDetector: 'encampment_notice',
    });
    const [counted, signal] = findingItems([finding('ENCAMPMENT_NOTICE_SHORTFALL'), finding('BOILERPLATE_DENIAL')]);
    expect(counted.provenance).toContain('A counted violation.');
    expect(counted.provenance).not.toContain('not a legal claim');
    expect(counted.provenance).toContain('encampment notice check');
    expect(signal.provenance).toContain('A pattern signal, not a legal claim.');
  });
});
