import type { AnalyticalSchema, RecordGapRequestInput, RecordGapRequestStatus, RequestSearchResult } from '../types';
import type { AutomaticProfile, DiscoveryObservation } from './automaticDiscovery';

/** A record that would answer what a lead's own tables cannot, keyed to the check the lead came from. */
export interface RecordGapDefinition {
  id: string;
  /** The checks whose leads leave this open, by query id. */
  reads: string[];
  record: string;
  recordTypes: string[];
  fields: string[];
  /** One search of held requests, or of the portal, per entry. */
  searchTerms: string[];
  /** What the lead's tables do not record. */
  missing: string;
  resolves: string;
  /** The check that reads the record once imported. The gap is answered only by that check's result. */
  answeredBy: string | null;
}

const ABATEMENT_CHECKS = ['abatement_notice_hours', 'abatement_repeat_places', 'abatement_reports_nearby'];

/** The first definitions. A definition names a record and a check, never a place or an agency. */
export const RECORD_GAP_DEFINITIONS: RecordGapDefinition[] = [
  { id: 'stored_property_log', reads: ABATEMENT_CHECKS, record: 'Stored-property intake and release log',
    recordTypes: ['Log of personal property taken into storage after an abatement or removal (intake log)', 'Log of stored property released to its owner, retrieved, or disposed of'],
    fields: ['Date and location the property was taken', 'Description of the property', 'Disposition as recorded (stored, discarded or other)', 'Storage site', 'Date released, retrieved or disposed of'],
    searchTerms: ['stored property', 'property storage', 'belongings', 'impound log'],
    missing: 'The abatement log records that an abatement was completed. It does not record what was stored, what was discarded, or what was later retrieved.',
    resolves: 'How many entries were recorded as stored, as discarded and as retrieved, for the places and dates in the log.', answeredBy: 'belongings_disposition' },
  { id: 'abatement_work_orders', reads: ['abatement_repeat_places', 'abatement_reports_nearby'], record: 'Abatement work orders',
    recordTypes: ['Work orders or service tickets for each abatement'],
    fields: ['Date and location', 'Crew or contractor', 'Notice posted and its date', 'Property stored and property discarded', 'Weight or volume removed'],
    searchTerms: ['abatement work order', 'encampment abatement', 'service ticket'],
    missing: 'The log gives a place and a closing time for each abatement, not what the crew recorded doing there.',
    resolves: 'What each abatement at the listed places recorded about people present, property and notice.', answeredBy: null },
  { id: 'abatement_invoices', reads: ['abatement_repeat_places'], record: 'Contractor invoices for abatement',
    recordTypes: ['Invoices and supporting detail from each contractor paid for abatement work'],
    fields: ['Invoice date and number', 'Service dates and locations', 'Hours, crew size and rates', 'Amount billed and amount paid'],
    searchTerms: ['abatement invoice', 'encampment contractor invoice'],
    missing: 'The log does not record what an abatement cost.',
    resolves: 'The cost of abatement per day and place, for the places abated most often.', answeredBy: null },
  { id: 'dispatch_unit_history', reads: ['stops_unhoused_outcomes', 'stops_unhoused_reasons'], record: 'Dispatch unit history',
    recordTypes: ['Computer-aided dispatch records listing each unit assigned to a call'],
    fields: ['Call number, type and priority', 'Date, time and location', 'Units and officers assigned', 'Time on scene per unit', 'Disposition'],
    searchTerms: ['CAD unit history', 'computer aided dispatch', 'calls for service units'],
    missing: 'The stop records list each person stopped, not how many units were sent or for how long.',
    resolves: 'How many units and officer hours each call type received.', answeredBy: null },
  { id: 'shelter_referrals', reads: ['capacity_comparison'], record: 'Shelter referral outcomes',
    recordTypes: ['Records of shelter referrals and their outcomes, including referrals where no bed was available'],
    fields: ['Referral date', 'Referring agency', 'Shelter requested', 'Outcome', 'Reason recorded when no placement was made'],
    searchTerms: ['shelter referral', 'no bed available', 'shelter intake'],
    missing: 'The tables count reports, beds and budget. They do not record what happened when a person asked for a bed.',
    resolves: 'How many referrals ended with no bed available, by period.', answeredBy: null },
  { id: 'site_rules_and_logs', reads: ['capacity_comparison'], record: 'Site rules, incident logs and grievance logs',
    recordTypes: ['Written rules for each shelter or sanctioned sleeping site', 'Incident logs', 'Grievance or complaint logs', 'Records of exits and their recorded reasons'],
    fields: ['Date', 'Site', 'Rule or category cited', 'Outcome'],
    searchTerms: ['safe sleeping rules', 'shelter rules', 'grievance log', 'incident log'],
    missing: 'The tables do not record the rules a site applies or why people leave it.',
    resolves: 'What rules each site applies and how many exits are recorded under each reason.', answeredBy: null },
  { id: 'charge_case_outcomes', reads: ['charges_living_outdoors'], record: 'Case outcomes for the listed charges',
    recordTypes: ['Disposition records for citations and arrests under the listed charges'],
    fields: ['Charge', 'Date of citation or arrest', 'Filing decision', 'Disposition and date', 'Fine or sentence'],
    searchTerms: ['citation disposition', 'encroachment', 'illegal lodging', 'case outcome'],
    missing: 'The charge log records that a charge was written, not what became of it.',
    resolves: 'How many of the listed charges were filed, dismissed or resolved, and how.', answeredBy: null },
  { id: 'written_procedures', reads: ['abatement_notice_hours', 'stops_unhoused_property'], record: 'Written procedures',
    recordTypes: ['Current and superseded written procedures on notice before an abatement', 'Written procedures on storing personal property and returning it'],
    fields: ['Effective date', 'Notice period required', 'Storage period required', 'How property is returned'],
    searchTerms: ['abatement procedure', 'property storage procedure', 'department procedure'],
    missing: 'The records show what was logged, not the procedure that applied on those dates.',
    resolves: 'The notice and storage periods the agency required of itself on the dates in the records.', answeredBy: null },
];

