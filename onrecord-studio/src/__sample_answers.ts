// Sample IPC answers for the UI capture / accessibility harness. Every agency, person, request,
// vendor and URL here is invented; example domains only. Dates are relative to module load.
import type { NativeCommandMap } from './generated/nativeCommands';
import type * as T from './types';

type SampleAnswers = {
  [K in keyof NativeCommandMap]?:
    | NativeCommandMap[K]['result']
    | ((args: NativeCommandMap[K]['args']) => NativeCommandMap[K]['result']);
};

const NOW = Date.now();
const DAY = 86_400_000;
const MINUTE = 60_000;

/** RFC 3339 instant `days` before now (negative = future), shifted by `minutes`. */
function iso(days: number, minutes = 0): string {
  return new Date(NOW - days * DAY + minutes * MINUTE).toISOString();
}
/** `YYYY-MM-DD`, `days` after today (negative = past). */
function ymd(days: number): string {
  return new Date(NOW + days * DAY).toISOString().slice(0, 10);
}
/** Deterministic 64-hex digest stand-in, so repeated captures render identical values. */
function hex64(seed: string): string {
  let h = 2166136261 >>> 0;
  let out = '';
  for (let round = 0; round < 8; round += 1) {
    for (const ch of `${seed}#${round}`) {
      h ^= ch.charCodeAt(0);
      h = Math.imul(h, 16777619) >>> 0;
    }
    out += h.toString(16).padStart(8, '0');
  }
  return out;
}
function countMasks(text: string): number {
  return (text.match(/\[MASKED_[A-Z_]+\]/g) ?? []).length;
}

// ---------------------------------------------------------------------------------------------
// Core: agencies, sources, requests, documents. Every command below is derived from these.
// ---------------------------------------------------------------------------------------------

interface Agency { id: number; portalKey: string; name: string; host: string }

const AGENCIES: Agency[] = [
  { id: 101, portalKey: 'harborcity-pd', name: 'Harbor City Police Department', host: 'records.harborcity.example' },
  { id: 102, portalKey: 'bayview-usd', name: 'Bayview Unified School District', host: 'publicrecords.bayviewusd.example' },
  { id: 103, portalKey: 'marlow-county-clerk', name: 'Marlow County Clerk', host: 'clerk.marlowcounty.example' },
  { id: 104, portalKey: 'alder-valley-water', name: 'Alder Valley Water District', host: 'records.aldervalleywater.example' },
  { id: 105, portalKey: 'cedar-hollow', name: 'City of Cedar Hollow', host: 'cedarhollow.records.example' },
  { id: 106, portalKey: 'gullwing-transit', name: 'Gullwing Bay Transit Authority', host: 'records.gullwingtransit.example' },
];
const agencyById = new Map(AGENCIES.map((agency) => [agency.id, agency]));
const agencyByPortal = new Map(AGENCIES.map((agency) => [agency.portalKey, agency]));

function registration(portalKey: string, displayName: string, host: string, sourceKind: T.SourceKind, verifiedDaysAgo: number): T.PortalRegistration {
  const isNextRequest = sourceKind === 'NEXT_REQUEST';
  return {
    portalKey,
    displayName,
    canonicalUrl: `https://${host}/`,
    originUrl: `https://${host}`,
    apiLayout: {
      requestsEndpoint: isNextRequest ? `https://${host}/client/requests` : `https://${host}/`,
      requestsPointer: isNextRequest ? '/requests' : '',
      metadataPointer: isNextRequest ? '/total_count' : null,
      pageParameter: 'page_number',
      pageSizeParameter: 'per_page',
    },
    verifiedAt: iso(verifiedDaysAgo),
    sourceKind,
  };
}

const REGISTRATIONS: T.PortalRegistration[] = [
  registration('harborcity-pd', 'Harbor City Police Department', 'records.harborcity.example', 'NEXT_REQUEST', 112),
  registration('bayview-usd', 'Bayview Unified School District', 'publicrecords.bayviewusd.example', 'NEXT_REQUEST', 108),
  registration('marlow-county-clerk', 'Marlow County Clerk', 'clerk.marlowcounty.example', 'WEB_CRAWL', 103),
  registration('alder-valley-water', 'Alder Valley Water District', 'records.aldervalleywater.example', 'NEXT_REQUEST', 96),
  registration('cedar-hollow', 'City of Cedar Hollow', 'cedarhollow.records.example', 'NEXT_REQUEST', 90),
  registration('gullwing-transit', 'Gullwing Bay Transit Authority', 'records.gullwingtransit.example', 'NEXT_REQUEST', 61),
  registration('harborcity-council', 'Harbor City Council Agendas', 'agendas.harborcity.example', 'WEB_CRAWL', 74),
  registration('marlow-open-data', 'Marlow County Open Data', 'data.marlowcounty.example', 'OPEN_DATA_CATALOG', 57),
];
const registrationByKey = new Map(REGISTRATIONS.map((reg) => [reg.portalKey, reg]));
function reg(portalKey: string): T.PortalRegistration {
  const found = registrationByKey.get(portalKey);
  if (!found) throw new Error(`sample data: unknown portal ${portalKey}`);
  return found;
}

const SAVED_SOURCES: T.SavedSource[] = REGISTRATIONS.map((registration) => {
  if (registration.portalKey === 'gullwing-transit') {
    return { registration, lifecycleState: 'PAUSED', healthStatus: 'verified', lastError: null };
  }
  if (registration.portalKey === 'marlow-open-data') {
    return {
      registration,
      lifecycleState: 'ACTIVE',
      healthStatus: 'degraded',
      lastError: 'HTTP 503 from data.marlowcounty.example after 3 retries; the next sync resumes from the saved frontier.',
    };
  }
  return { registration, lifecycleState: 'ACTIVE', healthStatus: 'verified', lastError: null };
});

// --- Document texts (already PII-masked, as the app stores and shows them) ---------------------

interface SampleDocument {
  documentId: number;
  portalKey: string;
  requestId: number;
  fileName: string;
  contentType: string;
  extractionFormat: string | null;
  extractionStatus: 'SEARCHABLE' | 'DEFERRED';
  extractionDeferredReason: string | null;
  text: string;
}

const DOCUMENTS: SampleDocument[] = [];
function addDoc(documentId: number, portalKey: string, requestId: number, fileName: string, text: string, contentType = 'application/pdf', extractionFormat: string | null = 'PDF_TEXT'): void {
  DOCUMENTS.push({ documentId, portalKey, requestId, fileName, contentType, extractionFormat, extractionStatus: 'SEARCHABLE', extractionDeferredReason: null, text });
}

addDoc(5101, 'harborcity-pd', 1142, 'HCPD_Response_Letter_26-1142.pdf', [
  'HARBOR CITY POLICE DEPARTMENT',
  'Records Unit',
  '',
  'Re: Public Records Act Request 26-1142',
  '',
  'The Department has located 214 pages of records responsive to your request for use-of-force review summaries for the second quarter. Due to the volume of records, the Department is extending the time to respond by 14 days under Government Code section 7922.535(b)(2).',
  '',
  'Portions of the attached records are withheld under Government Code section 7923.600, which exempts records of investigations conducted by a local police agency. Names of uninvolved witnesses have been redacted.',
  '',
  'A first installment of 38 pages is attached. Remaining installments will be produced on a rolling basis.',
  '',
  'Questions may be directed to the Records Unit at [MASKED_PHONE].',
  '',
  'Records Unit',
  'Harbor City Police Department',
].join('\n'));

addDoc(5102, 'harborcity-pd', 1142, 'Use_of_Force_Summary_Q2.pdf', [
  'USE OF FORCE REVIEW SUMMARY - SECOND QUARTER',
  '',
  'Incident 26-UF-0412. Location: [MASKED_PRIVATE_ADDRESS]. Force type: control hold. Review outcome: within policy.',
  'Incident 26-UF-0418. Location: [MASKED_PRIVATE_ADDRESS]. Force type: conducted energy device. Review outcome: referred for training.',
  'Incident 26-UF-0431. Location: [REDACTED - GC 7923.600]. Force type: [REDACTED - GC 7923.600]. Review outcome: within policy.',
  '',
  'Reviewing supervisor contact: [MASKED_PHONE].',
].join('\n'));

addDoc(5104, 'harborcity-pd', 1176, 'Use_of_Force_Summary_Q3.pdf', [
  'USE OF FORCE REVIEW SUMMARY - THIRD QUARTER',
  '',
  'Incident 26-UF-0431 (carried over from second quarter). Location: [MASKED_PRIVATE_ADDRESS]. Force type: control hold. Review outcome: within policy.',
  'Incident 26-UF-0507. Location: [MASKED_PRIVATE_ADDRESS]. Force type: takedown. Review outcome: pending review.',
].join('\n'));

const BOILERPLATE = 'the public interest served by not disclosing the requested records clearly outweighs the public interest served by disclosure of the records';

addDoc(5103, 'harborcity-pd', 1158, 'Response_26-1158.pdf', [
  'HARBOR CITY POLICE DEPARTMENT',
  'Re: Public Records Act Request 26-1158',
  '',
  `After a diligent search, the Department has determined that ${BOILERPLATE}. Gov. Code § 7922.000. Your request is therefore denied in its entirety.`,
  '',
  'Records Unit',
].join('\n'));

addDoc(5201, 'bayview-usd', 2087, 'Facilities_Agreement_24-FAC-019_Amendment_3.pdf', [
  'BAYVIEW UNIFIED SCHOOL DISTRICT',
  'Amendment No. 3 to Agreement 24-FAC-019',
  'Westshore Facilities Group LLC',
  '',
  'The not-to-exceed amount of the Agreement remains $1,250,000.',
  'This Amendment extends the term through the end of the fiscal year and adds HVAC replacement at Bayview Middle School.',
  'Approved by the Board of Education on consent.',
].join('\n'));

addDoc(5202, 'bayview-usd', 2087, 'Vendor_Payment_Register_FY26.csv', [
  'vendor,agreement,invoice,paid_date,amount',
  `Westshore Facilities Group LLC,24-FAC-019,INV-3301,${ymd(-104)},412500.00`,
  `Westshore Facilities Group LLC,24-FAC-019,INV-3317,${ymd(-77)},436200.00`,
  `Westshore Facilities Group LLC,24-FAC-019,INV-3342,${ymd(-41)},498750.00`,
  `Westshore Facilities Group LLC,24-FAC-019,INV-3350,${ymd(-23)},49800.00`,
  `Westshore Facilities Group LLC,24-FAC-019,INV-3351,${ymd(-23)},49800.00`,
  `Westshore Facilities Group LLC,24-FAC-019,INV-3352,${ymd(-22)},49800.00`,
  'Total paid under 24-FAC-019: 1,496,850.00',
].join('\n'), 'text/csv', 'CSV');

addDoc(5203, 'bayview-usd', 2071, 'Board_Agenda_Regular_Meeting.pdf', [
  'BAYVIEW UNIFIED SCHOOL DISTRICT',
  `Board of Education - Regular Meeting Agenda - ${ymd(13)}`,
  '',
  'Consent Item 12. Ratify Amendment No. 4 to Agreement 24-FAC-019, Westshore Facilities Group LLC.',
  'Item 15. Second interim budget report.',
  '',
  'Public comment cards must be submitted to the Board secretary before the item is called.',
].join('\n'));

addDoc(5301, 'marlow-county-clerk', 3310, 'Response_MC-3310.pdf', [
  'MARLOW COUNTY CLERK-RECORDER',
  'Re: Request MC-3310 - fleet fuel card statements for the Elections Division',
  '',
  'After a search of Elections Division and Fleet Services files, the County has no records responsive to your request.',
  '',
  "Clerk-Recorder's Office",
].join('\n'));

