import { useCallback, useEffect, useState, type ReactNode } from 'react';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import type {
  AnalyticalSchema, HybridSearchResult, RecordGapRequestStatus, RequestSearchResult, FilingExplanation, Form990Statement, OrganisationCheckSummary, OrganisationFilings, OrganisationMatch,
  OrganisationPaymentMapping, OrganisationReturn, ScheduleLPart, SingleAuditHistory,
} from '../../types';
import { save } from '@tauri-apps/plugin-dialog';
import { portfolioCsv, portfolioRow, type PortfolioRow } from '../../utils/organisationPortfolio';
import { isEin, normalisedName, parseOrganisationList } from '../../utils/organisationNames';
import { contractFileRequest } from '../../utils/organisationRequests';
import { peopleAcrossReturns, yearChange } from '../../utils/organisationPeople';
import { guidanceForSubject } from '../../utils/sourceGuidance';
import { useAppStore } from '../../store/useAppStore';

const dollars = new Intl.NumberFormat('en-US', { style: 'currency', currency: 'USD', maximumFractionDigits: 0 });
const NOT_STATED = 'not stated';
const answer = (value: boolean | null) => value === null ? 'The return carries no answer.' : `The return answers "${value ? 'Yes' : 'No'}".`;
const period = (value: string) => value.length === 6 ? `${value.slice(0, 4)}-${value.slice(4)}` : value;

const SCHEDULE_L_PARTS: Record<ScheduleLPart, string> = {
  EXCESS_BENEFIT_TRANSACTION: 'Part I, excess benefit transactions',
  LOAN: 'Part II, loans to or from interested persons',
  GRANT_OR_ASSISTANCE: 'Part III, grants or assistance benefiting interested persons',
  BUSINESS_TRANSACTION: 'Part IV, business transactions involving interested persons',
};

const money = (value: number | null) => value === null ? NOT_STATED : dollars.format(value);
/** A stored decimal string, shown with cents only when it has them. */
const decimalMoney = (value: string) => {
  const amount = Number(value);
  return Number.isFinite(amount) ? new Intl.NumberFormat('en-US', { style: 'currency', currency: 'USD', maximumFractionDigits: Number.isInteger(amount) ? 0 : 2 }).format(amount) : value;
};
let tableSequence = 0;
// The caption sits outside the scrolling container so a wide table does not clip it on a phone.
function Table({ caption, head, rows }: { caption: string; head: string[]; rows: ReactNode[][] }) {
  const [id] = useState(() => `organisation-table-${++tableSequence}`);
  return <div><p id={id} className="pb-1 text-label leading-4 text-muted">{caption}</p><div className="overflow-x-auto"><table aria-labelledby={id} className="w-full text-left text-label leading-4">
    <thead><tr>{head.map((label) => <th key={label} scope="col" className="pr-4 pb-1 font-normal text-muted">{label}</th>)}</tr></thead>
    <tbody>{rows.map((row, index) => <tr key={index} className="border-t border-line-strong align-top">{row.map((cell, at) => <td key={at} className="pr-4 py-1 text-fg-secondary">{cell}</td>)}</tr>)}</tbody>
  </table></div></div>;
}

function Explanations({ items }: { items: FilingExplanation[] }) {
  if (!items.length) return null;
  return <div className="space-y-2">{items.map((item, index) => <blockquote key={index} className="border-l-2 border-line-strong pl-3 text-label leading-4 text-fg-secondary">
    <p className="whitespace-pre-wrap break-words">{item.text}</p>
    <footer className="mt-1 text-muted">The filer's explanation{item.lineReference ? `, ${item.lineReference}` : ''}</footer>
  </blockquote>)}</div>;
}