export interface RecordGap {
  key: string;
  leadRef: string;
  leadTitle: string;
  leadQuestion: string;
  snapshotId: string;
  definition: RecordGapDefinition;
  /** From the lead's own result; empty when the result names none. */
  period: string;
  identifiers: string[];
  /** The table the lead was counted from, as its catalog records it. */
  table: { name: string; source: string | null; retrievedOn: string; sha256: string };
  /** The address the lead's table was retrieved from, when it is a web address. */
  sourceOrigin: string | null;
  answered: boolean;
}

function origin(source: string | null): string | null {
  if (!source) return null;
  try { const url = new URL(source); return /^https?:$/.test(url.protocol) ? url.origin : null; } catch { return null; }
}

/** One gap per definition, check and source. Two copies of one source share a gap, so a newer
 * import does not ask again for a record already requested. */
export function recordGaps(schema: AnalyticalSchema, leads: DiscoveryObservation[], profiles: Record<string, AutomaticProfile>, definitions = RECORD_GAP_DEFINITIONS): RecordGap[] {
  const ran = (id: string) => Object.values(profiles).some((profile) => profile.results.some((entry) => entry.query.id === id && Boolean(entry.result?.rows.length)));
  const gaps = new Map<string, RecordGap>();
  for (const lead of leads) {
    const dataset = schema.datasets.find((item) => item.snapshotId === lead.snapshotId);
    if (lead.kind !== 'lead' || !lead.query || !dataset) continue;
    const source = dataset.sourceUri ?? null;
    const leadRef = `${lead.query.id}|${source ?? dataset.displayName}`;
    for (const definition of definitions) {
      const key = `${definition.id}|${leadRef}`;
      if (!definition.reads.includes(lead.query.id) || gaps.has(key)) continue;
      gaps.set(key, { key, leadRef, leadTitle: lead.title, leadQuestion: lead.question, snapshotId: lead.snapshotId, definition,
        period: lead.scope?.period ?? '', identifiers: lead.scope?.identifiers ?? [],
        table: { name: dataset.displayName, source, retrievedOn: dataset.retrievedAt.slice(0, 10), sha256: dataset.sourceSha256 },
        sourceOrigin: origin(source), answered: definition.answeredBy != null && ran(definition.answeredBy) });
    }
  }
  return [...gaps.values()];
}

const bullets = (values: string[]) => values.map((value) => `- ${value}`).join('\n');