addDoc(5302, 'marlow-county-clerk', 3318, 'Fleet_Fuel_Card_Statements.pdf', [
  'MARLOW COUNTY FLEET SERVICES',
  'Fuel Card Statement - Elections Division',
  `Pool card ending 4417 - statement period ending ${ymd(-96)}`,
  '',
  '14 transactions. Total $2,184.36.',
  'Billing contact: [MASKED_PHONE].',
].join('\n'));

addDoc(5303, 'marlow-county-clerk', 3329, 'BOS_Agenda_Packet.pdf', [
  'MARLOW COUNTY BOARD OF SUPERVISORS',
  'Agenda Packet - Regular Session',
  '',
  'Item 3. Clerk-Recorder: approve elections equipment maintenance agreement.',
  'Item 9. Fleet Services: fuel card program policy update.',
].join('\n'));

addDoc(5401, 'alder-valley-water', 4410, 'Board_Minutes_Rate_Study.pdf', [
  'ALDER VALLEY WATER DISTRICT',
  `Regular Meeting of the Board of Directors - Minutes - ${ymd(-9)}`,
  '',
  'Item 6. Rate study consultant contract. Staff reported that correspondence with the consultant between March and May is attached as Exhibit C.',
  'Exhibit C contains messages dated March 4 through March 19 and May 22 through May 30.',
  'Item 7. Public comment. Two members of the public addressed the board regarding the proposed rate increase.',
].join('\n'));

addDoc(5501, 'cedar-hollow', 5520, 'Extension_Notice_CH-5520.pdf', [
  'CITY OF CEDAR HOLLOW - OFFICE OF THE CITY CLERK',
  'Notice of Extension - Request CH-5520',
  '',
  'The City requires additional time to search for and collect records from field facilities that are separate from the office processing the request. Gov. Code § 7922.535(c)(1).',
  'The City expects to issue its determination within 14 additional days.',
].join('\n'));

addDoc(5521, 'cedar-hollow', 5531, 'Response_CH-5531.pdf', [
  'CITY OF CEDAR HOLLOW - OFFICE OF THE CITY CLERK',
  'Re: Request CH-5531',
  '',
  `The City has determined that ${BOILERPLATE}. Gov. Code § 7922.000. Your request is therefore denied.`,
  '',
  'Office of the City Clerk',
].join('\n'));

addDoc(7001, 'harborcity-council', 0, 'Council_Agenda_Regular.pdf', [
  'HARBOR CITY COUNCIL',
  `Regular Meeting Agenda - ${ymd(6)}`,
  '',
  'Item 4. Second reading: Police Department body-worn camera policy revisions.',
  'Item 7. Approve renewal of records management system contract with the Police Department.',
  '',
  'Written comments received by 5:00 p.m. the day before the meeting will be distributed to the Council.',
  'Submit comments to clerk@harborcity.example or in person at the City Clerk counter.',
].join('\n'));

addDoc(7002, 'harborcity-council', 0, 'Council_Minutes_Regular.pdf', [
  'HARBOR CITY COUNCIL',
  `Regular Meeting Minutes - ${ymd(-15)}`,
  '',
  'Item 5. First reading: Police Department body-worn camera policy revisions. Passed 5-0.',
  'Item 8. Received quarterly public records request report.',
].join('\n'));

// --- Requests (search results) ------------------------------------------------------------------

interface SampleRequest {
  requestId: number;
  agencyId: number;
  title: string;
  description: string;
  status: 'open' | 'closed';
  createdDaysAgo: number;
  /** Days ago, or minutes after creation when `closedMinutesAfter` is set. */
  closedDaysAgo: number | null;
  closedMinutesAfter?: number;
  genericDocuments: number;
  flags: string[];
}

const REQUESTS: SampleRequest[] = [
  { requestId: 1142, agencyId: 101, title: 'Use-of-force review summaries, second quarter', description: 'All use-of-force review summaries and supervisor dispositions for the second quarter, including referral outcomes.', status: 'open', createdDaysAgo: 40, closedDaysAgo: null, genericDocuments: 0, flags: ['REDACTION_HEAVY'] },
  { requestId: 1158, agencyId: 101, title: 'Vehicle pursuit after-action reports', description: 'After-action reports for vehicle pursuits that ended in a collision this calendar year.', status: 'closed', createdDaysAgo: 55, closedDaysAgo: 47, genericDocuments: 0, flags: [] },
  { requestId: 1163, agencyId: 101, title: 'Body-worn camera policy and audit logs', description: 'Current body-worn camera policy, revision history, and quarterly activation audit logs.', status: 'closed', createdDaysAgo: 88, closedDaysAgo: 61, genericDocuments: 2, flags: [] },
  { requestId: 1171, agencyId: 101, title: 'Records management system contract', description: 'Contract, amendments, and invoices for the records management system vendor.', status: 'open', createdDaysAgo: 21, closedDaysAgo: null, genericDocuments: 0, flags: [] },
  { requestId: 1176, agencyId: 101, title: 'Use-of-force review summaries, third quarter', description: 'All use-of-force review summaries for the third quarter.', status: 'closed', createdDaysAgo: 75, closedDaysAgo: 49, genericDocuments: 0, flags: [] },
  { requestId: 1180, agencyId: 101, title: 'Patrol division overtime records', description: 'Overtime hours and pay by pay period for the patrol division.', status: 'open', createdDaysAgo: 12, closedDaysAgo: null, genericDocuments: 0, flags: [] },
  { requestId: 1185, agencyId: 101, title: 'Towing rotation contract invoices', description: 'Invoices submitted under the towing rotation agreement.', status: 'closed', createdDaysAgo: 100, closedDaysAgo: 79, genericDocuments: 3, flags: [] },
  { requestId: 2087, agencyId: 102, title: 'Facilities contract amendments, Westshore Facilities Group', description: 'Agreement 24-FAC-019 with all amendments and the vendor payment register for the current fiscal year.', status: 'open', createdDaysAgo: 6, closedDaysAgo: null, genericDocuments: 0, flags: [] },
  { requestId: 2071, agencyId: 102, title: 'Board of Education agenda packets', description: 'Agenda packets for regular and special Board meetings this semester.', status: 'closed', createdDaysAgo: 60, closedDaysAgo: 50, genericDocuments: 0, flags: [] },
  { requestId: 2064, agencyId: 102, title: 'Special education transportation contracts', description: 'Contracts and rate sheets for special education transportation providers.', status: 'closed', createdDaysAgo: 95, closedDaysAgo: 86, genericDocuments: 2, flags: [] },
  { requestId: 2090, agencyId: 102, title: 'Campus security camera vendor contracts', description: 'Contracts and data retention terms for campus security camera vendors.', status: 'open', createdDaysAgo: 3, closedDaysAgo: null, genericDocuments: 0, flags: [] },
  { requestId: 2055, agencyId: 102, title: 'Superintendent employment contract and amendments', description: 'Current superintendent employment agreement and all amendments.', status: 'closed', createdDaysAgo: 110, closedDaysAgo: 101, genericDocuments: 1, flags: [] },
  { requestId: 3310, agencyId: 103, title: 'Fleet fuel card statements, Elections Division', description: 'Fuel card statements for cards assigned to the Elections Division.', status: 'closed', createdDaysAgo: 80, closedDaysAgo: 79, genericDocuments: 0, flags: [] },
  { requestId: 3318, agencyId: 103, title: 'Fleet Services fuel card statements, all divisions', description: 'Monthly fuel card statements for every county division.', status: 'closed', createdDaysAgo: 44, closedDaysAgo: 36, genericDocuments: 0, flags: [] },
  { requestId: 3322, agencyId: 103, title: 'Ballot storage room access logs', description: 'Badge access logs for the ballot storage room during the canvass period.', status: 'closed', createdDaysAgo: 30, closedDaysAgo: null, closedMinutesAfter: 3, genericDocuments: 0, flags: ['SUSPICIOUSLY_FAST_CLOSURE'] },
  { requestId: 3329, agencyId: 103, title: 'Board of Supervisors agenda packets', description: 'Agenda packets for Board of Supervisors regular sessions.', status: 'open', createdDaysAgo: 9, closedDaysAgo: null, genericDocuments: 0, flags: [] },
  { requestId: 3335, agencyId: 103, title: "Recorder's office overtime", description: "Overtime hours for Recorder's office staff by pay period.", status: 'closed', createdDaysAgo: 25, closedDaysAgo: null, closedMinutesAfter: 4, genericDocuments: 0, flags: ['SUSPICIOUSLY_FAST_CLOSURE'] },
  { requestId: 4410, agencyId: 104, title: 'Rate study consultant correspondence', description: 'Correspondence between District staff and the rate study consultant, March through May.', status: 'closed', createdDaysAgo: 70, closedDaysAgo: 58, genericDocuments: 0, flags: [] },
  { requestId: 4416, agencyId: 104, title: 'Water main break repair invoices', description: 'Invoices and work orders for emergency water main repairs.', status: 'open', createdDaysAgo: 15, closedDaysAgo: null, genericDocuments: 0, flags: [] },
  { requestId: 4421, agencyId: 104, title: 'Groundwater well testing results', description: 'Laboratory results for District groundwater wells, last two sampling rounds.', status: 'closed', createdDaysAgo: 50, closedDaysAgo: 39, genericDocuments: 2, flags: ['REDACTION_HEAVY'] },
  { requestId: 5520, agencyId: 105, title: 'Code enforcement complaint logs, Ward 3', description: 'Code enforcement complaint intake logs and case dispositions for Ward 3.', status: 'closed', createdDaysAgo: 70, closedDaysAgo: 39, genericDocuments: 1, flags: ['STATUTORY_LATENCY_VIOLATION'] },
  { requestId: 5531, agencyId: 105, title: 'Encampment cleanup notices', description: 'Posted notices and photographs for encampment cleanups this year.', status: 'closed', createdDaysAgo: 52, closedDaysAgo: 44, genericDocuments: 0, flags: [] },
  { requestId: 5536, agencyId: 105, title: 'Parking citation revenue reports', description: 'Monthly parking citation revenue reports and the collection vendor contract.', status: 'open', createdDaysAgo: 18, closedDaysAgo: null, genericDocuments: 0, flags: [] },
  { requestId: 5540, agencyId: 105, title: 'Public works overtime and staffing study', description: 'Public works overtime reports and the most recent staffing study.', status: 'closed', createdDaysAgo: 65, closedDaysAgo: 30, genericDocuments: 2, flags: ['STATUTORY_LATENCY_VIOLATION', 'REDACTION_HEAVY'] },
  { requestId: 6602, agencyId: 106, title: 'Bus stop maintenance work orders', description: 'Work orders for bus stop shelter repairs and cleaning.', status: 'open', createdDaysAgo: 34, closedDaysAgo: null, genericDocuments: 0, flags: [] },
  { requestId: 6605, agencyId: 106, title: 'Fare enforcement citations', description: 'Fare enforcement citation counts by route and month.', status: 'open', createdDaysAgo: 20, closedDaysAgo: null, genericDocuments: 0, flags: [] },
];

for (const request of REQUESTS) {
  const agency = agencyById.get(request.agencyId);
  if (!agency) continue;
  for (let index = 0; index < request.genericDocuments; index += 1) {
    const documentId = 9000 + request.requestId * 10 + index;
    if (index === 1 && request.genericDocuments > 1) {
      DOCUMENTS.push({
        documentId,
        portalKey: agency.portalKey,
        requestId: request.requestId,
        fileName: `Attachment_B_${request.requestId}.xlsx`,
        contentType: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet',
        extractionFormat: 'XLSX',
        extractionStatus: 'DEFERRED',
        extractionDeferredReason: 'Spreadsheets are imported through Analysis rather than searched as text.',
        text: '',
      });
      continue;
    }
    addDoc(documentId, agency.portalKey, request.requestId, `Responsive_Records_${request.requestId}_${index + 1}.pdf`, [
      agency.name.toUpperCase(),
      `Re: ${request.title}`,
      '',
      request.description,
      '',
      'Responsive records are attached. Personal contact information has been withheld.',
      'Records contact: [MASKED_PHONE].',
    ].join('\n'));
  }
}