function Statement({ statement }: { statement: Form990Statement }) {
  const listed = statement.people.filter((person) => person.former || (person.reportableCompensation ?? 0) > 0 || (person.relatedOrganisationCompensation ?? 0) > 0 || (person.otherCompensation ?? 0) > 0);
  const unpaid = statement.people.length - listed.length;
  return <div className="mt-3 space-y-4">
    <dl className="grid grid-cols-2 gap-x-4 gap-y-1 text-label leading-4 sm:grid-cols-3">
      {([['Total revenue', statement.totalRevenue], ['Total expenses', statement.totalExpenses], ['Total assets, end of year', statement.totalAssetsEndOfYear], ['Total liabilities, end of year', statement.totalLiabilitiesEndOfYear], ['Net assets, end of year', statement.netAssetsEndOfYear], ['Government grants', statement.governmentGrants], ['Reportable compensation, Part VII total', statement.totalReportableCompensation]] as const)
        .map(([label, value]) => <div key={label}><dt className="text-muted">{label}</dt><dd className="text-fg">{money(value)}</dd></div>)}
    </dl>
    <div className="text-label leading-4 text-fg-secondary">
      <p><span className="text-muted">Part VI line 5, whether the organisation became aware of a significant diversion of its assets during the year:</span> {answer(statement.significantDiversionReported)}</p>
      <p><span className="text-muted">Part VII line 3, whether a former officer, director, trustee, key employee or highest compensated employee is listed:</span> {answer(statement.formerListed)}</p>
      {statement.amended === true && <p>The return is marked as amended.</p>}
    </div>
    {listed.length > 0 && <Table caption={`Part VII Section A: people the return lists as former or with compensation.${unpaid > 0 ? ` ${unpaid} more are listed with none.` : ''}`}
      head={['Name', 'Title', 'Position boxes checked', 'From the organisation', 'From related organisations', 'Other compensation']}
      rows={listed.map((person) => [person.name ?? NOT_STATED, person.title ?? NOT_STATED, person.positions.join(', ') || 'none', money(person.reportableCompensation), money(person.relatedOrganisationCompensation), money(person.otherCompensation)])} />}
    {statement.scheduleJAttached ? <div className="space-y-2">
      <p className="text-label leading-4 text-fg-secondary"><span className="text-muted">Schedule J Part I line 4a, whether a listed person received a severance or change-of-control payment:</span> {answer(statement.severancePaymentReported)}</p>
      {statement.scheduleJPeople.length > 0 && <Table caption="Schedule J Part II: compensation from the filing organisation, as the return reports it."
        head={['Name', 'Title', 'Base', 'Bonus and incentive', 'Other reportable', 'Deferred', 'Nontaxable benefits', 'Total', 'Total from related organisations']}
        rows={statement.scheduleJPeople.map((person) => [person.name ?? NOT_STATED, person.title ?? NOT_STATED, money(person.filingOrganisation.base), money(person.filingOrganisation.bonus), money(person.filingOrganisation.other), money(person.filingOrganisation.deferred), money(person.filingOrganisation.nontaxableBenefits), money(person.filingOrganisation.total), money(person.relatedOrganisations.total)])} />}
      <Explanations items={statement.scheduleJExplanations} />
    </div> : <p className="text-label leading-4 text-muted">No Schedule J is attached to this return, so it carries no severance answer.</p>}
    {statement.scheduleLAttached ? <div className="space-y-2">
      {statement.scheduleLTransactions.length > 0 ? <Table caption="Schedule L: transactions with interested persons, as the return reports them."
        head={['Part of the schedule', 'Name', 'Relationship', 'Description', 'Amount', 'Balance due', 'Other columns']}
        rows={statement.scheduleLTransactions.map((row) => [SCHEDULE_L_PARTS[row.part], row.name ?? NOT_STATED, row.relationship ?? NOT_STATED, row.description ?? NOT_STATED, money(row.amount), row.part === 'LOAN' ? money(row.balanceDue) : '', row.lines.map((line) => `${line.label}: ${line.value}`).join('; ')])} />
        : <p className="text-label leading-4 text-muted">A Schedule L is attached and lists no transaction rows.</p>}
      <Explanations items={statement.scheduleLExplanations} />
    </div> : <p className="text-label leading-4 text-muted">No Schedule L is attached to this return.</p>}
    {statement.scheduleOExplanations.length > 0 && <details open={statement.significantDiversionReported === true}>
      <summary className="text-label leading-4 text-accent">Schedule O: the filer's explanations ({statement.scheduleOExplanations.length}){statement.significantDiversionReported === true ? '. The form requires one here for the Yes on Part VI line 5.' : ''}</summary>
      <div className="mt-2"><Explanations items={statement.scheduleOExplanations} /></div>
    </details>}
  </div>;
}

function ReturnCard({ listed }: { listed: OrganisationReturn }) {
  return <article className="rounded-lg border border-line-strong p-3" aria-label={`Form ${listed.returnType}, tax period ${period(listed.taxPeriod)}`}>
    <h4 className="text-sm font-semibold text-fg">Form {listed.returnType}, tax period ending {period(listed.taxPeriod)}</h4>
    <p className="text-label leading-4 text-muted">Filed as {listed.taxpayerName}. IRS e-file index {listed.indexYear}, return {listed.objectId}.</p>
    {listed.statement ? <Statement statement={listed.statement} /> : <p className="mt-2 text-label leading-4 text-warning">Not read: {listed.unreadReason ?? 'no reason recorded.'}</p>}
    {listed.archiveUrl && <details className="mt-3 text-label leading-4 text-muted"><summary>Where this return was read</summary>
      <p className="break-all">Archive: <a className="text-accent underline" href={listed.archiveUrl} target="_blank" rel="noreferrer">{listed.archiveUrl}</a></p>
      <p className="break-all">Member: {listed.memberName}</p>
      {listed.xmlSha256 && <p className="break-all">SHA-256 of the return as read: {listed.xmlSha256}{listed.retrievedAt ? `, retrieved ${listed.retrievedAt.slice(0, 10)}` : ''}</p>}
    </details>}
  </article>;
}

function Audits({ history }: { history: SingleAuditHistory | null }) {
  if (!history) return <p className="text-label leading-4 text-muted">No single-audit lookup has run. Enter a Federal Audit Clearinghouse key above and check again.</p>;
  if (!history.audits.length) return <p className="text-label leading-4 text-muted">The Federal Audit Clearinghouse lists no single audit under this EIN (retrieved {history.retrievedAt.slice(0, 10)}).</p>;
  return <div className="space-y-2">
    {history.audits.map((audit) => <details key={audit.reportId} className="rounded-lg border border-line-strong p-3 text-label leading-4">
      <summary className="text-fg">Audit year {audit.auditYear ?? NOT_STATED}: federal awards expended {money(audit.federalAmountExpended)}; {audit.findings.length} finding {audit.findings.length === 1 ? 'row' : 'rows'} recorded</summary>
      <p className="mt-2 text-muted">Report {audit.reportId}{audit.fiscalYearEnd ? `, fiscal year ending ${audit.fiscalYearEnd}` : ''}{audit.auditor ? `, auditor ${audit.auditor}` : ''}{audit.acceptedDate ? `, accepted ${audit.acceptedDate}` : ''}.</p>
      <ul className="mt-2 list-disc pl-5 text-fg-secondary">{audit.lines.map((line) => <li key={line.label}>{line.label}: {line.value}</li>)}</ul>
      {audit.findings.length > 0 && <Table caption="Findings as the audit submission records them." head={['Reference', 'Compliance requirement', 'Recorded answers']}
        rows={audit.findings.map((finding) => [finding.referenceNumber ?? NOT_STATED, finding.complianceRequirement ?? NOT_STATED, finding.lines.map((line) => `${line.label}: ${line.value}`).join('; ')])} />}
    </details>)}
    <p className="text-label leading-4 text-muted">Source: <a className="text-accent underline" href={history.sourceUrl} target="_blank" rel="noreferrer">Federal Audit Clearinghouse</a>, retrieved {history.retrievedAt.slice(0, 10)}. An audit finding is the auditor's statement about the audited year.</p>
  </div>;
}

