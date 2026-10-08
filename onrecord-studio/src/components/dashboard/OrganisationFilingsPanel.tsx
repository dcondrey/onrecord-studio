import { useCallback, useEffect, useState, type ReactNode } from 'react';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import type {
  AnalyticalSchema, HybridSearchResult, RecordGapRequestStatus, RequestSearchResult, FilingExplanation, Form990Statement, OrganisationCheckSummary, OrganisationFilings, OrganisationMatch,
  OrganisationPaymentMapping, OrganisationReturn, OrganisationSummary, OrganisationSummaryYear, PublicMoneyLine, ScheduleLPart, SingleAuditHistory,
} from '../../types';
import { save } from '@tauri-apps/plugin-dialog';
import { portfolioCsv, portfolioRow, type PortfolioRow } from '../../utils/organisationPortfolio';
import { isEin, normalisedName, parseOrganisationList } from '../../utils/organisationNames';
import { contractFileRequest } from '../../utils/organisationRequests';
import { peopleAcrossReturns, yearChange } from '../../utils/organisationPeople';
import { guidanceForSubject } from '../../utils/sourceGuidance';
import { useAppStore } from '../../store/useAppStore';
import Button from '../ui/Button';
import InlineError from '../ui/InlineError';
import Time from '../ui/Time';
import { Field, Input, Select, Textarea } from '../ui/Field';
import BarTable, { type BarTableRow } from '../charts/figures/BarTable';
import SeriesSmallMultiples, { type SeriesLine } from '../charts/figures/SeriesSmallMultiples';
import ChartFrame from '../charts/frame/ChartFrame';
import { TICK_CLASS } from '../charts/frame/Axis';
import { AbsentSlot } from '../charts/marks/shared';
import { VIZ } from '../charts/core/claim';
import { formatCurrencyString, formatValue } from '../charts/core/format';
import { absent, fromNullable, isValue, measured, type Measured } from '../charts/core/nulls';
import { scaleBand } from '../charts/core/scale';

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
  return <div><p id={id} className="pb-1 text-body text-muted">{caption}</p><div className="overflow-x-auto"><table aria-labelledby={id} className="w-full text-left text-body">
    <thead><tr>{head.map((label) => <th key={label} scope="col" className="pr-4 pb-1 font-normal text-muted">{label}</th>)}</tr></thead>
    <tbody>{rows.map((row, index) => <tr key={index} className="border-t border-line-strong align-top">{row.map((cell, at) => <td key={at} className="pr-4 py-1 text-fg-secondary">{cell}</td>)}</tr>)}</tbody>
  </table></div></div>;
}

function Explanations({ items }: { items: FilingExplanation[] }) {
  if (!items.length) return null;
  return <div className="space-y-2">{items.map((item, index) => <blockquote key={index} className="border-l-2 border-line-strong pl-3 text-body text-fg-secondary">
    <p className="whitespace-pre-wrap break-words">{item.text}</p>
    <footer className="mt-1 text-muted">The filer's explanation{item.lineReference ? `, ${item.lineReference}` : ''}</footer>
  </blockquote>)}</div>;
}

function Statement({ statement }: { statement: Form990Statement }) {
  const listed = statement.people.filter((person) => person.former || (person.reportableCompensation ?? 0) > 0 || (person.relatedOrganisationCompensation ?? 0) > 0 || (person.otherCompensation ?? 0) > 0);
  const unpaid = statement.people.length - listed.length;
  return <div className="mt-3 space-y-4">
    <dl className="grid grid-cols-2 gap-x-4 gap-y-1 text-body sm:grid-cols-3">
      {([['Total revenue', statement.totalRevenue], ['Total expenses', statement.totalExpenses], ['Total assets, end of year', statement.totalAssetsEndOfYear], ['Total liabilities, end of year', statement.totalLiabilitiesEndOfYear], ['Net assets, end of year', statement.netAssetsEndOfYear], ['Government grants', statement.governmentGrants], ['Reportable compensation, Part VII total', statement.totalReportableCompensation]] as const)
        .map(([label, value]) => <div key={label}><dt className="text-muted">{label}</dt><dd className="text-fg">{money(value)}</dd></div>)}
    </dl>
    <div className="text-body text-fg-secondary">
      <p><span className="text-muted">Part VI line 5, whether the organization became aware of a significant diversion of its assets during the year:</span> {answer(statement.significantDiversionReported)}</p>
      <p><span className="text-muted">Part VII line 3, whether a former officer, director, trustee, key employee or highest compensated employee is listed:</span> {answer(statement.formerListed)}</p>
      {statement.amended === true && <p>The return is marked as amended.</p>}
    </div>
    {listed.length > 0 && <Table caption={`Part VII Section A: people the return lists as former or with compensation.${unpaid > 0 ? ` ${unpaid} more are listed with none.` : ''}`}
      head={['Name', 'Title', 'Position boxes checked', 'From the organization', 'From related organizations', 'Other compensation']}
      rows={listed.map((person) => [person.name ?? NOT_STATED, person.title ?? NOT_STATED, person.positions.join(', ') || 'none', money(person.reportableCompensation), money(person.relatedOrganisationCompensation), money(person.otherCompensation)])} />}
    {statement.scheduleJAttached ? <div className="space-y-2">
      <p className="text-body text-fg-secondary"><span className="text-muted">Schedule J Part I line 4a, whether a listed person received a severance or change-of-control payment:</span> {answer(statement.severancePaymentReported)}</p>
      {statement.scheduleJPeople.length > 0 && <Table caption="Schedule J Part II: compensation from the filing organization, as the return reports it."
        head={['Name', 'Title', 'Base', 'Bonus and incentive', 'Other reportable', 'Deferred', 'Nontaxable benefits', 'Total', 'Total from related organizations']}
        rows={statement.scheduleJPeople.map((person) => [person.name ?? NOT_STATED, person.title ?? NOT_STATED, money(person.filingOrganisation.base), money(person.filingOrganisation.bonus), money(person.filingOrganisation.other), money(person.filingOrganisation.deferred), money(person.filingOrganisation.nontaxableBenefits), money(person.filingOrganisation.total), money(person.relatedOrganisations.total)])} />}
      <Explanations items={statement.scheduleJExplanations} />
    </div> : <p className="text-body text-muted">No Schedule J is attached to this return, so it carries no severance answer.</p>}
    {statement.scheduleLAttached ? <div className="space-y-2">
      {statement.scheduleLTransactions.length > 0 ? <Table caption="Schedule L: transactions with interested persons, as the return reports them."
        head={['Part of the schedule', 'Name', 'Relationship', 'Description', 'Amount', 'Balance due', 'Other columns']}
        rows={statement.scheduleLTransactions.map((row) => [SCHEDULE_L_PARTS[row.part], row.name ?? NOT_STATED, row.relationship ?? NOT_STATED, row.description ?? NOT_STATED, money(row.amount), row.part === 'LOAN' ? money(row.balanceDue) : '', row.lines.map((line) => `${line.label}: ${line.value}`).join('; ')])} />
        : <p className="text-body text-muted">A Schedule L is attached and lists no transaction rows.</p>}
      <Explanations items={statement.scheduleLExplanations} />
    </div> : <p className="text-body text-muted">No Schedule L is attached to this return.</p>}
    {statement.scheduleOExplanations.length > 0 && <details open={statement.significantDiversionReported === true}>
      <summary className="text-body text-accent">Schedule O: the filer's explanations ({statement.scheduleOExplanations.length}){statement.significantDiversionReported === true ? '. The form requires one here for the Yes on Part VI line 5.' : ''}</summary>
      <div className="mt-2"><Explanations items={statement.scheduleOExplanations} /></div>
    </details>}
  </div>;
}