const documentByKey = new Map(DOCUMENTS.map((doc) => [`${doc.portalKey}:${doc.documentId}`, doc]));
function doc(documentId: number): SampleDocument {
  const found = DOCUMENTS.find((candidate) => candidate.documentId === documentId);
  if (!found) throw new Error(`sample data: unknown document ${documentId}`);
  return found;
}
function documentsFor(requestId: number, portalKey: string): SampleDocument[] {
  return DOCUMENTS.filter((candidate) => candidate.requestId === requestId && candidate.portalKey === portalKey);
}

/** A citation whose offsets are the quote's actual position in the stored text. */
function cite(documentId: number, quote: string, label: string | null = null): T.FindingCitation {
  const source = doc(documentId);
  const start = source.text.indexOf(quote);
  return {
    portalKey: source.portalKey,
    documentId,
    characterStart: start < 0 ? null : start,
    characterEnd: start < 0 ? null : start + quote.length,
    quote,
    label,
  };
}

const SEARCH_RESULTS: T.RequestSearchResult[] = REQUESTS.map((request) => {
  const agency = agencyById.get(request.agencyId) as Agency;
  const closedAt = request.closedMinutesAfter !== undefined
    ? iso(request.createdDaysAgo, request.closedMinutesAfter)
    : request.closedDaysAgo === null ? null : iso(request.closedDaysAgo);
  return {
    requestId: request.requestId,
    portalKey: agency.portalKey,
    agencyId: agency.id,
    agencyName: agency.name,
    title: request.title,
    descriptionPreview: request.description,
    status: request.status,
    createdAt: iso(request.createdDaysAgo),
    closedAt,
    documentCount: documentsFor(request.requestId, agency.portalKey).length,
    integrityFlags: request.flags,
  };
});

function matchingRecords(args: { searchTerm: string; portalKey?: string | null; agencyId?: number | null; status?: string | null; policyDeviation?: string | null }): T.RequestSearchResult[] {
  const term = args.searchTerm.trim().toLowerCase();
  return SEARCH_RESULTS.filter((record) =>
    (!term || `${record.title} ${record.descriptionPreview} ${record.agencyName}`.toLowerCase().includes(term))
    && (!args.portalKey || record.portalKey === args.portalKey)
    && (args.agencyId == null || record.agencyId === args.agencyId)
    && (!args.status || record.status === args.status.toLowerCase())
    && (!args.policyDeviation || record.integrityFlags.includes(args.policyDeviation)));
}

// --- Findings ---------------------------------------------------------------------------------

function subject(portalKey: string | null, agencyId: number | null, requestId: number | null, documentId: number | null, entityId: string | null = null): T.FindingSubject {
  return { portalKey, agencyId, requestId, documentId, entityId };
}

const FINDINGS: T.Finding[] = [
  {
    id: 'fnd-contract-ceiling-24fac019',
    kind: 'CONTRACT_CEILING_EXCEEDED',
    severity: 'SERIOUS',
    confidence: { kind: 'MEASURED', basisPoints: 9500 },
    subject: subject('bayview-usd', 102, 2087, 5202, 'ent-org-westshore'),
    headline: 'Payments under Agreement 24-FAC-019 total $1,496,850 against a $1,250,000 not-to-exceed amount',
    explanation: 'The released payment register sums six invoices under 24-FAC-019 to $1,496,850.00. Amendment No. 3 states the not-to-exceed amount remains $1,250,000. The difference is $246,850.00. No later amendment raising the ceiling is in the collected records.',
    citations: [
      cite(5201, 'The not-to-exceed amount of the Agreement remains $1,250,000.', 'Amendment No. 3'),
      cite(5202, 'Total paid under 24-FAC-019: 1,496,850.00', 'Payment register'),
    ],
    discoveredAt: iso(5),
    lastSeenAt: iso(0, -40),
    dismissedAt: null,
    sourceDetector: 'spending_cases',
  },
  {
    id: 'fnd-split-purchase-24fac019',
    kind: 'SPLIT_PURCHASE_PATTERN',
    severity: 'NOTABLE',
    confidence: { kind: 'MEASURED', basisPoints: 7100 },
    subject: subject('bayview-usd', 102, 2087, 5202, 'ent-org-westshore'),
    headline: 'Three $49,800 invoices to one vendor within two days',
    explanation: 'Three invoices of $49,800.00 each were paid to the same vendor within two days, each just under $50,000. This is a pattern signal, not a finding that a purchasing rule was broken: no approval threshold policy for this district has been collected.',
    citations: [
      cite(5202, `Westshore Facilities Group LLC,24-FAC-019,INV-3350,${ymd(-23)},49800.00`, 'INV-3350'),
      cite(5202, `Westshore Facilities Group LLC,24-FAC-019,INV-3352,${ymd(-22)},49800.00`, 'INV-3352'),
    ],
    discoveredAt: iso(5),
    lastSeenAt: iso(0, -40),
    dismissedAt: null,
    sourceDetector: 'spending_cases',
  },
  {
    id: 'fnd-no-records-contradicted-3310',
    kind: 'NO_RECORDS_CONTRADICTED_BY_RELEASE',
    severity: 'SERIOUS',
    confidence: { kind: 'MEASURED', basisPoints: 8800 },
    subject: subject('marlow-county-clerk', 103, 3310, 5301),
    headline: 'A "no responsive records" answer was followed by a release of matching statements',
    explanation: 'Request MC-3310 was answered with no responsive records for Elections Division fuel card statements. A later release under MC-3318 includes a fuel card statement labeled Elections Division for a period covered by MC-3310.',
    citations: [
      cite(5301, 'the County has no records responsive to your request', 'MC-3310 response'),
      cite(5302, 'Fuel Card Statement - Elections Division', 'MC-3318 release'),
    ],
    discoveredAt: iso(33),
    lastSeenAt: iso(0, -40),
    dismissedAt: null,
    sourceDetector: 'no_records_contradicted',
  },
  {
    id: 'fnd-boilerplate-denial-1158',
    kind: 'BOILERPLATE_DENIAL',
    severity: 'NOTABLE',
    confidence: { kind: 'MEASURED', basisPoints: 8200 },
    subject: subject('harborcity-pd', 101, 1158, 5103),
    headline: 'Denial rests on the catch-all balancing test with no request-specific reasoning',
    explanation: 'The response to 26-1158 denies the request in full citing only the public-interest balancing test, without describing the records or the interest weighed. The same sentence appears in denials from another agency in this corpus.',
    citations: [cite(5103, BOILERPLATE, '26-1158 denial')],
    discoveredAt: iso(46),
    lastSeenAt: iso(0, -40),
    dismissedAt: null,
    sourceDetector: 'boilerplate_denial',
  },
  {
    id: 'fnd-shared-language-balancing',
    kind: 'CROSS_CORPUS_SHARED_LANGUAGE',
    severity: 'INFO',
    confidence: { kind: 'MEASURED', basisPoints: 7600 },
    subject: subject(null, null, null, null),
    headline: 'Identical denial wording at Harbor City Police Department and City of Cedar Hollow',
    explanation: 'A 21-word denial passage is identical in responses from two unrelated agencies. Shared wording can come from a common template or counsel; it is not evidence of coordination.',
    citations: [cite(5103, BOILERPLATE, 'Harbor City PD'), cite(5521, BOILERPLATE, 'Cedar Hollow')],
    discoveredAt: iso(43),
    lastSeenAt: iso(0, -40),
    dismissedAt: null,
    sourceDetector: 'cross_corpus_shared_language',
  },
  {
    id: 'fnd-redaction-inconsistent-0431',
    kind: 'REDACTION_INCONSISTENT_ACROSS_RELEASES',
    severity: 'NOTABLE',
    confidence: { kind: 'UNASSESSED' },
    subject: subject('harborcity-pd', 101, 1142, 5102),
    headline: 'Incident 26-UF-0431 force type is redacted in one release and disclosed in another',
    explanation: 'The second-quarter summary withholds the force type for 26-UF-0431 under GC 7923.600; the third-quarter summary discloses it as a control hold. Which release reflects the intended withholding is not determined here.',
    citations: [
      cite(5102, 'Force type: [REDACTED - GC 7923.600]', 'Q2 summary'),
      cite(5104, 'Force type: control hold', 'Q3 summary'),
    ],
    discoveredAt: iso(18),
    lastSeenAt: iso(0, -40),
    dismissedAt: null,
    sourceDetector: 'redaction_inconsistency',
  },
  {
    id: 'fnd-extension-pattern-cedar-hollow',
    kind: 'EXTENSION_ABUSE',
    severity: 'NOTABLE',
    confidence: { kind: 'UNASSESSED' },
    subject: subject('cedar-hollow', 105, 5520, 5501),
    headline: 'City of Cedar Hollow invoked a statutory extension on 9 of 14 recent requests',
    explanation: 'Peer agencies in this corpus invoked an extension on 6 of 71 requests over the same window. This is a frequency comparison; each extension may be individually valid.',
    citations: [cite(5501, 'The City expects to issue its determination within 14 additional days.', 'CH-5520 extension notice')],
    discoveredAt: iso(28),
    lastSeenAt: iso(0, -40),
    dismissedAt: null,
    sourceDetector: 'extension_abuse',
  },
  {
    id: 'fnd-thread-gap-4410',
    kind: 'EMAIL_THREAD_GAP',
    severity: 'INFO',
    confidence: { kind: 'UNASSESSED' },
    subject: subject('alder-valley-water', 104, 4410, 5401),
    headline: 'Exhibit C correspondence skips from March 19 to May 22',
    explanation: 'The minutes describe Exhibit C as correspondence between March and May, but the messages it contains leave a nine-week interval with none. The interval may simply have had no correspondence.',
    citations: [cite(5401, 'Exhibit C contains messages dated March 4 through March 19 and May 22 through May 30.', 'Board minutes, Item 6')],
    discoveredAt: iso(9),
    lastSeenAt: iso(0, -40),
    dismissedAt: null,
    sourceDetector: 'email_thread_gap',
  },
  {
    id: 'fnd-held-back-1142',
    kind: 'HELD_BACK_RESPONSE',
    severity: 'INFO',
    confidence: { kind: 'UNASSESSED' },
    subject: subject('harborcity-pd', 101, 1142, 5101),
    headline: '38 of 214 located pages released so far on 26-1142',
    explanation: 'The agency located 214 responsive pages and released a first installment of 38. Rolling production is permitted; this tracks the remainder until it arrives.',
    citations: [cite(5101, 'A first installment of 38 pages is attached.', '26-1142 response letter')],
    discoveredAt: iso(26),
    lastSeenAt: iso(0, -40),
    dismissedAt: null,
    sourceDetector: 'held_back_response',
  },
];

function filterFindings(filter: T.FindingsFilter | null | undefined): T.Finding[] {
  const f = filter ?? {};
  return FINDINGS.filter((finding) =>
    (!f.id || finding.id === f.id)
    && (!f.portalKey || finding.subject.portalKey === f.portalKey || finding.citations.some((c) => c.portalKey === f.portalKey))
    && (f.agencyId == null || finding.subject.agencyId === f.agencyId)
    && (f.requestId == null || finding.subject.requestId === f.requestId)
    && (f.documentId == null || finding.subject.documentId === f.documentId || finding.citations.some((c) => c.documentId === f.documentId))
    && (!f.sourceDetector || finding.sourceDetector === f.sourceDetector)
    && (f.includeDismissed || finding.dismissedAt === null)
    && (!f.discoveredAfter || finding.discoveredAt > f.discoveredAfter));
}
const findingById = new Map(FINDINGS.map((finding) => [finding.id, finding]));

