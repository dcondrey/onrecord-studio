// @ts-check
// Invented San Diego sample data for the UI review harness (scripts/capture-ui-review.mjs). Every
// person, vendor, request number, quote and digest below is fabricated for screenshots; public
// bodies keep their real names so the views read as they would for a San Diego investigator.
// Under `--data=real` the surfaces the San Diego packs hold are answered from
// scripts/ui-review-fixtures-real.mjs instead (see `realHandlers`).
// Type-check against the IPC contract with:
//   npx tsc --noEmit --allowJs --checkJs --strict --module esnext --moduleResolution bundler \
//     --target es2022 --skipLibCheck scripts/ui-review-fixtures.mjs

/** @typedef {import('../src/generated/nativeCommands').NativeCommandMap} Commands */
/** @typedef {{ [K in keyof Commands]?: (args: Commands[K]['args']) => Commands[K]['result'] }} Handlers */

/** The instant every fixture date is measured from; the harness pins the page clock to it. */
export const FIXTURE_NOW = Date.parse('2026-10-07T17:00:00Z');

const iso = (/** @type {number} */ hoursAgo) => new Date(FIXTURE_NOW - hoursAgo * 3_600_000).toISOString();
const day = (/** @type {number} */ daysAgo) => iso(daysAgo * 24).slice(0, 10);
const hex = (/** @type {string} */ seed) => {
  let hash = 2166136261;
  let out = '';
  while (out.length < 64) {
    for (const ch of seed + out.length) hash = Math.imul(hash ^ ch.charCodeAt(0), 16777619) >>> 0;
    out += hash.toString(16).padStart(8, '0');
  }
  return out.slice(0, 64);
};

/** @param {string} key @param {string} name @param {string} url @param {import('../src/types').SourceKind} kind @returns {import('../src/types').PortalRegistration} */
const portal = (key, name, url, kind) => ({
  portalKey: key,
  displayName: name,
  canonicalUrl: url,
  originUrl: url,
  apiLayout: { requestsEndpoint: '/client/requests', requestsPointer: '/requests', metadataPointer: '/total_count', pageParameter: 'page_number', pageSizeParameter: 'page_size' },
  verifiedAt: iso(24 * 40),
  sourceKind: kind,
});

const PORTALS = [
  portal('sandiego', 'City of San Diego', 'https://sandiego.nextrequest.com', 'NEXT_REQUEST'),
  portal('sdcounty', 'County of San Diego', 'https://sandiegocounty.nextrequest.com', 'NEXT_REQUEST'),
  portal('sdhc', 'San Diego Housing Commission', 'https://www.sdhc.org/news-and-reports', 'WEB_CRAWL'),
  portal('sd-council-agendas', 'City Council agendas', 'https://www.sandiego.gov/city-clerk/officialdocs/legisdocs/brown-act', 'WEB_CRAWL'),
];

const AGENCIES = [
  { id: 101, portal: 'sandiego', name: 'San Diego Police Department' },
  { id: 102, portal: 'sandiego', name: 'Environmental Services Department' },
  { id: 103, portal: 'sandiego', name: 'Homelessness Strategies and Solutions Department' },
  { id: 104, portal: 'sandiego', name: 'Department of Finance' },
  { id: 105, portal: 'sandiego', name: 'Parks and Recreation' },
  { id: 106, portal: 'sandiego', name: 'Office of the City Attorney' },
  { id: 201, portal: 'sdcounty', name: 'Health and Human Services Agency' },
  { id: 202, portal: 'sdcounty', name: 'Sheriff\'s Department' },
];
const agency = (/** @type {number} */ id) => AGENCIES.find((a) => a.id === id) ?? AGENCIES[0];

/** [requestId, agencyId, title, status, createdDaysAgo, closedDaysAgo|null, documentCount, flags] */
/** @type {[number, number, string, string, number, number | null, number, string[]][]} */
const REQUEST_ROWS = [
  [26_0412, 102, 'Encampment abatement notices posted along Imperial Avenue, January through March 2026', 'Closed', 190, 151, 3, ['STATUTORY_LATENCY_VIOLATION']],
  [26_0418, 103, 'Contracts, amendments and invoices for the Golden Hall bridge shelter operator, FY2025', 'Closed', 186, 120, 3, ['STATUTORY_EXTENSION_DEFECTIVE']],
  [26_0533, 101, 'Unsafe Camping Ordinance citations and arrests by beat, July 2025 to present', 'Overdue', 64, null, 0, ['STATUTORY_LATENCY_VIOLATION']],
  [26_0540, 103, 'Safe Sleeping Program nightly capacity and occupancy reports, 20th & B and Rose Canyon', 'Closed', 120, 98, 2, []],
  [26_0561, 105, 'Public restroom cleaning and closure logs, East Village and Balboa Park', 'Open', 21, null, 1, []],
  [26_0577, 104, 'Homelessness program spending by fund and account, FY2024 adopted vs. actual', 'Closed', 88, 81, 2, ['SUSPICIOUSLY_FAST_CLOSURE']],
  [26_0602, 101, 'Body-worn camera footage retention schedule and audit trail for encampment sweeps', 'Closed', 75, 30, 1, []],
  [26_0615, 106, 'Communications with outreach providers about property storage and the 72-hour notice', 'Closed', 70, 33, 2, ['STATUTORY_LATENCY_VIOLATION']],
  [26_0688, 102, 'Get It Done encampment report closure codes and crew dispatch notes, 2025', 'Open', 12, null, 0, []],
  [26_0701, 103, 'Coordinated outreach referral outcomes for the Downtown Clean & Safe area', 'Closed', 54, 47, 1, []],
  [7_311, 201, 'Behavioral health bed availability reports provided to the City, 2025', 'Closed', 140, 100, 2, []],
  [7_355, 202, 'Sheriff jail releases to downtown San Diego without transportation, 2025', 'Open', 33, null, 0, []],
];

/** @type {import('../src/types').RequestSearchResult[]} */
const REQUESTS = REQUEST_ROWS.map(([requestId, agencyId, title, status, created, closed, documentCount, flags]) => ({
  requestId,
  portalKey: agency(agencyId).portal,
  agencyId,
  agencyName: agency(agencyId).name,
  title: `${agency(agencyId).portal === 'sandiego' ? '26' : '25'}-${String(requestId).slice(-4)}: ${title}`,
  descriptionPreview: `Requesting all records reflecting ${title.charAt(0).toLowerCase()}${title.slice(1)}. Electronic copies are preferred; please produce on a rolling basis.`,
  status,
  createdAt: iso(created * 24),
  closedAt: closed === null ? null : iso(closed * 24),
  documentCount,
  integrityFlags: flags,
}));
const request = (/** @type {number} */ id) => REQUESTS.find((r) => r.requestId === id) ?? REQUESTS[0];