function ReturnCard({ listed }: { listed: OrganisationReturn }) {
  return <article className="rounded-lg border border-line-strong p-3" aria-label={`Form ${listed.returnType}, tax period ${period(listed.taxPeriod)}`}>
    <h4 className="text-body font-medium text-fg">Form {listed.returnType}, tax period ending {period(listed.taxPeriod)}</h4>
    <p className="text-body text-muted">Filed as {listed.taxpayerName}. IRS e-file index {listed.indexYear}, return {listed.objectId}.</p>
    {listed.statement ? <Statement statement={listed.statement} /> : <p className="mt-2 text-body text-warning">Not read: {listed.unreadReason ?? 'no reason recorded.'}</p>}
    {listed.archiveUrl && <details className="mt-3 text-body text-muted"><summary>Where this return was read</summary>
      <p className="break-all">Archive: <a className="text-accent underline" href={listed.archiveUrl} target="_blank" rel="noreferrer">{listed.archiveUrl}</a></p>
      <p className="break-all">Member: {listed.memberName}</p>
      {listed.xmlSha256 && <p className="break-all">SHA-256 of the return as read: {listed.xmlSha256}{listed.retrievedAt ? `, retrieved ${listed.retrievedAt.slice(0, 10)}` : ''}</p>}
    </details>}
  </article>;
}

function Audits({ history }: { history: SingleAuditHistory | null }) {
  if (!history) return <p className="text-body text-muted">No single-audit lookup has run. Enter a Federal Audit Clearinghouse key above and check again.</p>;
  if (!history.audits.length) return <p className="text-body text-muted">The Federal Audit Clearinghouse lists no single audit under this EIN (retrieved {history.retrievedAt.slice(0, 10)}).</p>;
  return <div className="space-y-2">
    {history.audits.map((audit) => <details key={audit.reportId} className="rounded-lg border border-line-strong p-3 text-body">
      <summary className="text-fg">Audit year {audit.auditYear ?? NOT_STATED}: federal awards expended {money(audit.federalAmountExpended)}; {audit.findings.length} finding {audit.findings.length === 1 ? 'row' : 'rows'} recorded</summary>
      <p className="mt-2 text-muted">Report {audit.reportId}{audit.fiscalYearEnd ? `, fiscal year ending ${audit.fiscalYearEnd}` : ''}{audit.auditor ? `, auditor ${audit.auditor}` : ''}{audit.acceptedDate ? `, accepted ${audit.acceptedDate}` : ''}.</p>
      <ul className="mt-2 list-disc pl-5 text-fg-secondary">{audit.lines.map((line) => <li key={line.label}>{line.label}: {line.value}</li>)}</ul>
      {audit.findings.length > 0 && <Table caption="Findings as the audit submission records them." head={['Reference', 'Compliance requirement', 'Recorded answers']}
        rows={audit.findings.map((finding) => [finding.referenceNumber ?? NOT_STATED, finding.complianceRequirement ?? NOT_STATED, finding.lines.map((line) => `${line.label}: ${line.value}`).join('; ')])} />}
    </details>)}
    <p className="text-body text-muted">Source: <a className="text-accent underline" href={history.sourceUrl} target="_blank" rel="noreferrer">Federal Audit Clearinghouse</a>, retrieved {history.retrievedAt.slice(0, 10)}. An audit finding is the auditor's statement about the audited year.</p>
  </div>;
}

type TrackedState = { status: 'reading' } | { status: 'ready'; value: RecordGapRequestStatus | undefined } | { status: 'failed'; message: string };

/** What the library already holds about the organisation, and the request that would fill what it
 * does not: the same casework path the record gaps use. */