// --- Corpus entities --------------------------------------------------------------------------

const ENTITIES: T.CorpusEntityNode[] = [
  { entityId: 'ent-org-hcpd', kind: 'ORGANIZATION', displayValue: 'Harbor City Police Department', documentCount: 41, validationGrade: 'GAZETTEER_HIT' },
  { entityId: 'ent-statute-7922000', kind: 'STATUTE', displayValue: 'Cal. Gov. Code § 7922.000', documentCount: 33, validationGrade: 'FORMAT_VALIDATED' },
  { entityId: 'ent-statute-7922535', kind: 'STATUTE', displayValue: 'Cal. Gov. Code § 7922.535', documentCount: 27, validationGrade: 'FORMAT_VALIDATED' },
  { entityId: 'ent-org-bayview', kind: 'ORGANIZATION', displayValue: 'Bayview Unified School District', documentCount: 24, validationGrade: 'GAZETTEER_HIT' },
  { entityId: 'ent-statute-7923600', kind: 'STATUTE', displayValue: 'Cal. Gov. Code § 7923.600', documentCount: 19, validationGrade: 'FORMAT_VALIDATED' },
  { entityId: 'ent-org-westshore', kind: 'ORGANIZATION', displayValue: 'Westshore Facilities Group LLC', documentCount: 14, validationGrade: 'PATTERN_ONLY' },
  { entityId: 'ent-role-custodian', kind: 'ROLE', displayValue: 'Records Custodian', documentCount: 13, validationGrade: 'PATTERN_ONLY' },
  { entityId: 'ent-org-cedar-hollow', kind: 'ORGANIZATION', displayValue: 'City of Cedar Hollow', documentCount: 12, validationGrade: 'GAZETTEER_HIT' },
  { entityId: 'ent-money-1250000', kind: 'MONEY', displayValue: '$1,250,000', documentCount: 9, validationGrade: 'FORMAT_VALIDATED' },
  { entityId: 'ent-domain-harborcity', kind: 'EMAIL_DOMAIN', displayValue: 'harborcity.example', documentCount: 9, validationGrade: 'FORMAT_VALIDATED' },
  { entityId: 'ent-ein-073318842', kind: 'EIN', displayValue: '07-3318842', documentCount: 6, validationGrade: 'FORMAT_VALIDATED' },
  { entityId: 'ent-doc-extension', kind: 'DOCUMENT_CLASS', displayValue: 'Extension notice', documentCount: 6, validationGrade: 'PATTERN_ONLY' },
  { entityId: 'ent-position-cfo', kind: 'POSITION', displayValue: 'Chief Business Official', documentCount: 5, validationGrade: 'PATTERN_ONLY' },
  { entityId: 'ent-parcel-104-221-07', kind: 'PARCEL_NUMBER', displayValue: 'APN 104-221-07', documentCount: 3, validationGrade: 'FORMAT_VALIDATED' },
];
const entityById = new Map(ENTITIES.map((entity) => [entity.entityId, entity]));
function entity(entityId: string): T.CorpusEntityNode {
  const found = entityById.get(entityId);
  if (!found) throw new Error(`sample data: unknown entity ${entityId}`);
  return found;
}

const BROKERS: T.EntityBroker[] = [
  { entity: entity('ent-statute-7922000'), brokerageScore: 412, distinctNeighbors: 18, isApproximation: true },
  { entity: entity('ent-org-hcpd'), brokerageScore: 377, distinctNeighbors: 16, isApproximation: true },
  { entity: entity('ent-statute-7922535'), brokerageScore: 301, distinctNeighbors: 14, isApproximation: true },
  { entity: entity('ent-org-westshore'), brokerageScore: 188, distinctNeighbors: 9, isApproximation: true },
  { entity: entity('ent-role-custodian'), brokerageScore: 164, distinctNeighbors: 11, isApproximation: true },
  { entity: entity('ent-org-bayview'), brokerageScore: 152, distinctNeighbors: 8, isApproximation: true },
];

// --- Corpus patterns --------------------------------------------------------------------------

function evidenceRef(documentId: number, quote: string | null, label: string): T.CorpusEvidenceReference {
  const source = doc(documentId);
  const start = quote ? source.text.indexOf(quote) : -1;
  return {
    portalKey: source.portalKey,
    requestId: source.requestId,
    documentId,
    segmentIndex: start < 0 ? null : 0,
    characterStart: start < 0 ? null : start,
    characterEnd: start < 0 || !quote ? null : start + quote.length,
    evidenceLabel: label,
    contentSha256: hex64(`text:${source.portalKey}:${documentId}`),
  };
}

const PATTERNS: T.CorpusPattern[] = [
  { patternId: 'pat-near-dup-balancing', kind: 'NEAR_DUPLICATE', title: 'Near-identical denial letters at two agencies', explanation: 'Two denial letters share the same balancing-test sentence and closing structure; only the agency header and request number differ.', confidenceBasisPoints: 9100, evidence: [evidenceRef(5103, BOILERPLATE, 'Harbor City PD 26-1158'), evidenceRef(5521, BOILERPLATE, 'Cedar Hollow CH-5531')], algorithmVersion: 'deterministic-corpus-v3' },
  { patternId: 'pat-statute-7922000', kind: 'SHARED_STATUTE', title: 'Gov. Code § 7922.000 cited as the sole ground in full denials', explanation: 'The balancing-test section is the only cited authority in these full denials.', confidenceBasisPoints: 8600, evidence: [evidenceRef(5103, 'Gov. Code § 7922.000', '26-1158'), evidenceRef(5521, 'Gov. Code § 7922.000', 'CH-5531')], algorithmVersion: 'deterministic-corpus-v3' },
  { patternId: 'pat-statute-7922535', kind: 'SHARED_STATUTE', title: 'Extension authority cited across agencies', explanation: 'Documents from two agencies cite the CPRA extension provision.', confidenceBasisPoints: 8000, evidence: [evidenceRef(5101, '7922.535(b)(2)', '26-1142 response'), evidenceRef(5501, '7922.535(c)(1)', 'CH-5520 notice')], algorithmVersion: 'deterministic-corpus-v3' },
  { patternId: 'pat-entity-westshore', kind: 'SHARED_ENTITY', title: 'Westshore Facilities Group LLC across contract, register and agenda', explanation: 'The same vendor name appears in the amendment, the payment register, and an upcoming board agenda.', confidenceBasisPoints: 8400, evidence: [evidenceRef(5201, 'Westshore Facilities Group LLC', 'Amendment No. 3'), evidenceRef(5202, 'Westshore Facilities Group LLC', 'Payment register'), evidenceRef(5203, 'Westshore Facilities Group LLC', 'Board agenda')], algorithmVersion: 'deterministic-corpus-v3' },
  { patternId: 'pat-entity-uf-0431', kind: 'SHARED_ENTITY', title: 'Incident 26-UF-0431 in two quarterly summaries', explanation: 'The same incident number appears in consecutive quarterly releases.', confidenceBasisPoints: 9300, evidence: [evidenceRef(5102, 'Incident 26-UF-0431', 'Q2 summary'), evidenceRef(5104, 'Incident 26-UF-0431', 'Q3 summary')], algorithmVersion: 'deterministic-corpus-v3' },
  { patternId: 'pat-term-fuel-card', kind: 'SHARED_TERM', title: '"fuel card" in a no-records response and a later release', explanation: 'The term appears in both the no-records response to MC-3310 and the statements released under MC-3318.', confidenceBasisPoints: 7400, evidence: [evidenceRef(5301, 'fleet fuel card statements', 'MC-3310'), evidenceRef(5302, 'Fuel Card Statement', 'MC-3318')], algorithmVersion: 'deterministic-corpus-v3' },
  { patternId: 'pat-temporal-invoices', kind: 'TEMPORAL_CLUSTER', title: 'Three same-amount invoices within 48 hours', explanation: 'Invoices INV-3350 through INV-3352 were paid within two days of each other.', confidenceBasisPoints: 7800, evidence: [evidenceRef(5202, 'INV-3350', 'INV-3350'), evidenceRef(5202, 'INV-3351', 'INV-3351'), evidenceRef(5202, 'INV-3352', 'INV-3352')], algorithmVersion: 'deterministic-corpus-v3' },
  { patternId: 'pat-rel-bwc-policy', kind: 'ENTITY_RELATIONSHIP', title: 'Body-worn camera policy: council reading and department request', explanation: 'Council agendas and minutes reference the same policy revision that a pending request to the department covers.', confidenceBasisPoints: 6900, evidence: [evidenceRef(7001, 'body-worn camera policy revisions', 'Council agenda'), evidenceRef(7002, 'body-worn camera policy revisions', 'Council minutes')], algorithmVersion: 'deterministic-corpus-v3' },
];

// --- Agency profiles --------------------------------------------------------------------------

const GLOBAL_MEAN = 11.2;
const GLOBAL_STDDEV = 4.9;
function profile(agencyId: number, numbers: {
  total: number; closed: number; sample: number; excluded: number; average: number | null;
  indicator: T.AgencyPerformanceProfile['complianceIndicator']; anomalies: number; violations: number;
  fast: number; released: number; keywords: Record<string, number>;
}): T.AgencyPerformanceProfile {
  const agency = agencyById.get(agencyId) as Agency;
  const z = numbers.average === null || numbers.sample === 0
    ? null
    : Math.round(((numbers.average - GLOBAL_MEAN) / GLOBAL_STDDEV) * 100) / 100;
  return {
    agencyId,
    portalSubdomain: agency.portalKey,
    agencyName: agency.name,
    totalRequestsProcessed: numbers.total,
    totalClosed: numbers.closed,
    validLatencySampleSize: numbers.sample,
    excludedInvalidLifetimeCount: numbers.excluded,
    averageClosureDays: numbers.average,
    globalMeanClosureDays: GLOBAL_MEAN,
    globalStddevClosureDays: GLOBAL_STDDEV,
    performanceZScore: z,
    complianceIndicator: numbers.indicator,
    anomalyCount: numbers.anomalies,
    statutoryViolationCount: numbers.violations,
    statutoryViolationRate: numbers.violations === 0 ? 0 : Math.round((numbers.violations / numbers.closed) * 1000) / 1000,
    suspiciousFastClosureCount: numbers.fast,
    totalDocumentsReleased: numbers.released,
    keywordFrequencyMap: numbers.keywords,
  };
}

const AGENCY_PROFILES: T.AgencyPerformanceProfile[] = [
  profile(101, { total: 412, closed: 371, sample: 362, excluded: 3, average: 22.6, indicator: 'LatencyOutlier', anomalies: 4, violations: 0, fast: 0, released: 1844, keywords: { 'use of force': 38, 'body-worn camera': 27, overtime: 19, pursuit: 11 } }),
  profile(102, { total: 238, closed: 221, sample: 219, excluded: 0, average: 9.8, indicator: 'Compliant', anomalies: 1, violations: 0, fast: 0, released: 960, keywords: { contract: 44, amendment: 21, transportation: 12, superintendent: 6 } }),
  profile(103, { total: 156, closed: 149, sample: 141, excluded: 2, average: 6.1, indicator: 'Compliant', anomalies: 6, violations: 0, fast: 4, released: 402, keywords: { 'fuel card': 9, ballot: 14, overtime: 7, recorder: 11 } }),
  profile(104, { total: 97, closed: 90, sample: 88, excluded: 1, average: 12.7, indicator: 'Compliant', anomalies: 0, violations: 0, fast: 0, released: 311, keywords: { 'rate study': 13, groundwater: 9, 'main break': 6 } }),
  profile(105, { total: 104, closed: 88, sample: 86, excluded: 0, average: 15.9, indicator: 'StatutoryDeadlineExceeded', anomalies: 9, violations: 7, fast: 0, released: 377, keywords: { encampment: 16, 'code enforcement': 22, parking: 8, overtime: 10 } }),
  profile(106, { total: 14, closed: 0, sample: 0, excluded: 0, average: null, indicator: 'Compliant', anomalies: 0, violations: 0, fast: 0, released: 0, keywords: { 'bus stop': 5, fare: 4 } }),
];