/** What the library already holds about the organisation, and the request that would fill what it
 * does not: the same casework path the record gaps use. */
function Library({ filings }: { filings: OrganisationFilings }) {
  const portals = useAppStore((state) => state.portals);
  const [documents, setDocuments] = useState<HybridSearchResult[] | undefined>(undefined);
  const [requests, setRequests] = useState<RequestSearchResult[] | undefined>(undefined);
  const [notes, setNotes] = useState<string[]>([]);
  const [custodianKey, setCustodianKey] = useState('');
  const [tracked, setTracked] = useState<RecordGapRequestStatus | undefined>(undefined);
  const [message, setMessage] = useState('');
  const gate = useLatestRequestGate();
  const name = filings.name ?? '';
  const custodian = portals.find((portal) => portal.registration.portalKey === custodianKey);
  const input = custodian ? contractFileRequest(filings, custodian.registration.displayName, custodian.registration.portalKey) : undefined;
  useEffect(() => {
    if (!name) return;
    const request = gate.begin();
    const found: string[] = [];
    void Promise.all([
      invokeCommand('hybrid_search', { query: name, limit: 10 }).then((hits) => Array.isArray(hits) ? hits : [], (reason) => { found.push(`Document search: ${formatNativeError(reason)}`); return []; }),
      invokeCommand('query_audit_records_page', { searchTerm: name, pageSize: 10 }).then((page) => Array.isArray(page?.records) ? page.records : [], (reason) => { found.push(`Request search: ${formatNativeError(reason)}`); return []; }),
    ]).then(([hits, held]) => { if (!gate.isCurrent(request)) return; setDocuments(hits); setRequests(held); setNotes(found); });
    return gate.invalidate;
  }, [gate, name]);
  const gapKey = input?.gapKey;
  useEffect(() => {
    if (!gapKey) { setTracked(undefined); return; }
    const request = gate.begin();
    void invokeCommand('get_record_gap_requests', { gapKeys: [gapKey] }).then((statuses) => { if (gate.isCurrent(request)) setTracked(Array.isArray(statuses) ? statuses.find((s) => s.gapKey === gapKey) : undefined); }, () => undefined);
  }, [gate, gapKey, message]);
  const track = () => {
    if (!input) return;
    const request = gate.begin();
    invokeCommand('request_record_gap', { input }).then(() => { if (gate.isCurrent(request)) setMessage('Tracked. Record the submission date in casework once you have filed it.'); }, (reason) => { if (gate.isCurrent(request)) setMessage(formatNativeError(reason)); });
  };
  const collect = () => {
    if (!custodian || !name) return;
    const request = gate.begin();
    invokeCommand('start_data_source', { url: custodian.registration.originUrl, guidance: guidanceForSubject(name) })
      .then(() => { if (gate.isCurrent(request)) setMessage(`Collection started from ${custodian.registration.displayName} for "${name}"; the source is listed in the sidebar. The lists above are read again when you reopen this organisation.`); }, (reason) => { if (gate.isCurrent(request)) setMessage(formatNativeError(reason)); });
  };
  const date = (value: string | null) => value ? value.slice(0, 10) : '';
  return <section aria-label="In your library" className="space-y-3">
    <h3 className="text-sm font-semibold text-fg">In your library</h3>
    {notes.map((note) => <p key={note} className="text-label leading-4 text-warning">{note}</p>)}
    <div className="grid gap-4 sm:grid-cols-2">
      <div>
        <h4 className="text-label leading-4 text-muted">Documents that mention the name{documents ? ` (${documents.length}${documents.length === 10 ? ', first 10' : ''})` : ''}</h4>
        {documents?.length === 0 && <p className="text-label leading-4 text-muted">None of the collected documents mentions this name.</p>}
        <ul className="mt-1 space-y-2">{(documents ?? []).map((hit) => <li key={hit.stableId} className="text-label leading-4"><span className="text-fg">{hit.title || hit.fileName}</span><span className="text-muted"> · {hit.agencyName}{hit.publicationDate ? ` · ${date(hit.publicationDate)}` : ''}</span>{hit.excerpt && <blockquote className="mt-1 border-l-2 border-line-strong pl-2 text-fg-secondary">{hit.excerpt}</blockquote>}</li>)}</ul>
      </div>
      <div>
        <h4 className="text-label leading-4 text-muted">Held requests that mention the name{requests ? ` (${requests.length}${requests.length === 10 ? ', first 10' : ''})` : ''}</h4>
        {requests?.length === 0 && <p className="text-label leading-4 text-muted">No held request mentions this name.</p>}
        <ul className="mt-1 space-y-1">{(requests ?? []).map((held) => <li key={`${held.portalKey}:${held.requestId}`} className="text-label leading-4"><span className="text-fg">{held.title}</span><span className="text-muted"> · {held.agencyName} · {held.status} · {date(held.createdAt)}</span></li>)}</ul>
      </div>
    </div>
    <div className="rounded-lg border border-line-strong p-3 space-y-2">
      <h4 className="text-label leading-4 text-fg">The contract file</h4>
      <p className="text-label leading-4 text-muted">Filings and payment rows do not show what an agency contracted for, paid against, or found in monitoring. Choose the agency, then either collect what its request portal already released under this name, or track a drafted request for its contract file; you review and file it yourself.</p>
      <label className="text-label leading-4 text-fg-secondary">Agency<select className="analysis-input ml-2" value={custodianKey} onChange={(event) => { setCustodianKey(event.target.value); setMessage(''); }}><option value="">Choose a saved source</option>{portals.map((portal) => <option key={portal.registration.portalKey} value={portal.registration.portalKey}>{portal.registration.displayName}</option>)}</select></label>
      {portals.length === 0 && <p className="text-label leading-4 text-muted">No source is saved yet; add the agency's request portal in the sidebar first.</p>}
      {custodian && name && <button type="button" className="analysis-button" onClick={collect}>Search {custodian.registration.displayName} for this name</button>}
      {input && <details className="text-label leading-4"><summary className="text-accent">Drafted request</summary><pre className="mt-2 whitespace-pre-wrap text-fg-secondary">{input.draft}</pre></details>}
      {input && (tracked
        ? <p className="text-label leading-4 text-fg-secondary">Tracked as a casework request{tracked.submittedAt ? `, submitted ${date(tracked.submittedAt)}` : '; no submission date is recorded yet'}. <button type="button" className="text-accent underline" onClick={() => useAppStore.getState().openCasework(tracked.reportId)}>Open casework</button></p>
        : <button type="button" className="analysis-button" onClick={track}>Track this request</button>)}
      {message && <p role="status" className="text-label leading-4 text-muted">{message}</p>}
    </div>
  </section>;
}

const signed = (value: number | null) => value === null ? '' : `${value > 0 ? '+' : value < 0 ? '−' : ''}${dollars.format(Math.abs(value))}`;

function People({ filings }: { filings: OrganisationFilings }) {
  const people = peopleAcrossReturns(filings);
  if (people.length === 0) return null;
  const periods = [...new Set(people.flatMap((p) => p.years.map((y) => y.period)))].sort().reverse();
  return <section aria-label="People across returns" className="space-y-2">
    <h3 className="text-sm font-semibold text-fg">People across the read returns</h3>
    <Table caption="Everyone Part VII or Schedule J lists on any read return, with reportable compensation from the organisation per return (Schedule J total in brackets where listed). An empty cell is a return that does not list the person; &quot;former&quot; is the box the return checked."
      head={['Name', 'Title (latest)', ...periods]}
      rows={people.map((p) => [p.name, p.years[0].title ?? NOT_STATED, ...periods.map((period) => {
        const year = p.years.find((y) => y.period === period);
        if (!year) return '';
        const comp = year.reportable === null ? (year.scheduleJTotal === null ? NOT_STATED : `[${dollars.format(year.scheduleJTotal)}]`) : `${dollars.format(year.reportable)}${year.scheduleJTotal !== null ? ` [${dollars.format(year.scheduleJTotal)}]` : ''}`;
        return `${comp}${year.former ? ', former' : ''}`;
      })])} />
  </section>;
}

function Record({ filings }: { filings: OrganisationFilings }) {
  return <div className="space-y-5">
    {filings.warnings.length > 0 && <ul role="alert" className="list-disc pl-5 text-label leading-4 text-warning">{filings.warnings.map((warning) => <li key={warning}>{warning}</li>)}</ul>}
    <section aria-label="Returns as filed" className="space-y-3">
      <h3 className="text-sm font-semibold text-fg">Returns as filed</h3>
      <p className="text-label leading-4 text-muted">Read from the IRS e-file archives for index years {filings.indexYears.map((check) => check.year).join(', ') || 'none'}. Each figure and answer is the organisation's own statement on its return. Earlier years' returns are not read; the summary below covers them.</p>
      {filings.returns.length ? filings.returns.map((listed) => <ReturnCard key={listed.objectId} listed={listed} />) : <p className="text-label leading-4 text-muted">The IRS indexes that were read list no e-filed return under this EIN.</p>}
    </section>
    <People filings={filings} />
    <section aria-label="Summary by year" className="space-y-2">
      <h3 className="text-sm font-semibold text-fg">Summary by year</h3>
      {filings.summary ? <>
        {filings.summary.years.length > 0 ? <Table caption="Figures extracted from each year's return. The change column is the difference from the year before, shown only when both years state the figure." head={['Tax year', 'Form', 'Total revenue', 'Change', 'Total expenses', 'Change', 'Total assets', 'Total liabilities', 'Compensation of current officers']}
          rows={filings.summary.years.map((year, index) => [String(year.taxYear), year.formType ?? NOT_STATED, money(year.totalRevenue), signed(yearChange(filings.summary!.years, index, (y) => y.totalRevenue)), money(year.totalExpenses), signed(yearChange(filings.summary!.years, index, (y) => y.totalExpenses)), money(year.totalAssetsEndOfYear), money(year.totalLiabilitiesEndOfYear), money(year.officerCompensation)])} />
          : <p className="text-label leading-4 text-muted">The source holds no extracted year for this organisation.</p>}
        <p className="text-label leading-4 text-muted">Source: <a className="text-accent underline" href={filings.summary.sourceUrl} target="_blank" rel="noreferrer">ProPublica Nonprofit Explorer</a>, from IRS extracts, retrieved {filings.summary.retrievedAt.slice(0, 10)}.</p>
      </> : <p className="text-label leading-4 text-muted">No summary was read.</p>}
    </section>
    <section aria-label="Single audits" className="space-y-2">
      <h3 className="text-sm font-semibold text-fg">Single audits of federal money</h3>
      <Audits history={filings.singleAudits} />
    </section>
    <Library filings={filings} />
    <section aria-label="Public money in your library" className="space-y-2">
      <h3 className="text-sm font-semibold text-fg">Public money in your library</h3>
      {filings.publicMoney.length > 0 && <Table caption="Rows of your mapped contract, amendment and payment tables whose vendor value contains this organisation's name. The tables carry no EIN, so confirm each vendor value is this organisation."
        head={['Record', 'Table', 'Vendor as written', 'Fiscal year', 'Rows', 'Amount']}
        rows={filings.publicMoney.map((line) => [line.role, line.sourceUrl ? <a className="text-accent underline" href={line.sourceUrl} target="_blank" rel="noreferrer">{line.datasetName}</a> : line.datasetName, line.vendorValue, line.fiscalYear ?? NOT_STATED, String(line.rows), line.amount === null ? 'not summed: an amount could not be read' : decimalMoney(line.amount)])} />}
      {filings.publicMoneyNotes.map((note) => <p key={note} className="text-label leading-4 text-muted">{note}</p>)}
    </section>
  </div>;
}

type Resolution = { entry: string; candidates: OrganisationMatch[]; chosen: OrganisationMatch | null };

/** Checks several organisations one after another. Names are looked up first; a name is taken as
 * resolved only when a match's name equals it letter for letter, otherwise the reader chooses. Each
 * result is stored by the check itself, so the portfolio is reloaded once at the end. */
function BatchCheck({ auditKey, onDone }: { auditKey: string; onDone: () => void }) {
  const [text, setText] = useState('');
  const [resolutions, setResolutions] = useState<Resolution[] | undefined>(undefined);
  const [progress, setProgress] = useState<{ done: number; total: number; failed: string[] } | undefined>(undefined);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState<'' | 'payees' | 'lookup' | 'check'>('');
  const gate = useLatestRequestGate();
  const { eins, names } = parseOrganisationList(text);
  const stale = resolutions !== undefined && resolutions.map((r) => r.entry).join('\n') !== names.join('\n');
  const resolved = stale || !resolutions ? [] : resolutions.flatMap((r) => r.chosen ? [r.chosen.ein] : []);
  const targets = [...new Set([...eins, ...resolved])];
  const unresolved = stale ? names.length : (resolutions ?? []).filter((r) => !r.chosen).length;

  const addPayees = () => {
    const request = gate.begin();
    setBusy('payees'); setError('');
    invokeCommand('list_organisation_payees', {}).then((payees) => {
      if (!gate.isCurrent(request)) return;
      const held = new Set([...eins, ...names.map(normalisedName)]);
      const added = payees.map((p) => p.vendorValue).filter((value) => !held.has(normalisedName(value)));
      setText((current) => [current.trimEnd(), ...added].filter(Boolean).join('\n'));
      if (!payees.length) setError('No payee is listed: map a payment table under "Tables searched for payments" first.');
    }, (reason) => { if (gate.isCurrent(request)) setError(formatNativeError(reason)); })
      .finally(() => { if (gate.isCurrent(request)) setBusy(''); });
  };
  const lookUp = async () => {
    const request = gate.begin();
    setBusy('lookup'); setError('');
    const found: Resolution[] = [];
    for (const entry of names) {
      try {
        const candidates = (await invokeCommand('search_organisations', { query: entry })).slice(0, 5);
        const exact = candidates.filter((c) => normalisedName(c.name) === normalisedName(entry));
        found.push({ entry, candidates, chosen: exact.length === 1 ? exact[0] : null });
      } catch (reason) { found.push({ entry, candidates: [], chosen: null }); setError(`${entry}: ${formatNativeError(reason)}`); }
      if (!gate.isCurrent(request)) return;
      setResolutions([...found]);
    }
    setBusy('');
  };
  const start = async () => {
    const request = gate.begin();
    const failed: string[] = [];
    setBusy('check'); setProgress({ done: 0, total: targets.length, failed });
    for (const [index, ein] of targets.entries()) {
      try { await invokeCommand('check_organisation_filings', { ein, auditKey: auditKey.trim() || null }); }
      catch (reason) { failed.push(`${ein}: ${formatNativeError(reason)}`); }
      if (!gate.isCurrent(request)) return;
      setProgress({ done: index + 1, total: targets.length, failed: [...failed] });
    }
    setBusy('');
    onDone();
  };
  const choose = (entry: string, ein: string) => setResolutions((current) => current?.map((r) => r.entry === entry ? { ...r, chosen: r.candidates.find((c) => c.ein === ein) ?? null } : r));
  return <details className="rounded-lg border border-line-strong p-3">
    <summary className="text-label leading-4 text-accent">Check a list of organisations</summary>
    <form className="mt-2 space-y-2" onSubmit={(event) => { event.preventDefault(); void start(); }}>
      <label className="block text-label leading-4 text-fg-secondary">Organisation names or EINs, one per line
        <textarea className="analysis-input mt-1 block w-full" rows={4} value={text} onChange={(event) => setText(event.target.value)} placeholder={'Alpha Project for the Homeless\n33-0492302'} /></label>
      <div className="flex flex-wrap items-center gap-3">
        <button type="button" className="analysis-button" disabled={busy !== ''} onClick={addPayees}>{busy === 'payees' ? 'Reading payees' : 'Add payees from your payment tables'}</button>
        <button type="button" className="analysis-button" disabled={busy !== '' || names.length === 0} onClick={() => { void lookUp(); }}>{busy === 'lookup' ? 'Looking up' : `Look up ${names.length} ${names.length === 1 ? 'name' : 'names'}`}</button>
        <button type="submit" className="analysis-button" disabled={busy !== '' || targets.length === 0}>{busy === 'check' ? 'Checking' : `Check ${targets.length} ${targets.length === 1 ? 'organisation' : 'organisations'}`}</button>
        {progress && <p role="status" className="text-label leading-4 text-muted">{progress.done} of {progress.total} checked{progress.failed.length ? `, ${progress.failed.length} failed` : ''}.</p>}
      </div>
      {error && <p role="alert" className="text-label leading-4 text-warning">{error}</p>}
      {names.length > 0 && (stale || !resolutions) && <p className="text-label leading-4 text-muted">{names.length} {names.length === 1 ? 'name needs' : 'names need'} looking up before they can be checked.</p>}
      {resolutions && !stale && <ul aria-label="Name lookups" className="space-y-1 text-label leading-4">{resolutions.map((r) => <li key={r.entry} className="flex flex-wrap items-center gap-2">
        <span className="text-fg-secondary">{r.entry}</span>
        {r.chosen && normalisedName(r.chosen.name) === normalisedName(r.entry) && r.candidates.filter((c) => normalisedName(c.name) === normalisedName(r.entry)).length === 1
          ? <span className="text-muted">→ {r.chosen.name}, EIN {r.chosen.ein}{[r.chosen.city, r.chosen.state].filter(Boolean).length ? ` (${[r.chosen.city, r.chosen.state].filter(Boolean).join(', ')})` : ''}</span>
          : r.candidates.length === 0 ? <span className="text-warning">no match in ProPublica Nonprofit Explorer</span>
          : <select aria-label={`Match for ${r.entry}`} className="analysis-input" value={r.chosen?.ein ?? ''} onChange={(e) => choose(r.entry, e.target.value)}><option value="">Choose the organisation, or skip</option>{r.candidates.map((c) => <option key={c.ein} value={c.ein}>{c.name}{[c.city, c.state].filter(Boolean).length ? ` (${[c.city, c.state].filter(Boolean).join(', ')})` : ''} · EIN {c.ein}</option>)}</select>}
      </li>)}</ul>}
      {resolutions && !stale && unresolved > 0 && <p className="text-label leading-4 text-muted">{unresolved} {unresolved === 1 ? 'name is' : 'names are'} not matched and will be skipped.</p>}
      {progress && progress.failed.length > 0 && <ul role="alert" className="list-disc pl-5 text-label leading-4 text-warning">{progress.failed.map((line) => <li key={line}>{line}</li>)}</ul>}
    </form>
  </details>;
}

function Portfolio({ rows, onOpen }: { rows: PortfolioRow[]; onOpen: (row: PortfolioRow) => void }) {
  const [message, setMessage] = useState('');
  if (!rows.length) return null;
  const exportCsv = async () => {
    try {
      const outputPath = await save({ defaultPath: 'organisations-checked.csv', filters: [{ name: 'CSV', extensions: ['csv'] }] });
      if (!outputPath) return;
      await invokeCommand('export_organisation_portfolio_csv', { outputPath, content: portfolioCsv(rows) });
      setMessage(`Written to ${outputPath}.`);
    } catch (reason) { setMessage(formatNativeError(reason)); }
  };
  return <section aria-label="Checked organisations" className="space-y-2">
    <div className="flex flex-wrap items-center justify-between gap-2"><h3 className="text-sm font-semibold text-fg">Checked organisations</h3><button type="button" className="analysis-button" onClick={() => { void exportCsv(); }}>Export CSV</button></div>
    {message && <p role="status" className="text-label leading-4 text-muted">{message}</p>}
    <Table caption="Each organisation's latest read return, its single audits and the payments your tables record, as stated. Select a name to open its record."
      head={['Organisation', 'Latest return', 'Total revenue', 'Government grants', 'Diversion of assets (Part VI 5)', 'Former listed (Part VII 3)', 'Severance (Sch J 4a)', 'Sch L rows', 'Single audits', 'Audit finding rows', 'Payments in library', 'Not read']}
      rows={rows.map((row) => [
        <button type="button" className="text-left text-accent underline" onClick={() => onOpen(row)}>{row.name}</button>,
        row.latestPeriod ?? 'none read', money(row.totalRevenue), money(row.governmentGrants), row.significantDiversion, row.formerListed, row.severance,
        row.scheduleLRows === null ? 'not read' : String(row.scheduleLRows),
        row.audits === null ? 'no lookup' : `${row.audits}${row.latestAuditYear ? `, latest ${row.latestAuditYear}` : ''}`,
        row.auditFindingRows === null ? 'no lookup' : String(row.auditFindingRows),
        row.paymentRows === 0 ? 'none matched' : row.paymentsTotal === null ? `${row.paymentRows} rows, not summed` : `${dollars.format(row.paymentsTotal)} (${row.paymentRows} rows)`,
        row.unread.join(', ') || '',
      ])} />
  </section>;
}

function PaymentTables({ schema, mappings, onChange }: { schema: AnalyticalSchema | undefined; mappings: OrganisationPaymentMapping[]; onChange: (mappings: OrganisationPaymentMapping[]) => void }) {
  const [snapshotId, setSnapshotId] = useState('');
  const [vendorColumn, setVendorColumn] = useState('');
  const [amountColumn, setAmountColumn] = useState('');
  const [fiscalYearColumn, setFiscalYearColumn] = useState('');
  const [error, setError] = useState('');
  const gate = useLatestRequestGate();
  const datasets = schema?.datasets ?? [];
  const dataset = datasets.find((d) => d.snapshotId === snapshotId);
  const columns = dataset ? (schema?.columns ?? []).filter((c) => c.tableName === dataset.tableName).map((c) => c.columnName) : [];
  const run = (work: Promise<OrganisationPaymentMapping[]>) => {
    const request = gate.begin();
    work.then((saved) => { if (gate.isCurrent(request)) { onChange(saved); setError(''); } }, (reason) => { if (gate.isCurrent(request)) setError(formatNativeError(reason)); });
  };
  const nameOf = (id: string) => datasets.find((d) => d.snapshotId === id)?.displayName ?? id;
  return <details className="rounded-lg border border-line-strong p-3">
    <summary className="text-label leading-4 text-accent">Tables searched for payments ({mappings.length} payee {mappings.length === 1 ? 'table' : 'tables'})</summary>
    <p className="mt-2 text-label leading-4 text-muted">A table you imported in Analysis that lists payees and amounts, such as a contract or grant payment list, is searched by the organisation's name once you name its columns here. Contract, amendment and payment tables mapped under Spending and outcomes are searched as well.</p>
    {mappings.length > 0 && <ul className="mt-2 space-y-1 text-label leading-4 text-fg-secondary">{mappings.map((mapping) => <li key={mapping.snapshotId}>{nameOf(mapping.snapshotId)}: vendor {mapping.vendorColumn}, amount {mapping.amountColumn}{mapping.fiscalYearColumn ? `, fiscal year ${mapping.fiscalYearColumn}` : ''} <button type="button" className="text-accent underline" onClick={() => run(invokeCommand('remove_organisation_payment_mapping', { snapshotId: mapping.snapshotId }))}>Remove</button></li>)}</ul>}
    {error && <p role="alert" className="mt-2 text-label leading-4 text-danger">{error}</p>}
    {datasets.length === 0 ? <p className="mt-2 text-label leading-4 text-muted">No table has been imported in Analysis yet.</p>
      : <form className="mt-3 flex flex-wrap items-end gap-3" onSubmit={(event) => { event.preventDefault(); run(invokeCommand('save_organisation_payment_mapping', { mapping: { snapshotId, vendorColumn, amountColumn, fiscalYearColumn: fiscalYearColumn || null } })); }}>
        <label className="text-label leading-4 text-fg-secondary">Table<select className="analysis-input mt-1 block" value={snapshotId} onChange={(e) => { setSnapshotId(e.target.value); setVendorColumn(''); setAmountColumn(''); setFiscalYearColumn(''); }}><option value="">Choose a table</option>{datasets.map((d) => <option key={d.snapshotId} value={d.snapshotId}>{d.displayName}</option>)}</select></label>
        {([['Payee or vendor column', vendorColumn, setVendorColumn, true], ['Amount column', amountColumn, setAmountColumn, true], ['Fiscal year column (optional)', fiscalYearColumn, setFiscalYearColumn, false]] as const).map(([label, value, set, required]) =>
          <label key={label} className="text-label leading-4 text-fg-secondary">{label}<select className="analysis-input mt-1 block" value={value} onChange={(e) => set(e.target.value)} disabled={!dataset} required={required}><option value="">{required ? 'Choose a column' : 'None'}</option>{columns.map((c) => <option key={c} value={c}>{c}</option>)}</select></label>)}
        <button type="submit" className="analysis-button" disabled={!snapshotId || !vendorColumn || !amountColumn}>Search this table</button>
      </form>}
  </details>;
}

/** An organisation's public filings, each shown as the statement of the document it was read from.
 * An `initialQuery` (a vendor value handed over from another view) is looked up on mount. */
export default function OrganisationFilingsPanel({ initialQuery = '' }: { initialQuery?: string } = {}) {
  const [query, setQuery] = useState(initialQuery);
  const [auditKey, setAuditKey] = useState('');
  const [matches, setMatches] = useState<OrganisationMatch[] | undefined>(undefined);
  const [checked, setChecked] = useState<OrganisationCheckSummary[]>([]);
  const [portfolio, setPortfolio] = useState<PortfolioRow[]>([]);
  const [mappings, setMappings] = useState<OrganisationPaymentMapping[]>([]);
  const [schema, setSchema] = useState<AnalyticalSchema | undefined>(undefined);
  const [selected, setSelected] = useState<{ ein: string; name: string | null } | undefined>(undefined);
  const [filings, setFilings] = useState<OrganisationFilings | null | undefined>(undefined);
  const [busy, setBusy] = useState<'search' | 'load' | 'check' | ''>('');
  const [error, setError] = useState('');
  const searchGate = useLatestRequestGate();
  const recordGate = useLatestRequestGate();
  const listGate = useLatestRequestGate();

  // The portfolio is read from stored records only; a check that is still running is not in it
  // until it is saved and this runs again.
  const loadChecked = useCallback(() => {
    const request = listGate.begin();
    invokeCommand('list_organisation_checks', {}).then(async (list) => {
      if (!listGate.isCurrent(request)) return;
      setChecked(list);
      const records = await Promise.all(list.map((item) => invokeCommand('get_organisation_filings', { ein: item.ein })));
      if (!listGate.isCurrent(request)) return;
      setPortfolio(records.flatMap((record) => record ? [portfolioRow(record)] : []));
    }, () => undefined);
  }, [listGate]);
  useEffect(() => { loadChecked(); return listGate.invalidate; }, [listGate, loadChecked]);
  useEffect(() => {
    let current = true;
    void invokeCommand('list_organisation_payment_mappings', {}).then((saved) => { if (current) setMappings(saved); }, () => undefined);
    void invokeCommand('get_analytical_schema', {}).then((read) => { if (current) setSchema(read); }, () => undefined);
    return () => { current = false; };
  }, []);

  // One gate for both reads and checks of the record: whichever organisation was chosen last owns the view.
  const show = (organisation: { ein: string; name: string | null }, work: 'load' | 'check') => {
    const request = recordGate.begin();
    setSelected(organisation); setFilings(undefined); setError(''); setBusy(work);
    const ein = organisation.ein.replace('-', '');
    const pending = work === 'check'
      ? invokeCommand('check_organisation_filings', { ein, auditKey: auditKey.trim() || null })
      : invokeCommand('get_organisation_filings', { ein });
    pending.then((result) => {
      if (!recordGate.isCurrent(request)) return;
      setFilings(result);
      if (work === 'check') loadChecked();
    }, (reason) => { if (recordGate.isCurrent(request)) setError(formatNativeError(reason)); })
      .finally(() => { if (recordGate.isCurrent(request)) setBusy(''); });
  };

  const search = (text = query.trim()) => {
    if (!text) return;
    if (isEin(text)) { setMatches(undefined); show({ ein: text, name: null }, 'load'); return; }
    const request = searchGate.begin();
    setBusy('search'); setError('');
    invokeCommand('search_organisations', { query: text }).then((found) => { if (searchGate.isCurrent(request)) setMatches(found); },
      (reason) => { if (searchGate.isCurrent(request)) setError(formatNativeError(reason)); })
      .finally(() => { if (searchGate.isCurrent(request)) setBusy((current) => current === 'search' ? '' : current); });
  };

  const handedOver = initialQuery.trim();
  // Runs once per handed-over value; the host keys the panel by it.
  useEffect(() => { if (handedOver) search(handedOver); }, [handedOver]);
  return <section aria-label="Organisation filings" className="space-y-4">
    <div>
      <h3 className="text-sm font-semibold text-fg">Organisation filings</h3>
      <p className="text-label leading-4 text-muted">Name a tax-exempt organisation to read what its Form 990 returns state, its single audits, and the payments your library's tables record to it. Nothing here is a finding: a return's answer is the filer's statement and a payment is a payment.</p>
    </div>
    <form className="flex flex-wrap items-end gap-3" onSubmit={(event) => { event.preventDefault(); search(); }}>
      <label className="min-w-0 flex-1 text-label leading-4 text-fg-secondary">Organisation name or EIN
        <input className="analysis-input mt-1 w-full" value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Name, or EIN such as 33-0215585" /></label>
      <label className="text-label leading-4 text-fg-secondary">Federal Audit Clearinghouse key (optional)
        <input className="analysis-input mt-1 w-full" type="password" autoComplete="off" value={auditKey} onChange={(event) => setAuditKey(event.target.value)} aria-describedby="organisation-audit-key-note" /></label>
      <button type="submit" className="analysis-button" disabled={busy === 'search' || !query.trim()}>{busy === 'search' ? 'Searching' : 'Find'}</button>
    </form>
    <p id="organisation-audit-key-note" className="text-label leading-4 text-muted">A free api.data.gov key lets the check read single audits. It is sent only to api.fac.gov and is not saved.</p>
    {error && <p role="alert" className="text-sm text-danger">{error}</p>}
    {matches && <ul aria-label="Matching organisations" className="space-y-1">
      {matches.length === 0 && <li className="text-label leading-4 text-muted">ProPublica Nonprofit Explorer lists no organisation matching that name.</li>}
      {matches.map((match) => <li key={match.ein}><button type="button" className="text-left text-label leading-4 text-accent underline" onClick={() => show(match, 'load')}>{match.name}</button><span className="text-label leading-4 text-muted"> {[match.city, match.state].filter(Boolean).join(', ')} · EIN {match.ein}</span></li>)}
      {matches.length > 0 && <li className="text-label leading-4 text-muted">Name search by ProPublica Nonprofit Explorer.</li>}
    </ul>}
    <BatchCheck auditKey={auditKey} onDone={loadChecked} />
    <PaymentTables schema={schema} mappings={mappings} onChange={(saved) => { setMappings(saved); loadChecked(); if (selected && filings) show(selected, 'load'); }} />
    <Portfolio rows={portfolio} onOpen={(row) => show({ ein: row.ein, name: row.name }, 'load')} />
    {checked.length > 0 && portfolio.length === 0 && <div className="flex flex-wrap items-center gap-2 text-label leading-4"><span className="text-muted">Checked before:</span>{checked.map((item) => <button type="button" key={item.ein} className="rounded border border-control px-2 py-1 text-fg-secondary" onClick={() => show(item, 'load')}>{item.name ?? item.ein}</button>)}</div>}
    {selected && <div className="space-y-4 border-t border-line-strong pt-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div><h3 className="text-base font-semibold text-fg">{filings?.name ?? selected.name ?? `EIN ${selected.ein}`}</h3>
          <p className="text-label leading-4 text-muted">EIN {selected.ein}{filings ? ` · checked ${filings.checkedAt.slice(0, 10)}` : ''}</p></div>
        <button type="button" className="analysis-button" disabled={busy === 'check' || busy === 'load'} onClick={() => show(selected, 'check')}>{busy === 'check' ? 'Checking' : filings ? 'Check again' : 'Check filings'}</button>
      </div>
      {busy === 'check' && <p role="status" className="text-label leading-4 text-muted">Reading the IRS e-file index for each year your library does not hold yet (about 90 MB a year, kept for every organisation) and this organisation's returns. A first check can take several minutes.</p>}
      {filings === null && busy === '' && <p className="text-label leading-4 text-muted">Nothing is held for this organisation yet. Checking reads the IRS e-file index for each year from 2024 (about 90 MB a year, downloaded once for every organisation) and this organisation's returns.</p>}
      {filings && <Record filings={filings} />}
    </div>}
  </section>;
}