function Library({ filings }: { filings: OrganisationFilings }) {
  const portals = useAppStore((state) => state.portals);
  const [documents, setDocuments] = useState<HybridSearchResult[] | undefined>(undefined);
  const [requests, setRequests] = useState<RequestSearchResult[] | undefined>(undefined);
  const [notes, setNotes] = useState<string[]>([]);
  const [custodianKey, setCustodianKey] = useState('');
  const [tracked, setTracked] = useState<TrackedState>({ status: 'reading' });
  const [message, setMessage] = useState('');
  // Separate gates: the name search, the tracked-status read and the actions resolve independently,
  // and one superseding another would leave its section unread forever.
  const searchGate = useLatestRequestGate();
  const trackedGate = useLatestRequestGate();
  const actionGate = useLatestRequestGate();
  const name = filings.name ?? '';
  const custodian = portals.find((portal) => portal.registration.portalKey === custodianKey);
  const input = custodian ? contractFileRequest(filings, custodian.registration.displayName, custodian.registration.portalKey) : undefined;
  useEffect(() => {
    if (!name) return;
    const request = searchGate.begin();
    const found: string[] = [];
    void Promise.all([
      invokeCommand('hybrid_search', { query: name, limit: 10 }).then((hits) => Array.isArray(hits) ? hits : [], (reason) => { found.push(`Document search: ${formatNativeError(reason)}`); return []; }),
      invokeCommand('query_audit_records_page', { searchTerm: name, pageSize: 10 }).then((page) => Array.isArray(page?.records) ? page.records : [], (reason) => { found.push(`Request search: ${formatNativeError(reason)}`); return []; }),
    ]).then(([hits, held]) => { if (!searchGate.isCurrent(request)) return; setDocuments(hits); setRequests(held); setNotes(found); });
    return searchGate.invalidate;
  }, [searchGate, name]);
  const gapKey = input?.gapKey;
  const readTracked = useCallback(() => {
    if (!gapKey) return;
    const request = trackedGate.begin();
    setTracked({ status: 'reading' });
    invokeCommand('get_record_gap_requests', { gapKeys: [gapKey] }).then(
      (statuses) => { if (trackedGate.isCurrent(request)) setTracked({ status: 'ready', value: Array.isArray(statuses) ? statuses.find((s) => s.gapKey === gapKey) : undefined }); },
      (reason) => { if (trackedGate.isCurrent(request)) setTracked({ status: 'failed', message: formatNativeError(reason) }); },
    );
  }, [gapKey, trackedGate]);
  useEffect(() => { readTracked(); return trackedGate.invalidate; }, [readTracked, trackedGate]);
  const track = () => {
    if (!input) return;
    const request = actionGate.begin();
    invokeCommand('request_record_gap', { input }).then(() => {
      if (!actionGate.isCurrent(request)) return;
      setMessage('Tracked. Record the submission date in casework once you have filed it.');
      readTracked();
    }, (reason) => { if (actionGate.isCurrent(request)) setMessage(formatNativeError(reason)); });
  };
  const collect = () => {
    if (!custodian || !name) return;
    const request = actionGate.begin();
    invokeCommand('start_data_source', { url: custodian.registration.originUrl, guidance: guidanceForSubject(name) })
      .then(() => { if (actionGate.isCurrent(request)) setMessage(`Collection started from ${custodian.registration.displayName} for "${name}"; the source is listed in the sidebar. The lists above are read again when you reopen this organization.`); }, (reason) => { if (actionGate.isCurrent(request)) setMessage(formatNativeError(reason)); });
  };
  const date = (value: string | null) => value ? value.slice(0, 10) : '';
  const trackedRequest = tracked.status === 'ready' ? tracked.value : undefined;
  return <section aria-label="In your library" className="space-y-3">
    <h3 className="text-heading text-fg">In your library</h3>
    {notes.map((note) => <p key={note} className="text-body text-warning">{note}</p>)}
    {name && documents === undefined && <p role="status" className="text-body text-muted">Searching your library for this name…</p>}
    <div className="grid gap-4 sm:grid-cols-2">
      <div>
        <h4 className="text-body text-muted">Documents that mention the name{documents ? ` (${documents.length}${documents.length === 10 ? ', first 10' : ''})` : ''}</h4>
        {documents?.length === 0 && <p className="text-body text-muted">None of the collected documents mentions this name.</p>}
        <ul className="mt-1 space-y-2">{(documents ?? []).map((hit) => <li key={hit.stableId} className="text-body"><span className="text-fg">{hit.title || hit.fileName}</span><span className="text-muted"> · {hit.agencyName}{hit.publicationDate ? ` · ${date(hit.publicationDate)}` : ''}</span>{hit.excerpt && <blockquote className="mt-1 border-l-2 border-line-strong pl-2 text-fg-secondary">{hit.excerpt}</blockquote>}</li>)}</ul>
      </div>
      <div>
        <h4 className="text-body text-muted">Held requests that mention the name{requests ? ` (${requests.length}${requests.length === 10 ? ', first 10' : ''})` : ''}</h4>
        {requests?.length === 0 && <p className="text-body text-muted">No held request mentions this name.</p>}
        <ul className="mt-1 space-y-1">{(requests ?? []).map((held) => <li key={`${held.portalKey}:${held.requestId}`} className="text-body"><span className="text-fg">{held.title}</span><span className="text-muted"> · {held.agencyName} · {held.status} · {date(held.createdAt)}</span></li>)}</ul>
      </div>
    </div>
    <div className="rounded-lg border border-line-strong p-3 space-y-2">
      <h4 className="text-body text-fg">The contract file</h4>
      <p className="text-body text-muted">Filings and payment rows do not show what an agency contracted for, paid against, or found in monitoring. Choose the agency, then either collect what its request portal already released under this name, or track a drafted request for its contract file; you review and file it yourself.</p>
      <Select label="Agency" className="w-64" value={custodianKey} onChange={(event) => { actionGate.invalidate(); setCustodianKey(event.target.value); setMessage(''); }}><option value="">Choose a saved source</option>{portals.map((portal) => <option key={portal.registration.portalKey} value={portal.registration.portalKey}>{portal.registration.displayName}</option>)}</Select>
      {portals.length === 0 && <p className="text-body text-muted">No source is saved yet; add the agency's request portal in the sidebar first.</p>}
      {custodian && name && <Button size="sm" onClick={collect}>Search {custodian.registration.displayName} for this name</Button>}
      {input && <details className="text-body"><summary className="text-accent">Drafted request</summary><pre className="mt-2 whitespace-pre-wrap text-fg-secondary">{input.draft}</pre></details>}
      {input && (tracked.status === 'reading'
        ? <p role="status" className="text-body text-muted">Checking whether this request is already tracked…</p>
        : tracked.status === 'failed'
          ? <p role="alert" className="text-body text-warning">Whether this request is already tracked could not be read: {tracked.message}</p>
          : trackedRequest
            ? <p className="text-body text-fg-secondary">Tracked as a casework request{trackedRequest.submittedAt ? `, submitted ${date(trackedRequest.submittedAt)}` : '; no submission date is recorded yet'}. <button type="button" className="text-accent underline" onClick={() => useAppStore.getState().openCasework(trackedRequest.reportId)}>Open casework</button></p>
            : <Button size="sm" onClick={track}>Track this request</Button>)}
      {message && <p role="status" className="text-body text-muted">{message}</p>}
    </div>
  </section>;
}

const signed = (value: number | null) => value === null ? '' : `${value > 0 ? '+' : value < 0 ? '−' : ''}${dollars.format(Math.abs(value))}`;

function People({ filings }: { filings: OrganisationFilings }) {
  const people = peopleAcrossReturns(filings);
  if (people.length === 0) return null;
  const periods = [...new Set(people.flatMap((p) => p.years.map((y) => y.period)))].sort().reverse();
  return <section aria-label="People across returns" className="space-y-2">
    <h3 className="text-heading text-fg">People across the read returns</h3>
    <Table caption="Everyone Part VII or Schedule J lists on any read return, with reportable compensation from the organization per return (Schedule J total in brackets where listed). An empty cell is a return that does not list the person; &quot;former&quot; is the box the return checked."
      head={['Name', 'Title (latest)', ...periods]}
      rows={people.map((p) => [p.name, p.years[0].title ?? NOT_STATED, ...periods.map((period) => {
        const year = p.years.find((y) => y.period === period);
        if (!year) return '';
        const comp = year.reportable === null ? (year.scheduleJTotal === null ? NOT_STATED : `[${dollars.format(year.scheduleJTotal)}]`) : `${dollars.format(year.reportable)}${year.scheduleJTotal !== null ? ` [${dollars.format(year.scheduleJTotal)}]` : ''}`;
        return `${comp}${year.former ? ', former' : ''}`;
      })])} />
  </section>;
}

const FILING_SOURCES = ['organisation_filings', 'organisation_return_xml'];
const usd = (value: number) => formatValue(value, { kind: 'currency', compact: true });
const nullable = (value: number | null, reason: string): Measured => fromNullable(value, 'not-recorded', reason);

/**
 * Revenue and expenses, government grants, and assets and liabilities by tax year: three panels on
 * one tax-year axis. A figure the return does not state breaks its line. Grants come only from a
 * return read here whose tax period equals the summary year's, never from a derived year.
 */
function FilerFinances({ filings, summary }: { filings: OrganisationFilings; summary: OrganisationSummary }) {
  const years = [...summary.years].sort((a, b) => a.taxYear - b.taxYear);
  const read = filings.returns.filter((listed) => listed.statement);
  const grantFor = (year: OrganisationSummaryYear): Measured => {
    const match = year.taxPeriod === null ? undefined : read.find((listed) => listed.taxPeriod === year.taxPeriod);
    if (!match?.statement) return absent('not-covered', 'No return for this tax period was read here');
    return nullable(match.statement.governmentGrants, 'The return states no government grants figure');
  };
  const unmatched = read.filter((listed) => !years.some((year) => year.taxPeriod === listed.taxPeriod));
  const grants = years.map(grantFor);
  const value = (pick: (year: OrganisationSummaryYear) => number | null, what: string) => years.map((year) => nullable(pick(year), `The extract states no ${what} for this year`));
  const revenue = value((year) => year.totalRevenue, 'total revenue');
  const expenses = value((year) => year.totalExpenses, 'total expenses');
  const assets = value((year) => year.totalAssetsEndOfYear, 'total assets');
  const liabilities = value((year) => year.totalLiabilitiesEndOfYear, 'total liabilities');
  const filed = years.filter((year) => year.totalRevenue !== null);
  const both = [...years.keys()].reverse().find((i) => { const grant = grants[i]; return isValue(grant) && years[i].totalRevenue !== null && years[i].totalRevenue !== 0; });
  const notChecked = indexYearSpan(filings).filter((year) => !filings.indexYears.some((check) => check.year === year)).length;
  const parts = [
    filed.length ? `revenue ${usd(filed[0].totalRevenue ?? 0)}→${usd(filed[filed.length - 1].totalRevenue ?? 0)} (${filed.length} filed ${filed.length === 1 ? 'year' : 'years'})` : 'no revenue stated',
    ...(both === undefined ? [] : [`government grants were ${formatValue((grants[both] as { value: number }).value / (years[both].totalRevenue ?? 1), { kind: 'percent', digits: 0 })} of revenue in ${years[both].taxYear}`]),
    `${notChecked} index ${notChecked === 1 ? 'year' : 'years'} not checked`,
  ];
  const line = (key: string, label: string, role: SeriesLine['role'], values: Measured[]): SeriesLine => ({ key, label, role, values });
  const text = (m: Measured) => (isValue(m) ? money(m.value) : NOT_STATED);
  return (
    <>
      <SeriesSmallMultiples
        title="Filer finances by tax year"
        headline={`${filings.name ?? summary.name ?? 'This filer'}: ${parts.join('; ')}.`}
        sources={{ tables: FILING_SOURCES, retrievedAt: summary.retrievedAt, note: 'IRS Form 990 extracts via ProPublica Nonprofit Explorer; grants from returns read here' }}
        summary={`${years.length} tax years, ${years[0].taxYear} to ${years[years.length - 1].taxYear}. Lines break where the return states no figure.`}
        status="ready"
        slots={years.map((year) => ({ key: String(year.taxYear), label: String(year.taxYear), title: `Tax year ${year.taxYear}${year.formType ? `, Form ${year.formType}` : ''}` }))}
        panels={[
          { key: 'flows', title: 'Revenue and expenses', lines: [line('revenue', 'Revenue', 'primary', revenue), line('expenses', 'Expenses', 'comparison', expenses)], summary: `Revenue and expenses: ${years.map((year, i) => `${year.taxYear} ${text(revenue[i])} and ${text(expenses[i])}`).join('; ')}.` },
          { key: 'grants', title: 'Government grants', lines: [line('grants', 'Government grants', 'default', grants)], summary: `Government grants from returns read here: ${years.map((year, i) => `${year.taxYear} ${isValue(grants[i]) ? text(grants[i]) : 'not read'}`).join('; ')}.` },
          { key: 'balance', title: 'Assets and liabilities', lines: [line('assets', 'Assets', 'primary', assets), line('liabilities', 'Liabilities', 'comparison', liabilities)], summary: `Assets and liabilities: ${years.map((year, i) => `${year.taxYear} ${text(assets[i])} and ${text(liabilities[i])}`).join('; ')}.` },
        ]}
        kind="currency"
        yScale="per-panel"
        legend={[
          { key: 'revenue', label: 'Revenue · Assets', swatch: 'line', tone: 'ink' },
          { key: 'expenses', label: 'Expenses · Liabilities', swatch: 'dashed-line', tone: 'context' },
        ]}
        describe={(panelKey, slot) => {
          const year = years[slot];
          const rows = panelKey === 'flows'
            ? [{ key: 'revenue', value: text(revenue[slot]), label: 'revenue' }, { key: 'expenses', value: text(expenses[slot]), label: 'expenses' }]
            : panelKey === 'grants'
              ? [{ key: 'grants', value: isValue(grants[slot]) ? text(grants[slot]) : (grants[slot] as { reason: string }).reason, label: 'government grants' }]
              : [{ key: 'assets', value: text(assets[slot]), label: 'assets' }, { key: 'liabilities', value: text(liabilities[slot]), label: 'liabilities' }];
          return { rows, note: year.formType ? `Form ${year.formType}` : 'Form not stated' };
        }}
        table={{
          caption: "Figures extracted from each year's return. The change column is the difference from the year before, shown only when both years state the figure.",
          columns: [
            { key: 'year', label: 'Tax year' }, { key: 'form', label: 'Form' },
            { key: 'revenue', label: 'Total revenue', align: 'end' }, { key: 'revenueChange', label: 'Change', align: 'end' },
            { key: 'expenses', label: 'Total expenses', align: 'end' }, { key: 'expensesChange', label: 'Change', align: 'end' },
            { key: 'grants', label: 'Government grants', align: 'end' },
            { key: 'assets', label: 'Total assets', align: 'end' }, { key: 'liabilities', label: 'Total liabilities', align: 'end' },
            { key: 'officers', label: 'Compensation of current officers', align: 'end' },
          ],
          rows: summary.years.map((year, index) => ({
            year: String(year.taxYear),
            form: year.formType ?? NOT_STATED,
            revenue: money(year.totalRevenue),
            revenueChange: signed(yearChange(summary.years, index, (y) => y.totalRevenue)),
            expenses: money(year.totalExpenses),
            expensesChange: signed(yearChange(summary.years, index, (y) => y.totalExpenses)),
            grants: (() => { const grant = grantFor(year); return isValue(grant) ? money(grant.value) : grant; })(),
            assets: money(year.totalAssetsEndOfYear),
            liabilities: money(year.totalLiabilitiesEndOfYear),
            officers: money(year.officerCompensation),
          })),
        }}
        storageKey="filer-finances"
      />
      {unmatched.length > 0 && (
        <p className="text-body text-muted">
          {`Read returns whose tax period matches no summary year, so their grants are not plotted: ${unmatched.map((listed) => `Form ${listed.returnType}, period ${period(listed.taxPeriod)}${listed.statement?.governmentGrants != null ? `, government grants ${money(listed.statement.governmentGrants)}` : ''}`).join('; ')}.`}
        </p>
      )}
    </>
  );
}

/** Every index year from the earliest checked or summarised year to the latest, inclusive. */
function indexYearSpan(filings: OrganisationFilings): number[] {
  const known = [...filings.indexYears.map((check) => check.year), ...(filings.summary?.years ?? []).map((year) => year.taxYear)];
  if (!known.length) return [];
  const [first, last] = [Math.min(...known), Math.max(...known)];
  return last - first > 40 ? filings.indexYears.map((check) => check.year).sort((a, b) => a - b) : Array.from({ length: last - first + 1 }, (_, i) => first + i);
}

type CoverageState = 'read' | 'listed' | 'none' | 'unchecked';
const COVERAGE_TEXT: Record<CoverageState, string> = {
  read: 'return read',
  listed: 'listed, not read',
  none: 'checked: no e-filed return',
  unchecked: 'not checked',
};

/** IRS e-file index coverage: filled = a return read, ring = checked, dotted slot = not checked. */
function IndexCoverage({ filings }: { filings: OrganisationFilings }) {
  const span = indexYearSpan(filings);
  if (!span.length) return null;
  const stateOf = (year: number): CoverageState => {
    const check = filings.indexYears.find((candidate) => candidate.year === year);
    if (!check) return 'unchecked';
    if (filings.returns.some((listed) => listed.indexYear === year && listed.statement)) return 'read';
    return check.returnsListed > 0 ? 'listed' : 'none';
  };
  const states = span.map(stateOf);
  const count = (state: CoverageState) => states.filter((candidate) => candidate === state).length;
  return (
    <ChartFrame
      title="IRS e-file index coverage"
      headline={`${count('read')} index ${count('read') === 1 ? 'year' : 'years'} with a return read, ${count('none') + count('listed')} checked without one, ${count('unchecked')} not checked.`}
      sources={{ tables: ['organisation_filings'], note: 'An index year is the year the IRS published the return, not its tax year' }}
      summary={span.map((year, i) => `${year}: ${COVERAGE_TEXT[states[i]]}`).join('; ')}
      status="ready"
      minHeight={44}
      legend={[
        { key: 'read', label: COVERAGE_TEXT.read, swatch: 'dot' },
        { key: 'checked', label: 'checked, none read', swatch: 'ring' },
      ]}
      table={{
        caption: 'IRS e-file index years',
        columns: [{ key: 'year', label: 'Index year' }, { key: 'state', label: 'Coverage' }, { key: 'listed', label: 'Returns listed', align: 'end' }],
        rows: span.map((year, i) => {
          const check = filings.indexYears.find((candidate) => candidate.year === year);
          return { year: String(year), state: COVERAGE_TEXT[states[i]], listed: check ? check.returnsListed : absent('not-covered', 'This index year was not checked') };
        }),
      }}
    >
      {({ width }) => {
        const band = scaleBand(span.map(String), [0, width], { maxBand: 16 });
        return (
          <g>
            {span.map((year, i) => {
              const cx = band(String(year)) + band.bandwidth / 2;
              const state = states[i];
              return (
                <g key={year}>
                  <title>{`${year}: ${COVERAGE_TEXT[state]}`}</title>
                  {state === 'unchecked'
                    ? <AbsentSlot absence={absent('not-covered', 'This index year was not checked')} box={{ x: cx - 7, y: 3, width: 14, height: 14 }} text="" />
                    : <circle cx={cx} cy={10} r={state === 'read' ? 5 : 4.5} style={state === 'read' ? { fill: VIZ.ink } : { fill: 'none', stroke: VIZ.ink, strokeWidth: 1.5 }} />}
                  {(i === 0 || i === span.length - 1 || span.length <= 12 || year % 5 === 0) && (
                    <text x={cx} y={36} textAnchor="middle" className={TICK_CLASS}>{year}</text>
                  )}
                </g>
              );
            })}
          </g>
        );
      }}
    </ChartFrame>
  );
}

const PAYMENT_ROLES = new Set(['payment', 'payee table']);

/** A decimal string as integer cents, or null when it is not a plain amount with at most cents. */
function toCents(value: string): bigint | null {
  const match = /^(-?)(\d+)(?:\.(\d{1,2}))?$/.exec(value.trim());
  if (!match) return null;
  const cents = BigInt(match[2]) * 100n + BigInt((match[3] ?? '').padEnd(2, '0') || '0');
  return match[1] ? -cents : cents;
}

const centsText = (cents: bigint) => {
  const negative = cents < 0n;
  const abs = negative ? -cents : cents;
  return `${negative ? '-' : ''}${abs / 100n}.${(abs % 100n).toString().padStart(2, '0')}`;
};

/**
 * Payment rows by the city's fiscal year, on their own axis: a city fiscal year is not the filer's
 * tax period, so it never shares the tax-year axis. Amounts are summed in integer cents within one
 * table; contract and amendment rows are not payments and are not added.
 */
function PaymentBars({ lines }: { lines: PublicMoneyLine[] }) {
  const payments = lines.filter((line) => PAYMENT_ROLES.has(line.role));
  if (!payments.length) return null;
  const groups = new Map<string, { fiscalYear: string; dataset: string; cents: bigint; rows: number; unread: number }>();
  for (const line of payments) {
    if (line.fiscalYear === null) continue;
    const key = `${line.fiscalYear}\u0000${line.datasetName}`;
    const group = groups.get(key) ?? { fiscalYear: line.fiscalYear, dataset: line.datasetName, cents: 0n, rows: 0, unread: 0 };
    const cents = line.amount === null ? null : toCents(line.amount);
    if (cents === null) group.unread += line.rows;
    else group.cents += cents;
    group.rows += line.rows;
    groups.set(key, group);
  }
  const ordered = [...groups.values()].sort((a, b) => a.fiscalYear.localeCompare(b.fiscalYear) || a.dataset.localeCompare(b.dataset));
  const datasets = new Set(ordered.map((group) => group.dataset));
  const undated = payments.filter((line) => line.fiscalYear === null);
  const largest = ordered.reduce<typeof ordered[number] | null>((best, group) => (best === null || group.cents > best.cents ? group : best), null);
  const rows: BarTableRow[] = ordered.map((group) => {
    const amount = formatCurrencyString(centsText(group.cents));
    const label = datasets.size > 1 ? `${group.fiscalYear} · ${group.dataset}` : group.fiscalYear;
    return {
      key: `${group.fiscalYear}\u0000${group.dataset}`,
      name: label,
      label: `${label}: ${amount} over ${group.rows} rows${group.unread ? `; ${group.unread} rows with an unreadable amount not summed` : ''}`,
      header: <span className="viz-num text-fg">{label}</span>,
      cells: {
        amount: { value: measured(Number(centsText(group.cents))), text: amount, tooltip: { rows: [{ key: 'amount', value: amount, label: 'paid' }, { key: 'rows', value: String(group.rows), label: 'rows' }], note: group.dataset } },
        ...(ordered.some((other) => other.unread) ? { unread: { value: measured(group.unread), text: group.unread ? `${group.unread} not summed` : '0' } } : {}),
      },
    };
  });
  return (
    <BarTable
      title="Payments from mapped public datasets"
      headline={largest ? `Largest: ${formatCurrencyString(centsText(largest.cents))} in city fiscal year ${largest.fiscalYear}${datasets.size > 1 ? ` (${largest.dataset})` : ''}.` : 'No dated payment row has a readable amount.'}
      sources={{ tables: [...datasets, 'organisation_payment_mappings'], note: "City fiscal year; not aligned to the filer's tax period" }}
      summary={`${ordered.length} fiscal-year totals from ${datasets.size} ${datasets.size === 1 ? 'table' : 'tables'}.${undated.length ? ` ${undated.length} undated rows are listed below, not placed on the axis.` : ''}`}
      status={ordered.length ? 'ready' : 'empty'}
      emptyText="Every payment row is undated."
      rowHeader="City fiscal year"
      hatchLabel="Rows with an unreadable amount"
      columns={[
        { key: 'amount', label: 'Paid', scale: 'own-max', barWidth: 120 },
        ...(ordered.some((group) => group.unread) ? [{ key: 'unread', label: 'Rows not summed', note: 'amount unreadable', scale: 'own-max' as const, hatched: true, barWidth: 32 }] : []),
      ]}
      rows={rows}
      twin={{
        caption: 'Payments by city fiscal year',
        columns: [{ key: 'year', label: 'City fiscal year' }, { key: 'dataset', label: 'Table' }, { key: 'amount', label: 'Paid', align: 'end' }, { key: 'rows', label: 'Rows', align: 'end' }],
        rows: ordered.map((group) => ({ year: group.fiscalYear, dataset: group.dataset, amount: formatCurrencyString(centsText(group.cents)), rows: group.rows })),
      }}
      footer={undated.length > 0 && <p className="text-caption text-muted">{`Undated: ${undated.map((line) => `${line.datasetName}, ${line.rows} rows${line.amount === null ? '' : `, ${decimalMoney(line.amount)}`}`).join('; ')}.`}</p>}
    />
  );
}

function Record({ filings }: { filings: OrganisationFilings }) {
  return <div className="space-y-5">
    {filings.warnings.length > 0 && <ul role="alert" className="list-disc pl-5 text-body text-warning">{filings.warnings.map((warning) => <li key={warning}>{warning}</li>)}</ul>}
    <section aria-label="Returns as filed" className="space-y-3">
      <h3 className="text-heading text-fg">Returns as filed</h3>
      <p className="text-body text-muted">Read from the IRS e-file archives for the index years the coverage strip below marks as checked. Each figure and answer is the organization's own statement on its return. Earlier years' returns are not read; the summary below covers them.</p>
      {filings.returns.length ? filings.returns.map((listed) => <ReturnCard key={listed.objectId} listed={listed} />) : <p className="text-body text-muted">The IRS indexes that were read list no e-filed return under this EIN.</p>}
    </section>
    <People filings={filings} />
    <section aria-label="Summary by year" className="space-y-2">
      <h3 className="text-heading text-fg">Summary by year</h3>
      {filings.summary ? <>
        {filings.summary.years.length > 0 ? <FilerFinances filings={filings} summary={filings.summary} />
          : <p className="text-body text-muted">The source holds no extracted year for this organization.</p>}
        <p className="text-body text-muted">Source: <a className="text-accent underline" href={filings.summary.sourceUrl} target="_blank" rel="noreferrer">ProPublica Nonprofit Explorer</a>, from IRS extracts, retrieved {filings.summary.retrievedAt.slice(0, 10)}.</p>
      </> : <p className="text-body text-muted">No summary was read.</p>}
      <IndexCoverage filings={filings} />
    </section>
    <section aria-label="Single audits" className="space-y-2">
      <h3 className="text-heading text-fg">Single audits of federal money</h3>
      <Audits history={filings.singleAudits} />
    </section>
    <Library filings={filings} />
    <section aria-label="Public money in your library" className="space-y-2">
      <h3 className="text-heading text-fg">Public money in your library</h3>
      <PaymentBars lines={filings.publicMoney} />
      {filings.publicMoney.length > 0 && <Table caption="Rows of your mapped contract, amendment and payment tables whose vendor value contains this organization's name. The tables carry no EIN, so confirm each vendor value is this organization."
        head={['Record', 'Table', 'Vendor as written', 'Fiscal year', 'Rows', 'Amount']}
        rows={filings.publicMoney.map((line) => [line.role, line.sourceUrl ? <a className="text-accent underline" href={line.sourceUrl} target="_blank" rel="noreferrer">{line.datasetName}</a> : line.datasetName, line.vendorValue, line.fiscalYear ?? NOT_STATED, String(line.rows), line.amount === null ? 'not summed: an amount could not be read' : decimalMoney(line.amount)])} />}
      {filings.publicMoneyNotes.map((note) => <p key={note} className="text-body text-muted">{note}</p>)}
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
    <summary className="text-body text-accent">Check a list of organizations</summary>
    <form className="mt-2 space-y-2" onSubmit={(event) => { event.preventDefault(); void start(); }}>
      <label className="block text-body text-fg-secondary">Organization names or EINs, one per line
        <Textarea className="mt-1.5" rows={4} value={text} onChange={(event) => setText(event.target.value)} placeholder={'Alpha Project for the Homeless\n33-0492302'} /></label>
      <div className="flex flex-wrap items-center gap-3">
        <Button size="sm" disabled={busy !== ''} onClick={addPayees}>{busy === 'payees' ? 'Reading payees' : 'Add payees from your payment tables'}</Button>
        <Button size="sm" disabled={busy !== '' || names.length === 0} onClick={() => { void lookUp(); }}>{busy === 'lookup' ? 'Looking up' : `Look up ${names.length} ${names.length === 1 ? 'name' : 'names'}`}</Button>
        <Button type="submit" size="sm" disabled={busy !== '' || targets.length === 0}>{busy === 'check' ? 'Checking' : `Check ${targets.length} ${targets.length === 1 ? 'organization' : 'organizations'}`}</Button>
        {progress && <p role="status" className="text-body text-muted">{progress.done} of {progress.total} checked{progress.failed.length ? `, ${progress.failed.length} failed` : ''}.</p>}
      </div>
      {error && <p role="alert" className="text-body text-warning">{error}</p>}
      {names.length > 0 && (stale || !resolutions) && <p className="text-body text-muted">{names.length} {names.length === 1 ? 'name needs' : 'names need'} looking up before they can be checked.</p>}
      {resolutions && !stale && <ul aria-label="Name lookups" className="space-y-1 text-body">{resolutions.map((r) => <li key={r.entry} className="flex flex-wrap items-center gap-2">
        <span className="text-fg-secondary">{r.entry}</span>
        {r.chosen && normalisedName(r.chosen.name) === normalisedName(r.entry) && r.candidates.filter((c) => normalisedName(c.name) === normalisedName(r.entry)).length === 1
          ? <span className="text-muted">→ {r.chosen.name}, EIN {r.chosen.ein}{[r.chosen.city, r.chosen.state].filter(Boolean).length ? ` (${[r.chosen.city, r.chosen.state].filter(Boolean).join(', ')})` : ''}</span>
          : r.candidates.length === 0 ? <span className="text-warning">no match in ProPublica Nonprofit Explorer</span>
          : <Select aria-label={`Match for ${r.entry}`} size="sm" value={r.chosen?.ein ?? ''} onChange={(e) => choose(r.entry, e.target.value)}><option value="">Choose the organization, or skip</option>{r.candidates.map((c) => <option key={c.ein} value={c.ein}>{c.name}{[c.city, c.state].filter(Boolean).length ? ` (${[c.city, c.state].filter(Boolean).join(', ')})` : ''} · EIN {c.ein}</option>)}</Select>}
      </li>)}</ul>}
      {resolutions && !stale && unresolved > 0 && <p className="text-body text-muted">{unresolved} {unresolved === 1 ? 'name is' : 'names are'} not matched and will be skipped.</p>}
      {progress && progress.failed.length > 0 && <ul role="alert" className="list-disc pl-5 text-body text-warning">{progress.failed.map((line) => <li key={line}>{line}</li>)}</ul>}
    </form>
  </details>;
}

function Portfolio({ rows, onOpen }: { rows: PortfolioRow[]; onOpen: (row: PortfolioRow) => void }) {
  const [message, setMessage] = useState('');
  if (!rows.length) return null;
  const exportCsv = async () => {
    try {
      const outputPath = await save({ defaultPath: 'organizations-checked.csv', filters: [{ name: 'CSV', extensions: ['csv'] }] });
      if (!outputPath) return;
      await invokeCommand('export_organisation_portfolio_csv', { outputPath, content: portfolioCsv(rows) });
      setMessage(`Written to ${outputPath}.`);
    } catch (reason) { setMessage(formatNativeError(reason)); }
  };
  return <section aria-label="Checked organizations" className="space-y-2">
    <div className="flex flex-wrap items-center justify-between gap-2"><h3 className="text-heading text-fg">Checked organizations</h3><Button size="sm" onClick={() => { void exportCsv(); }}>Export CSV</Button></div>
    {message && <p role="status" className="text-body text-muted">{message}</p>}
    <Table caption="Each organization's latest read return, its single audits and the payments your tables record, as stated. Select a name to open its record."
      head={['Organization', 'Latest return', 'Total revenue', 'Government grants', 'Diversion of assets (Part VI 5)', 'Former listed (Part VII 3)', 'Severance (Sch J 4a)', 'Sch L rows', 'Single audits', 'Audit finding rows', 'Payments in library', 'Not read']}
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

function PaymentTables({ schema, loadError, mappings, onChange }: { schema: AnalyticalSchema | undefined; loadError: string; mappings: OrganisationPaymentMapping[]; onChange: (mappings: OrganisationPaymentMapping[]) => void }) {
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
    <summary className="text-body text-accent">Tables searched for payments ({mappings.length} payee {mappings.length === 1 ? 'table' : 'tables'})</summary>
    <p className="mt-2 text-body text-muted">A table you imported in Analysis that lists payees and amounts, such as a contract or grant payment list, is searched by the organization's name once you name its columns here. Contract, amendment and payment tables mapped under Spending are searched as well.</p>
    {mappings.length > 0 && <ul className="mt-2 space-y-1 text-body text-fg-secondary">{mappings.map((mapping) => <li key={mapping.snapshotId}>{nameOf(mapping.snapshotId)}: vendor {mapping.vendorColumn}, amount {mapping.amountColumn}{mapping.fiscalYearColumn ? `, fiscal year ${mapping.fiscalYearColumn}` : ''} <button type="button" className="text-accent underline" onClick={() => run(invokeCommand('remove_organisation_payment_mapping', { snapshotId: mapping.snapshotId }))}>Remove</button></li>)}</ul>}
    {error && <p role="alert" className="mt-2 text-body text-danger">{error}</p>}
    {loadError && <p role="alert" className="mt-2 text-body text-warning">{loadError}</p>}
    {schema === undefined ? (!loadError && <p role="status" className="mt-2 text-body text-muted">Reading imported tables…</p>)
      : datasets.length === 0 ? <p className="mt-2 text-body text-muted">No table has been imported in Analysis yet.</p>
      : <form className="mt-3 flex flex-wrap items-end gap-3" onSubmit={(event) => { event.preventDefault(); run(invokeCommand('save_organisation_payment_mapping', { mapping: { snapshotId, vendorColumn, amountColumn, fiscalYearColumn: fiscalYearColumn || null } })); }}>
        <Select label="Table" value={snapshotId} onChange={(e) => { setSnapshotId(e.target.value); setVendorColumn(''); setAmountColumn(''); setFiscalYearColumn(''); }}><option value="">Choose a table</option>{datasets.map((d) => <option key={d.snapshotId} value={d.snapshotId}>{d.displayName}</option>)}</Select>
        {([['Payee or vendor column', vendorColumn, setVendorColumn, true], ['Amount column', amountColumn, setAmountColumn, true], ['Fiscal year column (optional)', fiscalYearColumn, setFiscalYearColumn, false]] as const).map(([label, value, set, required]) =>
          <Select key={label} label={label} value={value} onChange={(e) => set(e.target.value)} disabled={!dataset} required={required}><option value="">{required ? 'Choose a column' : 'None'}</option>{columns.map((c) => <option key={c} value={c}>{c}</option>)}</Select>)}
        <Button type="submit" size="sm" disabled={!snapshotId || !vendorColumn || !amountColumn}>Search this table</Button>
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
  const [tablesError, setTablesError] = useState('');
  const [listError, setListError] = useState('');
  const [selected, setSelected] = useState<{ ein: string; name: string | null } | undefined>(undefined);
  const [filings, setFilings] = useState<OrganisationFilings | null | undefined>(undefined);
  const [busy, setBusy] = useState<'search' | 'load' | 'check' | ''>('');
  const [error, setError] = useState('');
  const searchGate = useLatestRequestGate();
  const recordGate = useLatestRequestGate();
  const listGate = useLatestRequestGate();

  // The portfolio is read from stored records only; a check that is still running is not in it
  // until it is saved and this runs again.
  const loadChecked = useCallback(async () => {
    const request = listGate.begin();
    try {
      const list = await invokeCommand('list_organisation_checks', {});
      if (!listGate.isCurrent(request)) return;
      setChecked(list);
      const records = await Promise.all(list.map((item) => invokeCommand('get_organisation_filings', { ein: item.ein })));
      if (!listGate.isCurrent(request)) return;
      setPortfolio(records.flatMap((record) => record ? [portfolioRow(record)] : []));
      setListError('');
    } catch (reason) {
      if (listGate.isCurrent(request)) setListError(formatNativeError(reason));
    }
  }, [listGate]);
  useEffect(() => { void loadChecked(); return listGate.invalidate; }, [listGate, loadChecked]);
  useEffect(() => {
    let current = true;
    void invokeCommand('list_organisation_payment_mappings', {}).then((saved) => { if (current) setMappings(saved); }, (reason) => { if (current) setTablesError(`Payee tables could not be read: ${formatNativeError(reason)}`); });
    void invokeCommand('get_analytical_schema', {}).then((read) => { if (current) setSchema(read); }, (reason) => { if (current) setTablesError(`Imported tables could not be read: ${formatNativeError(reason)}`); });
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
      if (work === 'check') void loadChecked();
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
  return <section aria-label="Organization filings" className="space-y-4">
    <p className="max-w-prose text-body text-muted">Name a tax-exempt organization to read what its Form 990 returns state, its single audits, and the payments your library's tables record to it. Nothing here is a finding: a return's answer is the filer's statement and a payment is a payment.</p>
    <form className="grid items-end gap-3 sm:grid-cols-[minmax(0,1fr)_minmax(0,18rem)_auto]" onSubmit={(event) => { event.preventDefault(); search(); }}>
      <Field label="Organization name or EIN">
        <Input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Name, or EIN such as 33-0215585" />
      </Field>
      <Field label="Federal Audit Clearinghouse key (optional)">
        <Input type="password" autoComplete="off" value={auditKey} onChange={(event) => setAuditKey(event.target.value)} aria-describedby="organisation-audit-key-note" />
      </Field>
      <Button type="submit" variant="primary" disabled={busy === 'search' || !query.trim()}>{busy === 'search' ? 'Searching…' : 'Find'}</Button>
    </form>
    <p id="organisation-audit-key-note" className="text-caption text-muted">A free api.data.gov key lets the check read single audits. It is sent only to api.fac.gov and is not saved.</p>
    {error && <InlineError summary="The organization search couldn’t complete." detail={error} />}
    {matches && <ul aria-label="Matching organizations" className="space-y-1 text-body">
      {matches.length === 0 && <li className="text-muted">ProPublica Nonprofit Explorer lists no organization matching that name.</li>}
      {matches.map((match) => <li key={match.ein}><Button variant="link" onClick={() => show(match, 'load')}>{match.name}</Button><span className="text-caption text-muted"> {[match.city, match.state].filter(Boolean).join(', ')} · EIN <span className="tabular-nums">{match.ein}</span></span></li>)}
      {matches.length > 0 && <li className="text-caption text-muted">Name search by ProPublica Nonprofit Explorer.</li>}
    </ul>}
    <BatchCheck auditKey={auditKey} onDone={() => void loadChecked()} />
    <PaymentTables schema={schema} loadError={tablesError} mappings={mappings} onChange={(saved) => { setMappings(saved); void loadChecked(); if (selected && filings) show(selected, 'load'); }} />
    {listError && <InlineError summary="Checked organizations couldn’t be read." detail={listError} />}
    <Portfolio rows={portfolio} onOpen={(row) => show({ ein: row.ein, name: row.name }, 'load')} />
    {checked.length > 0 && portfolio.length === 0 && <div className="flex flex-wrap items-center gap-2 text-body"><span className="text-muted">Checked before:</span>{checked.map((item) => <Button size="sm" key={item.ein} onClick={() => show(item, 'load')}>{item.name ?? item.ein}</Button>)}</div>}
    {selected && <div className="space-y-4 border-t border-line-strong pt-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div><h3 className="text-heading text-fg">{filings?.name ?? selected.name ?? `EIN ${selected.ein}`}</h3>
          <p className="text-caption tabular-nums text-muted">EIN {selected.ein}{filings && <> · checked <Time value={filings.checkedAt} /></>}</p></div>
        <Button disabled={busy === 'check' || busy === 'load'} onClick={() => show(selected, 'check')}>{busy === 'check' ? 'Checking…' : filings ? 'Check again' : 'Check filings'}</Button>
      </div>
      {busy === 'check' && <p role="status" className="text-body text-muted">Reading the IRS e-file index for each year your library does not hold yet (about 90 MB a year, kept for every organization) and this organization's returns. A first check can take several minutes.</p>}
      {filings === null && busy === '' && <p className="text-body text-muted">Nothing is held for this organization yet. Checking reads the IRS e-file index for each year from 2024 (about 90 MB a year, downloaded once for every organization) and this organization's returns.</p>}
      {filings && <Record filings={filings} />}
    </div>}
  </section>;
}