// --- Cross-agency correlations ----------------------------------------------------------------

function correlation(requestId: number, block: string, complaints: number, sources: number, keywordSignals: number, firstDaysAgo: number, lastDaysAgo: number): T.CrossAgencyCorrelation {
  const record = SEARCH_RESULTS.find((candidate) => candidate.requestId === requestId) as T.RequestSearchResult;
  return {
    portalKey: record.portalKey,
    requestId,
    agencyId: record.agencyId,
    agencyName: record.agencyName,
    requestTitle: record.title,
    requestTimestamp: record.createdAt,
    neighborhoodBlock: block,
    complaintCount: complaints,
    correlationWindowHours: 168,
    distinctSourceCount: sources,
    firstCorrelatedEventAt: iso(firstDaysAgo),
    lastCorrelatedEventAt: iso(lastDaysAgo),
    keywordSignalCount: keywordSignals,
    correlationDensity: Math.round((complaints / 168) * 1000) / 1000,
  };
}

const CORRELATIONS: T.CrossAgencyCorrelation[] = [
  correlation(5531, 'Riverside Ave 1400 block', 23, 3, 9, 55, 50),
  correlation(5520, 'Ward 3 - Linden St 200 block', 17, 2, 6, 73, 67),
  correlation(6602, 'Harbor Blvd 900 block', 12, 2, 4, 37, 32),
  correlation(4416, 'Orchard Rd 3100 block', 9, 2, 3, 18, 13),
];

// --- Meetings ---------------------------------------------------------------------------------

const MEETING_CALENDAR: T.MeetingCalendar = {
  today: ymd(0),
  meetings: [
    {
      id: 'mtg-harborcity-council-upcoming',
      jurisdiction: 'Harbor City',
      body: 'Harbor City Council',
      bodyCitation: cite(7001, 'HARBOR CITY COUNCIL'),
      date: ymd(6),
      dateCitation: cite(7001, `Regular Meeting Agenda - ${ymd(6)}`),
      dateNote: null,
      timing: 'UPCOMING',
      documents: [{ role: 'AGENDA', portalKey: 'harborcity-council', documentId: 7001, label: 'Regular meeting agenda', sourceUrl: 'https://agendas.harborcity.example/council/agenda-regular.pdf' }],
      commentDeadlines: [cite(7001, 'Written comments received by 5:00 p.m. the day before the meeting will be distributed to the Council.')],
      commentInstructions: [cite(7001, 'Submit comments to clerk@harborcity.example or in person at the City Clerk counter.')],
      suggestions: [{
        id: 'sug-hc-item-4',
        agendaItem: 'Item 4',
        title: 'Body-worn camera policy revisions',
        itemCitation: cite(7001, 'Item 4. Second reading: Police Department body-worn camera policy revisions.'),
        reasons: [{
          kind: 'WATCHED_AGENCY',
          basis: 'STATED',
          reason: 'You follow Harbor City Police Department, and a request for its body-worn camera policy is in your corpus.',
          matched: 'body-worn camera policy',
          citation: cite(7001, 'body-worn camera policy revisions'),
          findingId: null,
          gapKey: null,
          claimCheck: null,
        }],
        comment: {
          text: 'For Item 4: please publish the activation audit logs referenced in the current policy alongside the revised policy, so the public can compare stated practice with recorded activations.',
          bases: ['STATED'],
          deadline: cite(7001, 'Written comments received by 5:00 p.m. the day before the meeting will be distributed to the Council.'),
          instructions: [cite(7001, 'Submit comments to clerk@harborcity.example or in person at the City Clerk counter.')],
        },
      }],
    },
    {
      id: 'mtg-bayview-board-upcoming',
      jurisdiction: 'Bayview Unified School District',
      body: 'Board of Education',
      bodyCitation: cite(5203, 'BAYVIEW UNIFIED SCHOOL DISTRICT'),
      date: ymd(13),
      dateCitation: cite(5203, `Board of Education - Regular Meeting Agenda - ${ymd(13)}`),
      dateNote: null,
      timing: 'UPCOMING',
      documents: [{ role: 'AGENDA', portalKey: 'bayview-usd', documentId: 5203, label: 'Regular meeting agenda', sourceUrl: 'https://publicrecords.bayviewusd.example/documents/board-agenda-regular.pdf' }],
      commentDeadlines: [],
      commentInstructions: [cite(5203, 'Public comment cards must be submitted to the Board secretary before the item is called.')],
      suggestions: [{
        id: 'sug-bv-consent-12',
        agendaItem: 'Consent Item 12',
        title: 'Amendment No. 4 to Agreement 24-FAC-019',
        itemCitation: cite(5203, 'Consent Item 12. Ratify Amendment No. 4 to Agreement 24-FAC-019, Westshore Facilities Group LLC.'),
        reasons: [{
          kind: 'FINDING_SUBJECT',
          basis: 'COUNTED',
          reason: 'A finding records payments under this agreement above its stated not-to-exceed amount.',
          matched: '24-FAC-019',
          citation: cite(5202, 'Total paid under 24-FAC-019: 1,496,850.00'),
          findingId: 'fnd-contract-ceiling-24fac019',
          gapKey: null,
          claimCheck: null,
        }],
        comment: {
          text: 'On Consent Item 12: the released payment register totals $1,496,850.00 under Agreement 24-FAC-019, while Amendment No. 3 states a $1,250,000 not-to-exceed amount. Please pull this item from consent and explain whether Amendment No. 4 ratifies payments already made.',
          bases: ['COUNTED'],
          deadline: null,
          instructions: [cite(5203, 'Public comment cards must be submitted to the Board secretary before the item is called.')],
        },
      }],
    },
    {
      id: 'mtg-harborcity-council-past',
      jurisdiction: 'Harbor City',
      body: 'Harbor City Council',
      bodyCitation: cite(7002, 'HARBOR CITY COUNCIL'),
      date: ymd(-15),
      dateCitation: cite(7002, `Regular Meeting Minutes - ${ymd(-15)}`),
      dateNote: null,
      timing: 'PAST',
      documents: [{ role: 'MINUTES', portalKey: 'harborcity-council', documentId: 7002, label: 'Regular meeting minutes', sourceUrl: 'https://agendas.harborcity.example/council/minutes-regular.pdf' }],
      commentDeadlines: [],
      commentInstructions: [],
      suggestions: [],
    },
    {
      id: 'mtg-alder-valley-board-past',
      jurisdiction: 'Alder Valley Water District',
      body: 'Board of Directors',
      bodyCitation: cite(5401, 'ALDER VALLEY WATER DISTRICT'),
      date: ymd(-9),
      dateCitation: cite(5401, `Regular Meeting of the Board of Directors - Minutes - ${ymd(-9)}`),
      dateNote: null,
      timing: 'PAST',
      documents: [{ role: 'MINUTES', portalKey: 'alder-valley-water', documentId: 5401, label: 'Board minutes', sourceUrl: 'https://records.aldervalleywater.example/documents/board-minutes.pdf' }],
      commentDeadlines: [],
      commentInstructions: [],
      suggestions: [],
    },
    {
      id: 'mtg-marlow-bos-undated',
      jurisdiction: 'Marlow County',
      body: 'Board of Supervisors',
      bodyCitation: cite(5303, 'MARLOW COUNTY BOARD OF SUPERVISORS'),
      date: null,
      dateCitation: null,
      dateNote: 'The agenda packet carries no dated heading.',
      timing: 'DATE_NOT_RECORDED',
      documents: [{ role: 'AGENDA', portalKey: 'marlow-county-clerk', documentId: 5303, label: 'Agenda packet', sourceUrl: 'https://clerk.marlowcounty.example/bos/agenda-packet.pdf' }],
      commentDeadlines: [],
      commentInstructions: [],
      suggestions: [],
    },
  ],
  limitations: [
    'Meetings are read only from agendas and minutes already collected; a body with no collected documents does not appear.',
    'Comment deadlines are shown only where a collected document states one.',
  ],
};

// --- Story leads ------------------------------------------------------------------------------

function lead(findingId: string, total: number, factors: T.InterestFactor[], related: string[]): T.StoryLead {
  const finding = findingById.get(findingId) as T.Finding;
  return {
    findingId,
    kind: finding.kind,
    headline: finding.headline,
    severity: finding.severity,
    confidence: finding.confidence,
    subject: finding.subject,
    portalKey: finding.subject.portalKey,
    publicInterest: { totalBasisPoints: total, factors },
    relatedFindingIds: related,
    firstSeenAt: finding.discoveredAt,
    lastSeenAt: finding.lastSeenAt,
  };
}

const STORY_LEADS: T.StoryLead[] = [
  lead('fnd-contract-ceiling-24fac019', 8200, [
    { name: 'Public funds', basisPoints: 3500, evidence: '$246,850.00 paid above the stated ceiling' },
    { name: 'Upcoming decision', basisPoints: 2700, evidence: 'Amendment No. 4 is on the next Board consent calendar' },
    { name: 'Corroborated', basisPoints: 2000, evidence: 'Two independent documents: amendment and payment register' },
  ], ['fnd-split-purchase-24fac019']),
  lead('fnd-no-records-contradicted-3310', 7100, [
    { name: 'Response accuracy', basisPoints: 4000, evidence: 'A no-records answer was followed by a matching release' },
    { name: 'Corroborated', basisPoints: 3100, evidence: 'Response letter and released statement both collected' },
  ], []),
  lead('fnd-boilerplate-denial-1158', 5400, [
    { name: 'Repeated across agencies', basisPoints: 3000, evidence: 'Identical wording in two agencies\' denials' },
    { name: 'Full denial', basisPoints: 2400, evidence: 'Request denied in its entirety' },
  ], ['fnd-shared-language-balancing']),
  lead('fnd-redaction-inconsistent-0431', 4300, [
    { name: 'Disclosure inconsistency', basisPoints: 4300, evidence: 'Same field withheld in one release, disclosed in the next' },
  ], []),
];

// --- Evidence gaps ----------------------------------------------------------------------------

const EVIDENCE_GAPS: T.EvidenceGapView[] = [
  { detector: 'spending_cases', key: 'bayview-usd:approval-threshold-policy', route: 'Test whether the $49,800 invoices avoided a purchasing approval threshold', blocked: 'No purchasing policy with an approval threshold has been collected for Bayview Unified School District.', role: 'Purchasing policy', request: 'Board policy and administrative regulation on purchasing approval thresholds, current version', status: 'noAutomaticSource', sources: [] },
  { detector: 'email_thread_gap', key: 'alder-valley-water:4410:exhibit-c', route: 'Check whether correspondence exists between March 19 and May 22', blocked: 'Only Exhibit C of the board packet has been collected; the full correspondence file has not.', role: 'Correspondence file', request: 'All correspondence with the rate study consultant, March 20 through May 21', status: 'acquiring', sources: ['https://records.aldervalleywater.example/requests/4410'] },
  { detector: 'practice_departure', key: 'cedar-hollow:records-policy', route: "Compare Cedar Hollow's extension use with its own records policy", blocked: 'The search of the city website found no administrative policy on public records response times.', role: 'Records policy', request: 'Administrative policy governing public records request processing', status: 'exhausted', sources: ['https://cedarhollow.records.example/'] },
];

// --- Resolution -------------------------------------------------------------------------------

function snapshot(kind: string, value: string, displayValue: string, documentCount: number, documentId: number): T.ResolutionEntitySnapshot {
  const source = doc(documentId);
  const start = Math.max(0, source.text.indexOf(value));
  return {
    key: { kind, value: value.toLowerCase() },
    displayValue,
    documentCount,
    examples: [{ portalKey: source.portalKey, documentId, characterStart: start, characterEnd: start + value.length }],
  };
}