/** The request to track for a gap. `custodian` is empty when the table's source names none. */
export function gapRequestInput(gap: RecordGap, custodian: string): RecordGapRequestInput {
  const { definition, table } = gap;
  const draft = [
    `To: ${custodian || 'Records custodian'}`,
    `Subject: Public records request: ${definition.record}`,
    `I request the following public records${gap.period ? ` for ${gap.period}` : ''}:\n${bullets(definition.recordTypes)}`,
    `Within those records I am asking for:\n${bullets(definition.fields)}`,
    ...(gap.identifiers.length ? [`The places and identifiers these records concern:\n${bullets(gap.identifiers)}`] : []),
    `These records were identified from a record already released or published: '${table.name}' (${table.source ?? 'no source address recorded'}), retrieved ${table.retrievedOn}, SHA-256 ${table.sha256}. ${definition.missing}`,
    'Please provide the records in the electronic format in which they are kept. If a record does not exist, please say so.',
    'Draft for review; nothing has been sent.',
  ].join('\n\n');
  return { gapKey: gap.key, leadRef: gap.leadRef, leadTitle: gap.leadTitle, leadQuestion: gap.leadQuestion, title: definition.record, custodian,
    gap: definition.missing, resolves: definition.resolves, draft, recordTypes: definition.recordTypes, fields: definition.fields, period: gap.period };
}

export type RecordGapState =
  | { kind: 'answered' }
  | { kind: 'closed' }
  | { kind: 'produced'; productions: number; remainingOmissions: number }
  | { kind: 'overdue'; dueOn: string; daysPast: number }
  | { kind: 'requested'; dueOn: string | null; daysLeft: number | null }
  | { kind: 'drafted' }
  | { kind: 'found'; requests: number; withFiles: number }
  | { kind: 'not_asked' };

const days = (from: string, to: string) => Math.round((Date.parse(to.slice(0, 10)) - Date.parse(from.slice(0, 10))) / 86_400_000);

/** `matches` is undefined until held requests were searched: not searched is not "none found". */
export function recordGapState(gap: RecordGap, tracked: RecordGapRequestStatus | undefined, matches: RequestSearchResult[] | undefined, today: string): RecordGapState {
  if (gap.answered) return { kind: 'answered' };
  if (tracked) {
    if (tracked.productions > 0) return { kind: 'produced', productions: tracked.productions, remainingOmissions: tracked.remainingOmissions };
    if (tracked.state === 'CLOSED') return { kind: 'closed' };
    if (!tracked.submittedAt) return { kind: 'drafted' };
    const dueOn = tracked.determinationDueOn;
    if (dueOn && tracked.deadlinePassedWithoutRecordedResponse) return { kind: 'overdue', dueOn, daysPast: days(dueOn, today) };
    return { kind: 'requested', dueOn, daysLeft: dueOn ? days(today, dueOn) : null };
  }
  const withFiles = matches?.filter((match) => match.documentCount > 0).length ?? 0;
  return withFiles ? { kind: 'found', requests: matches!.length, withFiles } : { kind: 'not_asked' };
}

export function recordGapStateLabel(state: RecordGapState): string {
  switch (state.kind) {
    case 'answered': return 'Answered: the record is imported and its check has run.';
    case 'closed': return 'Request closed with no production recorded.';
    case 'produced': return `Produced: ${state.productions} ${state.productions === 1 ? 'production' : 'productions'} recorded${state.remainingOmissions ? `, ${state.remainingOmissions} expected ${state.remainingOmissions === 1 ? 'item' : 'items'} not yet recorded as produced` : ''}. The gap stays open until the record is imported and checked.`;
    case 'overdue': return `Overdue: a determination was due ${state.dueOn}, ${state.daysPast} ${state.daysPast === 1 ? 'day' : 'days'} ago, and no response is recorded.`;
    case 'requested': return state.dueOn == null ? 'Requested.' : `Requested: a determination is due ${state.dueOn}${state.daysLeft != null && state.daysLeft >= 0 ? `, in ${state.daysLeft} ${state.daysLeft === 1 ? 'day' : 'days'}` : ''}.`;
    case 'drafted': return 'Request drafted and tracked; no submission date is recorded.';
    case 'found': return `${state.withFiles} of ${state.requests} held requests that mention these records have released files. Similar words are not an answer: the gap closes when a released file is imported and checked.`;
    case 'not_asked': return 'Not asked.';
  }
}