/** [documentId, requestId, fileName, contentType, bytes] */
/** @type {[number, number, string, string, number][]} */
const DOCUMENT_ROWS = [
  [9001, 26_0412, 'Abatement_Notices_Imperial_Ave_Q1-2026.pdf', 'application/pdf', 2_481_220],
  [9002, 26_0412, 'ESD_Response_Letter_26-0412.pdf', 'application/pdf', 182_004],
  [9003, 26_0412, 'Crew_Dispatch_Log_Jan-Mar_2026.xlsx', 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet', 96_310],
  [9011, 26_0418, 'Operator_Agreement_Amendment_3.pdf', 'application/pdf', 1_208_442],
  [9012, 26_0418, 'Invoices_FY2025_GoldenHall.pdf', 'application/pdf', 4_870_115],
  [9013, 26_0418, 'Extension_Notice_26-0418.pdf', 'application/pdf', 64_220],
  [9021, 26_0540, 'SSP_Nightly_Capacity_20th_and_B.csv', 'text/csv', 41_802],
  [9022, 26_0540, 'SSP_Occupancy_Summary_Rose_Canyon.pdf', 'application/pdf', 512_330],
  [9031, 26_0561, 'Restroom_Service_Log_EastVillage.pdf', 'application/pdf', 288_190],
  [9041, 26_0577, 'Homelessness_Budget_FY2024_Adopted_vs_Actual.xlsx', 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet', 154_002],
  [9042, 26_0577, 'Finance_Response_26-0577.pdf', 'application/pdf', 99_481],
  [9051, 26_0602, 'BWC_Retention_Schedule_DP-3.33.pdf', 'application/pdf', 330_117],
  [9061, 26_0615, 'Outreach_Provider_Emails_Redacted.pdf', 'application/pdf', 3_204_998],
  [9062, 26_0615, 'Privilege_Log_26-0615.pdf', 'application/pdf', 72_104],
  [9071, 26_0701, 'Referral_Outcomes_DCS_Q2.pdf', 'application/pdf', 401_556],
  [9081, 7_311, 'BHS_Bed_Availability_2025.pdf', 'application/pdf', 655_340],
  [9082, 7_311, 'HHSA_Response_Letter.pdf', 'application/pdf', 88_115],
];

/** @type {import('../src/types').RequestDocumentSummary[]} */
const DOCUMENTS = DOCUMENT_ROWS.map(([documentId, requestId, fileName, contentType, fileSize]) => ({
  documentId,
  portalKey: request(requestId).portalKey,
  fileName,
  fileSize,
  contentType,
  visibility: 'public',
  downloadStatus: 'DOWNLOADED',
  rawFileSha256: hex(`raw${documentId}`),
  extractedTextSha256: hex(`text${documentId}`),
  extractionStatus: 'SEARCHABLE',
  extractionFormat: contentType === 'application/pdf' ? 'PDF_TEXT' : contentType === 'text/csv' ? 'CSV' : 'XLSX',
  extractionDeferredReason: null,
  originalSourceUri: `https://sandiego.nextrequest.com/documents/${documentId}`,
}));
const documentRequest = (/** @type {number} */ documentId) => DOCUMENT_ROWS.find(([id]) => id === documentId)?.[1] ?? 26_0412;
const documentName = (/** @type {number} */ documentId) => DOCUMENT_ROWS.find(([id]) => id === documentId)?.[2] ?? `Document ${documentId}`;

const LETTER = `CITY OF SAN DIEGO
ENVIRONMENTAL SERVICES DEPARTMENT

Re: Public Records Act Request 26-0412

Dear Requester,

The City of San Diego has completed its search for records responsive to your request dated March 31, 2026. Pursuant to Government Code section 7922.535, the City is producing 214 pages of abatement notices posted along Imperial Avenue between January 1 and March 31, 2026.

Notices for encampments cleared on February 12 and February 13 are not included because no responsive records were located. The City has determined that certain records are exempt from disclosure under Government Code section 7927.705 and section 7922.000, as the public interest served by not disclosing the records clearly outweighs the public interest served by disclosure.

Each notice states that property will be removed no sooner than 72 hours after posting, consistent with San Diego Municipal Code section 63.0403. Crew dispatch logs show removal operations beginning at 06:10 on February 12, 2026, at the 2900 block of Imperial Avenue.

Personal information including telephone numbers such as (619) 555-0142 has been redacted.

The total expenditure for contracted storage services under Agreement H-24-1180 was $1,486,200.00 in FY2025, against an authorized ceiling of $1,250,000.00.

This concludes the City's response. If you have questions, please contact the Environmental Services Department Public Records Coordinator.

Sincerely,
Records Coordinator
Environmental Services Department`;

/** @param {number} documentId @param {string} portalKey @param {number} offset @param {number} limit @returns {import('../src/types').DocumentTextChunk} */
const textChunk = (documentId, portalKey, offset, limit) => {
  const text = LETTER.replace('26-0412', String(documentRequest(documentId)).replace(/^(\d\d)(\d+)$/, '$1-$2'));
  const slice = text.slice(offset, offset + limit);
  return {
    documentId,
    portalKey,
    characterOffset: offset,
    returnedCharacterCount: slice.length,
    nextCharacterOffset: offset + slice.length < text.length ? offset + slice.length : null,
    totalCharacterCount: text.length,
    maskedText: slice.replace('(619) 555-0142', '[TELEPHONE NUMBER]'),
    maskedFindingCount: slice.includes('(619)') ? 1 : 0,
  };
};

/**
 * A one-page letter-size PDF of the sample letter, framed the way `read_document_original`
 * returns it: file bytes, a JSON header, then the header length as a big-endian u32.
 * @param {number} documentId @returns {ArrayBuffer}
 */
const originalFrame = (documentId) => {
  const escape = (/** @type {string} */ line) => line.replace(/[\\()]/g, (c) => `\\${c}`).replace(/[^\x20-\x7e]/g, '-');
  const lines = LETTER.split('\n').flatMap((line) => line.match(/.{1,92}(\s|$)/g) ?? ['']).slice(0, 52);
  const content = `BT /F1 10 Tf 13 TL 60 740 Td ${lines.map((line) => `(${escape(line.trimEnd())}) '`).join(' ')} ET`;
  const objects = [
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>',
    `<< /Length ${content.length} >>\nstream\n${content}\nendstream`,
    '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>',
  ];
  let pdf = '%PDF-1.4\n';
  const offsets = objects.map((body, index) => {
    const offset = pdf.length;
    pdf += `${index + 1} 0 obj\n${body}\nendobj\n`;
    return offset;
  });
  const xref = pdf.length;
  pdf += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n${offsets.map((o) => `${String(o).padStart(10, '0')} 00000 n \n`).join('')}`;
  pdf += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
  const bytes = new TextEncoder().encode(pdf);
  const header = new TextEncoder().encode(JSON.stringify({ byteLength: bytes.length, sha256: hex(`raw${documentId}`), recordedSha256: hex(`raw${documentId}`), matchesRecorded: true }));
  const frame = new Uint8Array(bytes.length + header.length + 4);
  frame.set(bytes, 0);
  frame.set(header, bytes.length);
  new DataView(frame.buffer).setUint32(bytes.length + header.length, header.length, false);
  return frame.buffer;
};

/** @param {number} documentId @param {number} start @param {number} end @param {string} label @returns {import('../src/types').FindingCitation} */
const citation = (documentId, start, end, label) => ({
  portalKey: request(documentRequest(documentId)).portalKey,
  documentId,
  characterStart: start,
  characterEnd: end,
  quote: LETTER.slice(start, end),
  label,
});

/** @param {string} id @param {import('../src/types').FindingKind} kind @param {import('../src/types').FindingSeverity} severity @param {number|null} bps @param {number} requestId @param {number|null} documentId @param {string} headline @param {string} explanation @param {number} hours @param {string} detector @param {boolean} [dismissed] @returns {import('../src/types').Finding} */
const finding = (id, kind, severity, bps, requestId, documentId, headline, explanation, hours, detector, dismissed = false) => ({
  id,
  kind,
  severity,
  confidence: bps === null ? { kind: 'UNASSESSED' } : { kind: 'MEASURED', basisPoints: bps },
  subject: { portalKey: request(requestId).portalKey, agencyId: request(requestId).agencyId, requestId, documentId, entityId: null },
  headline,
  explanation,
  citations: documentId === null ? [] : [citation(documentId, 330, 510, documentName(documentId))],
  discoveredAt: iso(hours),
  lastSeenAt: iso(Math.max(1, hours - 6)),
  dismissedAt: dismissed ? iso(hours - 2) : null,
  sourceDetector: detector,
});

const FINDINGS = [
  finding('f-policy-esd', 'POLICY_VIOLATION_DETECTED', 'SERIOUS', 9100, 26_0412, 9002, 'Environmental Services exceeded the 10-day CPRA determination period on 6 of 9 requests', 'Gov. Code § 7922.535 allows 10 calendar days for a determination. Six requests to the same department were answered after 14 to 39 days, each measured from a sourced determination letter.', 3, 'statutory_deadline'),
  finding('f-notice-shortfall', 'ENCAMPMENT_NOTICE_SHORTFALL', 'CRITICAL', 8800, 26_0412, 9003, 'Removal on Imperial Avenue began 61 hours after the posted 72-hour notice', 'The dispatch log records crews on site at 06:10 on February 12, 2026; the notice for the same block was posted at 17:05 on February 9. SDMC § 63.0403 requires 72 hours.', 5, 'encampment_notice'),
  finding('f-contract-ceiling', 'CONTRACT_CEILING_EXCEEDED', 'SERIOUS', 9400, 26_0418, 9012, 'Storage contract H-24-1180 paid $1,486,200 against a $1,250,000 ceiling', 'Invoices total $236,200 above the authorized ceiling in FY2025. No amendment raising the ceiling appears among the released records.', 8, 'financial_chain'),
  finding('f-boilerplate', 'BOILERPLATE_DENIAL', 'NOTABLE', 7800, 26_0615, 9062, 'The same balancing-test paragraph appears in 9 unrelated denials', 'Near-identical wording invoking § 7922.000 was reused across requests on different topics and departments.', 12, 'boilerplate'),
  finding('f-absence', 'ABSENCE_OF_RECORDS', 'NOTABLE', null, 26_0412, 9001, 'No notices produced for clearances on February 12 and 13', 'The released set covers January 1 to March 31 but omits two days on which dispatch logs record removal operations.', 20, 'absence'),
  finding('f-extension', 'EXTENSION_ABUSE', 'NOTABLE', 7200, 26_0418, 9013, 'Extension notice cites "voluminous records" for a 3-document production', 'The 14-day extension under § 7922.535(c) was invoked; the eventual production was three files totalling 61 pages.', 30, 'extension'),
  finding('f-no-records', 'NO_RECORDS_CONTRADICTED_BY_RELEASE', 'SERIOUS', 8600, 26_0615, 9061, '"No responsive records" contradicted by a later release from another department', 'The City Attorney reported no records of provider communications; Homelessness Strategies later released 41 pages of the same thread.', 40, 'contradiction'),
  finding('f-shared-language', 'CROSS_CORPUS_SHARED_LANGUAGE', 'INFO', 6400, 7_311, 9082, 'City and County denials share an identical 64-word exemption paragraph', 'The paragraph appears in County HHSA and City ESD responses within the same month.', 50, 'cross_corpus'),
  finding('f-service-lead', 'SERVICE_REPORT_LEAD', 'NOTABLE', null, 26_0688, null, 'Get It Done encampment reports closed as "referred" rose 48% in Council District 3', 'Counted from the Get It Done open dataset: 1,212 encampment reports closed as referred in 2025 vs. 819 in 2024.', 6, 'service_reports'),
  finding('f-stop-lead', 'POLICE_STOP_LEAD', 'NOTABLE', null, 26_0533, null, 'RIPA stops citing "lodging" more than doubled after the ordinance took effect', 'Counted from RIPA stop reasons: 2,904 stops between July 2025 and June 2026 vs. 1,377 in the prior twelve months.', 7, 'police_stops'),
  finding('f-abatement-lead', 'ABATEMENT_LOG_LEAD', 'NOTABLE', null, 26_0412, 9003, 'Abatement crews logged 37 removals between 05:00 and 06:30', 'The ordinance\'s posted hours begin at 06:00; 11 of the 37 entries precede that.', 9, 'abatement_logs'),
  finding('f-capacity-lead', 'CAPACITY_COMPARISON_LEAD', 'INFO', null, 26_0540, 9021, 'Safe Sleeping sites reported 112 open spaces on nights citations were issued for lack of shelter', 'Nightly capacity and citation dates overlap on 23 nights.', 14, 'capacity'),
  finding('f-spending-lead', 'SPENDING_RECORD_LEAD', 'NOTABLE', 8200, 26_0577, 9041, 'Homelessness fund actuals exceeded adopted budget by $9.4 million in FY2024', 'Actual expenditures in fund 100000 accounts 512xxx were $61.2M against $51.8M adopted.', 16, 'spending'),
  finding('f-parking-lead', 'PARKING_CITATION_LEAD', 'INFO', null, 26_0533, null, 'Oversize-vehicle citations concentrate on six blocks near Mission Bay', 'Six block faces account for 38% of 4,410 oversize-vehicle citations in 2025.', 26, 'parking'),
  finding('f-call-lead', 'POLICE_CALL_LEAD', 'INFO', null, 26_0533, null, '"Sleeper" calls closed with no report rose to 71% of dispositions', 'Calls coded SLEEPER: 3,880 with disposition K in 2025 of 5,462.', 32, 'police_calls'),
  finding('f-housing-lead', 'HOUSING_APPROVAL_LEAD', 'INFO', null, 26_0540, null, 'Permanent supportive housing approvals issued fell from 14 to 3 year over year', 'Counted from development approvals issued with the PSH designation.', 60, 'housing'),
  finding('f-dismissed', 'REGENERATED_RESPONSE', 'INFO', 5100, 26_0602, 9051, 'Retention schedule PDF regenerated two days after the request was closed', 'Producer metadata changed; text is identical. Dismissed as an administrative re-export.', 70, 'regeneration', true),
];

// ---- Stored lead figures (src/types/analysis.ts LeadFigure), one per lead finding -------------

/** @param {number} value */
const fv = (value) => ({ state: /** @type {const} */ ('value'), value });
/** @param {'not-recorded'|'not-covered'|'excluded'} state @param {string} reason */
const fgap = (state, reason) => ({ state, reason });
/** One call type-year of calls_outcomes as the builder draws it (callLeads.ts). */
const callRow = (type, year, partial, [arrest, report, noReport, unfounded, other], [blank, conflicting, cancelled], crisis) => ({
  key: `${type}:${year}`, group: type, label: `${year}${partial ? ' so far' : ''}`, partial, values: [arrest, noReport, report, unfounded, other],
  excluded: [{ key: 'blank', label: 'no disposition', count: blank }, { key: 'conflicting', label: 'conflicting', count: conflicting }, { key: 'cancelled', label: 'cancelled or duplicate', count: cancelled }].filter((stub) => stub.count > 0),
  overlay: { label: 'Crisis Response Team (also counted under its outcome)', value: crisis }, note: null,
});
const CALL_SEGMENTS = [{ key: 'arrest', label: 'Arrest', tone: 'lead' }, { key: 'noReport', label: 'No report required', tone: 'ord-1' }, { key: 'report', label: 'Report', tone: 'ord-2' }, { key: 'unfounded', label: 'Unfounded', tone: 'ord-3' }, { key: 'other', label: 'Other code', tone: 'ord-4' }];

/** @type {Record<string, import('../src/types').LeadFigure>} */
const LEAD_FIGURES = {
  'f-call-lead': { kind: 'stacked', measure: 'How the calls ended, by call type and year', claim: 'descriptive', unit: 'count', normalise: true,
    caption: 'Each bar is the calls with a disposition; blank, conflicting and cancelled calls sit beside it, outside the bar.', notes: ['14 SLEEPER calls have no readable date and are in no year.'], segments: CALL_SEGMENTS,
    rows: [
      callRow('SLEEPER', 2026, true, [96, 141, 2790, 205, 572], [22, 4, 380], 61), callRow('SLEEPER', 2025, false, [162, 230, 3856, 301, 882], [31, 6, 512], 88), callRow('SLEEPER', 2024, false, [211, 265, 3650, 344, 1072], [40, 3, 540], 12),
      callRow('CW (check the welfare)', 2026, true, [410, 2950, 9800, 1120, 1950], [80, 10, 2100], 640), callRow('602 (trespassing)', 2026, true, [690, 610, 3980, 520, 648], [30, 2, 820], 25),
      callRow('5150 (listed as mental case)', 2026, true, [40, 1810, 1180, 90, 457], [12, 1, 310], 820), callRow('647F (drunk, drugs or alcohol)', 2026, true, [380, 210, 520, 60, 112], [8, 0, 160], 14),
    ] },
  'f-abatement-lead': { kind: 'bins', measure: 'Hours from a posted notice to the abatement', claim: 'descriptive', unit: 'hours', caption: '2,067 places with exactly one notice and one abatement, matched on address and coordinates.', notes: ['214 of 11,104 abatements dated within the notice log\'s period are at a place with no notice in that log.'],
    bins: [{ key: 'under_3', label: 'under 3 h', from: 0, to: 3, count: 22 }, { key: 'from_3_to_24', label: '3 to 24 h', from: 3, to: 24, count: 79 }, { key: 'from_24_to_72', label: '24 to 72 h', from: 24, to: 72, count: 1811 }, { key: 'over_72', label: '72 h or more', from: 72, to: null, count: 144 }],
    before: { key: 'before', label: 'completed before the notice was logged', count: 11 }, median: 29.3, reference: { at: 72, label: '72 h reference (app yardstick; the logs do not record the notice period that applied)' },
    denominator: [{ key: 'paired', label: 'notices paired with one abatement', count: 2067 }, { key: 'not_matchable', label: 'notices with no place or time', count: 31 }, { key: 'other', label: 'notices at a place without exactly one of each', count: 312 }] },
  'f-capacity-lead': { kind: 'series', measure: 'Adopted budget, Encampment reports and shelter beds, by fiscal year', claim: 'descriptive', caption: 'Three publishers; each panel has its own scale. Nothing here shows why any series moved.',
    notes: ['212 Encampment reports have no readable filing date and are in no year.'],
    points: [2020, 2021, 2022, 2023, 2024, 2025].map((year) => ({ key: String(year), label: `FY ${year}`, partial: false })),
    panels: [
      { key: 'budget', label: 'Adopted operating expense', unit: 'currency', context: null, values: [fgap('not-recorded', 'no department with "homeless" in its name; not an amount of zero'), fv(14_210_400), fv(21_880_150), fv(30_516_002), fv(44_007_454), fv(53_916_571)] },
      { key: 'reports', label: 'Encampment reports filed', unit: 'count', context: null, values: [fgap('not-covered', 'the imported files cannot hold every Encampment report filed'), fgap('not-covered', 'the imported files cannot hold every Encampment report filed'), fv(31_204), fv(52_860), fv(69_987), fv(48_135)] },
      { key: 'beds', label: 'Year-round emergency shelter beds', unit: 'count', values: [fv(1_552), fv(1_871), fv(1_940), fv(2_210), fv(2_476), fv(1_970)], context: { label: 'Beds of every kind', values: [fv(1_690), fv(2_002), fv(2_051), fgap('not-recorded', 'the inventory records no total bed count'), fv(2_655), fv(2_118)] } },
    ] },
  'f-spending-lead': { kind: 'paired', display: 'bullet', measure: 'Adopted against actual operating expense: Homelessness Strategies & Solutions', claim: 'statistical', unit: 'currency', subjectLabel: 'Actual expense', comparisonLabel: 'Adopted',
    caption: 'Adopted is the original appropriation, not the revised budget.', notes: [], band: { percent: 10, minimum: 1_000_000, label: 'Lead threshold (10% and $1M): app yardstick, not a budget rule' },
    rows: [
      { key: '2021', label: 'FY 2021', subject: fv(12_904_115), comparison: fv(14_210_400), subjectBasis: null, comparisonBasis: null },
      { key: '2022', label: 'FY 2022', subject: fv(20_118_660), comparison: fv(21_880_150), subjectBasis: null, comparisonBasis: null },
      { key: '2023', label: 'FY 2023', subject: fgap('not-recorded', 'no rows in the actuals table; not spending of zero'), comparison: fv(30_516_002), subjectBasis: null, comparisonBasis: null },
      { key: '2024', label: 'FY 2024', subject: fv(53_407_454), comparison: fv(44_007_454), subjectBasis: null, comparisonBasis: null },
      { key: '2025', label: 'FY 2025', subject: fv(49_120_330), comparison: fv(53_916_571), subjectBasis: null, comparisonBasis: null },
    ] },
  'f-parking-lead': { kind: 'ranked', measure: 'Oversized-vehicle and 72-hour citations, by location as written', claim: 'descriptive', unit: 'count', caption: 'The ten locations with most citations; one block can appear under more than one spelling.', notes: [],
    excluded: [{ key: 'no_block', label: 'no location recorded', count: 96 }],
    rows: [['2500 MISSION BAY DR', 412], ['3700 MORENA BLVD', 301], ['1900 GRAND AVE', 266], ['2600 CLAIREMONT DR', 214], ['3300 DAKOTA ST', 188], ['1600 E MISSION BAY DR', 176]].map(([block, n], index) => ({ key: String(block), label: String(block), value: fv(Number(n)), subject: index === 0, detail: `${Math.round(Number(n) * 0.6)} by police` })) },
  'f-stop-lead': { kind: 'paired', display: 'dots', measure: 'What stops resulted in, people perceived as unhoused against everyone else', claim: 'statistical', unit: 'percent', subjectLabel: 'Perceived as unhoused', comparisonLabel: 'Everyone else', band: null,
    caption: 'Rates of persons with a recorded result; a difference in rate is not a finding about any stop.', notes: ['318 persons have no row in the results table and are left out of the rates.'],
    rows: [['arrested', 'Custodial arrest', 1214, 6420, 9_801, 112_550], ['warrantless', 'Arrest without a warrant', 702, 4012, 9_801, 112_550], ['warrant', 'Arrest on an outstanding warrant', 512, 2408, 9_801, 112_550], ['cited', 'Citation or cite and release', 2_310, 31_740, 9_801, 112_550]]
      .map(([key, label, u, o, uw, ow]) => ({ key: String(key), label: String(label), subject: fv(Number(u) / Number(uw)), comparison: fv(Number(o) / Number(ow)), subjectBasis: `${Number(u).toLocaleString('en-US')} of ${Number(uw).toLocaleString('en-US')}`, comparisonBasis: `${Number(o).toLocaleString('en-US')} of ${Number(ow).toLocaleString('en-US')}` })) },
  'f-housing-lead': { kind: 'stacked', measure: 'Dwelling units on building permits issued, by income tier and issue year', claim: 'descriptive', unit: 'count', normalise: false,
    caption: 'Units on permits issued, not built or occupied; a negative tier total is units removed, net.', notes: ['41 building permits have no job recorded and are in no total.'],
    segments: [{ key: 'lowest', label: 'Extremely low and very low', tone: 'lead' }, { key: 'low', label: 'Low', tone: 'ord-1' }, { key: 'moderate', label: 'Moderate', tone: 'ord-2' }, { key: 'above_moderate', label: 'Above moderate', tone: 'ord-4' }],
    rows: [[2023, false, [412, 388, 120, 4_310], '218 jobs with no tier recorded'], [2024, false, [296, 451, -6, 3_902], '190 jobs with no tier recorded'], [2025, false, [188, 302, 96, 3_455], null], [2026, true, [74, 140, 22, 1_610], 'no extremely low tier recorded']]
      .map(([year, partial, values, note]) => ({ key: String(year), group: 'Units by income tier', label: `${year}${partial ? ' so far' : ''}`, partial: Boolean(partial), values: /** @type {number[]} */ (values), excluded: [], overlay: null, note: /** @type {string|null} */ (note) })) },
  'f-service-lead': { kind: 'paired', display: 'dots', measure: 'Encampment reports closed as referred, by council district', claim: 'descriptive', unit: 'count', subjectLabel: '2025', comparisonLabel: '2024', band: null,
    caption: 'A referred report leaves this table; whether the receiving office acted is not recorded.', notes: [],
    rows: [[3, 1212, 819], [8, 640, 702], [9, 488, 377], [2, 301, 290], [1, 144, 160]].map(([district, now, then]) => ({ key: `cd${district}`, label: `Council District ${district}`, subject: fv(now), comparison: fv(then), subjectBasis: null, comparisonBasis: null })) },
};

/** The publisher file each stored lead names as the first table it read. */
const LEAD_TABLES = {
  'f-call-lead': ['pd_calls_for_service_2026_datasd.csv', 'https://seshat.datasd.org/police_calls_for_service/pd_calls_for_service_2026_datasd.csv'],
  'f-abatement-lead': ['Encampment abatement notices posted', 'https://www.sandiego.gov/environmental-services/ehs/encampment'],
  'f-capacity-lead': ['HIC emergency shelter inventory (UCSD Homelessness Hub)', 'https://homelessness.ucsd.edu/'],
  'f-spending-lead': ['actuals_operating_datasd.csv', 'https://seshat.datasd.org/operating_actuals/actuals_operating_datasd.csv'],
  'f-parking-lead': ['parking_citations_2026_part1_datasd.csv', 'https://seshat.datasd.org/parking_citations/parking_citations_2026_part1_datasd.csv'],
  'f-stop-lead': ['ripa_stops_datasd.csv', 'https://seshat.datasd.org/police_ripa/ripa_stops_datasd.csv'],
  'f-housing-lead': ['approvals_issued_2025_datasd.csv', 'https://seshat.datasd.org/development_permits/approvals_issued_2025_datasd.csv'],
  'f-service-lead': ['get_it_done_requests_closed_2025_datasd.csv', 'https://seshat.datasd.org/get_it_done_reports/get_it_done_requests_closed_2025_datasd.csv'],
};

// A stored lead's explanation ends with the tables it read and its SQL (analysis/service_leads.rs).
for (const lead of FINDINGS.filter((item) => LEAD_FIGURES[item.id])) {
  const [name, url] = LEAD_TABLES[/** @type {keyof typeof LEAD_TABLES} */ (lead.id)];
  lead.explanation += ` Method: counted from the published records. Worth asking: what explains the change?\n\nTables read:\n1. '${name}' (${url}), retrieved 2026-10-02, sha256 ${hex(`lead-${lead.id}`)}\n\nSQL:\nSELECT * FROM lead_rows`;
}

/** @param {import('../src/types').FindingsFilter | null | undefined} filter @param {import('../src/types').Finding[]} [findings] */
const filterFindings = (filter, findings = FINDINGS) => findings.filter((f) =>
  (filter?.includeDismissed || f.dismissedAt === null)
  && (!filter?.id || f.id === filter.id)
  && (!filter?.portalKey || f.subject.portalKey === filter.portalKey)
  && (filter?.requestId == null || f.subject.requestId === filter.requestId)
  && (filter?.documentId == null || f.subject.documentId === filter.documentId || f.citations.some((c) => c.documentId === filter.documentId))
  && (!filter?.discoveredAfter || f.discoveredAt >= filter.discoveredAfter));

/** @param {string} entityId @param {import('../src/types').CorpusEntityKind} kind @param {string} value @param {number} count @param {import('../src/types').EntityValidationGrade} [grade] @returns {import('../src/types').CorpusEntityNode} */
const entity = (entityId, kind, value, count, grade = 'PATTERN_ONLY') => ({ entityId, kind, displayValue: value, documentCount: count, validationGrade: grade });
const ENTITIES = [
  entity('e-org-harborview', 'ORGANIZATION', 'Harborview Shelter Partners', 38, 'GAZETTEER_HIT'),
  entity('e-org-mesa', 'ORGANIZATION', 'Mesa Outreach Collective', 22, 'GAZETTEER_HIT'),
  entity('e-org-pacific', 'ORGANIZATION', 'Pacific Crest Storage Services', 17),
  entity('e-person-whitfield', 'PERSON', 'Dana Whitfield', 29),
  entity('e-person-okafor', 'PERSON', 'Marcus Okafor', 14),
  entity('e-role-director', 'ROLE', 'Deputy Director, Homelessness Strategies', 11),
  entity('e-statute-63', 'STATUTE', 'SDMC § 63.0403', 44, 'FORMAT_VALIDATED'),
  entity('e-statute-7922', 'STATUTE', 'Gov. Code § 7922.535', 61, 'FORMAT_VALIDATED'),
  entity('e-money-1486', 'MONEY', '$1,486,200.00', 4, 'FORMAT_VALIDATED'),
  entity('e-ein-harborview', 'EIN', '95-4417820', 6, 'CHECK_DIGIT_PASSED'),
  entity('e-domain-sandiego', 'EMAIL_DOMAIN', 'sandiego.gov', 73, 'FORMAT_VALIDATED'),
  entity('e-series-h24', 'SERIES_NUMBER', 'H-24-1180', 9, 'PATTERN_ONLY'),
];
const entityById = (/** @type {string} */ id) => ENTITIES.find((e) => e.entityId === id) ?? ENTITIES[0];

/** @param {string} centerId @returns {import('../src/types').CorpusEntityEdge[]} */
const edgesFor = (centerId) => ENTITIES.filter((e) => e.entityId !== centerId).slice(0, 8).map((neighbor, index) => ({
  neighbor,
  basis: index % 3 === 0 ? 'OBSERVED' : index % 3 === 1 ? 'COMPUTED' : 'ASSERTED',
  kind: index === 0 ? 'SIGNED_BY' : index === 1 ? 'APPROVED' : index === 2 ? 'CITES' : 'CO_OCCURRENCE',
  sharedDocumentCount: 12 - index,
  confidenceBasisPoints: 9200 - index * 600,
  validFrom: index < 3 ? '2024-07-01' : null,
  validUntil: null,
}));

/** @type {import('../src/types').CorpusPattern[]} */
const PATTERNS = [
  ['p-near-dup', 'NEAR_DUPLICATE', 'Exemption paragraph reused across 9 denials', 'Nine responses from four departments share a 64-word passage invoking the § 7922.000 balancing test.', 8700],
  ['p-statute', 'SHARED_STATUTE', 'SDMC § 63.0403 cited in abatement notices and Council staff reports', 'The ordinance section links 44 documents across Environmental Services, SDPD and the Council docket.', 8100],
  ['p-entity', 'SHARED_ENTITY', 'Harborview Shelter Partners appears in contracts, invoices and Council minutes', 'Thirty-eight documents name the operator; six carry its EIN.', 7600],
  ['p-temporal', 'TEMPORAL_CLUSTER', 'Twelve abatement notices posted within 40 minutes on February 9', 'Notices along Imperial Avenue share a posting window of 16:40 to 17:20.', 6900],
  ['p-term', 'SHARED_TERM', '"Shelter bed available" recurs in citation narratives', 'The phrase appears in 31 citation narratives and 4 capacity reports.', 6200],
].map(([patternId, kind, title, explanation, confidenceBasisPoints], index) => ({
  patternId: String(patternId),
  kind: /** @type {import('../src/types').CorpusPatternKind} */ (kind),
  title: String(title),
  explanation: String(explanation),
  confidenceBasisPoints: Number(confidenceBasisPoints),
  evidence: DOCUMENT_ROWS.slice(index, index + 3).map(([documentId, requestId, fileName]) => ({
    portalKey: request(requestId).portalKey, requestId, documentId, segmentIndex: index, characterStart: 330, characterEnd: 510, evidenceLabel: fileName, contentSha256: hex(`seg${documentId}`),
  })),
  algorithmVersion: 'corpus-patterns/4',
}));

/** @type {import('../src/types').AgencyPerformanceProfile[]} */
const PROFILES = AGENCIES.map((a, index) => {
  const total = [214, 96, 158, 61, 44, 133, 302, 187][index];
  const closed = Math.round(total * (0.72 + (index % 3) * 0.08));
  const average = [18.4, 41.2, 27.9, 9.1, 12.6, 33.5, 22.0, 46.8][index];
  return {
    agencyId: a.id,
    portalSubdomain: a.portal,
    agencyName: a.name,
    totalRequestsProcessed: total,
    totalClosed: closed,
    validLatencySampleSize: closed - 3,
    excludedInvalidLifetimeCount: 3,
    averageClosureDays: average,
    globalMeanClosureDays: 26.4,
    globalStddevClosureDays: 10.9,
    performanceZScore: Math.round(((average - 26.4) / 10.9) * 100) / 100,
    complianceIndicator: average > 40 ? 'StatutoryDeadlineExceeded' : average > 30 ? 'LatencyOutlier' : 'Compliant',
    anomalyCount: [4, 11, 6, 1, 0, 7, 9, 13][index],
    statutoryViolationCount: [9, 22, 14, 0, 1, 12, 17, 31][index],
    statutoryViolationRate: [0.04, 0.23, 0.09, 0, 0.02, 0.09, 0.06, 0.17][index],
    suspiciousFastClosureCount: [2, 0, 1, 5, 0, 1, 3, 0][index],
    totalDocumentsReleased: total * 3,
    keywordFrequencyMap: { encampment: 40 - index * 3, shelter: 22 + index, citation: 12, contract: 9 + index * 2 },
  };
});

/** @type {import('../src/types').CrossAgencyCorrelation[]} */
const CORRELATIONS = REQUESTS.slice(0, 6).map((r, index) => ({
  portalKey: r.portalKey,
  requestId: r.requestId,
  agencyId: r.agencyId,
  agencyName: r.agencyName,
  requestTitle: r.title,
  requestTimestamp: r.createdAt,
  neighborhoodBlock: ['2900 Imperial Ave', '1400 Commercial St', '17th St & National Ave', '20th St & B St', 'Park Blvd & Russ Blvd', 'Sports Arena Blvd'][index],
  complaintCount: 14 - index * 2,
  correlationWindowHours: 48,
  distinctSourceCount: 3 - (index % 2),
  firstCorrelatedEventAt: iso(24 * (80 + index)),
  lastCorrelatedEventAt: iso(24 * (78 + index)),
  keywordSignalCount: 6 - index,
  correlationDensity: 0.82 - index * 0.09,
}));

/** @type {import('../src/types').AnalyticalDatasetSummary[]} */
const DATASETS = [
  ['snap-budget-fy24', 'OPERATING_BUDGET', 'Operating budget FY2024 (adopted)', 'analysis_operating_budget_fy24', 48_211, 'https://seshat.datasd.org/budget/operating_budget_fy24_datasd.csv', '2023-07-01', '2024-06-30'],
  ['snap-cfs-2025', 'CALLS_FOR_SERVICE', 'Police calls for service 2025', 'analysis_cfs_2025', 512_388, 'https://seshat.datasd.org/police_calls_for_service/pd_calls_for_service_2025_datasd.csv', '2025-01-01', '2025-12-31'],
  ['snap-ripa-2025', 'RIPA_STOPS', 'RIPA stop details 2025', 'analysis_ripa_stops_2025', 141_002, 'https://seshat.datasd.org/police_ripa/ripa_stops_datasd.csv', '2025-01-01', '2025-12-31'],
  ['snap-gid-enc', 'REGIONAL_DATA', 'Get It Done encampment reports', 'analysis_get_it_done_encampments', 38_455, 'https://seshat.datasd.org/get_it_done_encampment/get_it_done_encampment_requests_datasd.csv', '2024-01-01', '2026-09-30'],
  ['snap-pd-beats', 'REGIONAL_DATA', 'San Diego Police Department beats (polygons)', 'analysis_police_beats', 25, 'https://seshat.datasd.org/gis_police_beats/pd_beats_datasd.geojson', '2026-01-01', '2026-09-30'],
].map(([snapshotId, datasetKind, displayName, tableName, importedRows, sourceUri, start, end]) => ({
  snapshotId: String(snapshotId),
  datasetKind: /** @type {import('../src/types').AnalyticalDatasetKind} */ (datasetKind),
  displayName: String(displayName),
  tableName: String(tableName),
  importedRows: Number(importedRows),
  rejectedRows: 12,
  sourceUri: String(sourceUri),
  sourceSha256: hex(String(snapshotId)),
  retrievedAt: iso(24 * 6),
  observationPeriodStart: String(start),
  observationPeriodEnd: String(end),
  partialDataset: snapshotId === 'snap-gid-enc',
  validationSummary: `${Number(importedRows).toLocaleString('en-US')} rows imported, 12 rejected for malformed dates`,
  unconvertedByColumn: { amount: 0, date_time: 12 },
}));

/** @type {import('../src/types').AnalyticalSchema} */
const SCHEMA = {
  datasets: DATASETS,
  columns: [
    ['analysis_operating_budget_fy24', 'fund_number', 'VARCHAR'], ['analysis_operating_budget_fy24', 'dept_name', 'VARCHAR'], ['analysis_operating_budget_fy24', 'account', 'VARCHAR'], ['analysis_operating_budget_fy24', 'amount', 'DECIMAL(18,2)'],
    ['analysis_cfs_2025', 'incident_num', 'VARCHAR'], ['analysis_cfs_2025', 'date_time', 'TIMESTAMP'], ['analysis_cfs_2025', 'call_type', 'VARCHAR'], ['analysis_cfs_2025', 'disposition', 'VARCHAR'], ['analysis_cfs_2025', 'beat', 'INTEGER'],
    ['analysis_ripa_stops_2025', 'stop_id', 'VARCHAR'], ['analysis_ripa_stops_2025', 'date_stop', 'DATE'], ['analysis_ripa_stops_2025', 'reason_for_stop', 'VARCHAR'], ['analysis_ripa_stops_2025', 'beat', 'INTEGER'],
    ['analysis_police_beats', 'beat', 'VARCHAR'], ['analysis_police_beats', '_geometry', 'VARCHAR'], ['analysis_police_beats', '_crs', 'VARCHAR'],
    ['analysis_get_it_done_encampments', 'service_request_id', 'VARCHAR'], ['analysis_get_it_done_encampments', 'date_requested', 'TIMESTAMP'], ['analysis_get_it_done_encampments', 'status', 'VARCHAR'], ['analysis_get_it_done_encampments', 'council_district', 'INTEGER'],
  ].map(([tableName, columnName, logicalType]) => ({ tableName, columnName, logicalType, originalName: columnName.toUpperCase(), nullable: true, dictionary: null })),
  templates: [
    { templateId: 'budget-by-dept', title: 'Spending by department', description: 'Adopted operating budget summed by department.', sql: 'SELECT dept_name, SUM(amount) AS adopted FROM analysis_operating_budget_fy24 GROUP BY 1 ORDER BY 2 DESC', denominator: 'All adopted budget lines for FY2024' },
    { templateId: 'sleeper-dispositions', title: 'Sleeper call dispositions', description: 'How calls coded SLEEPER were closed.', sql: "SELECT disposition, COUNT(*) AS calls FROM analysis_cfs_2025 WHERE call_type = 'SLEEPER' GROUP BY 1 ORDER BY 2 DESC", denominator: 'Calls coded SLEEPER in 2025' },
    { templateId: 'encampment-by-district', title: 'Encampment reports by council district', description: 'Get It Done encampment reports per district.', sql: 'SELECT council_district, COUNT(*) AS reports FROM analysis_get_it_done_encampments GROUP BY 1 ORDER BY 1', denominator: 'Every encampment report in the snapshot' },
  ],
};

/** The SLEEPER beats the calls fixtures count: the ten listed, then a long tail. */
const LISTED_BEATS = [[521, 1210], [524, 980], [523, 744], [522, 611], [527, 402], [627, 388], [813, 301], [314, 266], [511, 240], [626, 233]];
const ALL_BEATS = [...LISTED_BEATS, ...[115, 122, 233, 312, 325, 432, 446, 512, 513, 614, 712, 722, 811, 833, 931].map((beat, index) => [beat, 200 - index * 12])];
const NO_BEAT = 23;

/**
 * Beat polygons as the GeoJSON import stores them: one feature per beat, a 6 by 5 grid of
 * irregular quadrilaterals over central San Diego, with one beat (931) left out of the file so the
 * map's "no boundary in the file" note has a case.
 */
const BEAT_SHAPES = ALL_BEATS.filter(([beat]) => beat !== 931).map(([beat], index) => {
  const column = index % 6; const row = Math.floor(index / 6);
  const jitter = (k) => (((beat * 31 + k * 17) % 11) - 5) * 0.0012;
  const west = -117.26 + column * 0.035; const south = 32.69 + row * 0.04;
  const ring = [[west + jitter(1), south + jitter(2)], [west + 0.035 + jitter(3), south + jitter(4)], [west + 0.035 + jitter(5), south + 0.04 + jitter(6)], [west + jitter(7), south + 0.04 + jitter(8)]];
  return [String(beat), JSON.stringify({ type: 'Polygon', coordinates: [[...ring, ring[0]]] }), null];
});

/** @param {string} sql @returns {import('../src/types').SqlQueryResult} */
const sqlResult = (sql) => {
  const lowered = sql.toLowerCase();
  // AutomaticDiscovery.tsx BeatMap: the beat polygons, read when the map is opened.
  if (lowered.includes('_geometry')) {
    return { columns: ['beat', '_geometry', '_crs'], rows: BEAT_SHAPES, rowCount: BEAT_SHAPES.length, truncated: false, elapsedMs: 31, provenance: ['snap-pd-beats'] };
  }
  // serviceAccountability.ts dailyCountsSql (calls_daily): one row per year, days packed 'MM-DD:n'.
  if (lowered.includes('span_days')) {
    const first = Date.UTC(2024, 0, 1);
    const last = Date.UTC(2026, 8, 30);
    const years = new Map();
    for (let ms = first, i = 0; ms <= last; ms += 86_400_000, i += 1) {
      const date = new Date(ms);
      const weekday = (date.getUTCDay() + 6) % 7;
      // A weekday rhythm, a slow seasonal swell and deterministic noise; some days have none.
      const n = Math.max(0, Math.round((weekday < 5 ? 17 : 9) + 5 * Math.sin(i / 58) + ((i * 7919) % 13) - 6));
      if (n === 0) continue;
      const year = date.getUTCFullYear();
      years.set(year, [...(years.get(year) ?? []), `${date.toISOString().slice(5, 10)}:${n}`]);
    }
    const columns = ['year', 'days', 'first_day', 'last_day', 'span_years', 'undated'];
    const rows = [...years].map(([year, days]) => [year, days.join(','), '2024-01-01', '2026-09-30', '2024,2025,2026', 14]);
    return { columns, rows, rowCount: rows.length, truncated: false, elapsedMs: 188, provenance: ['snap-cfs-2025'] };
  }
  // callLeads.ts calls_outcomes: one row per call type and year.
  if (lowered.includes('as crisis_team')) {
    const columns = ['call_year', 'call_type', 'calls', 'cancelled_duplicate', 'blank', 'conflicting', 'arrest', 'report', 'no_report', 'unfounded', 'other', 'crisis_team', 'other_codes', 'last_call'];
    const rows = [
      [2026, 'SLEEPER', 4210, 380, 22, 4, 96, 141, 2790, 205, 572, 61, 'O; C; P', '2026-09-30 23:41:00'],
      [2026, 'CW', 18420, 2100, 80, 10, 410, 2950, 9800, 1120, 1950, 640, 'O; C; P; T', '2026-09-30 23:58:00'],
      [2026, '602', 7300, 820, 30, 2, 690, 610, 3980, 520, 648, 25, 'O; C', '2026-09-30 22:10:00'],
      [2026, '5150', 3900, 310, 12, 1, 40, 1810, 1180, 90, 457, 820, 'O; T', '2026-09-30 21:02:00'],
      [2026, '647F', 1450, 160, 8, 0, 380, 210, 520, 60, 112, 14, 'O', '2026-09-30 20:44:00'],
      [2025, 'SLEEPER', 5980, 512, 31, 6, 162, 230, 3856, 301, 882, 88, 'O; C; P', '2025-12-31 23:50:00'],
      [2024, 'SLEEPER', 6125, 540, 40, 3, 211, 265, 3650, 344, 1072, 12, 'O; C', '2024-12-31 23:12:00'],
      [null, 'SLEEPER', 14, 0, 0, 0, 0, 0, 12, 0, 2, 0, null, null],
    ];
    return { columns, rows, rowCount: rows.length, truncated: false, elapsedMs: 212, provenance: ['snap-cfs-2025'] };
  }
  // callLeads.ts calls_sleeper_beats: the ten beats with most SLEEPER calls.
  if (lowered.includes('citywide_calls')) {
    const columns = ['beat', 'neighborhood', 'calls', 'citywide_calls', 'no_beat', 'first_year', 'last_year', 'all_beats'];
    // Every beat's count, as the map reads it; with the calls with no beat they make the citywide total.
    const allBeats = ALL_BEATS.map(([beat, calls]) => `${beat}=${calls}`).join(',');
    const citywide = ALL_BEATS.reduce((sum, [, calls]) => sum + calls, 0) + NO_BEAT;
    const rows = LISTED_BEATS.map(([beat, calls]) => [String(beat), null, calls, citywide, NO_BEAT, 2024, 2026, allBeats]);
    return { columns, rows, rowCount: rows.length, truncated: false, elapsedMs: 96, provenance: ['snap-cfs-2025'] };
  }
  if (lowered.includes('cfs')) {
    return { columns: ['disposition', 'calls'], rows: [['K', 3880], ['A', 612], ['R', 455], ['CAN', 301], ['U', 214]], rowCount: 5, truncated: false, elapsedMs: 38, provenance: ['snap-cfs-2025'] };
  }
  if (lowered.includes('encampment')) {
    return { columns: ['council_district', 'reports'], rows: [[1, 1902], [2, 3410], [3, 9876], [4, 2210], [5, 812], [6, 1544], [7, 3301], [8, 7120], [9, 5280]], rowCount: 9, truncated: false, elapsedMs: 21, provenance: ['snap-gid-enc'] };
  }
  return {
    columns: ['dept_name', 'adopted'],
    rows: [['Police', 662_317_042.0], ['Fire-Rescue', 389_220_101.0], ['Environmental Services', 118_442_918.0], ['Homelessness Strategies and Solutions', 61_208_555.0], ['Parks and Recreation', 59_004_210.0], ['Library', 63_118_406.0]],
    rowCount: 6,
    truncated: false,
    elapsedMs: 44,
    provenance: ['snap-budget-fy24'],
  };
};

const sourceRef = { url: 'https://sandiego.nextrequest.com/documents/9012', snapshot: 'snap-2026-09-30', sha256: hex('ref9012') };
/** @type {import('../src/types').ResearchEvidence[]} */
const RESEARCH_EVIDENCE = [
  { id: 'ev-1', label: 'Invoices_FY2025_GoldenHall.pdf, p. 14', sourceUrl: sourceRef.url, portalKey: 'sandiego', requestId: 26_0418, documentId: 9012, characterOffset: 1220, sourceSha256: hex('raw9012'), text: 'Invoice 2025-06 for storage services, Agreement H-24-1180: $142,800.00.', sql: null, snapshotProvenance: [], sourceReferences: [sourceRef], limitations: 'One invoice of twelve; totals computed separately.' },
  { id: 'ev-2', label: 'Operator_Agreement_Amendment_3.pdf, §4.1', sourceUrl: 'https://sandiego.nextrequest.com/documents/9011', portalKey: 'sandiego', requestId: 26_0418, documentId: 9011, characterOffset: 4410, sourceSha256: hex('raw9011'), text: 'The total compensation under this Agreement shall not exceed One Million Two Hundred Fifty Thousand Dollars ($1,250,000.00).', sql: null, snapshotProvenance: [], sourceReferences: [], limitations: 'Later amendments may exist outside the release.' },
  { id: 'ev-3', label: 'Operating budget FY2024 query', sourceUrl: null, portalKey: null, requestId: null, documentId: null, characterOffset: null, sourceSha256: hex('snap-budget-fy24'), text: 'Homelessness Strategies and Solutions adopted: $61,208,555', sql: SCHEMA.templates[0].sql, snapshotProvenance: ['snap-budget-fy24'], sourceReferences: [], limitations: 'Adopted, not actual.' },
];

/** @type {import('../src/types').ResearchReport} */
const RESEARCH_REPORT = {
  id: 'rr-storage-ceiling',
  topic: 'Encampment property storage spending',
  createdAt: iso(30),
  model: 'local/qwen2.5-14b-instruct',
  evidence: RESEARCH_EVIDENCE,
  synthesis: {
    title: 'Storage contract payments exceeded the authorized ceiling',
    summary: 'Released invoices for Agreement H-24-1180 total $1,486,200 in FY2025 against a $1,250,000 ceiling stated in Amendment 3. No ceiling increase appears in the released records or the Council docket.',
    findings: [{
      title: 'Payments exceed ceiling by $236,200',
      interpretation: 'Either an unreleased amendment raised the ceiling, or payments continued past the authorized amount.',
      whyItMatters: 'Unauthorized payments would bypass Council approval thresholds under Charter § 99.',
      citations: [{ evidenceId: 'ev-1', quote: 'Agreement H-24-1180: $142,800.00' }, { evidenceId: 'ev-2', quote: 'shall not exceed One Million Two Hundred Fifty Thousand Dollars' }],
      counterEvidence: [],
      alternativeExplanations: ['A fourth amendment exists but was not produced.', 'Some invoices were paid from a separate purchase order.'],
      nextTest: 'Request all amendments and purchase orders referencing H-24-1180.',
    }],
    questions: [{ question: 'Was a fourth amendment approved administratively?', why: 'Would explain the excess without Council action.', evidenceIds: ['ev-2'] }],
    requests: [{ title: 'All amendments to Agreement H-24-1180', custodian: 'Purchasing & Contracting', gap: 'Amendments after No. 3', resolves: 'Whether the ceiling was raised', draft: 'Please produce all amendments, change orders and purchase orders referencing Agreement H-24-1180 from July 1, 2024 to present.', evidenceIds: ['ev-2'] }],
    meetings: [{ body: 'City Council', date: day(-6), agendaItem: 'Item 331: Shelter operations contracts', relevance: 'Ceiling increase may be ratified', action: 'Submit public comment', citations: [{ evidenceId: 'ev-2', quote: '$1,250,000.00' }] }],
    limitations: ['Invoices for June 2025 are partially redacted.'],
  },
  investigationLog: ['Collected 3 invoices', 'Queried FY2024 adopted budget', 'Compared ceiling with payment total'],
  inputTokens: 18_402,
  outputTokens: 2_118,
};

/** @type {import('../src/types').CaseworkCitation[]} */
const CASE_CITATIONS = RESEARCH_EVIDENCE.map((e) => ({
  id: `cc-${e.id}`, reportId: RESEARCH_REPORT.id, evidenceId: e.id, label: e.label, quote: e.text,
  sourceUrl: e.sourceUrl, sourceSha256: e.sourceSha256, sourceReferences: e.sourceReferences,
  portalKey: e.portalKey, documentId: e.documentId, characterOffset: e.characterOffset, verification: 'VERIFIED',
}));

/** The ranking rule's four factors (storage/casework.rs), so a lead's score is their sum out of 11. */
const leadFactors = (consequence, evidence, novelty, urgency) => [
  { label: 'Public consequence', score: consequence, maximum: 3, reason: consequence === 3 ? 'Finding concerns access to essential services or oversight; assess local impact before prioritizing.' : 'Finding concerns an allocation or public contract; verify materiality.' },
  { label: 'Evidence available', score: evidence, maximum: 3, reason: `${evidence} distinct cited source identities, capped at three. This counts available records, not independent corroboration or correctness.` },
  { label: 'New lead', score: novelty, maximum: 2, reason: novelty === 2 ? 'No identical title and interpretation found in saved earlier dossiers; changed wording can affect this signal.' : 'Same title and interpretation appeared in an earlier dossier.' },
  { label: 'Decision urgency', score: urgency, maximum: 3, reason: urgency > 0 ? `A cited source also supports an agenda item dated ${day(-6)}; verify its relevance, latest agenda and participation deadline.` : 'No linked cited decision window in the next 14 days.' },
];

/** @type {import('../src/types').CaseworkCase[]} */
const CASES = [
  {
    id: 'case-storage', reportId: RESEARCH_REPORT.id, topic: 'Encampment property storage', title: 'Storage contract ceiling', owner: 'R. Alvarez', disposition: 'ACTIVE',
    revision: 7, createdAt: iso(24 * 20), updatedAt: iso(5),
    questions: [{ id: 'q1', question: 'Who approved payments above the ceiling?', hypothesis: 'Payments were approved by the department without Council action.', nextTest: 'Obtain payment approval chain.', disposition: 'OPEN', owner: 'R. Alvarez', citationIds: ['cc-ev-1'], counterCitationIds: [] }],
    requests: [{
      id: 'cr1', title: 'Amendments to H-24-1180', custodian: 'Purchasing & Contracting', gap: 'Amendments after No. 3', resolves: 'Ceiling authority', draft: 'Please produce all amendments…', state: 'SUBMITTED',
      expectedRecordTypes: ['Contract amendment', 'Purchase order'], expectedFields: ['Ceiling', 'Approver'], expectedPeriod: 'FY2025', submittedAt: iso(24 * 12), responseDue: day(-2),
      externalReference: '26-0790', productions: [], unresolvedOmissions: ['Approval memo'], followUpDraft: '', citationIds: ['cc-ev-2'], compiledDraft: '',
      filingPlan: {
        statuteCitation: 'Gov. Code § 7922.535', evaluatedAt: day(0), filedOn: day(12), determinationDueOn: day(2), calendarDaysElapsed: 12, allowedCalendarDays: 10,
        determinationBasis: 'OBSERVATION_WITHOUT_RECORDED_ACTIVITY', measuredTo: null, deadlinePassedWithoutRecordedResponse: true,
        followUps: [{ trigger: 'DETERMINATION_OVERDUE', scheduledFor: day(-1), rationale: 'No determination is recorded two days after the modelled date; ask the custodian for a date certain.' }],
      },
    }, {
      id: 'cr2', title: 'Storage facility intake logs', custodian: 'Environmental Services', gap: 'Intake counts by month', resolves: 'Utilization claims', draft: 'Please produce intake logs…', state: 'ACKNOWLEDGED',
      expectedRecordTypes: ['Intake log'], expectedFields: ['Date', 'Items stored'], expectedPeriod: 'FY2025', submittedAt: iso(24 * 4), responseDue: null,
      externalReference: '26-0811', productions: [{ id: 'pr1', receivedAt: iso(24 * 1), description: 'First installment: July to September logs', recordTypes: ['Intake log'], fields: ['Date'], period: 'Q1 FY2025', citationIds: [], notes: '' }],
      unresolvedOmissions: [], followUpDraft: '', citationIds: [], compiledDraft: '',
      filingPlan: {
        statuteCitation: 'Gov. Code § 7922.535', evaluatedAt: day(0), filedOn: day(4), determinationDueOn: day(-6), calendarDaysElapsed: 4, allowedCalendarDays: 10,
        determinationBasis: 'OBSERVATION_WITHOUT_RECORDED_ACTIVITY', measuredTo: null, deadlinePassedWithoutRecordedResponse: false,
        followUps: [{ trigger: 'DETERMINATION_DUE', scheduledFor: day(-6), rationale: 'The modelled determination date; check the portal for a response that day.' }],
      },
    }, {
      id: 'cr3', title: 'Council ratification memo', custodian: 'City Clerk', gap: 'Ratification record', resolves: 'Approval chain', draft: 'Draft only', state: 'DRAFT',
      expectedRecordTypes: ['Memo'], expectedFields: [], expectedPeriod: '', submittedAt: null, responseDue: null,
      externalReference: '', productions: [], unresolvedOmissions: [], followUpDraft: '', citationIds: [], compiledDraft: '', filingPlan: null,
    }],
    decisions: [{ id: 'd1', body: 'City Council', date: day(-6), agendaItem: 'Item 331', proposedDecision: 'Ratify shelter operations contracts', action: 'Public comment', outcome: '', vote: '', state: 'UPCOMING', citationIds: [], outcomeCitationIds: [] }],
    commitments: [
      { id: 'cm1', promise: 'Publish quarterly storage utilization', responsibleParty: 'Environmental Services', dueDate: day(10), completionTest: 'Report posted on sandiego.gov', state: 'OVERDUE', assessment: 'No report located.', citationIds: [], completionCitationIds: [] },
      { id: 'cm2', promise: 'Brief Council on storage capacity', responsibleParty: 'Homelessness Strategies', dueDate: null, completionTest: 'Briefing on a Council agenda', state: 'PENDING', assessment: '', citationIds: [], completionCitationIds: [] },
    ],
    evidence: CASE_CITATIONS, notes: 'Coordinate with Council District 3 staff before the Oct 13 hearing.',
  },
  {
    id: 'case-notice', reportId: '', topic: 'Abatement notice timing', title: 'Imperial Avenue 72-hour notices', owner: 'J. Nguyen', disposition: 'ACTIVE',
    revision: 3, createdAt: iso(24 * 9), updatedAt: iso(30),
    questions: [], requests: [], decisions: [], commitments: [], evidence: CASE_CITATIONS.slice(0, 1), notes: '',
  },
];

/** @type {import('../src/types').CalendarMeeting[]} */
const MEETINGS = [
  {
    id: 'm-council-1013', jurisdiction: 'City of San Diego', body: 'City Council', bodyCitation: citation(9002, 0, 17, 'Council agenda'),
    date: day(-6), dateCitation: citation(9002, 0, 40, 'Agenda header'), dateNote: null, timing: 'UPCOMING', startTime: '10:00',
    documents: [
      { role: 'AGENDA', portalKey: 'sd-council-agendas', documentId: 9101, label: 'Council agenda, October 13, 2026', sourceUrl: 'https://sandiego.hylandcloud.com/agenda-1013.pdf', posting: { state: 'RECORDED', lastModified: iso(70) }, minutesBeforeMeeting: 8460, differingFetches: 1 },
      { role: 'VIDEO', portalKey: 'sd-council-agendas', documentId: 9102, label: 'Council meeting video', sourceUrl: null, posting: { state: 'NOT_RECORDED', reason: 'NO_FETCH_RECORD' }, minutesBeforeMeeting: null, differingFetches: 0 },
    ],
    revisions: [{ earlierDocumentId: 9100, earlierLabel: 'Agenda (first posting)', laterDocumentId: 9101, laterLabel: 'Agenda (revised)', orderBasis: 'LAST_MODIFIED', changes: [{ kind: 'ADDED', item: 'Item 331', before: null, after: 'Ratification of shelter operations and property storage contracts' }, { kind: 'REWORDED', item: 'Item 204', before: 'Safe Sleeping Program update', after: 'Safe Sleeping Program update and site expansion' }], titlesNotCompared: [] }],
    commentDeadlines: [citation(9002, 1500, 1600, 'eComment deadline')],
    commentInstructions: [citation(9002, 1400, 1500, 'Comment instructions')],
    suggestions: [{
      id: 's-331', agendaItem: 'Item 331', title: 'Ratification of shelter operations and property storage contracts', itemCitation: citation(9002, 1200, 1300, 'Item 331'),
      reasons: [{ kind: 'FINDING_SUBJECT', basis: 'COUNTED', reason: 'A stored finding counts payments above this contract\'s ceiling.', matched: 'H-24-1180', citation: citation(9012, 1100, 1200, 'Invoices'), findingId: 'f-contract-ceiling', gapKey: null, claimCheck: null }],
      comment: { text: 'Released invoices show $1,486,200 paid under Agreement H-24-1180 against a $1,250,000 ceiling. Please explain the authority for the excess before ratifying.', bases: ['COUNTED'], deadline: citation(9002, 1500, 1600, 'eComment deadline'), instructions: [] },
    }],
  },
  {
    id: 'm-luh-1015', jurisdiction: 'City of San Diego', body: 'Land Use and Housing Committee', bodyCitation: citation(9002, 0, 17, 'LU&H agenda'),
    date: day(-8), dateCitation: null, dateNote: null, timing: 'UPCOMING', startTime: '13:00',
    documents: [{ role: 'AGENDA', portalKey: 'sd-council-agendas', documentId: 9104, label: 'LU&H agenda, October 15, 2026', sourceUrl: null, posting: { state: 'RECORDED', lastModified: iso(30) }, minutesBeforeMeeting: 9000, differingFetches: 0 }],
    revisions: [], commentDeadlines: [], commentInstructions: [], suggestions: [],
  },
  {
    id: 'm-cpp-0924', jurisdiction: 'City of San Diego', body: 'Commission on Police Practices', bodyCitation: citation(9002, 0, 17, 'CPP agenda'),
    date: day(13), dateCitation: null, dateNote: null, timing: 'PAST', startTime: '18:00',
    documents: [
      { role: 'AGENDA', portalKey: 'sd-council-agendas', documentId: 9105, label: 'CPP agenda, September 24, 2026', sourceUrl: null, posting: { state: 'RECORDED', lastModified: iso(24 * 16) }, minutesBeforeMeeting: 4300, differingFetches: 0 },
      { role: 'MINUTES', portalKey: 'sd-council-agendas', documentId: 9103, label: 'CPP minutes, September 24, 2026', sourceUrl: null, posting: { state: 'RECORDED', lastModified: iso(24 * 5) }, minutesBeforeMeeting: -11520, differingFetches: 0 },
    ],
    revisions: [], commentDeadlines: [], commentInstructions: [], suggestions: [],
  },
];

/** @type {import('../src/types').ClaimedRequest[]} */
const CLAIMS = [
  {
    id: 'claim-1', portalKey: 'sandiego', reference: '26-0533', title: 'Unsafe Camping Ordinance citations by beat', sourceUrl: 'https://sandiego.nextrequest.com/requests/26-533', jurisdiction: 'CA_CPRA', claimedAt: iso(24 * 64),
    events: [
      { id: 'ev-sub', kind: 'SUBMITTED', occurredAt: iso(24 * 64), basis: 'USER_ENTERED', evidence: null, extensionReason: null, expectedDeterminationAt: null, createdAt: iso(24 * 64) },
      { id: 'ev-ext', kind: 'EXTENSION_NOTICE', occurredAt: iso(24 * 54), basis: 'EMAIL_CONFIRMED', evidence: { messageIdHash: hex('m1'), subjectExcerpt: 'RE: Records request 26-533', emailDate: iso(24 * 54), excerpt: 'Due to the need to search for records at separate field facilities, the City is extending the time to respond by 14 days.', sentenceStart: 0, sentenceEnd: 120, matchedSentence: 'the City is extending the time to respond by 14 days.' }, extensionReason: 'SEPARATE_FIELD_FACILITIES', expectedDeterminationAt: iso(24 * 40), createdAt: iso(24 * 53) },
    ],
    suggestions: [{ id: 'sg-1', kind: 'PARTIAL_PRODUCTION', candidateOccurredAt: iso(24 * 3), evidence: { messageIdHash: hex('m2'), subjectExcerpt: 'Records released: 26-533', emailDate: iso(24 * 3), excerpt: 'A first installment of responsive records has been released to the portal.', sentenceStart: 0, sentenceEnd: 74, matchedSentence: 'A first installment of responsive records has been released to the portal.' }, suggestedAt: iso(24 * 2) }],
    importedEmails: [{ messageIdHash: hex('m2'), subjectExcerpt: 'Records released: 26-533', emailDate: iso(24 * 3), importedAt: iso(24 * 2), suggestionCount: 1 }],
  },
  {
    id: 'claim-2', portalKey: 'sandiego', reference: '26-0561', title: 'Public restroom cleaning logs', sourceUrl: null, jurisdiction: 'CA_CPRA', claimedAt: iso(24 * 7),
    events: [{ id: 'ev-sub2', kind: 'SUBMITTED', occurredAt: iso(24 * 7), basis: 'USER_ENTERED', evidence: null, extensionReason: null, expectedDeterminationAt: null, createdAt: iso(24 * 7) }],
    suggestions: [], importedEmails: [],
  },
  {
    id: 'claim-3', portalKey: 'sandiego', reference: '26-0540', title: 'Safe Sleeping Program nightly census', sourceUrl: null, jurisdiction: 'CA_CPRA', claimedAt: iso(24 * 30),
    events: [
      { id: 'ev-sub3', kind: 'SUBMITTED', occurredAt: iso(24 * 30), basis: 'USER_ENTERED', evidence: null, extensionReason: null, expectedDeterminationAt: null, createdAt: iso(24 * 30) },
      { id: 'ev-det3', kind: 'DETERMINATION', occurredAt: iso(24 * 22), basis: 'EMAIL_CONFIRMED', evidence: { messageIdHash: hex('m3'), subjectExcerpt: 'Records request 26-540', emailDate: iso(24 * 22), excerpt: 'The City has determined that responsive records exist and will produce them by October 20.', sentenceStart: 0, sentenceEnd: 90, matchedSentence: 'The City has determined that responsive records exist.' }, extensionReason: null, expectedDeterminationAt: null, createdAt: iso(24 * 22) },
    ],
    suggestions: [], importedEmails: [],
  },
  {
    id: 'claim-4', portalKey: 'sandiego', reference: '26-0615', title: 'Outreach provider property storage emails', sourceUrl: null, jurisdiction: 'CA_CPRA', claimedAt: iso(24 * 50),
    events: [
      { id: 'ev-sub4', kind: 'SUBMITTED', occurredAt: iso(24 * 50), basis: 'USER_ENTERED', evidence: null, extensionReason: null, expectedDeterminationAt: null, createdAt: iso(24 * 50) },
      { id: 'ev-det4', kind: 'DETERMINATION', occurredAt: iso(24 * 26), basis: 'EMAIL_CONFIRMED', evidence: { messageIdHash: hex('m4'), subjectExcerpt: 'RE: 26-615 determination', emailDate: iso(24 * 26), excerpt: 'We have completed our search and determined that 41 records are responsive.', sentenceStart: 0, sentenceEnd: 76, matchedSentence: 'determined that 41 records are responsive.' }, extensionReason: null, expectedDeterminationAt: null, createdAt: iso(24 * 26) },
    ],
    suggestions: [], importedEmails: [],
  },
];

/**
 * The statute engine's analysis as the tracker and the portal read command return it. Days are
 * counted back from FIXTURE_NOW; `determination` is [daysAgo, basis] or null.
 * @param {number} requestId @param {number} createdDaysAgo @param {{ allowed?: number, extension?: string | null, determination?: [number, string] | null, observedDaysAgo?: number }} options
 */
const deadlineAnalysis = (requestId, createdDaysAgo, { allowed = 10, extension = null, determination = null, observedDaysAgo = 0 } = {}) => {
  const endDaysAgo = determination ? determination[0] : observedDaysAgo;
  return {
    requestId, statuteCitation: 'Gov. Code § 7922.535', createdAt: iso(createdDaysAgo * 24),
    determinationAt: determination ? iso(determination[0] * 24) : null,
    determinationSource: determination?.[1] === 'SOURCED_DETERMINATION_EVENT' ? 'email' : null,
    determinationBasis: determination ? determination[1] : 'OBSERVATION_WITHOUT_RECORDED_ACTIVITY',
    observedAt: iso(observedDaysAgo * 24),
    initialDeadlineAt: iso((createdDaysAgo - 10) * 24), effectiveDeadlineAt: iso((createdDaysAgo - allowed) * 24),
    calendarDaysElapsed: createdDaysAgo - endDaysAgo, allowedCalendarDays: allowed,
    extensionStatus: extension ? 'VALID' : 'NOT_INVOKED', extensionReason: extension,
    isViolation: createdDaysAgo - endDaysAgo > allowed, anomalyMarkers: [], evidenceSummary: '',
  };
};

/** @type {Record<string, import('../src/types').RequestDeadline>} */
const TRACKER_DEADLINES = {
  'claim-1': { claimId: 'claim-1', model: 'CALIFORNIA_CPRA', analysis: deadlineAnalysis(260533, 64, { allowed: 24, extension: 'SEPARATE_FIELD_FACILITIES' }), countdown: { kind: 'DEADLINE_PASSED', days: 40 }, determinationBasis: 'OBSERVATION_WITHOUT_RECORDED_ACTIVITY', violationAsserted: false, basisStatement: 'No determination is recorded. The extended period ended 40 days ago, counted from the submission date you entered.', notes: [], pendingSuggestionCount: 1 },
  'claim-2': { claimId: 'claim-2', model: 'CALIFORNIA_CPRA', analysis: deadlineAnalysis(260561, 7), countdown: { kind: 'DAYS_REMAINING', days: 3 }, determinationBasis: 'OBSERVATION_WITHOUT_RECORDED_ACTIVITY', violationAsserted: false, basisStatement: 'The period is still running and is counted from the submission date you entered.', notes: [], pendingSuggestionCount: 0 },
  'claim-3': { claimId: 'claim-3', model: 'CALIFORNIA_CPRA', analysis: deadlineAnalysis(260540, 30, { determination: [22, 'SOURCED_DETERMINATION_EVENT'] }), countdown: { kind: 'DETERMINATION_RECORDED', days: null }, determinationBasis: 'SOURCED_DETERMINATION_EVENT', violationAsserted: false, basisStatement: 'A determination confirmed from an email is recorded on day 8 of 10.', notes: [], pendingSuggestionCount: 0 },
  'claim-4': { claimId: 'claim-4', model: 'CALIFORNIA_CPRA', analysis: deadlineAnalysis(260615, 50, { determination: [26, 'SOURCED_DETERMINATION_EVENT'] }), countdown: { kind: 'DETERMINATION_RECORDED', days: null }, determinationBasis: 'SOURCED_DETERMINATION_EVENT', violationAsserted: true, basisStatement: 'The determination confirmed from an email is dated 24 days after submission; 10 are allowed.', notes: [], pendingSuggestionCount: 0 },
};

/** @type {import('../src/types').WatchSummary[]} */
const WATCHES = [
  { watch: { id: 'w-esd', kind: 'AGENCY', target: 'sandiego:102', label: 'Environmental Services Department', createdAt: iso(24 * 30), enabled: true }, lastCheckedAt: iso(4), lastDimensions: [{ dimension: 'DOCUMENTS', outcome: 'CHANGED', changeCount: 3, baselineAsOf: iso(24 * 7), observedAsOf: iso(4), reason: null }, { dimension: 'FINDINGS', outcome: 'UNCHANGED', changeCount: 0, baselineAsOf: iso(24 * 7), observedAsOf: iso(4), reason: null }] },
  { watch: { id: 'w-harborview', kind: 'ENTITY', target: 'e-org-harborview', label: 'Harborview Shelter Partners', createdAt: iso(24 * 18), enabled: true }, lastCheckedAt: iso(4), lastDimensions: [{ dimension: 'FIGURES', outcome: 'CHANGED', changeCount: 1, baselineAsOf: iso(24 * 7), observedAsOf: iso(4), reason: null }] },
  { watch: { id: 'w-source', kind: 'SOURCE', target: 'sdhc', label: 'San Diego Housing Commission reports', createdAt: iso(24 * 10), enabled: false }, lastCheckedAt: null, lastDimensions: [] },
];
/** @type {import('../src/types').WatchChange[]} */
const WATCH_CHANGES = [
  { id: 'wc-1', watchId: 'w-esd', dimension: 'DOCUMENTS', changeKind: 'ADDED', subject: 'Crew_Dispatch_Log_Apr-Jun_2026.xlsx', oldValue: null, newValue: '96 KB', oldAsOf: null, newAsOf: iso(4), basis: 'Portal listing', detectedAt: iso(4), portalKey: 'sandiego' },
  { id: 'wc-2', watchId: 'w-harborview', dimension: 'FIGURES', changeKind: 'CHANGED', subject: 'Contract ceiling, Agreement H-24-1180', oldValue: '$1,250,000', newValue: '$1,650,000', oldAsOf: iso(24 * 7), newAsOf: iso(4), basis: 'Council staff report 26-118', detectedAt: iso(4), portalKey: 'sd-council-agendas' },
];

/** @param {string} taxPeriod @param {number} grants @returns {import('../src/types').Form990Statement} */
const statement990 = (taxPeriod, revenue, expenses, grants) => ({
  returnVersion: '2022v5.0', ein: '954417820', filerName: 'HARBORVIEW SHELTER PARTNERS', taxPeriodBegin: `${Number(taxPeriod.slice(0, 4)) - 1}-07-01`, taxPeriodEnd: `${taxPeriod.slice(0, 4)}-06-30`,
  amended: false, totalRevenue: revenue, totalExpenses: expenses, totalAssetsEndOfYear: 4_210_000, totalLiabilitiesEndOfYear: 1_380_000, netAssetsEndOfYear: 2_830_000, governmentGrants: grants,
  significantDiversionReported: false, formerListed: false, totalReportableCompensation: 612_000, people: [], scheduleJAttached: false, severancePaymentReported: null, scheduleJPeople: [], scheduleJExplanations: [],
  scheduleLAttached: false, scheduleLTransactions: [], scheduleLExplanations: [], scheduleOExplanations: [],
});
/** @param {number} taxYear @param {string | null} taxPeriod */
const summaryYear = (taxYear, taxPeriod, revenue, expenses, assets, liabilities) => ({ taxYear, taxPeriod, formType: '990', totalRevenue: revenue, totalExpenses: expenses, totalAssetsEndOfYear: assets, totalLiabilitiesEndOfYear: liabilities, officerCompensation: 410_000 });
/**
 * Harborview's record: no extract for tax year 2020 (a missing year), two extracts for 2022 (a
 * changed fiscal period), a null expense figure; index years read, listed-not-read, checked-none and
 * unchecked; payments with an undated row, an unreadable amount and a skipped fiscal year.
 * @type {import('../src/types').OrganisationFilings}
 */
const ORGANISATION_FILINGS = {
  ein: '954417820', name: 'Harborview Shelter Partners', checkedAt: iso(24 * 4),
  indexYears: [{ year: 2019, checkedAt: iso(24 * 4), returnsListed: 0 }, { year: 2021, checkedAt: iso(24 * 4), returnsListed: 1 }, { year: 2022, checkedAt: iso(24 * 4), returnsListed: 1 }, { year: 2023, checkedAt: iso(24 * 4), returnsListed: 2 }, { year: 2024, checkedAt: iso(24 * 4), returnsListed: 1 }],
  returns: [
    { objectId: '202301239349300410', indexYear: 2023, returnType: '990', taxPeriod: '202206', taxpayerName: 'HARBORVIEW SHELTER PARTNERS', archiveUrl: null, memberName: null, xmlSha256: hex('x2022'), retrievedAt: iso(24 * 4), statement: statement990('202206', 6_480_000, 6_120_000, 4_910_000), unreadReason: null },
    { objectId: '202401239349300515', indexYear: 2024, returnType: '990', taxPeriod: '202306', taxpayerName: 'HARBORVIEW SHELTER PARTNERS', archiveUrl: null, memberName: null, xmlSha256: hex('x2023'), retrievedAt: iso(24 * 4), statement: statement990('202306', 7_920_000, 7_610_000, 6_150_000), unreadReason: null },
    { objectId: '202201239349300222', indexYear: 2022, returnType: '990', taxPeriod: '202106', taxpayerName: 'HARBORVIEW SHELTER PARTNERS', archiveUrl: null, memberName: null, xmlSha256: null, retrievedAt: null, statement: null, unreadReason: 'Listed in the index; the return XML was not retrieved.' },
  ],
  summary: {
    name: 'HARBORVIEW SHELTER PARTNERS', city: 'SAN DIEGO', state: 'CA', sourceUrl: 'https://projects.propublica.org/nonprofits/organizations/954417820', retrievedAt: iso(24 * 4),
    years: [
      summaryYear(2018, '201806', 2_940_000, 2_810_000, 2_100_000, 900_000),
      summaryYear(2019, '201906', 3_310_000, 3_150_000, 2_380_000, 960_000),
      summaryYear(2021, '202106', 5_020_000, null, 3_400_000, 1_150_000),
      summaryYear(2022, '202206', 6_480_000, 6_120_000, 3_860_000, 1_240_000),
      summaryYear(2022, '202212', 3_390_000, 3_010_000, 3_990_000, 1_300_000),
      summaryYear(2023, '202306', 7_920_000, 7_610_000, 4_210_000, 1_380_000),
    ],
  },
  singleAudits: null, warnings: [],
  publicMoney: [
    { role: 'payment', datasetName: 'Vendor payments FY2025', sourceUrl: null, vendorValue: 'HARBORVIEW SHELTER PARTNERS', fiscalYear: '2021', rows: 14, amount: '1186400.00' },
    { role: 'payment', datasetName: 'Vendor payments FY2025', sourceUrl: null, vendorValue: 'HARBORVIEW SHELTER PARTNERS', fiscalYear: '2023', rows: 19, amount: '1940250.50' },
    { role: 'payment', datasetName: 'Vendor payments FY2025', sourceUrl: null, vendorValue: 'HARBORVIEW SHELTER PARTNERS', fiscalYear: '2024', rows: 9, amount: null },
    { role: 'payment', datasetName: 'Vendor payments FY2025', sourceUrl: null, vendorValue: 'HARBORVIEW SHELTER PARTNERS', fiscalYear: '2025', rows: 6, amount: '612000.00' },
    { role: 'payment', datasetName: 'Vendor payments FY2025', sourceUrl: null, vendorValue: 'HARBORVIEW SHELTER PARTNERS', fiscalYear: null, rows: 3, amount: '4800.00' },
    { role: 'contract', datasetName: 'Contracts register', sourceUrl: null, vendorValue: 'HARBORVIEW SHELTER PARTNERS', fiscalYear: '2023', rows: 1, amount: '1250000.00' },
  ],
  publicMoneyNotes: [],
};

/** @type {import('../src/types').StandingQuerySummary[]} */
const STANDING = [{
  query: { id: 'sq-1', label: 'Abatement notices citing 63.0403', astJson: '{"term":"63.0403"}', filters: { searchTerm: '63.0403', portalKey: 'sandiego', agencyId: 102, status: null, policyDeviation: null }, createdAt: iso(24 * 14) },
  lastRun: { runId: 'run-1', queryId: 'sq-1', ranAt: iso(6), corpusDigest: hex('corpus'), corpusDocumentCount: 1842, result: { matchCount: 44, matchesDigest: hex('m44'), countBasis: { basis: 'EXACT' }, facets: [
    { dimension: 'agency', values: [{ value: 'Environmental Services Department', count: 31 }, { value: 'San Diego Police Department', count: 13 }], truncated: false },
    { dimension: 'status', values: [{ value: 'Closed', count: 29 }, { value: 'Overdue', count: 9 }, { value: 'In progress', count: 6 }], truncated: false },
    // Exact and complete: 2021 has no matching request, a measured zero.
    { dimension: 'year', values: [{ value: '2026', count: 17 }, { value: '2025', count: 14 }, { value: '2024', count: 8 }, { value: '2022', count: 3 }, { value: '2020', count: 2 }], truncated: false },
  ], clusters: [{ extractedTextSha256: hex('cl1'), requestCount: 4, documentCount: 9, exampleFileName: 'Abatement_Notices_Imperial_Ave_Q1-2026.pdf' }] }, changedSincePrevious: true, previousMatchCount: 39 },
  currentCorpusDigest: hex('corpus'), due: false,
}, {
  query: { id: 'sq-2', label: 'Requests mentioning encampment', astJson: '{"term":"encampment"}', filters: { searchTerm: 'encampment', portalKey: null, agencyId: null, status: null, policyDeviation: null }, createdAt: iso(24 * 40) },
  lastRun: { runId: 'run-2', queryId: 'sq-2', ranAt: iso(30), corpusDigest: hex('corpus-prev'), corpusDocumentCount: 1790, result: { matchCount: 2614, matchesDigest: hex('m2614'), countBasis: { basis: 'ESTIMATED_FROM_SAMPLE', sampleSize: 1000 }, facets: [
    { dimension: 'portal', values: [{ value: 'sandiego', count: 2091 }, { value: 'sdcounty', count: 523 }], truncated: false },
    // Estimated and truncated: the 2018 and 2020 slots are 'none listed', never zero.
    { dimension: 'year', values: [{ value: '2026', count: 741 }, { value: '2025', count: 690 }, { value: '2024', count: 512 }, { value: '2023', count: 340 }, { value: '2022', count: 201 }, { value: '2021', count: 88 }, { value: '2019', count: 34 }, { value: '2017', count: 12 }], truncated: true },
  ], clusters: [] }, changedSincePrevious: false, previousMatchCount: 2614 },
  currentCorpusDigest: hex('corpus'), due: true,
}];

/** Kinds whose `is_legal_claim()` is true (analysis/finding.rs): only these earn the counted factor. */
const COUNTED_KINDS = new Set(['POLICY_VIOLATION_DETECTED', 'PRACTICE_DEPARTURE_COUNTED', 'CONTRACT_CEILING_EXCEEDED', 'PAYMENT_OUTSIDE_CONTRACT_TERM', 'APPROPRIATION_EXCEEDED', 'ENCAMPMENT_NOTICE_SHORTFALL']);
const CONTRADICTION_KINDS = new Set(['CONTRADICTORY_STATEMENT', 'METADATA_CONTRADICTS_CONTENT', 'NO_RECORDS_CONTRADICTED_BY_RELEASE', 'CLAIM_CONTRADICTS_DATA']);

/**
 * The ranker's factors (analysis/story_desk.rs), with its exact names and caps: money 800/1600/2500,
 * scale ≤ 900 + 400 + 1200, recurrence ≤ 1000, contradiction 2000, counted 1800, evidence strength
 * confidence/5 ≤ 2000 (0 when unassessed), freshness 400/200.
 * @param {typeof FINDINGS[number]} f @param {typeof FINDINGS} live
 */
function storyFactors(f, live) {
  const factors = [];
  const dollars = [...`${f.headline} ${f.explanation}`.matchAll(/\$([\d,]+(?:\.\d+)?)\s*(million|M)?/g)].map((m) => Number(m[1].replace(/,/g, '')) * (m[2] ? 1_000_000 : 1));
  const largest = Math.max(0, ...dollars);
  const money = largest >= 1_000_000 ? 2500 : largest >= 100_000 ? 1600 : largest >= 10_000 ? 800 : 0;
  if (money) factors.push({ name: 'money at stake', basisPoints: money, evidence: `largest figure named: $${largest.toLocaleString('en-US')}` });
  const related = live.filter((other) => other.id !== f.id && other.subject.requestId === f.subject.requestId);
  const documents = new Set([f, ...related].flatMap((item) => item.citations.map((c) => c.documentId)));
  const requests = new Set([f, ...related].map((item) => item.subject.requestId));
  const cross = f.kind.startsWith('CROSS_CORPUS_');
  const scale = Math.min(900, 150 * Math.max(0, documents.size - 1)) + (requests.size > 1 ? 400 : 0) + (cross ? 1200 : 0);
  if (scale) factors.push({ name: 'scale', basisPoints: scale, evidence: `${documents.size} documents, ${requests.size} requests${cross ? ' (cross-corpus finding)' : ''}` });
  const siblings = live.filter((other) => other.id !== f.id && other.sourceDetector === f.sourceDetector && other.subject.portalKey === f.subject.portalKey).length;
  if (siblings) factors.push({ name: 'recurrence', basisPoints: Math.min(1000, 200 * siblings), evidence: `${siblings} other undismissed findings from ${f.sourceDetector}` });
  if (CONTRADICTION_KINDS.has(f.kind)) factors.push({ name: 'record contradicts statement', basisPoints: 2000, evidence: `finding kind ${f.kind}` });
  if (COUNTED_KINDS.has(f.kind)) factors.push({ name: 'counted statutory or policy violation', basisPoints: 1800, evidence: `finding kind ${f.kind} is a counted violation` });
  factors.push(f.confidence.kind === 'MEASURED'
    ? { name: 'evidence strength', basisPoints: Math.min(2000, Math.floor(f.confidence.basisPoints / 5)), evidence: `measured confidence ${(f.confidence.basisPoints / 100).toFixed(2)}%` }
    : { name: 'evidence strength', basisPoints: 0, evidence: 'confidence unassessed' });
  const ageHours = (FIXTURE_NOW - Date.parse(f.lastSeenAt)) / 3_600_000;
  if (ageHours <= 24 * 7) factors.push({ name: 'freshness', basisPoints: 400, evidence: 'seen within the last 7 days' });
  else if (ageHours <= 24 * 30) factors.push({ name: 'freshness', basisPoints: 200, evidence: 'seen within the last 30 days' });
  return factors;
}

const LIVE_FINDINGS = FINDINGS.filter((f) => !f.dismissedAt);
/** @type {import('../src/types').StoryLead[]} */
const STORY_LEADS = LIVE_FINDINGS.map((f) => {
  const factors = storyFactors(f, LIVE_FINDINGS);
  return {
    findingId: f.id, kind: f.kind, headline: f.headline, severity: f.severity, confidence: f.confidence, subject: f.subject, portalKey: f.subject.portalKey,
    publicInterest: { totalBasisPoints: factors.reduce((sum, factor) => sum + factor.basisPoints, 0), factors },
    relatedFindingIds: FINDINGS.filter((other) => other.id !== f.id && other.subject.requestId === f.subject.requestId).map((other) => other.id),
    firstSeenAt: f.discoveredAt, lastSeenAt: f.lastSeenAt,
  };
}).sort((a, b) => b.publicInterest.totalBasisPoints - a.publicInterest.totalBasisPoints).slice(0, 10);

/** @type {import('../src/types').LicenseStatus} */
const LICENSE = {
  enforced: true, authorized: true, state: 'LICENSED', message: 'Licensed', bundleId: 'com.onrecord.studio', appVersion: '1.0.0', originalAppVersion: '1.0.0', appTransactionId: '2000000812345678',
  appId: 6_740_000_001, environment: 'direct', originalPurchaseDate: iso(24 * 90), signedDate: iso(24), revocationDate: null, plan: 'licensed', trialEndsAt: null, daysLeft: null,
  accountEmail: 'reporter@example.org', signInProvider: 'apple', entitlementExpiresAt: iso(-24 * 275),
};

/** @type {Record<string, Partial<import('../src/types').LicenseStatus>>} */
export const LICENSE_VARIANTS = {
  trial: { state: 'TRIAL', plan: 'trial', trialEndsAt: iso(-24 * 9), daysLeft: 9, message: 'Trial active' },
  trialExpired: { state: 'TRIAL_EXPIRED', authorized: false, plan: 'trial', trialEndsAt: iso(24 * 2), daysLeft: 0, message: 'Your trial has ended.' },
  signInRequired: { state: 'SIGN_IN_REQUIRED', authorized: false, plan: null, accountEmail: null, signInProvider: null, message: 'Sign in to continue.' },
  deviceLimit: { state: 'DEVICE_LIMIT_REACHED', authorized: false, message: 'This license is active on 3 devices.' },
};

/** @param {number} documentId @param {string} portalKey @returns {import('../src/types').DocumentProvenanceVerification} */
const provenance = (documentId, portalKey) => ({
  documentId, portalKey, expectedSha256: hex(`raw${documentId}`), computedSha256: hex(`raw${documentId}`), evidenceFilePresent: true, hashMatches: true,
  recordedDigestSource: 'INGESTION_RECORD', manifestPresent: false, isCryptographicallySigned: false, signatureVerification: 'NOT_ATTEMPTED', signatureIssuer: null,
  signingCertificateSerialNumber: null, signingTimestamp: null, assertionCount: 0, invalidAssertionCount: 0,
  claimGraph: { activeManifestLabel: null, claims: [], ingredients: [], validationStatuses: [], certificateChainTrusted: false, certificateChainDiagnostics: [], truncated: false },
  textProjectionSha256: hex(`text${documentId}`), textProjectionMatches: true, provenanceConfidence: 'LOCAL_DIGEST_MATCH', chainOfCustodyConfirmed: false, verificationErrors: [],
});

/** @param {number} requestId @param {string} portalKey @returns {import('../src/types').LegalComplianceFinding[]} */
const legalFinding = (requestId, portalKey, varianceDays, reviewStatus, id = `lv-${requestId}`) => ({
  violationId: id, requestId, portalKey, matchedStatuteId: 'ca-cpra', matchedVersionId: 'ca-cpra-2023', mandateId: 'determination-10-day', statuteCitation: 'Gov. Code § 7922.535',
  authorityTitle: 'California Public Records Act', mandateTitle: 'Determination within 10 days', classification: 'STATUTORY_DEADLINE', calculatedVarianceMetric: `+${varianceDays} days`, varianceDays,
  confidenceBasisPoints: 9000, evidenceSummary: `Determination dated ${varianceDays + 10} days after submission; no valid extension notice recorded.`, supportingDocumentIds: [9002], missingEvidencePhrases: [],
  reviewStatus, firstDetectedAt: iso(24 * 3), evaluatedAt: iso(3),
});

/**
 * Stored findings per request: an open one and an older dismissed one on 26-0412, an open one
 * measured three hours ago on the still-open 26-0533, and one re-evaluation withdrew on 26-0418.
 */
const legalFindings = (requestId, portalKey) => {
  if (requestId === 26_0412) return [legalFinding(requestId, portalKey, 29, 'OPEN'), legalFinding(requestId, portalKey, 31, 'DISMISSED', `lv-${requestId}-old`)];
  if (requestId === 26_0533) return [legalFinding(requestId, portalKey, 54, 'OPEN')];
  if (requestId === 26_0418) return [legalFinding(requestId, portalKey, 12, 'RESOLVED_BY_NEW_EVIDENCE')];
  if (requestId === 26_0615) return [legalFinding(requestId, portalKey, 27, 'CONFIRMED')];
  return [];
};

/** The portal deadline read: 26-0418 has a sourced determination and an extension; any other closed request uses closure as its proxy. */
const portalDeadline = (requestId) => {
  const row = REQUEST_ROWS.find(([id]) => id === requestId) ?? REQUEST_ROWS[0];
  const [, , , status, created, closed] = row;
  const analysis = requestId === 26_0418
    ? deadlineAnalysis(requestId, created, { allowed: 24, extension: 'VOLUMINOUS_RECORDS', determination: [created - 30, 'SOURCED_DETERMINATION_EVENT'] })
    : deadlineAnalysis(requestId, created, { determination: closed === null ? null : [closed, 'EARLIEST_RECORDED_ACTIVITY_PROXY'] });
  return { ...analysis, status, closedAt: closed === null ? null : iso(closed * 24) };
};

/** @type {import('../src/types').LegalAuthoritySummary[]} */
const AUTHORITIES = [
  { statuteId: 'ca-cpra', jurisdiction: 'California', statuteCitation: 'Gov. Code § 7920.000 et seq.', title: 'California Public Records Act', authorityKind: 'STATE_STATUTE', currentVersionId: 'ca-cpra-2023', contentSha256: hex('cpra'), mandatedDeadlineDays: 10, canonicalSourceUri: 'https://leginfo.legislature.ca.gov/faces/codes_displayexpandedbranch.xhtml?tocCode=GOV&division=10.', effectiveFrom: /** @type {import('../src/types').IsoDateString} */ ('2023-01-01'), effectiveTo: null, provisionCount: 214, mandateCount: 6, importedAt: iso(24 * 60) },
  { statuteId: 'sdmc-63-0403', jurisdiction: 'San Diego', statuteCitation: 'SDMC § 63.0403', title: 'Unsafe Camping Ordinance', authorityKind: 'MUNICIPAL_ORDINANCE', currentVersionId: 'sdmc-63-0403-2023', contentSha256: hex('uco'), mandatedDeadlineDays: null, canonicalSourceUri: 'https://docs.sandiego.gov/municode/MuniCodeChapter06/Ch06Art03Division04.pdf', effectiveFrom: /** @type {import('../src/types').IsoDateString} */ ('2023-07-30'), effectiveTo: null, provisionCount: 9, mandateCount: 3, importedAt: iso(24 * 45) },
  { statuteId: 'sdpd-dp-3-33', jurisdiction: 'San Diego', statuteCitation: 'SDPD DP 3.33', title: 'Body Worn Cameras', authorityKind: 'AGENCY_POLICY', currentVersionId: 'dp-3-33-2024', contentSha256: hex('dp333'), mandatedDeadlineDays: null, canonicalSourceUri: null, effectiveFrom: null, effectiveTo: null, provisionCount: 41, mandateCount: 2, importedAt: iso(24 * 20) },
];

/** @type {Handlers} */
const full = {
  get_license_status: () => LICENSE,
  refresh_license: () => LICENSE,
  get_account_status: () => ({ available: true, signedIn: true, email: 'reporter@example.org', provider: 'apple' }),
  list_seats: () => ({ subscriptionActive: true, quantity: 3, usedSeats: 2, seats: [{ email: 'reporter@example.org', invitedAt: iso(24 * 90), claimedAt: iso(24 * 90), revokedAt: null }, { email: 'editor@example.org', invitedAt: iso(24 * 30), claimedAt: iso(24 * 29), revokedAt: null }] }),
  list_saved_portals: () => PORTALS.map((registration, index) => ({ registration, lifecycleState: index === 2 ? 'PAUSED' : 'ACTIVE', healthStatus: index === 3 ? 'DEGRADED' : 'HEALTHY', lastError: index === 3 ? 'HTTP 503 from sandiego.hylandcloud.com after 3 retries' : null })),
  list_portal_runtimes: () => PORTALS.slice(0, 2).map((p, index) => ({
    portal: p, syncRunId: `run-${p.portalKey}`, queuedTasks: index === 0 ? 42 : 0, activeTasks: index === 0 ? 3 : 0, completedTasks: 1810 - index * 600, failedTasks: 4 - index * 4,
    pagesProcessed: 96 - index * 40, requestsProcessed: 1204 - index * 400, documentsProcessed: 2841 - index * 900, isRunning: index === 0, rateLimitTokens: 4,
    frontierExclusions: [{ rule: 'excludeUrlPrefixes', excludedLinks: 17 }], stoppedHosts: index === 0 ? [{ host: 'sandiego.hylandcloud.com', requestsSpent: 120, reason: 'Repeated HTTP 503' }] : [],
  })),
  get_collection_plan: () => ({ sources: [] }),
  get_ocr_capability: () => ({ engine: 'tesseract', engineAvailable: true, engineVersion: '5.4.1', languagePacks: ['eng', 'spa'], languageProbeComplete: true, pdfRenderer: 'pdfium', pdfRendererAvailable: true, pdfRendererVersion: '6721', imageOcrAvailable: true, pdfOcrAvailable: true, diagnostics: [] }),
  get_agency_performance_profiles: () => PROFILES,
  get_cross_agency_correlations: () => CORRELATIONS,
  // The sixth department has 3 requests: a small sample, dimmed with its chip.
  get_department_dispositions: () => AGENCIES.slice(0, 6).map((a, index) => {
    const counts = index === 5
      ? [['RELEASED', 1], ['RELEASED_IN_PART', 0], ['WITHHELD', 1], ['NO_RECORDS', 0], ['WITHDRAWN', 0], ['NOT_CLASSIFIED', 1]]
      : [['RELEASED', 22 - index], ['RELEASED_IN_PART', 14], ['WITHHELD', 6 + index], ['NO_RECORDS', 9], ['WITHDRAWN', 2], ['NOT_CLASSIFIED', 7 - index]];
    return {
      portalSubdomain: a.portal, agencyId: a.id, agencyName: a.name, totalRequests: counts.reduce((sum, [, n]) => sum + Number(n), 0),
      buckets: counts.map(([disposition, n]) => ({
        disposition: /** @type {import('../src/types').Disposition} */ (disposition), requests: Array.from({ length: Number(n) }, (_, i) => ({ requestId: 26_0400 + i * 7 + index, rpid: `26-${400 + i * 7 + index}` })),
      })),
    };
  }),
  refresh_findings: () => ({ detectorsRun: 28, findingsRecorded: FINDINGS.length, staleCleared: 2 }),
  // A lookup of one finding by id carries a stored lead's figure, as commands/findings.rs sends it.
  list_findings: ({ filter, limit }) => filterFindings(filter).slice(0, limit ?? 500)
    .map((item) => (filter?.id && LEAD_FIGURES[item.id] ? { ...item, figure: LEAD_FIGURES[item.id] } : item)),
  count_findings: ({ filter }) => filterFindings(filter).length,
  dismiss_finding: () => null,
  restore_finding: () => null,
  list_evidence_gaps: () => [
    { detector: 'abatement_logs', key: 'gap-crew-logs-2024', route: 'CPRA request', blocked: 'Abatement timing before 2025', role: 'Crew dispatch logs', request: 'Crew dispatch logs for encampment abatements, calendar 2024', status: 'noAutomaticSource', sources: [] },
    { detector: 'capacity', key: 'gap-ssp-o-street', route: 'Open data', blocked: 'Capacity comparison for O Lot', role: 'Nightly capacity', request: 'Safe Sleeping O Lot nightly capacity', status: 'acquiring', sources: ['https://www.sandiego.gov/homelessness-strategies-and-solutions'] },
    { detector: 'spending', key: 'gap-fy25-actuals', route: 'Open data', blocked: 'FY2025 actuals', role: 'Operating actuals', request: 'Operating actuals FY2025', status: 'exhausted', sources: ['https://data.sandiego.gov/datasets/operating-actuals/'] },
  ],
  list_story_leads: ({ limit }) => STORY_LEADS.slice(0, limit ?? 50),
  build_lead_dossier: ({ findingId }) => {
    const f = FINDINGS.find((x) => x.id === findingId) ?? FINDINGS[0];
    return {
      finding: f,
      evidence: f.citations.map((c) => ({ portalKey: c.portalKey, documentId: c.documentId, fileName: documentName(c.documentId), requestId: documentRequest(c.documentId), agencyLabel: request(documentRequest(c.documentId)).agencyName, rawFileSha256: hex(`raw${c.documentId}`), characterStart: c.characterStart, characterEnd: c.characterEnd, quote: c.quote, label: c.label })),
      evidenceTruncated: false,
      related: FINDINGS.filter((x) => x.id !== f.id && x.subject.requestId === f.subject.requestId).map((x) => ({ id: x.id, kind: x.kind, headline: x.headline, severity: x.severity, confidence: x.confidence, basis: /** @type {const} */ ('SAME_REQUEST'), discoveredAt: x.discoveredAt })),
      relatedTruncated: false,
      falsification: { statement: 'This lead is wrong if a notice or amendment dated before the removal exists but was not produced.', recordsToSeek: ['Posting photographs with timestamps', 'Amendment No. 4 to H-24-1180'] },
      gaps: [{ gap: 'notComputed', portalKey: 'sandiego', documentId: 9003, reason: 'Spreadsheet times are local without a zone; not converted.' }],
      gapsTruncated: false,
      assembledAt: iso(0.1),
    };
  },
  draft_availability_for_finding: () => ({ followup: true, appeal: true }),
  draft_followup_from_finding: ({ findingId }) => drafted(findingId, 'FOLLOWUP_REQUEST'),
  draft_appeal_from_finding: ({ findingId }) => drafted(findingId, 'APPEAL'),
  draft_appeal_from_statutory_deadline: () => drafted('f-policy-esd', 'APPEAL'),
  query_audit_records_page: ({ searchTerm, portalKey, agencyId, pageSize }) => {
    const term = searchTerm.trim().toLowerCase();
    const records = REQUESTS.filter((r) => (!term || r.title.toLowerCase().includes(term) || r.descriptionPreview.toLowerCase().includes(term)) && (!portalKey || r.portalKey === portalKey) && (agencyId == null || r.agencyId === agencyId));
    return { records: records.slice(0, pageSize), nextCursor: null, total: records.length };
  },
  count_audit_records: ({ searchTerm }) => ({ total: REQUESTS.filter((r) => !searchTerm || r.title.toLowerCase().includes(searchTerm.toLowerCase())).length }),
  list_request_documents: ({ requestId }) => DOCUMENTS.filter((d) => documentRequest(d.documentId) === requestId),
  get_document_text_chunk: ({ documentId, portalKey, characterOffset, characterLimit }) => textChunk(documentId, portalKey, characterOffset ?? 0, characterLimit ?? 20_000),
  get_document_entity_truncations: () => [],
  list_document_claim_checks: ({ documentId, portalKey }) => ({ documentId, portalKey, truncated: false, checks: [
    { claimText: '$1,486,200.00', kindLabel: 'Contract total', characterStart: 1180, characterEnd: 1193, scope: { fiscalYear: 2025, fund: '100000', department: 'Environmental Services', basis: 'actual' }, result: { outcome: 'mismatch', claimed: '$1,486,200.00', observed: '$1,402,950.00', difference: '$83,250.00', band: '±0.5%', queryIds: ['q-1'] } },
    { claimText: '214 pages', kindLabel: 'Production size', characterStart: 330, characterEnd: 339, scope: { fiscalYear: null, fund: null, department: null, basis: null }, result: { outcome: 'unreconciled', reason: 'NO_DATASET', detail: 'No dataset counts released pages.' } },
  ] }),
  export_claim_query_pack: ({ documentId, portalKey }) => ({ documentId, portalKey, generatedAt: iso(0), checks: [], queries: [], datasets: [], truncated: false }),
  describe_document_origin: ({ documentId, portalKey }) => ({ documentId, portalKey, origin: 'COLLECTED_FROM_SOURCE', sourceKind: 'NEXT_REQUEST', heldRelation: { exactCopies: { state: 'COMPARED', matches: [], truncated: false, comparedCount: 1842 }, sameText: { state: 'COMPARED', matches: [{ portalKey: 'sdcounty', sourceName: 'County of San Diego', requestId: 7_311, requestLabel: '25-7311', documentId: 9082, fileName: 'HHSA_Response_Letter.pdf' }], truncated: false, comparedCount: 1842 }, nearDuplicates: { state: 'NOT_COMPARED', reason: 'NOT_IN_SIMILARITY_INDEX' } } }),
  read_document_internal_metadata: () => ({ state: 'READ', format: 'PDF', fields: [{ name: 'Producer', value: 'Adobe PDF Library 23.1.175' }, { name: 'Creator', value: 'Microsoft Word for Microsoft 365' }, { name: 'CreationDate', value: '2026-04-10T09:12:44-07:00' }, { name: 'ModDate', value: '2026-04-10T09:14:02-07:00' }, { name: 'Author', value: null }] }),
  list_document_provenance_verifications: ({ documentId, portalKey }) => [{ verificationId: `pv-${documentId}`, documentId, portalKey, verifiedAt: iso(2), expectedSha256: hex(`raw${documentId}`), computedSha256: hex(`raw${documentId}`), evidenceFilePresent: true, hashMatches: true, manifestPresent: false, isCryptographicallySigned: false, signatureIssuer: null, signingCertificateSerialNumber: null, signingTimestamp: null, assertionCount: 0, invalidAssertionCount: 0, recordedDigestSource: 'INGESTION_RECORD', signatureVerification: 'NOT_ATTEMPTED', provenanceConfidence: 'LOCAL_DIGEST_MATCH', textProjectionSha256: hex(`text${documentId}`), textProjectionMatches: true, chainOfCustodyConfirmed: false, verificationErrors: [], predatesClassification: false }],
  verify_document_provenance: ({ documentId, portalKey }) => provenance(documentId, portalKey),
  carve_document_version_history: () => ({ sourceSha256: hex('raw9002'), sourceSizeBytes: 182_004, totalEofMarkers: 2, historicalVersionsFound: 1, inspectedHistoricalVersions: 1, revisionDiagnostics: [], recoveredTextSegments: [{ revisionNumber: 1, snapshotEndOffset: 171_220, recoveredCharacterCount: 64, recoveredTextSha256: hex('rec'), maskedFindingCount: 0, maskedText: 'Notices for February 12 and 13 are withheld pending review.' }], postEofMetadataArtifacts: [], analysisTruncated: false, warnings: [] }),
  audit_document_residual_evidence: () => ({ findings: [{ class: 'historical', confidence: 'strong', kind: 'INCREMENTAL_UPDATE', locator: { containerPart: null, objectReference: '42 0 R', byteOffset: 171_220, pageNumber: 2, bounds: null, xmlPath: null }, valueSha256: hex('res'), maskedPreview: 'withheld pending review', explanation: 'An earlier revision contains a sentence absent from the final text.' }], warnings: [], coverageComplete: true }),
  verify_document_redaction_integrity: () => ({ analysisVersion: 'redaction/3', sourceSha256: hex('raw9061'), scannedPageCount: 41, rasterBackdropPages: 0, coverageComplete: true, requiresManualReview: true, isLeakyRedaction: true, extractedHiddenText: '[2 masked spans]', boundingBoxClashesCount: 2, metadataLeaks: [], isolatedStructuralAnomalies: [], pageFindings: [{ pageNumber: 7, potentialIntersections: 2, darkOverlayIntersections: 2, redactionAnnotationIntersections: 0, invisibleTextIntersections: 0, unappliedRedactionAnnotations: 0, formInvocationsAnalyzed: 0, coverageComplete: true, intersections: [{ kind: 'DARK_OVERLAY', textBounds: [72, 400, 300, 412], maskBounds: [70, 398, 302, 414] }] }], calculatedRedactionRatio: 0.04 }),
  list_document_annotations: () => ({ textAvailable: true, currentTextSha256: hex('text9002'), annotations: [{ id: 'an-1', portalKey: 'sandiego', documentId: 9002, kind: 'ISSUE', characterOffset: 610, quote: 'no responsive records were located', prefix: 'February 13 are not included because ', suffix: '. The City has determined', textSha256: hex('text9002'), contextUnanchored: false, note: 'Dispatch log shows crews on site both days.', tag: 'ABSENCE', author: 'R. Alvarez', status: 'OPEN', findingId: 'f-absence', importedFromKey: null, createdAt: iso(26), anchor: { state: 'ANCHORED', sourceOffset: 610, sourceLength: 34, reason: null } }] }),
  get_annotation_settings: () => ({ reviewer: 'R. Alvarez', codebook: [{ code: 'ABSENCE', label: 'Missing records', definition: 'Records that should exist but were not produced.' }, { code: 'TIMING', label: 'Notice timing', definition: 'Posting or removal times relevant to SDMC § 63.0403.' }] }),
  search_document_annotations: () => ({ rows: [{ ...{ id: 'an-1', portalKey: 'sandiego', documentId: 9002, kind: /** @type {const} */ ('ISSUE'), characterOffset: 610, quote: 'no responsive records were located', prefix: '', suffix: '', textSha256: null, contextUnanchored: false, note: 'Dispatch log shows crews on site both days.', tag: 'ABSENCE', author: 'R. Alvarez', status: /** @type {const} */ ('OPEN'), findingId: 'f-absence', importedFromKey: null, createdAt: iso(26), anchor: null }, requestId: 26_0412, fileName: 'ESD_Response_Letter_26-0412.pdf' }], nextCursor: null }),
  resolve_page_regions: () => [],
  read_document_original: ({ documentId }) => originalFrame(documentId),
  get_request_statutory_extension: () => null,
  list_request_legal_findings: ({ requestId, portalKey }) => legalFindings(requestId, portalKey),
  get_portal_request_deadline: ({ requestId }) => portalDeadline(requestId),
  evaluate_request_legal_compliance: ({ requestId, portalKey }) => ({ requestId, portalKey, evaluatedMandateCount: 6, applicableMandateCount: 2, corpusCharacterCount: 18_220, evaluationComplete: true, warnings: [], findings: legalFindings(requestId, portalKey) }),
  list_legal_authorities: () => AUTHORITIES,
  list_legal_authority_versions: ({ statuteId }) => [{ versionId: `${statuteId}-v1`, statuteId, contentSha256: hex(statuteId), effectiveFrom: null, effectiveTo: null, sourceUri: null, importedAt: iso(24 * 60) }],
  get_legal_authority_version: ({ statuteId, versionId }) => ({ version: { versionId, statuteId, contentSha256: hex(statuteId), effectiveFrom: null, effectiveTo: null, sourceUri: null, importedAt: iso(24 * 60) }, fullText: '7922.535. (a) Each agency, upon a request for a copy of records, shall, within 10 days from receipt of the request, determine whether the request, in whole or in part, seeks copies of disclosable public records in the possession of the agency…' }),
  query_corpus_patterns: () => PATTERNS,
  get_corpus_index_status: () => ({ indexedDocumentCount: 1842, indexedSegmentCount: 61_220, indexedTermCount: 402_118, indexedRelationshipCount: 9_884, availableWriteCapacity: 1_000_000, algorithmVersion: 'corpus-patterns/4' }),
  list_top_corpus_entities: ({ limit }) => ENTITIES.slice(0, limit ?? 50),
  get_corpus_entity_neighborhood: ({ entityId }) => ({ center: entityById(entityId), edges: edgesFor(entityId), truncated: false }),
  page_corpus_entity_neighbors: ({ entityId, pageSize }) => ({ center: entityById(entityId), edges: edgesFor(entityId).slice(0, pageSize), nextCursor: null }),
  count_corpus_entity_neighbors: ({ entityId }) => ({ total: edgesFor(entityId).length, unfilteredTotal: edgesFor(entityId).length, byKind: [{ kind: 'ORGANIZATION', count: 2 }, { kind: 'PERSON', count: 2 }, { kind: 'STATUTE', count: 2 }, { kind: 'MONEY', count: 1 }, { kind: 'ROLE', count: 1 }] }),
  find_entity_paths: ({ fromEntityId }) => ({ outcome: 'found', path: { hops: 2, weakestConfidenceBasisPoints: 7800, steps: [{ neighbor: entityById(fromEntityId), edgeKind: 'SIGNED_BY', edgeBasis: 'OBSERVED', sharedDocumentCount: 6 }, { neighbor: ENTITIES[0], edgeKind: 'APPROVED', edgeBasis: 'OBSERVED', sharedDocumentCount: 4 }] } }),
  list_entity_brokers: () => ENTITIES.slice(0, 5).map((e, index) => ({ entity: e, brokerageScore: 0.81 - index * 0.11, distinctNeighbors: 24 - index * 3, isApproximation: index > 2 })),
  get_resolution_clusters: () => null,
  lookup_registry_anchor: ({ identifierKind }) => ({ state: 'NO_REGISTRY', identifierKind }),
  get_embedding_model_status: () => ({ modelId: 'BAAI/bge-small-en-v1.5', revision: '5c38ec7', dimension: 384, distanceMetric: 'cosine', normalization: 'l2', cacheDir: '~/Library/Application Support/OnRecord Studio/models', installed: true, ready: true, artifactCount: 4, artifactManifestSha256: hex('model'), lastError: null }),
  get_embedding_index_status: () => ({ indexedDocumentCount: 1780, indexedChunkCount: 58_902, pendingDocumentCount: 62, failedDocumentCount: 0, running: false, cancelled: false, lastError: null }),
  get_analytical_schema: () => SCHEMA,
  list_analytical_import_candidates: () => [{ documentId: 9041, portalKey: 'sandiego', fileName: 'Homelessness_Budget_FY2024_Adopted_vs_Actual.xlsx', localPath: '/Users/reporter/Library/Application Support/OnRecord Studio/evidence/9041.xlsx', sourceUri: 'https://sandiego.nextrequest.com/documents/9041', sourceSha256: hex('raw9041') }],
  run_analytical_sql: ({ sql }) => sqlResult(sql),
  hybrid_search: ({ query, limit }) => DOCUMENTS.slice(0, Math.min(limit ?? 10, 8)).map((d, index) => ({
    stableId: `hs-${d.documentId}`, portalKey: d.portalKey, requestId: documentRequest(d.documentId), documentId: d.documentId, fileName: d.fileName, title: request(documentRequest(d.documentId)).title,
    agencyName: request(documentRequest(d.documentId)).agencyName, jurisdiction: 'San Diego', documentType: 'letter', publicationDate: day(30 + index), observationDate: null, effectiveDate: null,
    characterStart: 330, characterEnd: 510, excerpt: `…${LETTER.slice(330, 480)}… (${query})`, lexicalRank: index + 1, semanticScore: 0.82 - index * 0.04, fusedScore: 0.9 - index * 0.05, retrievalStatus: 'HYBRID',
  })),
  get_encampment_ordinance_report: () => ({
    nightHours: { sources: [{ snapshotId: 'snap-cfs-2025', displayName: 'Police calls for service 2025', sourceSha256: hex('snap-cfs-2025') }], firstActivity: iso(24 * 400), lastActivity: iso(24 * 30), sections: [{ section: '63.0403(a)(1)', activityType: 'Citation', activities: 1204, insideWindow: 980, atWindowBoundary: 41, outsideWindow: 160, timeNotRecorded: 23 }], listedActivities: [{ activityNumber: 'E25-118204', activityType: 'Citation', section: '63.0403(a)(1)', recordedAt: iso(24 * 200), atWindowBoundary: true, location: '1400 block Commercial St' }] },
    locations: { incidentSources: [], schoolSource: null, stationSource: null, sections: [{ section: '63.0403(a)(2)', offenses: 412, distinctCases: 388, notGeocoded: 19, withinNearBand: 140, withinFarBand: 201, beyondFarBand: 52 }], incidents: [] },
    notMeasured: ['Shelter availability on the night of each citation is not in these datasets.'],
  }),
  get_data_maintenance_status: () => ({ policy: { enabled: true, autoImport: true, autoRefresh: true, refreshHours: 24, retentionMode: 'ORIGINALS', retainedFileBudgetBytes: 20_000_000_000 }, effectiveAt: iso(24 * 3), retainedFileBytes: 6_420_118_004, referencedDocuments: 12, reclaimedFileBytes: 0, events: [{ action: 'REFRESH', detail: 'Refreshed 4 city datasets', recordedAt: iso(6) }] }),
  run_data_maintenance: () => ({ policy: { enabled: true, autoImport: true, autoRefresh: true, refreshHours: 24, retentionMode: 'ORIGINALS', retainedFileBudgetBytes: 20_000_000_000 }, effectiveAt: iso(24 * 3), retainedFileBytes: 6_420_118_004, referencedDocuments: 12, reclaimedFileBytes: 0, events: [] }),
  list_registry_snapshots: () => [{ registry: 'CSLB_LICENSE_MASTER', identifierKind: 'CONTRACTOR_LICENSE', sourcePath: '/Users/reporter/Downloads/MasterLicenseData.csv', fileSha256: hex('cslb'), importedAt: iso(24 * 15), recordCount: 301_442, skippedRowCount: 18 }],
  list_published_packs: () => [{ pack: 'san-diego-city', repositoryUrl: 'https://huggingface.co/datasets/onrecord/san-diego-city' }],
  fetch_published_pack_manifest: ({ pack }) => ({ pack, snapshots: true, packVersion: '2026.09.30', schemaVersion: '3', builtAt: iso(24 * 7), signerKeyId: 'onrecord-pack-2026', licenceId: 'ODC-By-1.0', licenceUrl: 'https://opendatacommons.org/licenses/by/1-0/', disclaimer: 'Republished City of San Diego open data. Check the source for corrections.', manifestSha256: hex('pack'), tables: [{ table: 'calls_for_service', files: [{ path: 'calls_for_service/2025.parquet', year: 2025, rows: 512_388, bytes: 21_044_100, partial: false, sourceUrl: DATASETS[1].sourceUri ?? '', retrievedAt: iso(24 * 7), datasetKind: 'CALLS_FOR_SERVICE', licence: null }], unchangedChecks: 4, lastCheckedAt: iso(24), changeRecords: 0 }] }),
  list_topic_packs: () => ({ topics: [{ id: 'encampments', title: 'Encampments and abatements', cues: ['encampment', 'abatement', '63.0403', 'unsafe camping'], documentTotal: 44, documents: [{ portalKey: 'sandiego', documentId: 9001, requestId: 26_0412, fileName: 'Abatement_Notices_Imperial_Ave_Q1-2026.pdf', matchedCues: ['abatement', '63.0403'], textCoverage: 'FULL' }], requestTotal: 6, requests: [{ portalKey: 'sandiego', requestId: 26_0412, title: REQUESTS[0].title, matchedCues: ['abatement'] }], findingTotal: 2, findings: [{ basis: 'COUNTED', anchorRequestId: 26_0412, finding: FINDINGS[1] }] }, { id: 'restrooms', title: 'Public restrooms', cues: ['restroom', 'comfort station'], documentTotal: 3, documents: [], requestTotal: 1, requests: [], findingTotal: 0, findings: [] }], documentsScanned: 1842, documentsPrefixOnly: 120, documentsFileNameOnly: 8, documentLimitReached: false, requestsScanned: 412, requestLimitReached: false, textPrefixCharacters: 20_000 }),
  list_starter_sets: () => ({ registryVersion: 4, sets: [{ id: 'homelessness', title: 'Homelessness accountability', summary: 'Sources for shelter capacity, abatements and spending.', entries: [{ id: 'sd-hssd', kind: 'SOURCE', title: 'Homelessness Strategies and Solutions', url: 'https://www.sandiego.gov/homelessness-strategies-and-solutions', what: 'Program reports and shelter dashboards', publisher: 'City of San Diego', termsNote: null, verifiedAsOf: day(20) }, { id: 'sd-nextrequest', kind: 'RECORD_REQUEST_TARGET', title: 'City of San Diego public records portal', url: 'https://sandiego.nextrequest.com', what: 'Released records and request status', publisher: 'City of San Diego', termsNote: null, verifiedAsOf: day(20) }] }] }),
  run_intent_query: ({ utterance }) => ({ utterance, status: 'ANSWERED', plan: { kind: 'FINDINGS_FOR_AGENCY', agency: 'Environmental Services Department' }, summary: '4 stored findings name Environmental Services Department.', rows: FINDINGS.slice(0, 4).map((f) => ({ id: f.id, headline: f.headline, detail: f.explanation, basis: 'STORED_FINDING', basisStatement: 'A stored finding with its own citations.', relationshipKind: null, confidenceBasisPoints: f.confidence.kind === 'MEASURED' ? f.confidence.basisPoints : null, agencyName: 'Environmental Services Department', portalKeys: ['sandiego'], citations: f.citations, omittedCitationCount: 0, citationsUnavailableReason: null })), truncated: false, rowLimit: 25, notes: [], options: [] }),
  get_meeting_calendar: () => ({ today: day(0), meetings: MEETINGS, postingCounts: { agendaFilesWithBothTimes: 3, under72HoursBeforeStart: 1, afterMeetingStart: 0 }, limitations: ['Posting times come from HTTP Last-Modified headers, not the Clerk\'s posting log.'] }),
  list_watches: () => WATCHES,
  list_watch_changes: ({ limit }) => WATCH_CHANGES.slice(0, limit),
  check_watches: () => [],
  list_standing_queries: () => STANDING,
  run_standing_queries: () => [],
  list_series_number_gaps: () => ({ gaps: [{ label: 'Agreement H-24-####', prefix: 'H-24-', fromNumber: 1181, toNumber: 1184, width: 4, before: { entityId: 'e-series-h24', displayValue: 'H-24-1180', documentCount: 9 }, after: { entityId: 'e-series-h24b', displayValue: 'H-24-1185', documentCount: 2 }, seriesMemberCount: 14 }], seriesCount: 3, truncated: false }),
  get_casework_state: () => ({ availableEvidence: CASE_CITATIONS, cases: CASES, events: [{ caseId: 'case-storage', revision: 7, author: 'R. Alvarez', at: iso(5), summary: 'Added request 26-0790' }], leads: [
    { reportId: RESEARCH_REPORT.id, findingIndex: 0, title: 'Payments exceed ceiling by $236,200', nextAction: 'Request amendments', score: 10, factors: leadFactors(2, 3, 2, 3), citations: CASE_CITATIONS.slice(0, 2), caseId: 'case-storage' },
    { reportId: RESEARCH_REPORT.id, findingIndex: 1, title: 'Storage utilization report not published', nextAction: 'Ask Environmental Services for the Q1 report', score: 5, factors: leadFactors(3, 1, 1, 0), citations: CASE_CITATIONS.slice(0, 1), caseId: 'case-storage' },
  ] }),
  get_case_scope: ({ caseId }) => ({ caseId, documents: [{ portalKey: 'sandiego', documentId: 9011 }, { portalKey: 'sandiego', documentId: 9012 }], requests: [{ portalKey: 'sandiego', requestId: 26_0418 }], findingIds: ['f-contract-ceiling'] }),
  get_record_gap_requests: () => [],
  compromised_case_imports: () => [],
  case_export_chain: () => [],
  get_research_status: () => ({ policy: { enabled: true, autoCollect: false, intervalHours: 24, provider: 'ollama', model: 'qwen2.5:14b-instruct', dailyCallLimit: 40, researchPasses: 2, discoverSources: true }, credentialAvailable: true, phase: 'IDLE', lastError: null, lastFinishedAt: iso(30), callsToday: 6, reports: [RESEARCH_REPORT], sources: [{ label: 'City of San Diego', url: 'https://sandiego.nextrequest.com', state: 'OK', detail: '1,204 requests' }], actionStates: {} }),
  get_review_workspace: () => ({ report: RESEARCH_REPORT, reviews: [], challenges: [{ findingIndex: 0, title: 'Payments exceed ceiling by $236,200', independentSources: 2, checks: ['Invoice totals recomputed'], blockers: ['Amendment No. 4 not located'], nextTest: 'Request amendments' }], replays: [{ evidenceId: 'ev-1', checkedAt: iso(3), state: 'MATCHED', detail: 'Digest unchanged' }], relationships: [], connections: [], identityWarnings: [] }),
  get_investigation_evaluations: () => [{ id: 'eval-1', title: 'Storage ceiling investigation', checkedAt: iso(20), checks: [{ name: 'Every claim cited', passed: true, detail: '4 of 4' }, { name: 'Counter-evidence searched', passed: false, detail: 'No counter search recorded' }], assessments: [], limitations: [] }],
  get_financial_workspace: () => ({ datasets: [{ snapshotId: 'snap-budget-fy24', name: 'Operating budget FY2024', tableName: 'analysis_operating_budget_fy24', sourceUrl: DATASETS[0].sourceUri, sourceSha256: DATASETS[0].sourceSha256, columns: ['fund_number', 'dept_name', 'account', 'amount'], partial: false }], mappings: [], defaultScope: { fiscalYear: 2024, fund: null, department: null }, roles: ['BUDGET', 'ACTUALS', 'CONTRACTS', 'PAYMENTS'], fields: ['amount', 'fund', 'department'], requirements: { BUDGET: ['amount', 'fund'] }, warnings: ['No payments dataset mapped.'], fiscalYears: [2023, 2024], funds: ['100000'], departments: ['Homelessness Strategies and Solutions'] }),
  get_facility_coverage: () => ({ reports: [], warnings: ['No facility dataset mapped yet.'] }),
  get_monitoring_status: ({ topic, caseId }) => ({ topic, caseId: caseId ?? null, coverage: [], changes: [], observations: [], comparisons: [], sourceCandidates: [], limitations: ['Monitoring compares stored snapshots only.'] }),
  list_organisation_checks: () => [{ ein: '954417820', name: 'Harborview Shelter Partners', checkedAt: iso(24 * 4) }],
  list_organisation_payment_mappings: () => [],
  list_organisation_payees: () => [{ vendorValue: 'HARBORVIEW SHELTER PARTNERS', rows: 48, datasetNames: ['Vendor payments FY2025'] }],
  get_organisation_filings: ({ ein }) => (ein === ORGANISATION_FILINGS.ein ? ORGANISATION_FILINGS : null),
  list_trusted_contributors: () => [{ keyId: 'ed25519:7f3a91c2', label: 'J. Nguyen (desk partner)', trustedAt: iso(24 * 12) }],
  get_local_contributor_identity: () => ({ keyId: 'ed25519:1b9e44d0' }),
  get_profile: () => ({ status: 'SET', profile: { schemaVersion: 1, displayName: 'R. Alvarez', affiliations: [{ id: 'aff-1', name: 'Voice of the Bay Newsroom', aliases: ['VOTB'], basis: 'SELF_DECLARED', createdAt: iso(24 * 60), verifiedDomain: null }], matchDecisions: [], verifiedDomains: [], updatedAt: iso(24 * 60) } }),
  list_affiliation_matches: () => ({ status: 'NO_MATCH_YET', candidates: [], searchedPortalCount: 4, searchedAgencyCount: 8, searchedEntityCount: 12, limitations: [] }),
  get_affiliation_disclosure: () => ({ status: 'DECLARED', sentence: 'The author is affiliated with Voice of the Bay Newsroom.', affiliations: [{ name: 'Voice of the Bay Newsroom', basis: 'SELF_DECLARED' }], ownPortalKeys: [], ownAgencies: [], ownEntityIds: [] }),
  list_claimed_requests: () => CLAIMS,
  get_request_deadline: ({ claimId }) => TRACKER_DEADLINES[claimId] ?? TRACKER_DEADLINES['claim-2'],
  list_resolution_candidates: () => ({ candidates: [{ pairId: 'pair-1', leftEntityId: 'e-org-harborview', rightEntityId: 'e-org-harborview-inc', evidence: { basis: 'STRONG_IDENTIFIER', left: { key: { kind: 'ORGANIZATION', value: 'harborview shelter partners' }, displayValue: 'Harborview Shelter Partners', documentCount: 38, examples: [] }, right: { key: { kind: 'ORGANIZATION', value: 'harborview shelter partners inc' }, displayValue: 'Harborview Shelter Partners, Inc.', documentCount: 6, examples: [] }, sharedIdentifier: { kind: 'EIN', displayValue: '95-4417820', leftDocuments: 4, rightDocuments: 2 }, nameVariants: ['HARBORVIEW SHELTER PTNRS'] } }], nextCursor: null, remaining: 0, scanTruncated: false, guards: { skippedNameGroups: 0, skippedIdentifiers: 0 }, ledgerHead: hex('ledger') }),
  list_resolution_ledger: () => ({ items: [], nextCursor: null, total: 0, entryCount: 0, head: hex('ledger'), summary: { byBasis: [] }, contradictions: [] }),
  get_corpus_state_digest: () => ({ digest: hex('corpus'), documentCount: 1842 }),
  list_shared_folder_packets: () => ({ packets: [], truncated: false }),
  get_signing_key_revocations: () => ({ schemaVersion: '1', entries: [] }),
  preview_shareable_bundle: ({ request: { findingIds } }) => ({ schemaVersion: 2, included: FINDINGS.filter((f) => findingIds.includes(f.id)).map((f) => ({ findingId: f.id, headline: f.headline, severity: f.severity, basisLabel: 'Counted', sourceCount: 1, excerptCount: f.citations.length, maskedValueCount: 0 })), excluded: [], sourceNames: ['City of San Diego'], contentSha256: hex('bundle') }),
  preview_packet_annotations: () => ({ included: 1, orphaned: 0, notPublicOmitted: 0 }),
  draft_overdue_follow_up: () => ({ artifact: drafted('f-policy-esd', 'FOLLOWUP_REQUEST'), agencyRecord: null, agencyParagraphIncluded: false, agencyOptions: [] }),
  verify_evidence_package: () => ({ schemaVersion: '3', canonicalBytesHashMatches: true, signatureValid: true, manifestMatchesCanonicalBytes: true, overallPassed: true, identityAssurance: 'LOCAL_KEY', unverifiedCitationLabels: [], withheldCitationLabels: [], ledgerSequence: 4, previousExportSha256: hex('prev'), revokedSigner: null, parseError: null }),
  reconstruct_document_context: ({ documentId, portalKey }) => ({ analysisVersion: 'reconstruct/2', documentId, portalKey, sourceSha256: hex(`raw${documentId}`), corpusFingerprint: hex('corpus'), documentsCompared: 1842, passagesCompared: 61_220, findings: [], limitations: ['Only text-layer documents were compared.'], searchTerms: ['February 12', 'Imperial'] }),
  infer_redaction: () => ({ candidates: [], corroboratingDocs: [], confidenceCeiling: { kind: 'UNASSESSED' }, semanticRankingUsed: false, contextSnippet: '' }),
  // Writes the views fire on their own once data is present; answered so they settle.
  start_data_source: ({ url }) => ({ source: portal('new-source', url, url, 'WEB_CRAWL'), syncRunId: 'run-new', detectedKind: 'WEB_CRAWL', usedSavedGuidance: false }),
  record_service_leads: () => ({ findingsRecorded: 0, staleCleared: 0 }),
  fetch_city_dataset: ({ dataset }) => ({ snapshotId: `snap-${dataset.toLowerCase()}`, datasetKind: 'REGIONAL_DATA', displayName: dataset, tableName: `analysis_${dataset.toLowerCase()}`, importedRows: 0, rejectedRows: 0, malformedRows: 0, duplicateKeyRows: 0, unconvertedCells: 0, schemaWarnings: [], sourceSha256: hex(dataset) }),
};

/** @param {string} findingId @param {import('../src/types').ArtifactKind} kind @returns {import('../src/types').DraftedArtifact} */
function drafted(findingId, kind) {
  const f = FINDINGS.find((x) => x.id === findingId) ?? FINDINGS[0];
  return {
    id: `draft-${findingId}-${kind}`, kind, subjectFindingId: f.id, jurisdiction: 'CA_CPRA', templateId: kind === 'APPEAL' ? 'cpra-appeal-v2' : 'cpra-followup-v3',
    generatedText: `To the Records Coordinator, ${request(f.subject.requestId ?? 26_0412).agencyName}:\n\nRegarding request ${f.subject.requestId}: ${f.headline}. Under Government Code section 7922.535, please provide a written determination and the records described below within 10 days.\n\n[Your name]`,
    disclosure: 'The author is affiliated with Voice of the Bay Newsroom.', citedAuthorities: [{ statute: 'Gov. Code § 7922.535', quote: 'within 10 days from receipt of the request' }],
    evidenceBasis: 'COUNTED_STATUTORY_DEADLINE', confidence: f.confidence, citations: f.citations, unknownFacts: ['Date the agency received the request'], generatedAt: iso(0.1),
  };
}

/** Collections empty, singletons in their honest "nothing yet" form: the first-launch workspace. */
/** @type {Handlers} */
const empty = {
  list_saved_portals: () => [],
  list_portal_runtimes: () => [],
  get_agency_performance_profiles: () => [],
  get_cross_agency_correlations: () => [],
  get_department_dispositions: () => [],
  refresh_findings: () => ({ detectorsRun: 28, findingsRecorded: 0, staleCleared: 0 }),
  list_findings: () => [],
  count_findings: () => 0,
  list_evidence_gaps: () => [],
  list_story_leads: () => [],
  query_audit_records_page: () => ({ records: [], nextCursor: null, total: 0 }),
  count_audit_records: () => ({ total: 0 }),
  list_request_documents: () => [],
  query_corpus_patterns: () => [],
  get_corpus_index_status: () => ({ indexedDocumentCount: 0, indexedSegmentCount: 0, indexedTermCount: 0, indexedRelationshipCount: 0, availableWriteCapacity: 1_000_000, algorithmVersion: 'corpus-patterns/4' }),
  list_top_corpus_entities: () => [],
  list_entity_brokers: () => [],
  get_embedding_index_status: () => ({ indexedDocumentCount: 0, indexedChunkCount: 0, pendingDocumentCount: 0, failedDocumentCount: 0, running: false, cancelled: false, lastError: null }),
  get_analytical_schema: () => ({ datasets: [], columns: [], templates: SCHEMA.templates }),
  list_analytical_import_candidates: () => [],
  hybrid_search: () => [],
  get_meeting_calendar: () => ({ today: day(0), meetings: [], postingCounts: { agendaFilesWithBothTimes: 0, under72HoursBeforeStart: 0, afterMeetingStart: 0 }, limitations: [] }),
  list_watches: () => [],
  list_watch_changes: () => [],
  list_standing_queries: () => [],
  list_series_number_gaps: () => ({ gaps: [], seriesCount: 0, truncated: false }),
  get_casework_state: () => ({ availableEvidence: [], cases: [], events: [], leads: [] }),
  get_research_status: () => ({ policy: { enabled: false, autoCollect: false, intervalHours: 24, provider: 'ollama', model: '', dailyCallLimit: 40, researchPasses: 2, discoverSources: false }, credentialAvailable: false, phase: 'IDLE', lastError: null, lastFinishedAt: null, callsToday: 0, reports: [], sources: [], actionStates: {} }),
  list_claimed_requests: () => [],
  list_legal_authorities: () => [],
  list_topic_packs: () => ({ topics: [], documentsScanned: 0, documentsPrefixOnly: 0, documentsFileNameOnly: 0, documentLimitReached: false, requestsScanned: 0, requestLimitReached: false, textPrefixCharacters: 20_000 }),
  list_registry_snapshots: () => [],
  list_organisation_checks: () => [],
  list_organisation_payees: () => [],
  list_trusted_contributors: () => [],
  get_profile: () => ({ status: 'UNSET', profile: null }),
  get_affiliation_disclosure: () => ({ status: 'UNSET', sentence: null, affiliations: [], ownPortalKeys: [], ownAgencies: [], ownEntityIds: [] }),
  list_resolution_candidates: () => ({ candidates: [], nextCursor: null, remaining: 0, scanTruncated: false, guards: { skippedNameGroups: 0, skippedIdentifiers: 0 }, ledgerHead: hex('ledger') }),
  get_investigation_evaluations: () => [],
  get_financial_workspace: () => ({ datasets: [], mappings: [], defaultScope: { fiscalYear: null, fund: null, department: null }, roles: ['BUDGET', 'ACTUALS', 'CONTRACTS', 'PAYMENTS'], fields: [], requirements: {}, warnings: [], fiscalYears: [], funds: [], departments: [] }),
};

/** A collection plan with unfinished work, which opens the plan dialog at launch. */
/** @type {Handlers} */
const collectionPlan = {
  get_collection_plan: () => ({ sources: PORTALS.slice(0, 3).map((p, index) => ({ portalKey: p.portalKey, displayName: p.displayName, canonicalUrl: p.canonicalUrl, sourceKind: p.sourceKind, hasUnfinishedWork: index < 2, subject: index === 0 ? 'encampment abatement' : null })) }),
};

// ---- Real data (`--data=real`) --------------------------------------------------------------
// The packs hold City tables, not records requests, documents or casework, so those stay
// invented; what follows replaces every surface the packs can answer with the generated
// scripts/ui-review-fixtures-real.mjs, and gives the invented requests text of realistic length.

/** Request descriptions as requesters write them: uneven length, lists, dates, statute citations. */
/** @type {Record<number, string>} */
const REQUEST_TEXT = {
  [26_0412]: 'Pursuant to the California Public Records Act (Gov. Code § 7920.000 et seq.), I request copies of every 72-hour notice of abatement posted along Imperial Avenue between 16th Street and 32nd Street from January 1, 2026 through March 31, 2026, together with:\n1. photographs of each posted notice, with their metadata;\n2. the crew dispatch log entries for each removal operation at those locations;\n3. any property storage receipts issued on those dates.\nIf any portion is withheld, please cite the specific exemption for each record and produce the remainder.',
  [26_0418]: 'All contracts, amendments, change orders and invoices for the operator of the Golden Hall bridge shelter for fiscal year 2025, including attachments and exhibits that set compensation or a not-to-exceed amount.',
  [26_0533]: 'Records sufficient to show the number of citations and custodial arrests made under SDMC § 63.0403 (Unsafe Camping Ordinance), by police beat and by month, from July 1, 2025 to the date of your search. A database export is acceptable; I am not requesting names or other personal identifiers. Please also produce the field codes or data dictionary needed to read the export.',
  [26_0540]: 'Nightly capacity and occupancy reports for the Safe Sleeping Program sites at 20th & B and Rose Canyon.',
  [26_0561]: 'Cleaning, service and closure logs for the public restrooms in East Village (including the comfort station at 14th and Imperial) and Balboa Park, January through June 2026. Where a restroom was closed, please include the stated reason and the dates it reopened. I would prefer spreadsheets if the logs are kept electronically.',
  [26_0577]: 'Budget-to-actual reports for homelessness programs by fund and account for FY2024.',
  [26_0602]: 'The retention schedule that applies to body-worn camera footage recorded during encampment removal operations, and the audit trail showing any footage deleted or reclassified between July 2025 and February 2026. This request does not seek the footage itself.',
  [26_0615]: 'Emails and text messages between the City Attorney\'s Office or the Homelessness Strategies and Solutions Department and any contracted outreach provider that discuss property storage, the 72-hour notice, or "bag and tag" procedures, from September 1, 2025 to January 31, 2026. Suggested search terms: "storage", "72 hour", "bag and tag", "property", "notice". Please produce on a rolling basis rather than waiting until the search is complete; if the request is overly broad, contact me to narrow it before denying it.',
  [26_0688]: 'Closure codes and crew dispatch notes for Get It Done encampment reports closed in 2025.',
  [26_0701]: 'Referral outcome summaries for coordinated outreach in the Downtown Clean & Safe area, quarter two of 2026, including counts of referrals made, accepted, and resulting in a shelter bed. Aggregate counts are sufficient.',
  [7_311]: 'Behavioral health bed availability reports the County provided to the City of San Diego during 2025, and any cover emails transmitting them.',
  [7_355]: 'Records showing the number of people released from San Diego Central Jail between 10 p.m. and 6 a.m. in 2025 without transportation arranged, by month. If the Sheriff\'s Department does not track transportation at release, please say so in writing rather than reporting that no records exist.',
};
const LEAD_KINDS = new Set(['SERVICE_REPORT_LEAD', 'POLICE_STOP_LEAD', 'ABATEMENT_LOG_LEAD', 'CHARGE_LOG_LEAD', 'DISCLOSURE_FILING_LEAD', 'CAPACITY_COMPARISON_LEAD', 'SPENDING_RECORD_LEAD', 'POLICE_CALL_LEAD', 'PARKING_CITATION_LEAD', 'HOUSING_APPROVAL_LEAD']);

/**
 * analysis/story_desk.rs `lead_for` for a table lead: no citations or subject, unassessed, so only
 * money named in its text, recurrence among its detector's siblings and freshness contribute.
 * @param {import('../src/types').Finding} finding @param {import('../src/types').Finding[]} live @returns {import('../src/types').StoryLead}
 */
function tableStoryLead(finding, live) {
  /** @type {import('../src/types').InterestFactor[]} */
  const factors = [];
  const amounts = [...`${finding.headline} ${finding.explanation}`.matchAll(/\$([\d,]+(?:\.\d+)?)(\s+(?:million|billion))?/g)]
    .map(([, figure, scale]) => Number(figure.replace(/,/g, '')) * (scale?.includes('billion') ? 1e9 : scale?.includes('million') ? 1e6 : 1));
  const largest = Math.max(0, ...amounts);
  const money = largest >= 1_000_000 ? 2500 : largest >= 100_000 ? 1600 : largest >= 10_000 ? 800 : 0;
  if (money) factors.push({ name: 'money at stake', basisPoints: money, evidence: `largest figure named: ${largest.toLocaleString('en-US', { style: 'currency', currency: 'USD' })}` });
  const siblings = live.filter((other) => other.id !== finding.id && other.sourceDetector === finding.sourceDetector && other.subject.portalKey === finding.subject.portalKey).length;
  if (siblings) factors.push({ name: 'recurrence', basisPoints: Math.min(1000, 200 * siblings), evidence: `${siblings} other undismissed finding${siblings === 1 ? '' : 's'} from ${finding.sourceDetector} for the same source` });
  factors.push({ name: 'evidence strength', basisPoints: 0, evidence: 'confidence unassessed' }, { name: 'freshness', basisPoints: 400, evidence: 'seen within the last 7 days' });
  return {
    findingId: finding.id, kind: finding.kind, headline: finding.headline, severity: finding.severity, confidence: finding.confidence, subject: finding.subject, portalKey: finding.subject.portalKey,
    publicInterest: { totalBasisPoints: factors.reduce((sum, factor) => sum + factor.basisPoints, 0), factors }, relatedFindingIds: [], firstSeenAt: finding.discoveredAt, lastSeenAt: finding.lastSeenAt,
  };
}

/**
 * @typedef {{ SCHEMA: import('../src/types').AnalyticalSchema, SQL: Record<string, { columns: string[], rows: unknown[][], elapsedMs: number, provenance: string[] } | { error: string }>,
 *   LEAD_FINDINGS: import('../src/types').Finding[], PACKS: import('../src/types').DataPackSummary[], FINANCE: { datasets: import('../src/types').FinancialWorkspace['datasets'], fiscalYears: number[], funds: string[], departments: string[] },
 *   PAYEES: import('../src/types').OrganisationPayee[], BLOCKS: { address: string, reports: number }[] }} RealData
 */
/** @param {RealData} real @returns {Handlers} */
function realHandlers(real) {
  const findings = [...FINDINGS.filter((f) => !LEAD_KINDS.has(f.kind)), ...real.LEAD_FINDINGS];
  const live = findings.filter((f) => !f.dismissedAt);
  const storyLeads = [...STORY_LEADS.filter((lead) => !LEAD_KINDS.has(lead.kind)), ...real.LEAD_FINDINGS.map((f) => tableStoryLead(f, live))]
    .sort((a, b) => b.publicInterest.totalBasisPoints - a.publicInterest.totalBasisPoints || a.findingId.localeCompare(b.findingId));
  const requests = REQUESTS.map((r) => ({ ...r, descriptionPreview: REQUEST_TEXT[r.requestId] ?? r.descriptionPreview }));
  const matches = (/** @type {string} */ term) => (/** @type {import('../src/types').RequestSearchResult} */ r) => !term || r.title.toLowerCase().includes(term) || r.descriptionPreview.toLowerCase().includes(term);
  const latestYear = real.FINANCE.fiscalYears.length ? real.FINANCE.fiscalYears[real.FINANCE.fiscalYears.length - 1] : null;
  return {
    get_analytical_schema: () => real.SCHEMA,
    run_analytical_sql: ({ sql, limit }) => {
      const entry = real.SQL[sql];
      if (!entry) throw { errorType: 'UI_REVIEW_SQL_MISS', message: `The real-data fixtures hold no result for this SQL; rebuild with --harvest. ${sql.slice(0, 200)}` };
      if ('error' in entry) throw { errorType: 'DATA_PARSING', message: entry.error };
      const cap = limit ?? 500;
      return { columns: entry.columns, rows: entry.rows.slice(0, cap), rowCount: Math.min(entry.rows.length, cap), truncated: entry.rows.length > cap, elapsedMs: entry.elapsedMs, provenance: entry.provenance };
    },
    refresh_findings: () => ({ detectorsRun: 28, findingsRecorded: findings.length, staleCleared: 0 }),
    list_findings: ({ filter, limit }) => filterFindings(filter, findings).slice(0, limit ?? 500),
    count_findings: ({ filter }) => filterFindings(filter, findings).length,
    list_story_leads: ({ limit }) => storyLeads.slice(0, limit ?? 50),
    build_lead_dossier: (args) => {
      const dossier = /** @type {NonNullable<Handlers['build_lead_dossier']>} */ (full.build_lead_dossier)(args);
      const lead = real.LEAD_FINDINGS.find((f) => f.id === args.findingId);
      return lead ? { ...dossier, finding: lead, evidence: [], related: [], gaps: [] } : dossier;
    },
    query_audit_records_page: ({ searchTerm, portalKey, agencyId, pageSize }) => {
      const records = requests.filter((r) => matches(searchTerm.trim().toLowerCase())(r) && (!portalKey || r.portalKey === portalKey) && (agencyId == null || r.agencyId === agencyId));
      return { records: records.slice(0, pageSize), nextCursor: null, total: records.length };
    },
    count_audit_records: ({ searchTerm }) => ({ total: requests.filter(matches((searchTerm ?? '').trim().toLowerCase())).length }),
    get_cross_agency_correlations: () => CORRELATIONS.map((c, index) => ({ ...c, neighborhoodBlock: real.BLOCKS[index]?.address ?? c.neighborhoodBlock })),
    list_published_packs: () => real.PACKS.map((p) => ({ pack: p.pack, repositoryUrl: `https://huggingface.co/datasets/Writerslogic/onrecord-${p.pack}` })),
    fetch_published_pack_manifest: ({ pack }) => real.PACKS.find((p) => p.pack === pack) ?? real.PACKS[0],
    get_financial_workspace: () => ({ datasets: real.FINANCE.datasets, mappings: [], defaultScope: { fiscalYear: latestYear, fund: null, department: null }, roles: ['BUDGET', 'ACTUALS', 'CONTRACTS', 'PAYMENTS'], fields: ['amount', 'fund', 'department'], requirements: { BUDGET: ['amount', 'fund'] }, warnings: [], fiscalYears: real.FINANCE.fiscalYears, funds: real.FINANCE.funds, departments: real.FINANCE.departments }),
    list_organisation_payees: () => real.PAYEES,
  };
}

// ---- Populated figure fixtures (scenario.figures) -------------------------------------------

const FIN_CITATION = { id: 'fin-1', role: 'budget', snapshotId: 'snap-budget-fy25', sourceUrl: 'https://seshat.datasd.org/operating_budget/budget_operating_datasd.csv', sourceSha256: hex('snap-budget-fy25'), queryId: 'q-fin', columns: ['amount'], row: ['3600000.00'] };
const reconcileLine = (account, accountLabel, department, adopted, revised, actual, encumbered, currency = 'USD') => ({
  agency: 'City of San Diego', fundingDuration: 'ongoing', key: `fin-${account}`, fiscalYear: '2025', fund: '100000', account, accountLabel, department, center: '1914', program: accountLabel, currency,
  adopted, revised, actual, encumbered,
  actualMinusAdopted: adopted !== null && actual !== null ? (Number(actual) - Number(adopted)).toFixed(2) : null,
  uncommittedRevised: revised !== null && actual !== null && encumbered !== null ? (Number(revised) - Number(actual) - Number(encumbered)).toFixed(2) : null,
  citations: ['fin-1'], limitations: [],
});
const chain = (contractId, program, currency, base, amendments, payments, stages, payMapped) => {
  const total = base === null ? null : (Number(base) + Number(amendments ?? 0)).toFixed(2);
  const paid = payMapped ? payments : null;
  return {
    agency: 'City of San Diego', contractId, program, vendor: 'V-2201', currency, baseAuthority: base, amendments, totalAuthority: total, payments: paid,
    remainingAuthority: total !== null && paid !== null ? (Number(total) - Number(paid)).toFixed(2) : null,
    stages: payMapped ? stages : Object.fromEntries(Object.entries(stages).filter(([role]) => role !== 'payment')), missingLinks: [], limitations: [],
  };
};
const financialReport = (payMapped) => ({
  id: 'fin-report', createdAt: iso(1), scope: { fiscalYear: 2025, fund: '100000', department: null },
  reconciliation: [
    reconcileLine('512010', 'Shelter operations', 'Homelessness Strategies and Solutions', '3200000.00', '3600000.00', '3910000.00', '120000.00'),
    reconcileLine('512020', 'Property storage services', 'Homelessness Strategies and Solutions', '1000000.00', '1250000.00', '1486200.10', '0.00'),
    reconcileLine('512030', 'Outreach', 'Homelessness Strategies and Solutions', '900000.00', '850000.00', '640000.00', '95000.00'),
    reconcileLine('512040', 'Safe parking', 'Homelessness Strategies and Solutions', '450000.00', null, '410000.00', null),
    reconcileLine('512050', 'Hygiene stations', 'Homelessness Strategies and Solutions', '300000.00', '300000.00', null, '40000.00'),
    reconcileLine('511000', 'Overtime, abatement details', 'Police', '2000000.00', '2100000.00', '2350000.00', '60000.00'),
  ],
  chains: [
    chain('H-24-1180', 'Property storage', 'USD', '1000000.00', '250000.00', '1486200.10', { budget: ['fin-1'], contract: ['fin-1'], amendment: ['fin-1'], payment: ['fin-1'], outcome: [] }, payMapped),
    chain('H-23-0412', 'Shelter operations', 'USD', '3000000.00', '600000.00', '2940000.00', { budget: ['fin-1'], contract: ['fin-1'], amendment: ['fin-1'], payment: ['fin-1'], outcome: ['fin-1'] }, payMapped),
    chain('H-25-0077', 'Outreach', 'USD', '850000.00', null, '312500.00', { budget: ['fin-1'], contract: ['fin-1'], payment: ['fin-1'] }, payMapped),
    chain('H-25-0102', 'Safe parking', 'USD', null, null, '205000.00', { budget: ['fin-1'], payment: ['fin-1'] }, payMapped),
    chain('HUD-CoC-17', 'Rapid rehousing grant', '', '480000.00', '0.00', '120000.00', { contract: ['fin-1'], payment: ['fin-1'], outcome: [] }, payMapped),
  ],
  findings: [], outcomes: [], gaps: [], evidence: [FIN_CITATION], queries: { 'q-fin': 'SELECT amount FROM analysis_fin_budget WHERE fund_number = \'100000\'' }, records: [], incompleteRoles: [], limitations: [],
});
const financialWorkspace = (payMapped) => ({
  datasets: [{ snapshotId: 'snap-budget-fy25', name: 'Operating budget FY2025', tableName: 'analysis_fin_budget', sourceUrl: FIN_CITATION.sourceUrl, sourceSha256: FIN_CITATION.sourceSha256, columns: ['fund_number', 'dept_name', 'account', 'amount'], partial: false }],
  mappings: ['budget', 'actual', 'encumbrance', 'contract', 'amendment', ...(payMapped ? ['payment'] : [])].map((role) => ({ role, snapshotId: 'snap-budget-fy25', columns: {}, currency: 'USD', agency: 'City of San Diego' })),
  defaultScope: { fiscalYear: 2025, fund: '100000', department: null },
  roles: ['budget', 'actual', 'encumbrance', 'contract', 'amendment', 'payment', 'outcome'], fields: ['amount', 'fund', 'department'], requirements: {},
  warnings: payMapped ? [] : ['No payments dataset mapped.'], fiscalYears: [2024, 2025], funds: ['100000'], departments: ['Homelessness Strategies and Solutions', 'Police'],
});
/** The budget-comparison query's columns, largest |difference| first as the builder orders them. */
const comparisonResult = (truncated) => {
  const columns = ['report_fy', 'fund_number', 'funds_center_number', 'account_number', 'budget_departments', 'actuals_departments', 'budget_amount', 'actuals_amount', 'budget_rows', 'actuals_rows', 'budget_missing_amounts', 'actuals_missing_amounts', 'comparison_status', 'actual_minus_budget', 'variance_percent'];
  const matched = Array.from({ length: 26 }, (_, i) => {
    const budget = 400_000 + ((i * 7919) % 23) * 85_000;
    const difference = Math.round((((i * 4111) % 41) - 18) * 9_250 * (1 + (i % 3)));
    return { budget, actual: budget + difference, difference };
  }).sort((a, b) => Math.abs(b.difference) - Math.abs(a.difference));
  const rows = [
    ...matched.map((line, i) => ['25', '100000', `19${String(10 + i).padStart(2, '0')}`, `5120${String(i).padStart(2, '0')}`, 'Homelessness Strategies and Solutions', 'Homelessness Strategies and Solutions', line.budget.toFixed(2), line.actual.toFixed(2), 3, 12, 0, 0, 'Matched', line.difference.toFixed(2), ((100 * line.difference) / line.budget).toFixed(2)]),
    ['25', '100000', '1990', '512900', 'Homelessness Strategies and Solutions', null, '220000.00', null, 2, null, 0, null, 'Missing actuals', null, null],
    ['25', '100000', '1991', '512901', 'Homelessness Strategies and Solutions', null, '75000.00', null, 1, null, 0, null, 'Missing actuals', null, null],
    ['25', '100000', '1992', '512902', null, 'Homelessness Strategies and Solutions', null, '18250.00', null, 4, null, 0, 'Missing budget baseline', null, null],
    ['25', '100000', '1993', '512903', 'Homelessness Strategies and Solutions', 'Homelessness Strategies and Solutions', '91000.00', null, 2, 3, 0, 1, 'Incomplete amounts', null, null],
  ];
  const kept = truncated ? rows.slice(0, 20) : rows;
  return { columns, rows: kept, rowCount: kept.length, truncated, elapsedMs: 52, provenance: ['snap-budget-fy25', 'snap-actuals-fy25'] };
};
const clock = (minutes) => `${String(Math.floor(minutes / 60) % 24).padStart(2, '0')}:${String(minutes % 60).padStart(2, '0')}`;
/** Night-window activities: spread over 9 p.m.–5:30 a.m., a pile at 10 p.m., and the two endpoints. */
const listedActivities = (section, activityType, count, prefix) => Array.from({ length: count }, (_, i) => {
  const pile = i % 4 === 0;
  const minutes = pile ? 22 * 60 : (21 * 60 + ((i * 47) % 510)) % 1440;
  const boundary = i === 1 || i === 2;
  return { activityNumber: `${prefix}-${String(1000 + i)}`, activityType, section, recordedAt: `2025-${String(1 + (i % 12)).padStart(2, '0')}-${String(1 + (i % 27)).padStart(2, '0')} ${boundary ? (i === 1 ? '21:00' : '05:30') : clock(minutes)}`, atWindowBoundary: boundary, location: `${1400 + i * 100} block Imperial Ave` };
});
const incident = (i, feet) => ({ caseNumber: `25-${String(40_100 + i)}`, occurredOn: i % 9 === 0 ? null : `2025-${String(1 + (i % 12)).padStart(2, '0')}-${String(1 + (i % 27)).padStart(2, '0')}`, section: '63.0404(c)', blockAddress: `${1200 + i * 50} block Market St`, latitude: 32.71, longitude: -117.15, nearestSchool: feet === null ? null : { name: 'Washington Elementary', distanceFeet: feet }, nearestStation: feet === null ? null : { name: '25th & Commercial', distanceFeet: feet + 600 } });
const INCIDENTS = [...Array.from({ length: 53 }, (_, i) => incident(i, i < 3 ? 5600 + i * 400 : 120 + ((i * 2971) % 4800))), incident(60, null), incident(61, null)];
const measuredFeet = INCIDENTS.flatMap((row) => (row.nearestSchool ? [row.nearestSchool.distanceFeet] : []));
const ENCAMPMENT_FIGURES = {
  nightHours: {
    sources: [{ snapshotId: 'snap-arrests-2025', displayName: 'SDPD arrests and citations 2025', sourceSha256: hex('snap-arrests-2025') }], firstActivity: '2025-01-01', lastActivity: '2025-12-30',
    sections: [
      { section: '63.0404(a)', activityType: 'Citation', activities: 168, insideWindow: 41, atWindowBoundary: 2, outsideWindow: 112, timeNotRecorded: 13 },
      { section: '63.0404(a)', activityType: 'Arrest', activities: 22, insideWindow: 9, atWindowBoundary: 0, outsideWindow: 11, timeNotRecorded: 2 },
      { section: '63.0404(b)', activityType: 'Citation', activities: 60, insideWindow: 12, atWindowBoundary: 0, outsideWindow: 42, timeNotRecorded: 6 },
    ],
    listedActivities: [...listedActivities('63.0404(a)', 'Citation', 43, 'E25'), ...listedActivities('63.0404(a)', 'Arrest', 9, 'A25').map((a) => ({ ...a, atWindowBoundary: false, recordedAt: a.recordedAt.replace(/ (21:00|05:30)$/, ' 23:40') })), ...listedActivities('63.0404(b)', 'Citation', 12, 'B25').map((a) => ({ ...a, atWindowBoundary: false, recordedAt: a.recordedAt.replace(/ (21:00|05:30)$/, ' 01:15') }))],
  },
  locations: {
    incidentSources: [{ snapshotId: 'snap-nibrs-2025', displayName: 'SDPD NIBRS offenses 2025', sourceSha256: hex('snap-nibrs-2025') }], schoolSource: { snapshotId: 'snap-schools', displayName: 'SANDAG schools', sourceSha256: hex('snap-schools') }, stationSource: { snapshotId: 'snap-gtfs', displayName: 'MTS GTFS stops', sourceSha256: hex('snap-gtfs') },
    sections: [{ section: '63.0404(c)', offenses: INCIDENTS.length + 6, distinctCases: INCIDENTS.length + 6, notGeocoded: 6, withinNearBand: measuredFeet.filter((f) => f <= 1320).length, withinFarBand: measuredFeet.filter((f) => f > 1320 && f <= 2640).length, beyondFarBand: measuredFeet.filter((f) => f > 2640).length }],
    incidents: INCIDENTS,
  },
  notMeasured: ['Shelter availability on the night of each citation is not in these datasets.'],
};

/** @param {{ payments?: boolean, truncated?: boolean }} figures */
function figureHandlers({ payments = true, truncated = false }) {
  return {
    get_financial_workspace: () => financialWorkspace(payments),
    run_financial_investigation: () => financialReport(payments),
    run_analytical_sql: ({ sql }) => (sql.toLowerCase().includes('actual_minus_budget') ? comparisonResult(truncated) : sqlResult(sql)),
    get_encampment_ordinance_report: () => ENCAMPMENT_FIGURES,
  };
}

/**
 * @param {{ mode?: 'full' | 'empty', license?: keyof typeof LICENSE_VARIANTS | null, collectionPlan?: boolean, figures?: { payments?: boolean, truncated?: boolean } }} scenario
 * @param {RealData | null} [real] the generated real-data module, when the scenario asks for it
 * @returns {Record<string, (args: any) => unknown>}
 */
export function createHandlers(scenario, real = null) {
  /** @type {Handlers} */
  const merged = { ...full, ...(real ? realHandlers(real) : {}), ...(scenario.mode === 'empty' ? empty : {}), ...(scenario.collectionPlan ? collectionPlan : {}), ...(scenario.figures ? figureHandlers(scenario.figures) : {}) };
  if (scenario.license) {
    const status = { ...LICENSE, ...LICENSE_VARIANTS[scenario.license] };
    merged.get_license_status = () => status;
    merged.refresh_license = () => status;
  }
  return /** @type {Record<string, (args: any) => unknown>} */ (merged);
}

/** The failure every injected error state reports, shaped like the native `{ errorType, message }`. */
export function injectedError(/** @type {string} */ command) {
  return { errorType: 'DATABASE_UNAVAILABLE', message: `The local evidence store could not answer ${command}: database is locked by another process (retry in a moment).` };
}