const LEDGER_HEAD = hex64('resolution-ledger-head');

const RESOLUTION_CANDIDATES: T.ResolutionCandidatePage = {
  candidates: [
    {
      pairId: 'pair-westshore-llc-grp',
      leftEntityId: 'ent-org-westshore',
      rightEntityId: 'ent-org-westshore-grp',
      evidence: {
        basis: 'STRONG_IDENTIFIER',
        left: snapshot('ORGANIZATION', 'Westshore Facilities Group LLC', 'Westshore Facilities Group LLC', 14, 5201),
        right: snapshot('ORGANIZATION', 'Westshore Facilities Group', 'Westshore Facilities Grp.', 3, 5202),
        sharedIdentifier: { kind: 'EIN', displayValue: '07-3318842', leftDocuments: 4, rightDocuments: 2 },
        nameVariants: ['Westshore Facilities Group LLC', 'Westshore Facilities Grp.'],
      },
    },
    {
      pairId: 'pair-hcpd-dept',
      leftEntityId: 'ent-org-hcpd',
      rightEntityId: 'ent-org-hcpd-abbrev',
      evidence: {
        basis: 'NAME_ONLY',
        left: snapshot('ORGANIZATION', 'Harbor City Police Department', 'Harbor City Police Department', 41, 5101),
        right: snapshot('ORGANIZATION', 'Police Department', 'Harbor City Police Dept.', 7, 7001),
        sharedIdentifier: null,
        nameVariants: ['Harbor City Police Department', 'Harbor City Police Dept.'],
      },
    },
    {
      pairId: 'pair-custodian-role',
      leftEntityId: 'ent-role-custodian',
      rightEntityId: 'ent-role-custodian-alt',
      evidence: {
        basis: 'NAME_ONLY',
        left: snapshot('ROLE', 'Records Unit', 'Records Custodian', 13, 5101),
        right: snapshot('ROLE', 'Office of the City Clerk', 'Custodian of Records', 4, 5521),
        sharedIdentifier: null,
        nameVariants: ['Records Custodian', 'Custodian of Records'],
      },
    },
  ],
  nextCursor: null,
  remaining: 0,
  scanTruncated: false,
  guards: { skippedNameGroups: 2, skippedIdentifiers: 0 },
  ledgerHead: LEDGER_HEAD,
};

function ledgerItem(id: string, verdict: T.ResolutionVerdict, decidedDaysAgo: number, reason: string, evidence: T.ResolutionEvidence, isLatest = true): T.ResolutionLedgerItem {
  return {
    entry: { id, left: evidence.left.key, right: evidence.right.key, verdict, author: 'Sample Reporter', decidedAt: iso(decidedDaysAgo), reason, supersedes: null, evidence },
    importedFrom: null,
    hash: hex64(`ledger:${id}`),
    isLatest,
  };
}

const LEDGER_ITEMS: T.ResolutionLedgerItem[] = [
  ledgerItem('led-cedar-clerk', 'SAME', 21, 'Both letters are signed by the clerk\'s office of the same city and share a mailing block.', {
    basis: 'NAME_ONLY',
    left: snapshot('ORGANIZATION', 'Office of the City Clerk', 'Office of the City Clerk', 6, 5521),
    right: snapshot('ORGANIZATION', 'OFFICE OF THE CITY CLERK', 'City of Cedar Hollow - Office of the City Clerk', 4, 5501),
    sharedIdentifier: null,
    nameVariants: ['Office of the City Clerk', 'City of Cedar Hollow - Office of the City Clerk'],
  }),
  ledgerItem('led-bayview-middle', 'DIFFERENT', 14, 'A school site is not the district that operates it.', {
    basis: 'NAME_ONLY',
    left: snapshot('ORGANIZATION', 'Bayview Middle School', 'Bayview Middle School', 3, 5201),
    right: snapshot('ORGANIZATION', 'BAYVIEW UNIFIED SCHOOL DISTRICT', 'Bayview Unified School District', 24, 5201),
    sharedIdentifier: null,
    nameVariants: ['Bayview Middle School', 'Bayview Unified School District'],
  }),
  ledgerItem('led-fleet-services', 'UNSURE', 7, 'The statement header names Fleet Services; the response names the Clerk-Recorder. Waiting on an org chart.', {
    basis: 'NAME_ONLY',
    left: snapshot('ORGANIZATION', 'MARLOW COUNTY FLEET SERVICES', 'Marlow County Fleet Services', 5, 5302),
    right: snapshot('ORGANIZATION', 'MARLOW COUNTY CLERK-RECORDER', 'Marlow County Clerk-Recorder', 11, 5301),
    sharedIdentifier: null,
    nameVariants: ['Marlow County Fleet Services', 'Marlow County Clerk-Recorder'],
  }),
];

const RESOLUTION_LEDGER: T.ResolutionLedgerPage = {
  items: LEDGER_ITEMS,
  nextCursor: null,
  total: LEDGER_ITEMS.length,
  entryCount: LEDGER_ITEMS.length,
  head: LEDGER_HEAD,
  summary: { byBasis: [{ basis: 'NAME_ONLY', same: 1, different: 1, unsure: 1 }] },
  contradictions: [],
};

// --- Tracker (claimed requests + deadlines) ---------------------------------------------------

function emailEvidence(seed: string, subjectExcerpt: string, emailDaysAgo: number, sentence: string): T.EmailEvidence {
  const excerpt = `Good afternoon. ${sentence} Thank you for your patience.`;
  const sentenceStart = new TextEncoder().encode('Good afternoon. ').length;
  return {
    messageIdHash: hex64(`msg:${seed}`),
    subjectExcerpt,
    emailDate: iso(emailDaysAgo),
    excerpt,
    sentenceStart,
    sentenceEnd: sentenceStart + new TextEncoder().encode(sentence).length,
    matchedSentence: sentence,
  };
}

function event(id: string, kind: T.TimelineEventKind, daysAgo: number, evidence: T.EmailEvidence | null = null, extension?: { reason: T.StatutoryExtensionReason; expectedDaysAgo: number }): T.TimelineEvent {
  return {
    id,
    kind,
    occurredAt: iso(daysAgo),
    basis: evidence ? 'EMAIL_CONFIRMED' : 'USER_ENTERED',
    evidence,
    extensionReason: extension?.reason ?? null,
    expectedDeterminationAt: extension ? iso(extension.expectedDaysAgo) : null,
    createdAt: iso(daysAgo, 30),
  };
}

const CLAIMED_REQUESTS: T.ClaimedRequest[] = [
  {
    id: 'claim-hcpd-1142',
    portalKey: 'harborcity-pd',
    reference: '26-1142',
    title: 'Use-of-force review summaries, second quarter',
    sourceUrl: 'https://records.harborcity.example/requests/26-1142',
    jurisdiction: 'CA_CPRA',
    claimedAt: iso(40, 60),
    events: [
      event('ev-1142-submitted', 'SUBMITTED', 40),
      event('ev-1142-ack', 'ACKNOWLEDGED', 39, emailEvidence('1142-ack', 'Request 26-1142 received', 39, 'Your request 26-1142 has been received by the Records Unit.')),
      event('ev-1142-ext', 'EXTENSION_NOTICE', 31, emailEvidence('1142-ext', 'Request 26-1142: extension', 31, 'Due to the volume of records, the Department is extending the time to respond by 14 days.'), { reason: 'VOLUMINOUS_RECORDS', expectedDaysAgo: 16 }),
      event('ev-1142-partial', 'PARTIAL_PRODUCTION', 26),
    ],
    suggestions: [{
      id: 'sug-1142-installment',
      kind: 'PARTIAL_PRODUCTION',
      candidateOccurredAt: iso(4),
      evidence: emailEvidence('1142-inst2', 'Request 26-1142: second installment', 4, 'A second installment of records is now available in the portal.'),
      suggestedAt: iso(3),
    }],
    importedEmails: [
      { messageIdHash: hex64('msg:1142-ack'), subjectExcerpt: 'Request 26-1142 received', emailDate: iso(39), importedAt: iso(38), suggestionCount: 0 },
      { messageIdHash: hex64('msg:1142-ext'), subjectExcerpt: 'Request 26-1142: extension', emailDate: iso(31), importedAt: iso(30), suggestionCount: 0 },
      { messageIdHash: hex64('msg:1142-inst2'), subjectExcerpt: 'Request 26-1142: second installment', emailDate: iso(4), importedAt: iso(3), suggestionCount: 1 },
    ],
  },
  {
    id: 'claim-bayview-2087',
    portalKey: 'bayview-usd',
    reference: '26-2087',
    title: 'Facilities contract amendments, Westshore Facilities Group',
    sourceUrl: 'https://publicrecords.bayviewusd.example/requests/26-2087',
    jurisdiction: 'CA_CPRA',
    claimedAt: iso(6, 30),
    events: [event('ev-2087-submitted', 'SUBMITTED', 6), event('ev-2087-ack', 'ACKNOWLEDGED', 5)],
    suggestions: [],
    importedEmails: [],
  },
  {
    id: 'claim-cedar-5520',
    portalKey: 'cedar-hollow',
    reference: 'CH-5520',
    title: 'Code enforcement complaint logs, Ward 3',
    sourceUrl: 'https://cedarhollow.records.example/requests/CH-5520',
    jurisdiction: 'CA_CPRA',
    claimedAt: iso(70, 45),
    events: [
      event('ev-5520-submitted', 'SUBMITTED', 70),
      event('ev-5520-ext', 'EXTENSION_NOTICE', 62, emailEvidence('5520-ext', 'Notice of Extension - Request CH-5520', 62, 'The City expects to issue its determination within 14 additional days.'), { reason: 'SEPARATE_FIELD_FACILITIES', expectedDaysAgo: 46 }),
      event('ev-5520-det', 'DETERMINATION', 39, emailEvidence('5520-det', 'Request CH-5520 determination', 39, 'The City has completed its review and will produce responsive records.')),
      event('ev-5520-released', 'RECORDS_RELEASED', 39),
      event('ev-5520-closed', 'CLOSED', 39),
    ],
    suggestions: [],
    importedEmails: [
      { messageIdHash: hex64('msg:5520-ext'), subjectExcerpt: 'Notice of Extension - Request CH-5520', emailDate: iso(62), importedAt: iso(38), suggestionCount: 0 },
      { messageIdHash: hex64('msg:5520-det'), subjectExcerpt: 'Request CH-5520 determination', emailDate: iso(39), importedAt: iso(38), suggestionCount: 0 },
    ],
  },
  {
    id: 'claim-marlow-3329',
    portalKey: 'marlow-county-clerk',
    reference: 'MC-3329',
    title: 'Board of Supervisors agenda packets',
    sourceUrl: 'https://clerk.marlowcounty.example/records/MC-3329',
    jurisdiction: null,
    claimedAt: iso(9, 20),
    events: [event('ev-3329-submitted', 'SUBMITTED', 9)],
    suggestions: [],
    importedEmails: [],
  },
];

const CPRA = 'California Government Code § 7922.535';
const COURT_CALENDAR = 'calendar days are counted from receipt and a deadline falling on a weekend or holiday moves to the next business day';

