import { useCallback, useEffect, useState, type ReactNode } from 'react';
import { invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import type {
  FilingExplanation, Form990Statement, OrganisationCheckSummary, OrganisationFilings, OrganisationMatch,
  OrganisationReturn, ScheduleLPart, SingleAuditHistory,
} from '../../types';

const dollars = new Intl.NumberFormat('en-US', { style: 'currency', currency: 'USD', maximumFractionDigits: 0 });
const NOT_STATED = 'not stated';
const money = (value: number | null) => value === null ? NOT_STATED : dollars.format(value);
const answer = (value: boolean | null) => value === null ? 'The return carries no answer.' : `The return answers "${value ? 'Yes' : 'No'}".`;
const period = (value: string) => value.length === 6 ? `${value.slice(0, 4)}-${value.slice(4)}` : value;
const isEin = (value: string) => /^\d{2}-?\d{7}$/.test(value.trim());

const SCHEDULE_L_PARTS: Record<ScheduleLPart, string> = {
  EXCESS_BENEFIT_TRANSACTION: 'Part I, excess benefit transactions',
  LOAN: 'Part II, loans to or from interested persons',
  GRANT_OR_ASSISTANCE: 'Part III, grants or assistance benefiting interested persons',
  BUSINESS_TRANSACTION: 'Part IV, business transactions involving interested persons',
};

function Table({ caption, head, rows }: { caption: string; head: string[]; rows: ReactNode[][] }) {
  return <div className="overflow-x-auto"><table className="w-full text-left text-label leading-4">
    <caption className="pb-1 text-left text-muted">{caption}</caption>
    <thead><tr>{head.map((label) => <th key={label} scope="col" className="pr-4 pb-1 font-normal text-muted">{label}</th>)}</tr></thead>
    <tbody>{rows.map((row, index) => <tr key={index} className="border-t border-line-strong align-top">{row.map((cell, at) => <td key={at} className="pr-4 py-1 text-fg-secondary">{cell}</td>)}</tr>)}</tbody>
  </table></div>;
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

function Record({ filings }: { filings: OrganisationFilings }) {
  return <div className="space-y-5">
    {filings.warnings.length > 0 && <ul role="alert" className="list-disc pl-5 text-label leading-4 text-warning">{filings.warnings.map((warning) => <li key={warning}>{warning}</li>)}</ul>}
    <section aria-label="Returns as filed" className="space-y-3">
      <h3 className="text-sm font-semibold text-fg">Returns as filed</h3>
      <p className="text-label leading-4 text-muted">Read from the IRS e-file archives for index years {filings.indexYears.map((check) => check.year).join(', ') || 'none'}. Each figure and answer is the organisation's own statement on its return. Earlier years' returns are not read; the summary below covers them.</p>
      {filings.returns.length ? filings.returns.map((listed) => <ReturnCard key={listed.objectId} listed={listed} />) : <p className="text-label leading-4 text-muted">The IRS indexes that were read list no e-filed return under this EIN.</p>}
    </section>
    <section aria-label="Summary by year" className="space-y-2">
      <h3 className="text-sm font-semibold text-fg">Summary by year</h3>
      {filings.summary ? <>
        {filings.summary.years.length > 0 ? <Table caption="Figures extracted from each year's return." head={['Tax year', 'Form', 'Total revenue', 'Total expenses', 'Total assets', 'Total liabilities', 'Compensation of current officers']}
          rows={filings.summary.years.map((year) => [String(year.taxYear), year.formType ?? NOT_STATED, money(year.totalRevenue), money(year.totalExpenses), money(year.totalAssetsEndOfYear), money(year.totalLiabilitiesEndOfYear), money(year.officerCompensation)])} />
          : <p className="text-label leading-4 text-muted">The source holds no extracted year for this organisation.</p>}
        <p className="text-label leading-4 text-muted">Source: <a className="text-accent underline" href={filings.summary.sourceUrl} target="_blank" rel="noreferrer">ProPublica Nonprofit Explorer</a>, from IRS extracts, retrieved {filings.summary.retrievedAt.slice(0, 10)}.</p>
      </> : <p className="text-label leading-4 text-muted">No summary was read.</p>}
    </section>
    <section aria-label="Single audits" className="space-y-2">
      <h3 className="text-sm font-semibold text-fg">Single audits of federal money</h3>
      <Audits history={filings.singleAudits} />
    </section>
    <section aria-label="Public money in your library" className="space-y-2">
      <h3 className="text-sm font-semibold text-fg">Public money in your library</h3>
      {filings.publicMoney.length > 0 && <Table caption="Rows of your mapped contract, amendment and payment tables whose vendor value contains this organisation's name. The tables carry no EIN, so confirm each vendor value is this organisation."
        head={['Record', 'Table', 'Vendor as written', 'Fiscal year', 'Rows', 'Amount']}
        rows={filings.publicMoney.map((line) => [line.role, line.sourceUrl ? <a className="text-accent underline" href={line.sourceUrl} target="_blank" rel="noreferrer">{line.datasetName}</a> : line.datasetName, line.vendorValue, line.fiscalYear ?? NOT_STATED, String(line.rows), line.amount ?? 'not summed: an amount could not be read'])} />}
      {filings.publicMoneyNotes.map((note) => <p key={note} className="text-label leading-4 text-muted">{note}</p>)}
    </section>
  </div>;
}

/** An organisation's public filings, each shown as the statement of the document it was read from. */
export default function OrganisationFilingsPanel() {
  const [query, setQuery] = useState('');
  const [auditKey, setAuditKey] = useState('');
  const [matches, setMatches] = useState<OrganisationMatch[] | undefined>(undefined);
  const [checked, setChecked] = useState<OrganisationCheckSummary[]>([]);
  const [selected, setSelected] = useState<{ ein: string; name: string | null } | undefined>(undefined);
  const [filings, setFilings] = useState<OrganisationFilings | null | undefined>(undefined);
  const [busy, setBusy] = useState<'search' | 'load' | 'check' | ''>('');
  const [error, setError] = useState('');
  const searchGate = useLatestRequestGate();
  const recordGate = useLatestRequestGate();
  const listGate = useLatestRequestGate();

  const loadChecked = useCallback(() => {
    const request = listGate.begin();
    invokeCommand('list_organisation_checks', {}).then((list) => { if (listGate.isCurrent(request)) setChecked(list); }, () => undefined);
  }, [listGate]);
  useEffect(() => { loadChecked(); return listGate.invalidate; }, [listGate, loadChecked]);

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
    }, (reason) => { if (recordGate.isCurrent(request)) setError(String(reason)); })
      .finally(() => { if (recordGate.isCurrent(request)) setBusy(''); });
  };

  const search = () => {
    const text = query.trim();
    if (!text) return;
    if (isEin(text)) { setMatches(undefined); show({ ein: text, name: null }, 'load'); return; }
    const request = searchGate.begin();
    setBusy('search'); setError('');
    invokeCommand('search_organisations', { query: text }).then((found) => { if (searchGate.isCurrent(request)) setMatches(found); },
      (reason) => { if (searchGate.isCurrent(request)) setError(String(reason)); })
      .finally(() => { if (searchGate.isCurrent(request)) setBusy((current) => current === 'search' ? '' : current); });
  };

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
    {checked.length > 0 && <div className="flex flex-wrap items-center gap-2 text-label leading-4"><span className="text-muted">Checked before:</span>{checked.map((item) => <button type="button" key={item.ein} className="rounded border border-control px-2 py-1 text-fg-secondary" onClick={() => show(item, 'load')}>{item.name ?? item.ein}</button>)}</div>}
    {selected && <div className="space-y-4 border-t border-line-strong pt-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div><h3 className="text-base font-semibold text-fg">{filings?.name ?? selected.name ?? `EIN ${selected.ein}`}</h3>
          <p className="text-label leading-4 text-muted">EIN {selected.ein}{filings ? ` · checked ${filings.checkedAt.slice(0, 10)}` : ''}</p></div>
        <button type="button" className="analysis-button" disabled={busy === 'check' || busy === 'load'} onClick={() => show(selected, 'check')}>{busy === 'check' ? 'Checking' : filings ? 'Check again' : 'Check filings'}</button>
      </div>
      {busy === 'check' && <p role="status" className="text-label leading-4 text-muted">Reading the IRS e-file index for each year not yet held (about 90 MB a year, downloaded and discarded) and this organisation's returns. This can take several minutes.</p>}
      {filings === null && busy === '' && <p className="text-label leading-4 text-muted">Nothing is held for this organisation yet. Checking downloads the IRS e-file index for each year from 2024 (about 90 MB a year), keeps only this organisation's rows, and reads its returns.</p>}
      {filings && <Record filings={filings} />}
    </div>}
  </section>;
}