function deadlineAnalysis(requestId: number, createdDaysAgo: number, allowed: number, determinationDaysAgo: number | null, determinationSource: string | null, basis: T.DeadlineDeterminationBasis, extensionStatus: string, extensionReason: string | null): T.DeadlineAnalysis {
  const endDaysAgo = determinationDaysAgo ?? 0;
  const elapsed = createdDaysAgo - endDaysAgo;
  const isViolation = elapsed > allowed;
  const milestone = determinationDaysAgo === null ? 'observation' : 'determination';
  const sourceClause = determinationSource ? `; determination source: ${determinationSource}` : '';
  return {
    requestId,
    statuteCitation: CPRA,
    createdAt: iso(createdDaysAgo),
    determinationAt: determinationDaysAgo === null ? null : iso(determinationDaysAgo),
    determinationSource,
    determinationBasis: basis,
    observedAt: iso(0),
    initialDeadlineAt: iso(createdDaysAgo - 10),
    effectiveDeadlineAt: iso(createdDaysAgo - allowed),
    calendarDaysElapsed: elapsed,
    allowedCalendarDays: allowed,
    extensionStatus,
    extensionReason,
    isViolation,
    anomalyMarkers: isViolation && basis === 'SOURCED_DETERMINATION_EVENT' ? ['STATUTORY_LATENCY_VIOLATION'] : [],
    evidenceSummary: `Request ${requestId}: ${elapsed} calendar days from receipt to ${milestone}${sourceClause}; effective deadline was ${iso(createdDaysAgo - allowed)} under ${CPRA}, where ${COURT_CALENDAR}; extension status ${extensionStatus === 'VALID' ? 'Valid' : 'NotInvoked'}.`,
  };
}

function cpraDeadline(claimId: string, analysis: T.DeadlineAnalysis, pendingSuggestionCount: number): T.RequestDeadline {
  const basis = analysis.determinationBasis;
  const violationAsserted = analysis.isViolation && basis === 'SOURCED_DETERMINATION_EVENT';
  const remaining = analysis.allowedCalendarDays - analysis.calendarDaysElapsed;
  const countdown: T.Countdown = basis !== 'OBSERVATION_WITHOUT_RECORDED_ACTIVITY'
    ? { kind: 'DETERMINATION_RECORDED', days: null }
    : remaining >= 0 ? { kind: 'DAYS_REMAINING', days: remaining } : { kind: 'DEADLINE_PASSED', days: -remaining };
  const counted = `${analysis.calendarDaysElapsed} of ${analysis.allowedCalendarDays} allowed calendar days from submission`;
  const basisStatement = basis === 'SOURCED_DETERMINATION_EVENT'
    ? `Measured to a determination on ${(analysis.determinationAt ?? '').slice(0, 10)} that you confirmed from an email: ${counted}. ${violationAsserted ? `This exceeds the allowed period: a violation of ${CPRA} is counted.` : 'This is within the allowed period.'}`
    : `No determination is recorded, so the period is measured to today: ${counted}. This is provisional: the agency may have responded outside this record.`;
  return {
    claimId,
    model: 'CALIFORNIA_CPRA',
    analysis,
    countdown,
    determinationBasis: basis,
    violationAsserted,
    basisStatement,
    notes: pendingSuggestionCount > 0 ? [`${pendingSuggestionCount} suggested event(s) from emails await your confirmation and are not counted.`] : [],
    pendingSuggestionCount,
  };
}

const DEADLINES: Record<string, T.RequestDeadline> = {
  'claim-hcpd-1142': cpraDeadline('claim-hcpd-1142', deadlineAnalysis(1142, 40, 24, null, null, 'OBSERVATION_WITHOUT_RECORDED_ACTIVITY', 'VALID', 'VOLUMINOUS_RECORDS'), 1),
  'claim-bayview-2087': cpraDeadline('claim-bayview-2087', deadlineAnalysis(2087, 6, 10, null, null, 'OBSERVATION_WITHOUT_RECORDED_ACTIVITY', 'NOT_INVOKED', null), 0),
  'claim-cedar-5520': cpraDeadline('claim-cedar-5520', deadlineAnalysis(5520, 70, 24, 39, hex64('msg:5520-det'), 'SOURCED_DETERMINATION_EVENT', 'VALID', 'SEPARATE_FIELD_FACILITIES'), 0),
  'claim-marlow-3329': {
    claimId: 'claim-marlow-3329',
    model: 'NO_MODEL',
    analysis: null,
    countdown: { kind: 'NO_DEADLINE_MODEL', days: null },
    determinationBasis: null,
    violationAsserted: false,
    basisStatement: 'No deadline model for this jurisdiction. No jurisdiction is set, and this app models only California CPRA deadlines, so no deadline is computed.',
    notes: [],
    pendingSuggestionCount: 0,
  },
};

// --- Profile ----------------------------------------------------------------------------------

const PROFILE: T.ProfileState = {
  status: 'SET',
  profile: {
    schemaVersion: 1,
    displayName: 'Sample Reporter',
    affiliations: [{ id: 'aff-civic-ledger', name: 'Bayside Civic Ledger', aliases: ['Civic Ledger'], basis: 'SELF_DECLARED', createdAt: iso(118), verifiedDomain: null }],
    matchDecisions: [],
    verifiedDomains: [],
    updatedAt: iso(118),
  },
};

const DISCLOSURE: T.AffiliationDisclosure = {
  status: 'DECLARED',
  sentence: 'Prepared by a user who has declared an affiliation with Bayside Civic Ledger (self-declared, not verified).',
  affiliations: [{ name: 'Bayside Civic Ledger', basis: 'SELF_DECLARED' }],
  ownPortalKeys: [],
  ownAgencies: [],
  ownEntityIds: [],
};

// --- Runtimes ---------------------------------------------------------------------------------

function runtime(portalKey: string, overrides: Partial<T.PortalRuntimeSnapshot>): T.PortalRuntimeSnapshot {
  const requests = SEARCH_RESULTS.filter((record) => record.portalKey === portalKey);
  const documents = DOCUMENTS.filter((candidate) => candidate.portalKey === portalKey).length;
  return {
    portal: reg(portalKey),
    syncRunId: `sync-${portalKey}-${hex64(portalKey).slice(0, 8)}`,
    queuedTasks: 0,
    activeTasks: 0,
    completedTasks: requests.length * 3 + documents,
    failedTasks: 0,
    pagesProcessed: Math.max(1, Math.ceil(requests.length / 2)),
    requestsProcessed: requests.length,
    documentsProcessed: documents,
    isRunning: false,
    rateLimitTokens: 10,
    frontierExclusions: [],
    stoppedHosts: [],
    ...overrides,
  };
}

const RUNTIMES: T.PortalRuntimeSnapshot[] = [
  runtime('harborcity-pd', {}),
  runtime('bayview-usd', { isRunning: true, queuedTasks: 14, activeTasks: 2, rateLimitTokens: 3 }),
  runtime('marlow-county-clerk', { frontierExclusions: [{ rule: 'exclude prefix /calendar/', excludedLinks: 37 }, { rule: 'exclude filename *.ics', excludedLinks: 12 }] }),
  runtime('alder-valley-water', {}),
  runtime('cedar-hollow', {}),
  runtime('harborcity-council', { frontierExclusions: [{ rule: 'same-host only', excludedLinks: 9 }] }),
  runtime('marlow-open-data', { failedTasks: 3, stoppedHosts: [{ host: 'data.marlowcounty.example', requestsSpent: 3, reason: 'HTTP 503 Service Unavailable after 3 retries' }] }),
];

// --- Analysis workbench -----------------------------------------------------------------------

const APP_DATA = '~/Library/Application Support/OnRecord Studio';

const ANALYTICAL_SCHEMA: T.AnalyticalSchema = {
  datasets: [
    { snapshotId: 'snap-bayview-budget-fy26', datasetKind: 'OPERATING_BUDGET', displayName: 'Bayview USD adopted operating budget', tableName: 'operating_budget_bayview_fy26', importedRows: 1284, rejectedRows: 0, sourceUri: 'https://publicrecords.bayviewusd.example/documents/adopted-budget.csv', sourceSha256: hex64('budget-bayview'), retrievedAt: iso(48), observationPeriodStart: ymd(-120), observationPeriodEnd: ymd(245), partialDataset: false, validationSummary: '1,284 rows imported; 0 rejected; fund and object codes matched the budget dimension table.' },
    { snapshotId: 'snap-hc-calls-q2', datasetKind: 'CALLS_FOR_SERVICE', displayName: 'Harbor City calls for service', tableName: 'calls_for_service_harborcity', importedRows: 48213, rejectedRows: 112, sourceUri: 'https://data.harborcity.example/calls-for-service.csv', sourceSha256: hex64('calls-hc'), retrievedAt: iso(31), observationPeriodStart: ymd(-118), observationPeriodEnd: ymd(-28), partialDataset: true, validationSummary: '48,213 rows imported; 112 rejected for unparseable timestamps; the publisher states the most recent month is preliminary.' },
  ],
  columns: [
    { tableName: 'operating_budget_bayview_fy26', columnName: 'fund', logicalType: 'VARCHAR', originalName: 'Fund', nullable: false, dictionary: 'budget_dimension.fund' },
    { tableName: 'operating_budget_bayview_fy26', columnName: 'object_code', logicalType: 'VARCHAR', originalName: 'Object', nullable: false, dictionary: 'budget_dimension.object' },
    { tableName: 'operating_budget_bayview_fy26', columnName: 'adopted_amount', logicalType: 'DECIMAL(14,2)', originalName: 'Adopted', nullable: false, dictionary: null },
    { tableName: 'calls_for_service_harborcity', columnName: 'received_at', logicalType: 'TIMESTAMP', originalName: 'Received Date/Time', nullable: false, dictionary: null },
    { tableName: 'calls_for_service_harborcity', columnName: 'call_type', logicalType: 'VARCHAR', originalName: 'Call Type', nullable: true, dictionary: 'calls_codebook.call_type' },
    { tableName: 'calls_for_service_harborcity', columnName: 'beat', logicalType: 'VARCHAR', originalName: 'Beat', nullable: true, dictionary: null },
  ],
  templates: [
    { templateId: 'tpl-calls-by-beat', title: 'Calls by beat and month', description: 'Counts calls for service per beat per month.', sql: "SELECT beat, date_trunc('month', received_at) AS month, count(*) AS calls FROM calls_for_service_harborcity GROUP BY ALL ORDER BY month, beat", denominator: 'All imported calls in the observation period' },
    { templateId: 'tpl-budget-by-fund', title: 'Adopted budget by fund', description: 'Sums adopted amounts per fund.', sql: 'SELECT fund, sum(adopted_amount) AS adopted FROM operating_budget_bayview_fy26 GROUP BY fund ORDER BY adopted DESC', denominator: 'Total adopted budget' },
  ],
};

const IMPORT_CANDIDATES: T.AnalyticalImportCandidate[] = [
  { documentId: 5202, portalKey: 'bayview-usd', fileName: 'Vendor_Payment_Register_FY26.csv', localPath: `${APP_DATA}/evidence/bayview-usd/5202/Vendor_Payment_Register_FY26.csv`, sourceUri: 'https://publicrecords.bayviewusd.example/documents/5202', sourceSha256: hex64('raw:bayview-usd:5202') },
  ...DOCUMENTS.filter((candidate) => candidate.extractionStatus === 'DEFERRED').slice(0, 2).map((candidate) => ({
    documentId: candidate.documentId,
    portalKey: candidate.portalKey,
    fileName: candidate.fileName,
    localPath: `${APP_DATA}/evidence/${candidate.portalKey}/${candidate.documentId}/${candidate.fileName}`,
    sourceUri: `https://${reg(candidate.portalKey).canonicalUrl.replace(/^https:\/\//, '').replace(/\/$/, '')}/documents/${candidate.documentId}`,
    sourceSha256: hex64(`raw:${candidate.portalKey}:${candidate.documentId}`),
  })),
];

function hybridSearch(args: { query: string; limit?: number | null }): T.HybridSearchResult[] {
  const terms = args.query.toLowerCase().split(/\s+/).filter(Boolean);
  const scored = DOCUMENTS
    .filter((candidate) => candidate.text.length > 0)
    .map((candidate) => {
      const lower = candidate.text.toLowerCase();
      const hits = terms.filter((term) => lower.includes(term)).length;
      return { candidate, hits };
    })
    .filter(({ hits }) => terms.length === 0 || hits > 0)
    .sort((a, b) => b.hits - a.hits || a.candidate.documentId - b.candidate.documentId)
    .slice(0, args.limit ?? 10);
  return scored.map(({ candidate, hits }, rank) => {
    const lower = candidate.text.toLowerCase();
    const first = terms.map((term) => lower.indexOf(term)).filter((index) => index >= 0);
    const start = first.length > 0 ? Math.max(0, Math.min(...first) - 40) : 0;
    const end = Math.min(candidate.text.length, start + 220);
    const agency = agencyByPortal.get(candidate.portalKey);
    const semantic = Math.round((0.82 - rank * 0.04) * 1000) / 1000;
    return {
      stableId: `${candidate.portalKey}:${candidate.documentId}:${start}`,
      portalKey: candidate.portalKey,
      requestId: candidate.requestId,
      documentId: candidate.documentId,
      fileName: candidate.fileName,
      title: SEARCH_RESULTS.find((record) => record.requestId === candidate.requestId)?.title ?? candidate.fileName,
      agencyName: agency?.name ?? reg(candidate.portalKey).displayName,
      jurisdiction: 'California',
      documentType: candidate.contentType === 'text/csv' ? 'Dataset' : 'Letter',
      publicationDate: null,
      observationDate: null,
      effectiveDate: null,
      characterStart: start,
      characterEnd: end,
      excerpt: candidate.text.slice(start, end),
      lexicalRank: hits > 0 ? rank + 1 : null,
      semanticScore: semantic,
      fusedScore: Math.round((1 / (60 + rank + 1)) * 2 * 10000) / 10000,
      retrievalStatus: hits > 0 ? 'HYBRID' : 'SEMANTIC_ONLY',
    };
  });
}

// --- Document viewer --------------------------------------------------------------------------

function provenanceRecord(portalKey: string, documentId: number): T.ProvenanceVerificationRecord {
  const raw = hex64(`raw:${portalKey}:${documentId}`);
  return {
    verificationId: `ver-${portalKey}-${documentId}`,
    documentId,
    portalKey,
    verifiedAt: iso(2),
    expectedSha256: raw,
    computedSha256: raw,
    evidenceFilePresent: true,
    hashMatches: true,
    manifestPresent: false,
    isCryptographicallySigned: false,
    signatureIssuer: null,
    signingCertificateSerialNumber: null,
    signingTimestamp: null,
    assertionCount: 0,
    invalidAssertionCount: 0,
    recordedDigestSource: 'INGESTION_RECORD',
    signatureVerification: 'NOT_ATTEMPTED',
    provenanceConfidence: 'LOCAL_DIGEST_MATCH',
    textProjectionSha256: hex64(`text:${portalKey}:${documentId}`),
    textProjectionMatches: true,
    chainOfCustodyConfirmed: false,
    verificationErrors: [],
    predatesClassification: false,
  };
}

// ---------------------------------------------------------------------------------------------

const answers: SampleAnswers = {
  get_affiliation_disclosure: DISCLOSURE,
  get_profile: PROFILE,
  get_ocr_capability: {
    engine: 'tesseract',
    engineAvailable: true,
    engineVersion: '5.4.1',
    languagePacks: ['eng', 'spa'],
    languageProbeComplete: true,
    pdfRenderer: 'pdftoppm',
    pdfRendererAvailable: true,
    pdfRendererVersion: '24.08.0',
    imageOcrAvailable: true,
    pdfOcrAvailable: true,
    diagnostics: [],
  },

  list_saved_portals: SAVED_SOURCES,
  list_portal_runtimes: RUNTIMES,

  get_agency_performance_profiles: AGENCY_PROFILES,
  get_cross_agency_correlations: (args) => CORRELATIONS
    .filter((row) => !args.portalKey || row.portalKey === args.portalKey)
    .filter((row) => args.minimumComplaints == null || row.complaintCount >= args.minimumComplaints)
    .slice(0, args.resultLimit ?? CORRELATIONS.length),

  query_audit_records_page: (args) => {
    const matches = matchingRecords(args);
    const offset = args.cursor ? Number.parseInt(args.cursor, 10) || 0 : 0;
    const page = matches.slice(offset, offset + args.pageSize);
    const next = offset + page.length;
    return {
      records: page,
      nextCursor: next < matches.length ? String(next) : null,
      total: args.cursor ? null : matches.length,
    };
  },
  count_audit_records: (args) => ({ total: matchingRecords(args).length }),

  list_request_documents: (args) => documentsFor(args.requestId, args.portalKey).map((candidate) => ({
    documentId: candidate.documentId,
    portalKey: candidate.portalKey,
    fileName: candidate.fileName,
    fileSize: candidate.text.length > 0 ? 18_000 + candidate.text.length * 37 : 46_208,
    contentType: candidate.contentType,
    visibility: 'public',
    downloadStatus: 'DOWNLOADED',
    rawFileSha256: hex64(`raw:${candidate.portalKey}:${candidate.documentId}`),
    extractedTextSha256: candidate.text.length > 0 ? hex64(`text:${candidate.portalKey}:${candidate.documentId}`) : null,
    extractionStatus: candidate.extractionStatus,
    extractionFormat: candidate.extractionFormat,
    extractionDeferredReason: candidate.extractionDeferredReason,
    originalSourceUri: `${reg(candidate.portalKey).canonicalUrl}documents/${candidate.documentId}`,
  })),
  get_document_text_chunk: (args) => {
    const source = documentByKey.get(`${args.portalKey}:${args.documentId}`);
    const text = source?.text ?? '';
    const offset = Math.min(args.characterOffset ?? 0, text.length);
    const limit = args.characterLimit ?? 100_000;
    const slice = text.slice(offset, offset + limit);
    const end = offset + slice.length;
    return {
      documentId: args.documentId,
      portalKey: args.portalKey,
      characterOffset: offset,
      returnedCharacterCount: slice.length,
      nextCharacterOffset: end < text.length ? end : null,
      totalCharacterCount: text.length,
      maskedText: slice,
      maskedFindingCount: countMasks(slice),
    };
  },
  get_document_entity_truncations: [],
  verify_document_provenance: (args) => {
    const record = provenanceRecord(args.portalKey, args.documentId);
    return {
      documentId: args.documentId,
      portalKey: args.portalKey,
      expectedSha256: record.expectedSha256,
      computedSha256: record.computedSha256,
      evidenceFilePresent: true,
      hashMatches: true,
      recordedDigestSource: 'INGESTION_RECORD',
      manifestPresent: false,
      isCryptographicallySigned: false,
      signatureVerification: 'NOT_ATTEMPTED',
      signatureIssuer: null,
      signingCertificateSerialNumber: null,
      signingTimestamp: null,
      assertionCount: 0,
      invalidAssertionCount: 0,
      claimGraph: { activeManifestLabel: null, claims: [], ingredients: [], validationStatuses: [], certificateChainTrusted: false, certificateChainDiagnostics: [], truncated: false },
      textProjectionSha256: record.textProjectionSha256,
      textProjectionMatches: true,
      provenanceConfidence: 'LOCAL_DIGEST_MATCH',
      chainOfCustodyConfirmed: false,
      verificationErrors: [],
    };
  },
  list_document_provenance_verifications: (args) => [provenanceRecord(args.portalKey, args.documentId)],
  list_document_annotations: (args) => ({
    textAvailable: (documentByKey.get(`${args.portalKey}:${args.documentId}`)?.text.length ?? 0) > 0,
    currentTextSha256: hex64(`text:${args.portalKey}:${args.documentId}`),
    annotations: [],
  }),
  get_annotation_settings: {
    reviewer: 'Sample Reporter',
    codebook: [
      { code: 'WITHHOLD', label: 'Withholding ground', definition: 'A passage that states a legal basis for withholding records.' },
      { code: 'AMOUNT', label: 'Dollar figure', definition: 'A stated amount to reconcile against a dataset or another record.' },
    ],
  },

  list_findings: (args) => filterFindings(args.filter).slice(0, args.limit ?? FINDINGS.length),
  count_findings: (args) => filterFindings(args.filter).length,
  refresh_findings: { detectorsRun: 31, findingsRecorded: FINDINGS.length, staleCleared: 0 },
  list_evidence_gaps: EVIDENCE_GAPS,
  list_story_leads: (args) => STORY_LEADS.slice(0, args.limit ?? STORY_LEADS.length),

  get_meeting_calendar: MEETING_CALENDAR,

  get_corpus_index_status: {
    indexedDocumentCount: 1_486,
    indexedSegmentCount: 22_914,
    indexedTermCount: 61_377,
    indexedRelationshipCount: 9_842,
    availableWriteCapacity: 64,
    algorithmVersion: 'deterministic-corpus-v3',
  },
  query_corpus_patterns: (args) => {
    const query = args.query;
    if (!query) return PATTERNS;
    return PATTERNS
      .filter((pattern) => !query.portalKey || pattern.evidence.some((ref) => ref.portalKey === query.portalKey))
      .filter((pattern) => pattern.evidence.length >= query.minimumEvidenceCount)
      .filter((pattern) => pattern.confidenceBasisPoints >= (query.minimumConfidenceBasisPoints ?? 0))
      .filter((pattern) => !query.patternKinds?.length || query.patternKinds.includes(pattern.kind))
      .slice(0, query.resultLimit);
  },
  list_top_corpus_entities: (args) => ENTITIES.slice(0, args.limit ?? ENTITIES.length),
  list_entity_brokers: (args) => BROKERS.slice(0, args.limit ?? BROKERS.length),

  get_embedding_model_status: {
    modelId: 'Qdrant/bge-small-en-v1.5-onnx-Q',
    revision: 'fastembed-6.1.0:BGESmallENV15Q',
    dimension: 384,
    distanceMetric: 'COSINE',
    normalization: 'L2_UNIT',
    cacheDir: `${APP_DATA}/models`,
    installed: true,
    ready: true,
    artifactCount: 5,
    artifactManifestSha256: hex64('embedding-manifest'),
    lastError: null,
  },
  get_embedding_index_status: {
    indexedDocumentCount: 1_412,
    indexedChunkCount: 18_630,
    pendingDocumentCount: 71,
    failedDocumentCount: 3,
    running: false,
    cancelled: false,
    lastError: null,
  },
  hybrid_search: (args) => hybridSearch(args),
  get_analytical_schema: ANALYTICAL_SCHEMA,
  list_analytical_import_candidates: IMPORT_CANDIDATES,

  get_data_maintenance_status: {
    policy: { enabled: true, autoImport: true, autoRefresh: true, refreshHours: 24, retentionMode: 'ORIGINALS', retainedFileBudgetBytes: 20 * 1024 ** 3 },
    effectiveAt: iso(60),
    retainedFileBytes: 3_884_105_216,
    referencedDocuments: 1_486,
    reclaimedFileBytes: 0,
    events: [
      { action: 'REFRESH', detail: 'Refreshed 6 sources; 41 new documents, 0 changed.', recordedAt: iso(0, -180) },
      { action: 'IMPORT', detail: 'Imported Harbor City calls for service (48,213 rows).', recordedAt: iso(31) },
      { action: 'REFRESH', detail: 'Refreshed 6 sources; 17 new documents, 1 changed.', recordedAt: iso(1, -180) },
    ],
  },

  list_resolution_candidates: RESOLUTION_CANDIDATES,
  list_resolution_ledger: RESOLUTION_LEDGER,

  list_claimed_requests: CLAIMED_REQUESTS,
  get_request_deadline: (args) => {
    const deadline = DEADLINES[args.claimId];
    if (!deadline) throw new Error(`No claimed request ${args.claimId}`);
    return deadline;
  },
};

export default answers;
